#!/usr/bin/env python3
"""A clean-room H.264 decoder: the golden model for the H.264 decoder IP.

Written from Rec. ITU-T H.264 (V16, 06/2026) and nothing else: no other
decoder's source was read. Clause numbers in comments are that edition's.
Standard library only. The tables live in h264_tables.py and
h264_cabac_tables.py beside this file.

What it decodes: progressive (frame_mbs_only) 8-bit 4:2:0 streams with one
slice group: I, P and B slices, CAVLC and CABAC, the 4x4 and 8x8 transforms
with scaling matrices, Intra_4x4/8x8/16x16 and I_PCM, every partition and
direct mode (spatial and temporal), default, explicit and implicit weighted
prediction, the deblocking filter, and reference marking with MMCO and list
modification. That is Baseline, Main and High for progressive video.
It is checked byte for byte against ffmpeg's output on every stream in
testdata/ip/h264/ (tests/h264_model.rs).

Usage:
  h264_model.py decode IN.264 [-o OUT.yuv] [--md5]
  h264_model.py check IN.264 REF.md5          exit status 1 on any difference
  h264_model.py dump IN.264 OUTDIR [--frames N] [--name NAME] [--desc TEXT]
  h264_model.py stats IN.264                  which syntax features occur
  h264_model.py selftest
"""

import hashlib
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import h264_tables as T  # noqa: E402


class DecodeError(Exception):
    """The stream uses something this model does not implement, or is broken."""


def clip3(lo, hi, v):
    return lo if v < lo else hi if v > hi else v


def clip1(v):
    return 0 if v < 0 else 255 if v > 255 else v


def median(a, b, c):
    return a + b + c - min(a, b, c) - max(a, b, c)


# ---------------------------------------------------------------------------
# Bits (7.2, 9.1)

class VLC:
    """A prefix code as a lookup table indexed by the next `bits` bits."""

    def __init__(self, codes):
        self.bits = max(len(c) for c in codes)
        self.table = [None] * (1 << self.bits)
        for code, value in codes.items():
            n = len(code)
            base = int(code, 2) << (self.bits - n)
            for i in range(1 << (self.bits - n)):
                if self.table[base + i] is not None:
                    raise ValueError("not prefix-free: %s" % code)
                self.table[base + i] = (n, value)


class BitReader:
    def __init__(self, data):
        self.data = bytes(data) + b"\0" * 8
        self.pos = 0
        i = len(data) - 1
        while i >= 0 and data[i] == 0:
            i -= 1
        if i < 0:
            self.stop = 0
        else:
            b = data[i]
            self.stop = i * 8 + 7 - ((b & -b).bit_length() - 1)

    def u(self, n):
        if n == 0:
            return 0
        p = self.pos
        b = p >> 3
        e = (p + n + 7) >> 3
        v = int.from_bytes(self.data[b:e], "big")
        self.pos = p + n
        return (v >> ((e << 3) - p - n)) & ((1 << n) - 1)

    def peek(self, n):
        p = self.pos
        b = p >> 3
        e = (p + n + 7) >> 3
        v = int.from_bytes(self.data[b:e], "big")
        return (v >> ((e << 3) - p - n)) & ((1 << n) - 1)

    def u1(self):
        p = self.pos
        self.pos = p + 1
        return (self.data[p >> 3] >> (7 - (p & 7))) & 1

    def ue(self):
        v = self.peek(32)
        if v == 0:
            lz = 0
            while self.u1() == 0:
                lz += 1
                if lz > 64:
                    raise DecodeError("bad Exp-Golomb code")
            return (1 << lz) - 1 + self.u(lz)
        lz = 32 - v.bit_length()
        self.pos += lz + 1
        return (1 << lz) - 1 + self.u(lz)

    def se(self):
        k = self.ue()
        return (k + 1) >> 1 if k & 1 else -(k >> 1)

    def te(self, cmax):
        if cmax > 1:
            return self.ue()
        return 1 - self.u1()

    def vlc(self, t):
        n, v = t.table[self.peek(t.bits)]
        self.pos += n
        return v

    def more_rbsp_data(self):
        return self.pos < self.stop

    def byte_align(self):
        self.pos = (self.pos + 7) & ~7


COEFF_TOKEN_VLC = [VLC(t) for t in T.COEFF_TOKEN]
TOTAL_ZEROS_VLC = {k: VLC(v) for k, v in T.TOTAL_ZEROS_4X4.items()}
TOTAL_ZEROS_DC420_VLC = {k: VLC(v) for k, v in T.TOTAL_ZEROS_DC420.items()}
TOTAL_ZEROS_DC422_VLC = {k: VLC(v) for k, v in T.TOTAL_ZEROS_DC422.items()}
RUN_BEFORE_VLC = {k: VLC(v) for k, v in T.RUN_BEFORE.items()}


def nal_units(stream):
    """Splits an Annex B byte stream into NAL units (B.2)."""
    out = []
    i = 0
    n = len(stream)
    starts = []
    while True:
        j = stream.find(b"\x00\x00\x01", i)
        if j < 0:
            break
        starts.append(j + 3)
        i = j + 3
    for k, s in enumerate(starts):
        e = starts[k + 1] - 3 if k + 1 < len(starts) else n
        nal = stream[s:e]
        while nal and nal[-1] == 0:
            nal = nal[:-1]  # trailing_zero_8bits and the next start code's zero
        if nal:
            out.append(nal)
    return out


def unescape(nal):
    """Removes emulation_prevention_three_byte (7.3.1, 7.4.1)."""
    if b"\x00\x00\x03" not in nal:
        return nal
    out = bytearray()
    zeros = 0
    for b in nal:
        if zeros >= 2 and b == 3:
            zeros = 0
            continue
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
    return bytes(out)


# ---------------------------------------------------------------------------
# Parameter sets (7.3.2.1, 7.3.2.2)

FLAT4 = [16] * 16
FLAT8 = [16] * 64
# Table 7-3 and 7-4: default scaling lists, in zig-zag order.
DEFAULT_4X4_INTRA = [6, 13, 13, 20, 20, 20, 28, 28, 28, 28, 32, 32, 32, 37, 37, 42]
DEFAULT_4X4_INTER = [10, 14, 14, 20, 20, 20, 24, 24, 24, 24, 27, 27, 27, 30, 30, 34]
DEFAULT_8X8_INTRA = [
    6, 10, 10, 13, 11, 13, 16, 16, 16, 16, 18, 18, 18, 18, 18, 23,
    23, 23, 23, 23, 23, 25, 25, 25, 25, 25, 25, 25, 27, 27, 27, 27,
    27, 27, 27, 27, 29, 29, 29, 29, 29, 29, 29, 31, 31, 31, 31, 31,
    31, 33, 33, 33, 33, 33, 36, 36, 36, 36, 38, 38, 38, 40, 40, 42]
DEFAULT_8X8_INTER = [
    9, 13, 13, 15, 13, 15, 17, 17, 17, 17, 19, 19, 19, 19, 19, 21,
    21, 21, 21, 21, 21, 22, 22, 22, 22, 22, 22, 22, 24, 24, 24, 24,
    24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 27, 27, 27, 27, 27,
    27, 28, 28, 28, 28, 28, 30, 30, 30, 30, 32, 32, 32, 33, 33, 35]


class Obj:
    def __init__(self, **kw):
        self.__dict__.update(kw)


def scaling_list(br, size):
    """7.3.2.1.1.1. Returns (list, use_default)."""
    last = 8
    nxt = 8
    out = [0] * size
    use_default = False
    for j in range(size):
        if nxt != 0:
            delta = br.se()
            nxt = (last + delta + 256) % 256
            use_default = (j == 0 and nxt == 0)
        out[j] = last if nxt == 0 else nxt
        last = out[j]
    return out, use_default


def parse_scaling_lists(br, count, fallback):
    """Parses `count` scaling_list() flags/lists. `fallback` gives the
    fall-back rule (7.4.2.1.1 Table 7-2): a list of 12 lists to fall back to
    for i = 0 and 3 (4x4) and 6, 7 (8x8) and so on; returns 12 lists, 4x4
    ones first (6), then 8x8 ones (6)."""
    lists = [None] * 12
    for i in range(12):
        present = br.u1() if i < count else 0
        if present:
            lst, use_default = scaling_list(br, 16 if i < 6 else 64)
            if use_default:
                lst = default_list(i)
            lists[i] = lst
        else:
            # Fall-back rule: the first of each kind falls back to `fallback`,
            # later ones to the previous list of the same kind.
            if i in (0, 3, 6, 7):
                lists[i] = fallback[i]
            elif i < 6:
                lists[i] = lists[i - 1]
            else:
                lists[i] = lists[i - 2]
    return lists


def default_list(i):
    if i < 6:
        return DEFAULT_4X4_INTRA if i < 3 else DEFAULT_4X4_INTER
    return DEFAULT_8X8_INTRA if i % 2 == 0 else DEFAULT_8X8_INTER


DEFAULT_LISTS = [default_list(i) for i in range(12)]
FLAT_LISTS = [FLAT4] * 6 + [FLAT8] * 6


def parse_sps(br):
    s = Obj()
    s.profile_idc = br.u(8)
    s.constraint_flags = br.u(8)
    s.level_idc = br.u(8)
    s.sps_id = br.ue()
    s.chroma_format_idc = 1
    s.separate_colour_plane = 0
    s.bit_depth_luma = 8
    s.bit_depth_chroma = 8
    s.transform_bypass = 0
    s.scaling_lists = None
    if s.profile_idc in (100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135):
        s.chroma_format_idc = br.ue()
        if s.chroma_format_idc == 3:
            s.separate_colour_plane = br.u1()
        s.bit_depth_luma = 8 + br.ue()
        s.bit_depth_chroma = 8 + br.ue()
        s.transform_bypass = br.u1()
        if br.u1():
            s.scaling_lists = parse_scaling_lists(
                br, 8 if s.chroma_format_idc != 3 else 12, DEFAULT_LISTS)
    s.log2_max_frame_num = br.ue() + 4
    s.poc_type = br.ue()
    if s.poc_type == 0:
        s.log2_max_poc_lsb = br.ue() + 4
    elif s.poc_type == 1:
        s.delta_pic_order_always_zero = br.u1()
        s.offset_for_non_ref_pic = br.se()
        s.offset_for_top_to_bottom_field = br.se()
        n = br.ue()
        s.offset_for_ref_frame = [br.se() for _ in range(n)]
    s.max_num_ref_frames = br.ue()
    s.gaps_allowed = br.u1()
    s.width_mbs = br.ue() + 1
    s.height_map_units = br.ue() + 1
    s.frame_mbs_only = br.u1()
    if not s.frame_mbs_only:
        s.mb_adaptive_frame_field = br.u1()
    s.direct_8x8_inference = br.u1()
    s.crop = (0, 0, 0, 0)
    if br.u1():
        s.crop = (br.ue(), br.ue(), br.ue(), br.ue())
    # VUI is not needed for decoding; it is not parsed.
    if not s.frame_mbs_only:
        raise DecodeError("interlaced (frame_mbs_only_flag = 0) is not supported")
    if s.chroma_format_idc != 1 or s.separate_colour_plane:
        raise DecodeError("only 4:2:0 is supported")
    if s.bit_depth_luma != 8 or s.bit_depth_chroma != 8:
        raise DecodeError("only 8-bit samples are supported")
    if s.transform_bypass:
        raise DecodeError("qpprime_y_zero_transform_bypass is not supported")
    s.height_mbs = s.height_map_units
    return s


def parse_pps(br, sps_map):
    p = Obj()
    p.pps_id = br.ue()
    p.sps_id = br.ue()
    sps = sps_map.get(p.sps_id)
    if sps is None:
        raise DecodeError("PPS refers to a missing SPS")
    p.entropy_coding_mode = br.u1()
    p.bottom_field_pic_order_in_frame_present = br.u1()
    p.num_slice_groups = br.ue() + 1
    if p.num_slice_groups > 1:
        raise DecodeError("slice groups (FMO) are not supported")
    p.num_ref_idx_l0_default = br.ue() + 1
    p.num_ref_idx_l1_default = br.ue() + 1
    p.weighted_pred = br.u1()
    p.weighted_bipred_idc = br.u(2)
    p.pic_init_qp = 26 + br.se()
    p.pic_init_qs = 26 + br.se()
    p.chroma_qp_index_offset = br.se()
    p.deblocking_filter_control_present = br.u1()
    p.constrained_intra_pred = br.u1()
    p.redundant_pic_cnt_present = br.u1()
    p.transform_8x8_mode = 0
    p.scaling_lists = None
    p.second_chroma_qp_index_offset = p.chroma_qp_index_offset
    if br.more_rbsp_data():
        p.transform_8x8_mode = br.u1()
        if br.u1():
            # Fall-back rule B when the SPS has lists, A otherwise (Table 7-2).
            fb = sps.scaling_lists if sps.scaling_lists else DEFAULT_LISTS
            count = 6 + ((2 if sps.chroma_format_idc != 3 else 6) * p.transform_8x8_mode)
            p.scaling_lists = parse_scaling_lists(br, count, fb)
        p.second_chroma_qp_index_offset = br.se()
    return p


# ---------------------------------------------------------------------------
# Macroblock types (7.4.5, Tables 7-11, 7-13, 7-14, 7-17, 7-18)

P_TYPES = [  # (name, num parts, part w, part h)
    ("P_L0_16x16", 1, 16, 16), ("P_L0_L0_16x8", 2, 16, 8),
    ("P_L0_L0_8x16", 2, 8, 16), ("P_8x8", 4, 8, 8), ("P_8x8ref0", 4, 8, 8)]
P_SUB = [("P_L0_8x8", 1, 8, 8), ("P_L0_8x4", 2, 8, 4), ("P_L0_4x8", 2, 4, 8),
         ("P_L0_4x4", 4, 4, 4)]
# B types: (name, parts, w, h, pred mode of part 0, part 1); 'D' direct,
# 'L0', 'L1', 'Bi'.
B_TYPES = [("B_Direct_16x16", 0, 8, 8, "D", None)]
for _n, _p0, _p1 in [("B_L0_16x16", "L0", None), ("B_L1_16x16", "L1", None),
                     ("B_Bi_16x16", "Bi", None)]:
    B_TYPES.append((_n, 1, 16, 16, _p0, _p1))
for _a, _b in [("L0", "L0"), ("L1", "L1"), ("L0", "L1"), ("L1", "L0"),
               ("L0", "Bi"), ("L1", "Bi"), ("Bi", "L0"), ("Bi", "L1"),
               ("Bi", "Bi")]:
    B_TYPES.append(("B_%s_%s_16x8" % (_a, _b), 2, 16, 8, _a, _b))
    B_TYPES.append(("B_%s_%s_8x16" % (_a, _b), 2, 8, 16, _a, _b))
B_TYPES.append(("B_8x8", 4, 8, 8, None, None))
B_SUB = [("B_Direct_8x8", 4, 4, 4, "D"), ("B_L0_8x8", 1, 8, 8, "L0"),
         ("B_L1_8x8", 1, 8, 8, "L1"), ("B_Bi_8x8", 1, 8, 8, "Bi"),
         ("B_L0_8x4", 2, 8, 4, "L0"), ("B_L0_4x8", 2, 4, 8, "L0"),
         ("B_L1_8x4", 2, 8, 4, "L1"), ("B_L1_4x8", 2, 4, 8, "L1"),
         ("B_Bi_8x4", 2, 8, 4, "Bi"), ("B_Bi_4x8", 2, 4, 8, "Bi"),
         ("B_L0_4x4", 4, 4, 4, "L0"), ("B_L1_4x4", 4, 4, 4, "L1"),
         ("B_Bi_4x4", 4, 4, 4, "Bi")]

# luma4x4BlkIdx -> position (6.4.3), and raster index (4 * row + col).
BLK_X = [0, 4, 0, 4, 8, 12, 8, 12, 0, 4, 0, 4, 8, 12, 8, 12]
BLK_Y = [0, 0, 4, 4, 0, 0, 4, 4, 8, 8, 12, 12, 8, 8, 12, 12]
BLK_RASTER = [(BLK_Y[i] >> 2) * 4 + (BLK_X[i] >> 2) for i in range(16)]
RASTER_BLK = [0] * 16
for _i, _r in enumerate(BLK_RASTER):
    RASTER_BLK[_r] = _i

SLICE_P, SLICE_B, SLICE_I, SLICE_SP, SLICE_SI = 0, 1, 2, 3, 4


class MB:
    """Everything later stages need to know about one macroblock."""

    def __init__(self, addr, mbx, mby, slice_num):
        self.addr = addr
        self.mbx = mbx
        self.mby = mby
        self.slice_num = slice_num
        self.kind = None          # I4 I8 I16 IPCM P B PSKIP BSKIP
        self.is_intra = False
        self.name = ""
        self.mb_type = 0
        self.cbp = 0
        self.t8 = 0
        self.qp = 0
        self.qp_delta = 0
        self.i16_mode = 0
        self.chroma_mode = 0
        self.modes4 = None        # Intra4x4PredMode by raster 4x4 index
        self.modes8 = None        # Intra8x8PredMode by 8x8 index
        self.tc = [0] * 16        # TotalCoeff per luma 4x4 (raster)
        self.tcc = [[0] * 4, [0] * 4]  # per chroma 4x4 (raster), Cb, Cr
        self.nz = [0] * 16        # luma 4x4 has nonzero coefficients (raster)
        self.pred_flag = [[0] * 16, [0] * 16]
        self.ref = [[-1] * 16, [-1] * 16]
        self.refpic = [[None] * 16, [None] * 16]
        self.mv = [[(0, 0)] * 16, [(0, 0)] * 16]
        self.mvd = [[(0, 0)] * 16, [(0, 0)] * 16]
        self.refsyn = [[None] * 16, [None] * 16]  # CABAC: coded ref_idx per 4x4
        self.sub = None
        self.parts = None
        self.direct = [0] * 4     # 8x8 quadrant predicted by direct mode
        self.skip = False
        self.tokens = []
        # CABAC context bookkeeping
        self.cbf = {}             # coded_block_flag per (cat, idx)
        self.ref_ctx = [[0] * 4, [0] * 4]


class PicBuf:
    """A decoded frame and what the reference processes need of it."""

    def __init__(self, w, h):
        self.w = w
        self.h = h
        self.cw = w // 2
        self.ch = h // 2
        self.Y = bytearray(w * h)
        self.U = bytearray(self.cw * self.ch)
        self.V = bytearray(self.cw * self.ch)
        self.mbs = None
        self.frame_num = 0
        self.frame_num_wrap = 0
        self.poc = 0
        self.ref = None            # None, 'S' or 'L'
        self.long_term_idx = 0
        self.idr = False
        self.mmco5 = False
        self.serial = 0
        self.group = 0


# ---------------------------------------------------------------------------
# The decoder

class Decoder:
    def __init__(self, trace=None):
        self.sps = {}
        self.pps = {}
        self.dpb = []          # reference pictures (and the current one)
        self.out = []          # every decoded picture, for output
        self.cur = None
        self.trace = trace
        self.prev_ref_frame_num = 0
        self.prev_poc_msb = 0
        self.prev_poc_lsb = 0
        self.prev_frame_num_offset = 0
        self.prev_frame_num = 0
        self.prev_has_mmco5 = False
        self.serial = 0
        self.group = 0
        self.stats = {}
        self.collect = False

    def stat(self, key):
        self.stats[key] = self.stats.get(key, 0) + 1

    # -- NAL level -------------------------------------------------------
    def decode_stream(self, stream):
        for nal in nal_units(stream):
            self.decode_nal(nal)
        self.finish_picture()
        return self.output_order()

    def decode_nal(self, nal):
        hdr = nal[0]
        if hdr & 0x80:
            raise DecodeError("forbidden_zero_bit set")
        nal_ref_idc = (hdr >> 5) & 3
        nal_type = hdr & 31
        rbsp = unescape(nal[1:])
        if nal_type == 7:
            s = parse_sps(BitReader(rbsp))
            self.sps[s.sps_id] = s
        elif nal_type == 8:
            p = parse_pps(BitReader(rbsp), self.sps)
            self.pps[p.pps_id] = p
        elif nal_type in (1, 5):
            self.decode_slice(BitReader(rbsp), nal_type, nal_ref_idc)
        elif nal_type in (2, 3, 4):
            raise DecodeError("data partitioning is not supported")
        # SEI (6), AUD (9), end of sequence/stream, filler: nothing to do.

    # -- slice header (7.3.3) -------------------------------------------
    def parse_slice_header(self, br, nal_type, nal_ref_idc):
        h = Obj()
        h.nal_type = nal_type
        h.idr = nal_type == 5
        h.nal_ref_idc = nal_ref_idc
        h.first_mb = br.ue()
        st = br.ue()
        h.slice_type_raw = st
        h.slice_type = st % 5
        h.pps_id = br.ue()
        pps = self.pps.get(h.pps_id)
        if pps is None:
            raise DecodeError("slice refers to a missing PPS")
        sps = self.sps[pps.sps_id]
        h.pps = pps
        h.sps = sps
        h.frame_num = br.u(sps.log2_max_frame_num)
        h.idr_pic_id = br.ue() if h.idr else 0
        h.poc_lsb = 0
        h.delta_poc_bottom = 0
        h.delta_poc = [0, 0]
        if sps.poc_type == 0:
            h.poc_lsb = br.u(sps.log2_max_poc_lsb)
            if pps.bottom_field_pic_order_in_frame_present:
                h.delta_poc_bottom = br.se()
        if sps.poc_type == 1 and not sps.delta_pic_order_always_zero:
            h.delta_poc[0] = br.se()
            if pps.bottom_field_pic_order_in_frame_present:
                h.delta_poc[1] = br.se()
        if pps.redundant_pic_cnt_present:
            h.redundant_pic_cnt = br.ue()
            if h.redundant_pic_cnt:
                raise DecodeError("redundant pictures are not supported")
        h.direct_spatial = 0
        if h.slice_type == SLICE_B:
            h.direct_spatial = br.u1()
        h.num_ref_idx = [pps.num_ref_idx_l0_default, pps.num_ref_idx_l1_default]
        if h.slice_type in (SLICE_P, SLICE_SP, SLICE_B):
            if br.u1():
                h.num_ref_idx[0] = br.ue() + 1
                if h.slice_type == SLICE_B:
                    h.num_ref_idx[1] = br.ue() + 1
        if h.slice_type not in (SLICE_P, SLICE_SP, SLICE_B):
            h.num_ref_idx = [0, 0]
        elif h.slice_type != SLICE_B:
            h.num_ref_idx[1] = 0
        # ref_pic_list_modification (7.3.3.1)
        h.rplm = [[], []]
        for lx in (0, 1):
            if (lx == 0 and h.slice_type in (SLICE_P, SLICE_SP, SLICE_B)) or \
                    (lx == 1 and h.slice_type == SLICE_B):
                if br.u1():
                    while True:
                        idc = br.ue()
                        if idc == 3:
                            break
                        if idc > 5:
                            raise DecodeError("bad modification_of_pic_nums_idc")
                        h.rplm[lx].append((idc, br.ue()))
        # pred_weight_table (7.3.3.2)
        h.wp = None
        if (pps.weighted_pred and h.slice_type in (SLICE_P, SLICE_SP)) or \
                (pps.weighted_bipred_idc == 1 and h.slice_type == SLICE_B):
            h.wp = self.parse_pred_weight_table(br, h)
        # dec_ref_pic_marking (7.3.3.3)
        h.no_output_of_prior_pics = 0
        h.long_term_reference = 0
        h.mmco = None
        if nal_ref_idc:
            if h.idr:
                h.no_output_of_prior_pics = br.u1()
                h.long_term_reference = br.u1()
            elif br.u1():
                h.mmco = []
                while True:
                    op = br.ue()
                    if op == 0:
                        break
                    a = b = 0
                    if op in (1, 3):
                        a = br.ue()
                    if op == 2:
                        a = br.ue()
                    if op in (3, 6):
                        b = br.ue()
                    if op == 4:
                        a = br.ue()
                    h.mmco.append((op, a, b))
        h.cabac_init_idc = 0
        if pps.entropy_coding_mode and h.slice_type not in (SLICE_I, SLICE_SI):
            h.cabac_init_idc = br.ue()
        h.slice_qp_delta = br.se()
        h.qp = pps.pic_init_qp + h.slice_qp_delta
        if h.slice_type in (SLICE_SP, SLICE_SI):
            raise DecodeError("SP and SI slices are not supported")
        h.disable_deblocking = 0
        h.alpha_off = 0
        h.beta_off = 0
        if pps.deblocking_filter_control_present:
            h.disable_deblocking = br.ue()
            if h.disable_deblocking != 1:
                h.alpha_off = br.se() * 2
                h.beta_off = br.se() * 2
        return h

    def parse_pred_weight_table(self, br, h):
        wp = Obj()
        wp.luma_log2 = br.ue()
        wp.chroma_log2 = br.ue()
        wp.w = [[], []]   # per list, per ref: (lw, lo, [(cw, co), (cw, co)], lflag, cflag)
        for lx in range(2 if h.slice_type == SLICE_B else 1):
            for i in range(h.num_ref_idx[lx]):
                lw, lo = 1 << wp.luma_log2, 0
                lf = br.u1()
                if lf:
                    lw = br.se()
                    lo = br.se()
                cws = [(1 << wp.chroma_log2, 0), (1 << wp.chroma_log2, 0)]
                cf = br.u1()
                if cf:
                    cws = [(br.se(), br.se()) for _ in range(2)]
                wp.w[lx].append((lw, lo, cws, lf, cf))
        return wp

    # -- picture boundaries ---------------------------------------------
    def decode_slice(self, br, nal_type, nal_ref_idc):
        h = self.parse_slice_header(br, nal_type, nal_ref_idc)
        if h.first_mb == 0 or self.cur is None:
            self.finish_picture()
            self.start_picture(h)
        elif h.frame_num != self.cur_h.frame_num:
            raise DecodeError("a picture was missing its first slice (ASO is not supported)")
        self.slice_num += 1
        h.num = self.slice_num
        self.slice_hdrs.append(h)
        if self.collect:
            st = self.stat
            st("slice type %d" % h.slice_type)
            if h.rplm[0] or h.rplm[1]:
                st("ref_pic_list_modification")
            if h.mmco:
                st("adaptive_ref_pic_marking")
                for op in h.mmco:
                    st("mmco %d" % op[0])
            if h.disable_deblocking:
                st("disable_deblocking_filter_idc %d" % h.disable_deblocking)
            if h.alpha_off or h.beta_off:
                st("deblock offsets %d,%d" % (h.alpha_off, h.beta_off))
            if h.num_ref_idx[0] > 1:
                st("num_ref_idx_l0_active %d" % h.num_ref_idx[0])
            if h.first_mb:
                st("first_mb_in_slice > 0")
            if h.wp:
                st("pred_weight_table")
        self.build_ref_lists(h)
        if self.trace:
            self.trace.slice(self, h)
        sd = SliceDecoder(self, h, br)
        sd.run()

    def start_picture(self, h):
        sps = h.sps
        if h.idr:
            # IDR: every reference is gone; output what was decoded before.
            for p in self.dpb:
                p.ref = None
            self.dpb = []
            self.group += 1
        elif h.frame_num != self.prev_ref_frame_num and \
                h.frame_num != (self.prev_ref_frame_num + 1) % (1 << sps.log2_max_frame_num):
            raise DecodeError("gaps in frame_num are not supported")
        w = sps.width_mbs * 16
        hh = sps.height_mbs * 16
        pic = PicBuf(w, hh)
        pic.mbs = [None] * (sps.width_mbs * sps.height_mbs)
        pic.frame_num = h.frame_num
        pic.idr = h.idr
        pic.group = self.group
        self.serial += 1
        pic.serial = self.serial
        self.cur = pic
        self.cur_h = h
        self.slice_num = 0
        self.slice_hdrs = []
        self.compute_poc(h, pic)
        if self.trace:
            self.trace.picture_start(self, h, pic)

    def compute_poc(self, h, pic):
        """8.2.1, frames only."""
        sps = h.sps
        max_fn = 1 << sps.log2_max_frame_num
        if sps.poc_type == 0:
            if h.idr:
                prev_msb, prev_lsb = 0, 0
            elif self.prev_has_mmco5:
                prev_msb, prev_lsb = 0, self.prev_mmco5_top
            else:
                prev_msb, prev_lsb = self.prev_poc_msb, self.prev_poc_lsb
            max_lsb = 1 << sps.log2_max_poc_lsb
            lsb = h.poc_lsb
            if lsb < prev_lsb and prev_lsb - lsb >= max_lsb // 2:
                msb = prev_msb + max_lsb
            elif lsb > prev_lsb and lsb - prev_lsb > max_lsb // 2:
                msb = prev_msb - max_lsb
            else:
                msb = prev_msb
            top = msb + lsb
            bottom = top + h.delta_poc_bottom
            pic.poc_msb = msb
            pic.poc = min(top, bottom)
            pic.top_poc = top
        else:
            if h.idr:
                offset = 0
            else:
                prev_off = 0 if self.prev_has_mmco5 else self.prev_frame_num_offset
                offset = prev_off + max_fn if self.prev_frame_num > h.frame_num else prev_off
            if sps.poc_type == 1:
                n = len(sps.offset_for_ref_frame)
                abs_fn = offset + h.frame_num if n else 0
                if h.nal_ref_idc == 0 and abs_fn > 0:
                    abs_fn -= 1
                expected = 0
                if abs_fn > 0:
                    cycle = (abs_fn - 1) // n
                    in_cycle = (abs_fn - 1) % n
                    delta_cycle = sum(sps.offset_for_ref_frame)
                    expected = cycle * delta_cycle
                    for i in range(in_cycle + 1):
                        expected += sps.offset_for_ref_frame[i]
                if h.nal_ref_idc == 0:
                    expected += sps.offset_for_non_ref_pic
                top = expected + h.delta_poc[0]
                bottom = top + sps.offset_for_top_to_bottom_field + h.delta_poc[1]
            else:
                if h.idr:
                    t = 0
                elif h.nal_ref_idc == 0:
                    t = 2 * (offset + h.frame_num) - 1
                else:
                    t = 2 * (offset + h.frame_num)
                top = bottom = t
            pic.frame_num_offset = offset
            pic.poc = min(top, bottom)
            pic.top_poc = top
            pic.bottom_poc = bottom

    def finish_picture(self):
        pic = self.cur
        if pic is None:
            return
        h = self.cur_h
        if any(m is None for m in pic.mbs):
            raise DecodeError("picture %d is missing macroblocks" % pic.serial)
        if self.trace:
            self.trace.pre_deblock(self, pic)
        deblock_picture(self, pic)
        if self.trace:
            self.trace.picture_end(self, pic)
        sps = h.sps
        # Reference marking (8.2.5).
        has_mmco5 = False
        if h.nal_ref_idc:
            if h.idr:
                if h.long_term_reference:
                    pic.ref = "L"
                    pic.long_term_idx = 0
                    self.max_long_term_idx = 0
                else:
                    pic.ref = "S"
                    self.max_long_term_idx = None
            else:
                pic.ref = "S"
                if h.mmco is not None:
                    has_mmco5 = self.adaptive_marking(h, pic)
                else:
                    self.sliding_window(sps)
            if pic.ref is not None:
                self.dpb.append(pic)
        pic.mmco5 = has_mmco5
        # Remember what the next picture's POC and frame_num need.
        if has_mmco5:
            # 8.2.1: after memory_management_control_operation 5 the picture
            # is treated as having frame_num 0 and its POC shifted to 0.
            temp = pic.poc
            pic.top_poc -= temp
            pic.poc = 0
            pic.frame_num = 0
            self.prev_mmco5_top = pic.top_poc
            self.prev_frame_num_offset = 0
            self.prev_frame_num = 0
            self.prev_ref_frame_num = 0
            self.group += 1
            pic.group = self.group
        else:
            if sps.poc_type != 0:
                self.prev_frame_num_offset = pic.frame_num_offset
            self.prev_frame_num = h.frame_num
        self.prev_has_mmco5 = has_mmco5
        if h.nal_ref_idc:
            if not has_mmco5:
                self.prev_ref_frame_num = h.frame_num
                if sps.poc_type == 0:
                    self.prev_poc_msb = pic.poc_msb
                    self.prev_poc_lsb = h.poc_lsb
            elif sps.poc_type == 0:
                self.prev_poc_msb = 0
                self.prev_poc_lsb = pic.top_poc
        self.out.append(pic)
        self.cur = None

    def sliding_window(self, sps):
        """8.2.5.3, invoked before the current picture is added."""
        short = [p for p in self.dpb if p.ref == "S"]
        long_ = [p for p in self.dpb if p.ref == "L"]
        if len(short) + len(long_) >= max(sps.max_num_ref_frames, 1) and short:
            fnw = self.frame_num_wraps(self.cur_h)
            victim = min(short, key=lambda p: fnw[id(p)])
            victim.ref = None
            self.dpb.remove(victim)

    def frame_num_wraps(self, h):
        max_fn = 1 << h.sps.log2_max_frame_num
        out = {}
        for p in self.dpb:
            if p.ref == "S":
                out[id(p)] = p.frame_num - max_fn if p.frame_num > h.frame_num else p.frame_num
        return out

    def adaptive_marking(self, h, pic):
        """8.2.5.4. Returns True when operation 5 was present."""
        curr_pic_num = h.frame_num
        fnw = self.frame_num_wraps(h)
        mmco5 = False
        for op, a, b in h.mmco:
            if op == 1:
                num = curr_pic_num - (a + 1)
                for p in list(self.dpb):
                    if p.ref == "S" and fnw[id(p)] == num:
                        p.ref = None
                        self.dpb.remove(p)
            elif op == 2:
                for p in list(self.dpb):
                    if p.ref == "L" and p.long_term_idx == a:
                        p.ref = None
                        self.dpb.remove(p)
            elif op == 3:
                num = curr_pic_num - (a + 1)
                for p in list(self.dpb):
                    if p.ref == "L" and p.long_term_idx == b:
                        p.ref = None
                        self.dpb.remove(p)
                for p in self.dpb:
                    if p.ref == "S" and fnw.get(id(p)) == num:
                        p.ref = "L"
                        p.long_term_idx = b
            elif op == 4:
                self.max_long_term_idx = a - 1 if a else None
                for p in list(self.dpb):
                    if p.ref == "L" and (a == 0 or p.long_term_idx > a - 1):
                        p.ref = None
                        self.dpb.remove(p)
            elif op == 5:
                for p in self.dpb:
                    p.ref = None
                self.dpb = []
                self.max_long_term_idx = None
                mmco5 = True
            elif op == 6:
                for p in list(self.dpb):
                    if p.ref == "L" and p.long_term_idx == b:
                        p.ref = None
                        self.dpb.remove(p)
                pic.ref = "L"
                pic.long_term_idx = b
        return mmco5

    # -- reference lists (8.2.4) ------------------------------------------
    def build_ref_lists(self, h):
        h.ref_list = [[], []]
        if h.slice_type in (SLICE_I, SLICE_SI):
            return
        max_fn = 1 << h.sps.log2_max_frame_num
        refs = [p for p in self.dpb if p.ref]
        for p in refs:
            if p.ref == "S":
                p.frame_num_wrap = p.frame_num - max_fn if p.frame_num > h.frame_num else p.frame_num
        short = [p for p in refs if p.ref == "S"]
        long_ = sorted([p for p in refs if p.ref == "L"], key=lambda p: p.long_term_idx)
        cur_poc = self.cur.poc
        if h.slice_type == SLICE_P:
            l0 = sorted(short, key=lambda p: -p.frame_num_wrap) + long_
            lists = [l0]
        else:
            before = sorted([p for p in short if p.poc < cur_poc], key=lambda p: -p.poc)
            after = sorted([p for p in short if p.poc > cur_poc], key=lambda p: p.poc)
            l0 = before + after + long_
            l1 = after + before + long_
            if len(l1) > 1 and l1 == l0:
                l1[0], l1[1] = l1[1], l1[0]
            lists = [l0, l1]
        for lx, lst in enumerate(lists):
            n = h.num_ref_idx[lx]
            lst = lst[:n]
            lst = self.modify_list(h, lx, lst, n, short, long_)
            while len(lst) < n:
                lst.append(None)
            h.ref_list[lx] = lst

    def modify_list(self, h, lx, lst, n, short, long_):
        """8.2.4.3 for frames."""
        if not h.rplm[lx]:
            return lst
        max_pic_num = 1 << h.sps.log2_max_frame_num
        curr_pic_num = h.frame_num
        pred = curr_pic_num
        lst = list(lst) + [None] * (n + 1 - len(lst))
        ref_idx = 0
        for idc, val in h.rplm[lx]:
            if idc in (0, 1):
                d = val + 1
                if idc == 0:
                    nowrap = pred - d
                    if nowrap < 0:
                        nowrap += max_pic_num
                else:
                    nowrap = pred + d
                    if nowrap >= max_pic_num:
                        nowrap -= max_pic_num
                pred = nowrap
                pic_num = nowrap - max_pic_num if nowrap > curr_pic_num else nowrap
                target = None
                for p in short:
                    if p.frame_num_wrap == pic_num:
                        target = p
                if target is None:
                    raise DecodeError("reordering names a missing picture")
                lst = lst[:ref_idx] + [target] + lst[ref_idx:]
                ref_idx += 1
                nidx = ref_idx
                for c in range(ref_idx, n + 1):
                    q = lst[c]
                    if not (q is not None and q.ref == "S" and q.frame_num_wrap == pic_num):
                        lst[nidx] = q
                        nidx += 1
                lst = lst[:n + 1]
            elif idc == 2:
                target = None
                for p in long_:
                    if p.long_term_idx == val:
                        target = p
                if target is None:
                    raise DecodeError("reordering names a missing long-term picture")
                lst = lst[:ref_idx] + [target] + lst[ref_idx:]
                ref_idx += 1
                nidx = ref_idx
                for c in range(ref_idx, n + 1):
                    q = lst[c]
                    if not (q is not None and q.ref == "L" and q.long_term_idx == val):
                        lst[nidx] = q
                        nidx += 1
                lst = lst[:n + 1]
            else:
                raise DecodeError("MVC list modification is not supported")
        return lst[:n]

    # -- output ----------------------------------------------------------
    def output_order(self):
        return sorted(self.out, key=lambda p: (p.group, p.poc, p.serial))

    def cropped_frame(self, pic):
        sps = self.sps_of(pic)
        cl, cr, ct, cb = sps.crop
        x0, x1 = 2 * cl, pic.w - 2 * cr
        y0, y1 = 2 * ct, pic.h - 2 * cb
        out = bytearray()
        for y in range(y0, y1):
            out += pic.Y[y * pic.w + x0:y * pic.w + x1]
        for plane in (pic.U, pic.V):
            for y in range(y0 // 2, y1 // 2):
                out += plane[y * pic.cw + x0 // 2:y * pic.cw + x1 // 2]
        return bytes(out)

    def sps_of(self, pic):
        return pic.sps


# ---------------------------------------------------------------------------
# Slice data: parsing (CAVLC here, CABAC further down) and reconstruction

class SliceDecoder:
    def __init__(self, dec, h, br):
        self.dec = dec
        self.h = h
        self.br = br
        self.sps = h.sps
        self.pps = h.pps
        self.pic = dec.cur
        self.pic.sps = h.sps
        self.W = h.sps.width_mbs
        self.H = h.sps.height_mbs
        self.trace = dec.trace
        self.constrained = h.pps.constrained_intra_pred
        self.qp = h.qp
        self.ref_list = h.ref_list

    def mb_at(self, addr):
        m = self.pic.mbs[addr]
        if m is None or m.slice_num != self.h.num:
            return None
        return m

    def new_mb(self, addr):
        mbx = addr % self.W
        mby = addr // self.W
        mb = MB(addr, mbx, mby, self.h.num)
        mb.slice_type = self.h.slice_type
        mb.mbA = self.mb_at(addr - 1) if mbx > 0 else None
        mb.mbB = self.mb_at(addr - self.W) if mby > 0 else None
        mb.mbC = self.mb_at(addr - self.W + 1) if mby > 0 and mbx < self.W - 1 else None
        mb.mbD = self.mb_at(addr - self.W - 1) if mby > 0 and mbx > 0 else None
        mb.hdr = self.h
        return mb

    def run(self):
        h = self.h
        if h.pps.entropy_coding_mode:
            self.run_cabac()
            return
        br = self.br
        addr = h.first_mb
        total = self.W * self.H
        more = True
        while more:
            if h.slice_type != SLICE_I:
                run = br.ue()
                for _ in range(run):
                    if addr >= total:
                        raise DecodeError("mb_skip_run runs past the picture")
                    self.decode_skip(addr)
                    addr += 1
                if run > 0:
                    more = br.more_rbsp_data()
                    if not more:
                        break
            if addr >= total:
                raise DecodeError("slice data runs past the picture")
            mb = self.new_mb(addr)
            self.parse_mb_cavlc(mb)
            self.finish_mb(mb)
            more = br.more_rbsp_data()
            addr += 1

    # -- skipped macroblocks -------------------------------------------
    def decode_skip(self, addr):
        mb = self.new_mb(addr)
        mb.skip = True
        mb.qp = self.qp
        if self.h.slice_type == SLICE_P:
            mb.kind = "PSKIP"
            mb.name = "P_Skip"
            self.dec.stat("P_Skip")
        else:
            mb.kind = "BSKIP"
            mb.name = "B_Skip"
            self.dec.stat("B_Skip")
        self.finish_mb(mb)

    # -- CAVLC macroblock_layer (7.3.5) ---------------------------------
    def parse_mb_cavlc(self, mb):
        br = self.br
        h = self.h
        mt = br.ue()
        mb.mb_type = mt
        if h.slice_type == SLICE_P:
            if mt < 5:
                self.set_p_type(mb, mt)
            else:
                self.set_i_type(mb, mt - 5)
        elif h.slice_type == SLICE_B:
            if mt < 23:
                self.set_b_type(mb, mt)
            else:
                self.set_i_type(mb, mt - 23)
        else:
            self.set_i_type(mb, mt)
        if mb.kind == "IPCM":
            br.byte_align()
            self.read_pcm(mb, lambda: br.u(8))
            return
        if mb.kind in ("P", "B") and mb.num_parts == 4:
            self.parse_sub_mb_pred_cavlc(mb)
        else:
            if mb.kind == "I4" and self.pps.transform_8x8_mode:
                if br.u1():
                    mb.kind = "I8"
                    mb.t8 = 1
            self.parse_mb_pred_cavlc(mb)
        if mb.kind != "I16":
            code = br.ue()
            if code > 47:
                raise DecodeError("bad coded_block_pattern")
            mb.cbp = (T.CBP_INTRA if mb.is_intra else T.CBP_INTER)[code]
            if (mb.cbp & 15) and self.pps.transform_8x8_mode and not mb.is_intra \
                    and self.no_sub_lt8x8(mb):
                mb.t8 = br.u1()
        mb.qp_delta = 0
        if (mb.cbp & 15) or (mb.cbp >> 4) or mb.kind == "I16":
            mb.qp_delta = br.se()
            if mb.qp_delta < -26 or mb.qp_delta > 25:
                raise DecodeError("mb_qp_delta out of range")
        self.qp = (self.qp + mb.qp_delta + 52) % 52
        mb.qp = self.qp
        self.emit_mb_header(mb)
        self.parse_residual_cavlc(mb)

    def no_sub_lt8x8(self, mb):
        if mb.num_parts != 4:
            return mb.name != "B_Direct_16x16" or self.sps.direct_8x8_inference
        for s in mb.sub:
            if s[0] == "B_Direct_8x8":
                if not self.sps.direct_8x8_inference:
                    return False
            elif s[1] > 1:
                return False
        return True

    def set_i_type(self, mb, t):
        mb.is_intra = True
        if t == 0:
            mb.kind = "I4"
            mb.name = "I_NxN"
        elif t == 25:
            mb.kind = "IPCM"
            mb.name = "I_PCM"
        elif 1 <= t <= 24:
            mb.kind = "I16"
            mb.i16_mode = (t - 1) % 4
            cbp_c = ((t - 1) // 4) % 3
            cbp_l = 15 if t >= 13 else 0
            mb.cbp = cbp_l | (cbp_c << 4)
            mb.name = "I_16x16_%d_%d_%d" % (mb.i16_mode, cbp_c, 1 if cbp_l else 0)
        else:
            raise DecodeError("bad intra mb_type %d" % t)
        mb.i_type = t

    def set_p_type(self, mb, t):
        mb.kind = "P"
        mb.name, mb.num_parts, mb.part_w, mb.part_h = P_TYPES[t]
        mb.part_pred = ["L0"] * mb.num_parts

    def set_b_type(self, mb, t):
        name, n, w, hh, p0, p1 = B_TYPES[t]
        mb.kind = "B"
        mb.name = name
        mb.num_parts = n
        mb.part_w = w
        mb.part_h = hh
        mb.part_pred = [p for p in (p0, p1) if p is not None][:n] if n in (1, 2) else []
        if t == 0:
            mb.num_parts = 0  # B_Direct_16x16

    def read_pcm(self, mb, read8):
        mb.qp = self.qp
        mb.pcm = [read8() for _ in range(256 + 128)]
        mb.tc = [16] * 16
        mb.tcc = [[16] * 4, [16] * 4]
        mb.nz = [1] * 16
        self.emit_mb_header(mb)
        self.dec.stat("I_PCM")

    def parse_mb_pred_cavlc(self, mb):
        br = self.br
        if mb.kind == "I4":
            flags = []
            for i in range(16):
                f = br.u1()
                rem = None if f else br.u(3)
                flags.append((f, rem))
            mb.modes4 = [0] * 16
            for blk in range(16):
                f, rem = flags[blk]
                self.resolve_intra4(mb, blk, f, rem)
            mb.chroma_mode = br.ue()
        elif mb.kind == "I8":
            flags = []
            for i in range(4):
                f = br.u1()
                rem = None if f else br.u(3)
                flags.append((f, rem))
            mb.modes8 = [0] * 4
            for b8 in range(4):
                self.resolve_intra8(mb, b8, flags[b8][0], flags[b8][1])
            mb.chroma_mode = br.ue()
        elif mb.kind == "I16":
            mb.chroma_mode = br.ue()
        else:
            h = self.h
            n = mb.num_parts
            mb.part_ref = [[0] * n, [0] * n]
            for lx in (0, 1):
                for i in range(n):
                    if mb.part_pred[i] in ("L" + str(lx), "Bi"):
                        if h.num_ref_idx[lx] > 1:
                            mb.part_ref[lx][i] = br.te(h.num_ref_idx[lx] - 1)
            mb.part_mvd = [[None] * n, [None] * n]
            for lx in (0, 1):
                for i in range(n):
                    if mb.part_pred[i] in ("L" + str(lx), "Bi"):
                        mb.part_mvd[lx][i] = (br.se(), br.se())
        if mb.chroma_mode > 3:
            raise DecodeError("bad intra_chroma_pred_mode")

    def parse_sub_mb_pred_cavlc(self, mb):
        br = self.br
        h = self.h
        is_b = mb.kind == "B"
        mb.sub = []
        for i in range(4):
            t = br.ue()
            mb.sub.append(B_SUB[t] if is_b else P_SUB[t] + ("L0",))
        mb.sub_ref = [[0] * 4, [0] * 4]
        for lx in (0, 1):
            for i in range(4):
                s = mb.sub[i]
                if s[4] in ("L" + str(lx), "Bi"):
                    if h.num_ref_idx[lx] > 1 and mb.name != "P_8x8ref0":
                        mb.sub_ref[lx][i] = br.te(h.num_ref_idx[lx] - 1)
        mb.sub_mvd = [[[None] * 4 for _ in range(4)], [[None] * 4 for _ in range(4)]]
        for lx in (0, 1):
            for i in range(4):
                s = mb.sub[i]
                if s[4] in ("L" + str(lx), "Bi"):
                    for j in range(s[1]):
                        mb.sub_mvd[lx][i][j] = (br.se(), br.se())

    # -- intra mode prediction (8.3.1.1, 8.3.2.1) -----------------------
    def nb4(self, mb, x, y):
        """Neighbour of luma location (x, y) relative to mb (6.4.12)."""
        if x < 0:
            if y < 0:
                return mb.mbD, x + 16, y + 16
            if y < 16:
                return mb.mbA, x + 16, y
            return None, 0, 0
        if x < 16:
            if y < 0:
                return mb.mbB, x, y + 16
            if y < 16:
                return mb, x, y
            return None, 0, 0
        if y < 0:
            return mb.mbC, x - 16, y + 16
        return None, 0, 0

    def neighbour_mode(self, mb, x, y):
        n, xw, yw = self.nb4(mb, x, y)
        if n is None or (not n.is_intra and self.constrained):
            return None
        if n.kind == "I4":
            return n.modes4[(yw >> 2) * 4 + (xw >> 2)]
        if n.kind == "I8":
            return n.modes8[(yw >> 3) * 2 + (xw >> 3)]
        return 2

    def resolve_intra4(self, mb, blk, flag, rem):
        x, y = BLK_X[blk], BLK_Y[blk]
        a = self.neighbour_mode(mb, x - 1, y)
        b = self.neighbour_mode(mb, x, y - 1)
        pred = 2 if a is None or b is None else min(a, b)
        mode = pred if flag else (rem if rem < pred else rem + 1)
        mb.modes4[(y >> 2) * 4 + (x >> 2)] = mode

    def resolve_intra8(self, mb, b8, flag, rem):
        x, y = (b8 & 1) * 8, (b8 >> 1) * 8
        a = self.neighbour_mode8(mb, x - 1, y, 1)
        b = self.neighbour_mode8(mb, x, y - 1, 2)
        pred = 2 if a is None or b is None else min(a, b)
        mode = pred if flag else (rem if rem < pred else rem + 1)
        mb.modes8[b8] = mode

    def neighbour_mode8(self, mb, x, y, n):
        nm, xw, yw = self.nb4(mb, x, y)
        if nm is None or (not nm.is_intra and self.constrained):
            return None
        if nm.kind == "I8":
            return nm.modes8[(yw >> 3) * 2 + (xw >> 3)]
        if nm.kind == "I4":
            # 8.3.2.1: the 4x4 block luma8x8BlkIdxN * 4 + n of the 8x8 block.
            b8 = (yw >> 3) * 2 + (xw >> 3)
            blk = b8 * 4 + n
            return nm.modes4[BLK_RASTER[blk]]
        return 2

    # -- CAVLC residual (7.3.5.3, 9.2) ----------------------------------
    def nc_luma(self, mb, r):
        bx, by = r & 3, r >> 2
        if bx > 0:
            na = mb.tc[r - 1]
        else:
            na = mb.mbA.tc[r + 3] if mb.mbA is not None else None
        if by > 0:
            nb = mb.tc[r - 4]
        else:
            nb = mb.mbB.tc[r + 12] if mb.mbB is not None else None
        return combine_nc(na, nb)

    def nc_chroma(self, mb, c, r):
        bx, by = r & 1, r >> 1
        if bx > 0:
            na = mb.tcc[c][r - 1]
        else:
            na = mb.mbA.tcc[c][r + 1] if mb.mbA is not None else None
        if by > 0:
            nb = mb.tcc[c][r - 2]
        else:
            nb = mb.mbB.tcc[c][r + 2] if mb.mbB is not None else None
        return combine_nc(na, nb)

    def residual_block_cavlc(self, nc, start, end, maxnum):
        br = self.br
        if nc < 0:
            t = COEFF_TOKEN_VLC[4 if nc == -1 else 5]
        elif nc < 2:
            t = COEFF_TOKEN_VLC[0]
        elif nc < 4:
            t = COEFF_TOKEN_VLC[1]
        elif nc < 8:
            t = COEFF_TOKEN_VLC[2]
        else:
            t = COEFF_TOKEN_VLC[3]
        ent = t.table[br.peek(t.bits)]
        if ent is None:
            raise DecodeError("bad coeff_token")
        br.pos += ent[0]
        t1, total = ent[1]
        coeff = [0] * maxnum
        if total == 0:
            return coeff, 0
        if total > end - start + 1:
            raise DecodeError("TotalCoeff exceeds the block")
        levels = []
        suffix_len = 1 if total > 10 and t1 < 3 else 0
        for i in range(total):
            if i < t1:
                levels.append(-1 if br.u1() else 1)
                continue
            prefix = 0
            while br.u1() == 0:
                prefix += 1
                if prefix > 32:
                    raise DecodeError("bad level_prefix")
            if prefix >= 14 and self.dec.collect:
                self.dec.stat("level_prefix %s" % (prefix if prefix < 16 else ">= 16"))
            if prefix == 14 and suffix_len == 0:
                ssize = 4
            elif prefix >= 15:
                ssize = prefix - 3
            else:
                ssize = suffix_len
            suffix = br.u(ssize) if ssize > 0 else 0
            code = (min(15, prefix) << suffix_len) + suffix
            if prefix >= 15 and suffix_len == 0:
                code += 15
            if prefix >= 16:
                code += (1 << (prefix - 3)) - 4096
            if i == t1 and t1 < 3:
                code += 2
            lv = (code + 2) >> 1 if code % 2 == 0 else (-code - 1) >> 1
            levels.append(lv)
            if suffix_len == 0:
                suffix_len = 1
            if abs(lv) > (3 << (suffix_len - 1)) and suffix_len < 6:
                suffix_len += 1
        if total < end - start + 1:
            if maxnum == 4:
                tz = TOTAL_ZEROS_DC420_VLC[total]
            elif maxnum == 8:
                tz = TOTAL_ZEROS_DC422_VLC[total]
            else:
                tz = TOTAL_ZEROS_VLC[total]
            ent = tz.table[br.peek(tz.bits)]
            if ent is None:
                raise DecodeError("bad total_zeros")
            br.pos += ent[0]
            zeros_left = ent[1]
        else:
            zeros_left = 0
        runs = []
        for i in range(total - 1):
            if zeros_left > 0:
                rb = RUN_BEFORE_VLC[min(zeros_left, 7)]
                ent = rb.table[br.peek(rb.bits)]
                if ent is None:
                    raise DecodeError("bad run_before")
                br.pos += ent[0]
                run = ent[1]
            else:
                run = 0
            runs.append(run)
            zeros_left -= run
            if zeros_left < 0:
                raise DecodeError("run_before exceeds zerosLeft")
        runs.append(zeros_left)
        num = -1
        for i in range(total - 1, -1, -1):
            num += runs[i] + 1
            if start + num >= maxnum:
                raise DecodeError("coefficients run past the block")
            coeff[start + num] = levels[i]
        return coeff, total

    def parse_residual_cavlc(self, mb):
        """residual( 0, 15 ) with CAVLC. Coefficient lists are kept in scan
        order: mb.coef[raster4x4] is a 16-list (index 0 = DC), mb.dc16 the
        Intra16x16 DC list, mb.cdc[c] and mb.cac[c][raster] for chroma."""
        mb.coef = [None] * 16
        mb.coef8 = [None] * 4
        if mb.kind == "I16":
            nc = self.nc_luma(mb, 0)
            mb.dc16, n = self.residual_block_cavlc(nc, 0, 15, 16)
            self.emit_coef(mb, "Intra16x16DC", 0, mb.dc16, n)
        cbp_l = mb.cbp & 15
        for b8 in range(4):
            for b4 in range(4):
                blk = b8 * 4 + b4
                r = BLK_RASTER[blk]
                if cbp_l & (1 << b8):
                    nc = self.nc_luma(mb, r)
                    if mb.kind == "I16":
                        ac, n = self.residual_block_cavlc(nc, 0, 14, 15)
                        mb.coef[r] = [0] + ac
                        self.emit_coef(mb, "Intra16x16AC", blk, mb.coef[r], n)
                    else:
                        mb.coef[r], n = self.residual_block_cavlc(nc, 0, 15, 16)
                        self.emit_coef(mb, "Luma4x4" if not mb.t8 else "Luma8x8part",
                                       blk, mb.coef[r], n)
                    mb.tc[r] = n
                    mb.nz[r] = 1 if n else 0
            if mb.t8 and cbp_l & (1 << b8):
                lv = [0] * 64
                for b4 in range(4):
                    c4 = mb.coef[BLK_RASTER[b8 * 4 + b4]]
                    for i in range(16):
                        lv[4 * i + b4] = c4[i]
                mb.coef8[b8] = lv
                anynz = any(lv)
                for b4 in range(4):
                    mb.nz[BLK_RASTER[b8 * 4 + b4]] = 1 if anynz else 0
        cbp_c = mb.cbp >> 4
        mb.cdc = [[0] * 4, [0] * 4]
        mb.cac = [[None] * 4, [None] * 4]
        if cbp_c & 3:
            for c in range(2):
                mb.cdc[c], n = self.residual_block_cavlc(-1, 0, 3, 4)
                self.emit_coef(mb, "ChromaDC" + "BR"[c], 0, mb.cdc[c], n)
        if cbp_c & 2:
            for c in range(2):
                for r in range(4):
                    nc = self.nc_chroma(mb, c, r)
                    ac, n = self.residual_block_cavlc(nc, 0, 14, 15)
                    mb.cac[c][r] = [0] + ac
                    mb.tcc[c][r] = n
                    self.emit_coef(mb, "ChromaAC" + "BR"[c], r, mb.cac[c][r], n)

    # -- tokens for dumps -------------------------------------------------
    def emit_mb_header(self, mb):
        if self.trace:
            self.trace.mb_header(self, mb)

    def emit_coef(self, mb, cat, idx, coeffs, n):
        if self.trace:
            self.trace.coef(self, mb, cat, idx, coeffs, n)

    # -- after parsing: motion, prediction, residual ----------------------
    def finish_mb(self, mb):
        self.pic.mbs[mb.addr] = mb
        dec = self.dec
        if mb.kind not in ("PSKIP", "BSKIP"):
            dec.stat(mb.name if not mb.name.startswith("I_16x16") else "I_16x16")
        if mb.kind in ("P", "PSKIP", "B", "BSKIP"):
            derive_motion(self, mb)
            inter_predict(self, mb)
        if mb.kind == "IPCM":
            write_pcm(self, mb)
        elif mb.kind in ("I4", "I8", "I16"):
            intra_reconstruct(self, mb)
        else:
            inter_reconstruct(self, mb)
        if dec.collect:
            collect_stats(dec, mb)
        if self.trace:
            self.trace.mb_done(self, mb)


def collect_stats(dec, mb):
    st = dec.stat
    if mb.kind == "I4":
        for m in mb.modes4:
            st("I4x4 mode %d" % m)
    if mb.kind == "I8":
        for m in mb.modes8:
            st("I8x8 mode %d" % m)
    if mb.kind == "I16":
        st("I16x16 mode %d" % mb.i16_mode)
    if mb.is_intra and mb.kind != "IPCM":
        st("chroma mode %d" % mb.chroma_mode)
    if mb.sub:
        for s in mb.sub:
            st(s[0])
    if mb.qp_delta:
        st("mb_qp_delta != 0")
    if mb.kind == "PSKIP" and mb.mv[0][0] != (0, 0):
        st("P_Skip with a nonzero vector")
    if mb.is_intra and mb.slice_type != SLICE_I:
        st("intra MB in P/B slice")
        if mb.hdr.pps.constrained_intra_pred:
            st("intra MB with constrained_intra_pred")
    if mb.t8:
        st("transform_size_8x8")
    for lx in (0, 1):
        for r in range(16):
            if mb.pred_flag[lx][r]:
                mv = mb.mv[lx][r]
                st("L%d mv frac (%d,%d)" % (lx, mv[0] & 3, mv[1] & 3))
                if mb.ref[lx][r] > 0:
                    st("L%d ref_idx %d" % (lx, mb.ref[lx][r]))
    if mb.kind in ("I4", "I16", "P", "B") and getattr(mb, "coef", None):
        for c in mb.coef:
            if c and max(abs(v) for v in c) > 255:
                st("|level| > 255")
            if c and max(abs(v) for v in c) > 15:
                st("|level| > 15")


def combine_nc(na, nb):
    if na is not None and nb is not None:
        return (na + nb + 1) >> 1
    if na is not None:
        return na
    if nb is not None:
        return nb
    return 0


# ---------------------------------------------------------------------------
# Motion vectors (8.4.1)

def nb_motion(sd, mb, x, y, lx, done):
    """8.4.1.3.2 for luma location (x, y) of the current macroblock:
    returns (available, mv, refIdx)."""
    n, xw, yw = sd.nb4(mb, x, y)
    if n is None:
        return (False, (0, 0), -1)
    r = (yw >> 2) * 4 + (xw >> 2)
    if n is mb and not done[r]:
        return (False, (0, 0), -1)
    if n.is_intra or not n.pred_flag[lx][r]:
        return (True, (0, 0), -1)
    return (True, n.mv[lx][r], n.ref[lx][r])


def mv_pred(sd, mb, x, y, w, h, ref, lx, done, shape=None, part=0):
    """8.4.1.3: prediction for the partition at (x, y) of width w."""
    A = nb_motion(sd, mb, x - 1, y, lx, done)
    B = nb_motion(sd, mb, x, y - 1, lx, done)
    C = nb_motion(sd, mb, x + w, y - 1, lx, done)
    if not C[0]:
        C = nb_motion(sd, mb, x - 1, y - 1, lx, done)
    if shape == (16, 8):
        if part == 0 and B[2] == ref:
            return B[1]
        if part == 1 and A[2] == ref:
            return A[1]
    elif shape == (8, 16):
        if part == 0 and A[2] == ref:
            return A[1]
        if part == 1 and C[2] == ref:
            return C[1]
    if not B[0] and not C[0] and A[0]:
        B = C = A
    match = [N for N in (A, B, C) if N[2] == ref]
    if len(match) == 1:
        return match[0][1]
    return (median(A[1][0], B[1][0], C[1][0]), median(A[1][1], B[1][1], C[1][1]))


def set_motion(mb, x, y, w, h, lx, ref, mv, refpic):
    for yy in range(y >> 2, (y + h) >> 2):
        for xx in range(x >> 2, (x + w) >> 2):
            r = yy * 4 + xx
            mb.pred_flag[lx][r] = 1
            mb.ref[lx][r] = ref
            mb.mv[lx][r] = mv
            mb.refpic[lx][r] = refpic


def mark_done(done, x, y, w, h):
    for yy in range(y >> 2, (y + h) >> 2):
        for xx in range(x >> 2, (x + w) >> 2):
            done[yy * 4 + xx] = True


def ref_pic(sd, lx, ref):
    lst = sd.ref_list[lx]
    if ref < 0 or ref >= len(lst) or lst[ref] is None:
        raise DecodeError("ref_idx %d names no picture" % ref)
    return lst[ref]


def derive_motion(sd, mb):
    done = [False] * 16
    if mb.kind == "PSKIP":
        A = nb_motion(sd, mb, -1, 0, 0, done)
        B = nb_motion(sd, mb, 0, -1, 0, done)
        if not A[0] or not B[0] or (A[2] == 0 and A[1] == (0, 0)) or \
                (B[2] == 0 and B[1] == (0, 0)):
            mv = (0, 0)
        else:
            mv = mv_pred(sd, mb, 0, 0, 16, 16, 0, 0, done)
        set_motion(mb, 0, 0, 16, 16, 0, 0, mv, ref_pic(sd, 0, 0))
        mb.parts = [(0, 0, 16, 16)]
        return
    if mb.kind == "BSKIP" or mb.name == "B_Direct_16x16":
        direct_mb(sd, mb, done)
        return
    if mb.num_parts != 4:
        n = mb.num_parts
        w, h = mb.part_w, mb.part_h
        mb.parts = []
        for i in range(n):
            x = 0 if w == 16 else 8 * i
            y = 0 if h == 16 else 8 * i
            for lx in (0, 1):
                if mb.part_pred[i] in ("L" + str(lx), "Bi"):
                    ref = mb.part_ref[lx][i]
                    mvp = mv_pred(sd, mb, x, y, w, h, ref, lx, done, (w, h), i)
                    d = mb.part_mvd[lx][i]
                    mv = (mvp[0] + d[0], mvp[1] + d[1])
                    set_motion(mb, x, y, w, h, lx, ref, mv, ref_pic(sd, lx, ref))
                    set_mvd(mb, x, y, w, h, lx, d)
            mark_done(done, x, y, w, h)
            mb.parts.append((x, y, w, h))
        return
    mb.parts = []
    for i in range(4):
        x0, y0 = (i & 1) * 8, (i >> 1) * 8
        s = mb.sub
        name, n, sw, sh, pred = s[i]
        if name == "B_Direct_8x8":
            direct_8x8(sd, mb, i, done)
            mark_done(done, x0, y0, 8, 8)
            mb.parts.extend(mb.direct_parts)
            continue
        for j in range(n):
            if sw == 8 and sh == 8:
                x, y = x0, y0
            elif sw == 8:
                x, y = x0, y0 + 4 * j
            elif sh == 8:
                x, y = x0 + 4 * j, y0
            else:
                x, y = x0 + 4 * (j & 1), y0 + 4 * (j >> 1)
            for lx in (0, 1):
                if pred in ("L" + str(lx), "Bi"):
                    ref = mb.sub_ref[lx][i]
                    mvp = mv_pred(sd, mb, x, y, sw, sh, ref, lx, done)
                    d = mb.sub_mvd[lx][i][j]
                    mv = (mvp[0] + d[0], mvp[1] + d[1])
                    set_motion(mb, x, y, sw, sh, lx, ref, mv, ref_pic(sd, lx, ref))
                    set_mvd(mb, x, y, sw, sh, lx, d)
            mark_done(done, x, y, sw, sh)
            mb.parts.append((x, y, sw, sh))


def set_mvd(mb, x, y, w, h, lx, d):
    for yy in range(y >> 2, (y + h) >> 2):
        for xx in range(x >> 2, (x + w) >> 2):
            mb.mvd[lx][yy * 4 + xx] = d


# ---------------------------------------------------------------------------
# B slices: direct prediction (8.4.1.2) and implicit weights (8.4.2.3.1)

def min_positive(x, y):
    return min(x, y) if x >= 0 and y >= 0 else max(x, y)


def colocated(sd, mb, r):
    """8.4.1.2.1 for frames: (mvCol, refIdxCol, refPicCol, colPic, isIntra)
    of the 4x4 block at raster r of the co-located macroblock (r is already
    the corner block when direct_8x8_inference_flag is 1)."""
    col_pic = sd.ref_list[1][0]
    if col_pic is None:
        raise DecodeError("direct prediction without RefPicList1[0]")
    cm = col_pic.mbs[mb.addr]
    if cm.is_intra:
        return (0, 0), -1, None, col_pic
    if cm.pred_flag[0][r]:
        return cm.mv[0][r], cm.ref[0][r], cm.refpic[0][r], col_pic
    return cm.mv[1][r], cm.ref[1][r], cm.refpic[1][r], col_pic


def col_block(sd, r):
    """The 4x4 block of the co-located macroblock used for raster block r."""
    if sd.sps.direct_8x8_inference:
        bx, by = r & 3, r >> 2
        return (3 if by >= 2 else 0) * 4 + (3 if bx >= 2 else 0)
    return r


def spatial_setup(sd, mb):
    """The macroblock-wide part of 8.4.1.2.2: reference indices, the
    predicted vectors and directZeroPredictionFlag."""
    if getattr(mb, "spatial", None) is not None:
        return mb.spatial
    done = [False] * 16
    refs = []
    for lx in (0, 1):
        A = nb_motion(sd, mb, -1, 0, lx, done)
        B = nb_motion(sd, mb, 0, -1, lx, done)
        C = nb_motion(sd, mb, 16, -1, lx, done)
        if not C[0]:
            C = nb_motion(sd, mb, -1, -1, lx, done)
        refs.append(min_positive(A[2], min_positive(B[2], C[2])))
    zero = refs[0] < 0 and refs[1] < 0
    if zero:
        refs = [0, 0]
    mvp = []
    for lx in (0, 1):
        if refs[lx] >= 0 and not zero:
            mvp.append(mv_pred(sd, mb, 0, 0, 16, 16, refs[lx], lx, done))
        else:
            mvp.append((0, 0))
    mb.spatial = (refs, mvp, zero)
    return mb.spatial


def direct_block(sd, mb, r, x, y, w, h):
    """Direct prediction for the w x h block at (x, y) whose co-located
    block is decided by raster index r."""
    h_ = sd.h
    if h_.direct_spatial:
        refs, mvp, zero = spatial_setup(sd, mb)
        mv_col, ref_col, _, col_pic = colocated(sd, mb, col_block(sd, r))
        col_zero = (sd.ref_list[1][0].ref == "S" and ref_col == 0 and
                    -1 <= mv_col[0] <= 1 and -1 <= mv_col[1] <= 1)
        for lx in (0, 1):
            if refs[lx] < 0:
                continue
            if zero or (refs[lx] == 0 and col_zero):
                mv = (0, 0)
            else:
                mv = mvp[lx]
            set_motion(mb, x, y, w, h, lx, refs[lx], mv, ref_pic(sd, lx, refs[lx]))
        return
    # Temporal (8.4.1.2.3)
    mv_col, ref_col, ref_pic_col, col_pic = colocated(sd, mb, col_block(sd, r))
    l0 = sd.ref_list[0]
    if ref_col < 0:
        ref0 = 0
    else:
        ref0 = None
        for i, p in enumerate(l0):
            if p is ref_pic_col:
                ref0 = i
                break
        if ref0 is None:
            raise DecodeError("temporal direct: the co-located reference is not in list 0")
    pic0 = l0[ref0]
    pic1 = sd.ref_list[1][0]
    cur = sd.pic
    tb = clip3(-128, 127, cur.poc - pic0.poc)
    td = clip3(-128, 127, pic1.poc - pic0.poc)
    if pic0.ref == "L" or td == 0:
        mv0 = mv_col
        mv1 = (0, 0)
    else:
        tx = c_div(16384 + abs(c_div(td, 2)), td)
        dsf = clip3(-1024, 1023, (tb * tx + 32) >> 6)
        mv0 = ((dsf * mv_col[0] + 128) >> 8, (dsf * mv_col[1] + 128) >> 8)
        mv1 = (mv0[0] - mv_col[0], mv0[1] - mv_col[1])
    set_motion(mb, x, y, w, h, 0, ref0, mv0, pic0)
    set_motion(mb, x, y, w, h, 1, 0, mv1, pic1)


def direct_8x8(sd, mb, i, done):
    """One 8x8 quadrant predicted in direct mode (B_Skip, B_Direct_16x16 or
    B_Direct_8x8): sets the motion and mb.direct_parts."""
    x0, y0 = (i & 1) * 8, (i >> 1) * 8
    mb.direct_parts = []
    if sd.sps.direct_8x8_inference:
        direct_block(sd, mb, (y0 >> 2) * 4 + (x0 >> 2), x0, y0, 8, 8)
        mb.direct_parts.append((x0, y0, 8, 8))
    else:
        for j in range(4):
            x, y = x0 + 4 * (j & 1), y0 + 4 * (j >> 1)
            direct_block(sd, mb, (y >> 2) * 4 + (x >> 2), x, y, 4, 4)
            mb.direct_parts.append((x, y, 4, 4))
    mark_done(done, x0, y0, 8, 8)


def direct_mb(sd, mb, done):
    mb.parts = []
    for i in range(4):
        direct_8x8(sd, mb, i, done)
        mb.parts.extend(mb.direct_parts)


def implicit_weights(sd, mb, r0, r1):
    """8.4.2.3.1, implicit mode: (w0, w1) for a bi-predicted block."""
    pic0 = sd.ref_list[0][r0]
    pic1 = sd.ref_list[1][r1]
    cur = sd.pic
    tb = clip3(-128, 127, cur.poc - pic0.poc)
    td = clip3(-128, 127, pic1.poc - pic0.poc)
    if pic1.poc - pic0.poc == 0 or pic0.ref == "L" or pic1.ref == "L":
        return 32, 32
    tx = c_div(16384 + abs(c_div(td, 2)), td)
    dsf = clip3(-1024, 1023, (tb * tx + 32) >> 6)
    if (dsf >> 2) < -64 or (dsf >> 2) > 128:
        return 32, 32
    return 64 - (dsf >> 2), dsf >> 2


def c_div(a, b):
    """Integer division with truncation toward zero, the '/' of the
    Recommendation (5.7)."""
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


# ---------------------------------------------------------------------------
# Inter prediction samples (8.4.2)

def luma_block(ref, xa, ya, w, h, mvx, mvy):
    """8.4.2.2.1: a w x h block of luma prediction at absolute (xa, ya)."""
    W, H = ref.w, ref.h
    Y = ref.Y
    xi = xa + (mvx >> 2)
    yi = ya + (mvy >> 2)
    fx = mvx & 3
    fy = mvy & 3
    # Window of integer samples from (xi - 2, yi - 2), clamped (8-228/229).
    xs = [clip3(0, W - 1, xi + i - 2) for i in range(w + 5)]
    win = []
    for j in range(h + 5):
        row0 = clip3(0, H - 1, yi + j - 2) * W
        win.append([Y[row0 + x] for x in xs])
    if fx == 0 and fy == 0:
        return [win[j + 2][2:2 + w] for j in range(h)]

    def b1(i, j):  # horizontal half-sample intermediate at (i + 1/2, j)
        r = win[j + 2]
        return r[i] - 5 * r[i + 1] + 20 * r[i + 2] + 20 * r[i + 3] - 5 * r[i + 4] + r[i + 5]

    def h1(i, j):  # vertical half-sample intermediate at (i, j + 1/2)
        c = i + 2
        return (win[j][c] - 5 * win[j + 1][c] + 20 * win[j + 2][c] + 20 * win[j + 3][c]
                - 5 * win[j + 4][c] + win[j + 5][c])

    def G(i, j):
        return win[j + 2][i + 2]

    def bb(i, j):
        return clip1((b1(i, j) + 16) >> 5)

    def hh(i, j):
        return clip1((h1(i, j) + 16) >> 5)

    def jj(i, j):
        v = (b1(i, j - 2) - 5 * b1(i, j - 1) + 20 * b1(i, j) + 20 * b1(i, j + 1)
             - 5 * b1(i, j + 2) + b1(i, j + 3))
        return clip1((v + 512) >> 10)

    out = []
    for j in range(h):
        row = []
        for i in range(w):
            if fy == 0:
                b = bb(i, j)
                if fx == 1:
                    v = (G(i, j) + b + 1) >> 1
                elif fx == 2:
                    v = b
                else:
                    v = (G(i + 1, j) + b + 1) >> 1
            elif fx == 0:
                hv = hh(i, j)
                if fy == 1:
                    v = (G(i, j) + hv + 1) >> 1
                elif fy == 2:
                    v = hv
                else:
                    v = (G(i, j + 1) + hv + 1) >> 1
            elif fx == 2 and fy == 2:
                v = jj(i, j)
            elif fx == 2:
                # f (fy = 1) or q (fy = 3): average of j with b or s
                jv = jj(i, j)
                other = bb(i, j) if fy == 1 else bb(i, j + 1)
                v = (jv + other + 1) >> 1
            elif fy == 2:
                # i (fx = 1) or k (fx = 3): average of j with h or m
                jv = jj(i, j)
                other = hh(i, j) if fx == 1 else hh(i + 1, j)
                v = (jv + other + 1) >> 1
            else:
                # e, g, p, r: diagonal averages of b/s with h/m
                bv = bb(i, j) if fy == 1 else bb(i, j + 1)
                hv = hh(i, j) if fx == 1 else hh(i + 1, j)
                v = (bv + hv + 1) >> 1
            row.append(v)
        out.append(row)
    return out


def chroma_block(plane, cw, ch, xa, ya, w, h, mvx, mvy):
    """8.4.2.2.2 for 4:2:0: (xa, ya) in chroma samples, mv in luma units
    (1/8 chroma sample)."""
    xi = xa + (mvx >> 3)
    yi = ya + (mvy >> 3)
    fx = mvx & 7
    fy = mvy & 7
    xs = [clip3(0, cw - 1, xi + i) for i in range(w + 1)]
    rows = []
    for j in range(h + 1):
        r0 = clip3(0, ch - 1, yi + j) * cw
        rows.append([plane[r0 + x] for x in xs])
    c00 = (8 - fx) * (8 - fy)
    c10 = fx * (8 - fy)
    c01 = (8 - fx) * fy
    c11 = fx * fy
    out = []
    for j in range(h):
        a = rows[j]
        b = rows[j + 1]
        out.append([(c00 * a[i] + c10 * a[i + 1] + c01 * b[i] + c11 * b[i + 1] + 32) >> 6
                    for i in range(w)])
    return out


def inter_predict(sd, mb):
    """Builds mb.pred_y (16x16), mb.pred_u, mb.pred_v (8x8) for an inter MB."""
    py = [[0] * 16 for _ in range(16)]
    pu = [[0] * 8 for _ in range(8)]
    pv = [[0] * 8 for _ in range(8)]
    X0 = mb.mbx * 16
    Y0 = mb.mby * 16
    h = sd.h
    for (x, y, w, hh) in mb.parts:
        r = (y >> 2) * 4 + (x >> 2)
        preds = []
        for lx in (0, 1):
            if mb.pred_flag[lx][r]:
                ref = mb.refpic[lx][r]
                mv = mb.mv[lx][r]
                ly = luma_block(ref, X0 + x, Y0 + y, w, hh, mv[0], mv[1])
                lu = chroma_block(ref.U, ref.cw, ref.ch, (X0 + x) >> 1, (Y0 + y) >> 1,
                                  w >> 1, hh >> 1, mv[0], mv[1])
                lv = chroma_block(ref.V, ref.cw, ref.ch, (X0 + x) >> 1, (Y0 + y) >> 1,
                                  w >> 1, hh >> 1, mv[0], mv[1])
                preds.append((lx, mb.ref[lx][r], ly, lu, lv))
        out = weighted(sd, mb, preds, r)
        ly, lu, lv = out
        for j in range(hh):
            py[y + j][x:x + w] = ly[j]
        for j in range(hh >> 1):
            pu[(y >> 1) + j][x >> 1:(x + w) >> 1] = lu[j]
            pv[(y >> 1) + j][x >> 1:(x + w) >> 1] = lv[j]
    mb.pred_y, mb.pred_u, mb.pred_v = py, pu, pv


def weighted(sd, mb, preds, r):
    """8.4.2.3: default, explicit or implicit weighted sample prediction."""
    h = sd.h
    pps = h.pps
    if len(preds) == 1:
        lx, ref, ly, lu, lv = preds[0]
        if h.slice_type == SLICE_P and pps.weighted_pred or \
                h.slice_type == SLICE_B and pps.weighted_bipred_idc == 1:
            wp = h.wp
            lw, lo, cws, lf, cf = wp.w[lx][ref]
            return (explicit1(ly, lw, lo, wp.luma_log2),
                    explicit1(lu, cws[0][0], cws[0][1], wp.chroma_log2),
                    explicit1(lv, cws[1][0], cws[1][1], wp.chroma_log2))
        return ly, lu, lv
    (_, r0, ly0, lu0, lv0), (_, r1, ly1, lu1, lv1) = preds
    if h.slice_type == SLICE_B and pps.weighted_bipred_idc == 1:
        wp = h.wp
        a = wp.w[0][r0]
        b = wp.w[1][r1]
        return (explicit2(ly0, ly1, a[0], b[0], a[1], b[1], wp.luma_log2),
                explicit2(lu0, lu1, a[2][0][0], b[2][0][0], a[2][0][1], b[2][0][1], wp.chroma_log2),
                explicit2(lv0, lv1, a[2][1][0], b[2][1][0], a[2][1][1], b[2][1][1], wp.chroma_log2))
    if h.slice_type == SLICE_B and pps.weighted_bipred_idc == 2:
        w0, w1 = implicit_weights(sd, mb, r0, r1)
        return (explicit2(ly0, ly1, w0, w1, 0, 0, 5),
                explicit2(lu0, lu1, w0, w1, 0, 0, 5),
                explicit2(lv0, lv1, w0, w1, 0, 0, 5))
    return (avg2(ly0, ly1), avg2(lu0, lu1), avg2(lv0, lv1))


def avg2(a, b):
    return [[(x + y + 1) >> 1 for x, y in zip(ra, rb)] for ra, rb in zip(a, b)]


def explicit1(p, w, o, logwd):
    if logwd >= 1:
        rnd = 1 << (logwd - 1)
        return [[clip1(((x * w + rnd) >> logwd) + o) for x in row] for row in p]
    return [[clip1(x * w + o) for x in row] for row in p]


def explicit2(p0, p1, w0, w1, o0, o1, logwd):
    rnd = 1 << logwd
    o = (o0 + o1 + 1) >> 1
    return [[clip1(((x * w0 + y * w1 + rnd) >> (logwd + 1)) + o) for x, y in zip(ra, rb)]
            for ra, rb in zip(p0, p1)]


# ---------------------------------------------------------------------------
# Scaling and inverse transforms (8.5)

def chroma_qp(qp, offset):
    qpi = clip3(0, 51, qp + offset)
    return qpi if qpi < 30 else T.QPC_TABLE[qpi - 30]


class Scale:
    """LevelScale4x4/8x8 (8.5.9) for one picture parameter set."""

    def __init__(self, sps, pps):
        lists = FLAT_LISTS
        if sps.scaling_lists is not None:
            lists = sps.scaling_lists
        if pps.scaling_lists is not None:
            lists = pps.scaling_lists
        self.ls4 = []   # [list idx 0..5][m] -> 16 raster values
        for i in range(6):
            w = [0] * 16
            for k, pos in enumerate(T.ZIGZAG4):
                w[pos] = lists[i][k]
            per_m = []
            for m in range(6):
                v = T.V4[m]
                arr = [0] * 16
                for pos in range(16):
                    ii, jj = pos >> 2, pos & 3
                    if ii % 2 == 0 and jj % 2 == 0:
                        n = v[0]
                    elif ii % 2 == 1 and jj % 2 == 1:
                        n = v[1]
                    else:
                        n = v[2]
                    arr[pos] = w[pos] * n
                per_m.append(arr)
            self.ls4.append(per_m)
        self.ls8 = []   # [0 intra Y, 1 inter Y][m] -> 64 raster values
        for i in (6, 7):
            w = [0] * 64
            for k, pos in enumerate(T.ZIGZAG8):
                w[pos] = lists[i][k]
            per_m = []
            for m in range(6):
                v = T.V8[m]
                arr = [0] * 64
                for pos in range(64):
                    ii, jj = pos >> 3, pos & 7
                    if ii % 4 == 0 and jj % 4 == 0:
                        n = v[0]
                    elif ii % 2 == 1 and jj % 2 == 1:
                        n = v[1]
                    elif ii % 4 == 2 and jj % 4 == 2:
                        n = v[2]
                    elif (ii % 4 == 0 and jj % 2 == 1) or (ii % 2 == 1 and jj % 4 == 0):
                        n = v[3]
                    elif (ii % 4 == 0 and jj % 4 == 2) or (ii % 4 == 2 and jj % 4 == 0):
                        n = v[4]
                    else:
                        n = v[5]
                    arr[pos] = w[pos] * n
                per_m.append(arr)
            self.ls8.append(per_m)


_SCALES = {}


def get_scale(sps, pps):
    key = (id(sps), id(pps))
    s = _SCALES.get(key)
    if s is None:
        s = _SCALES[key] = Scale(sps, pps)
    return s


def idct4(d):
    """8.5.12.2 on a raster 4x4 list; returns the residual r (raster)."""
    f = [0] * 16
    for i in range(4):
        d0, d1, d2, d3 = d[4 * i], d[4 * i + 1], d[4 * i + 2], d[4 * i + 3]
        e0 = d0 + d2
        e1 = d0 - d2
        e2 = (d1 >> 1) - d3
        e3 = d1 + (d3 >> 1)
        f[4 * i] = e0 + e3
        f[4 * i + 1] = e1 + e2
        f[4 * i + 2] = e1 - e2
        f[4 * i + 3] = e0 - e3
    r = [0] * 16
    for j in range(4):
        f0, f1, f2, f3 = f[j], f[4 + j], f[8 + j], f[12 + j]
        g0 = f0 + f2
        g1 = f0 - f2
        g2 = (f1 >> 1) - f3
        g3 = f1 + (f3 >> 1)
        r[j] = (g0 + g3 + 32) >> 6
        r[4 + j] = (g1 + g2 + 32) >> 6
        r[8 + j] = (g1 - g2 + 32) >> 6
        r[12 + j] = (g0 - g3 + 32) >> 6
    return r


def idct8(d):
    """8.5.13.2 on a raster 8x8 list."""
    def one(v):
        a0 = v[0] + v[4]
        a4 = v[0] - v[4]
        a2 = (v[2] >> 1) - v[6]
        a6 = v[2] + (v[6] >> 1)
        b0 = a0 + a6
        b2 = a4 + a2
        b4 = a4 - a2
        b6 = a0 - a6
        a1 = -v[3] + v[5] - v[7] - (v[7] >> 1)
        a3 = v[1] + v[7] - v[3] - (v[3] >> 1)
        a5 = -v[1] + v[7] + v[5] + (v[5] >> 1)
        a7 = v[3] + v[5] + v[1] + (v[1] >> 1)
        b1 = a1 + (a7 >> 2)
        b7 = a7 - (a1 >> 2)
        b3 = a3 + (a5 >> 2)
        b5 = (a3 >> 2) - a5
        return [b0 + b7, b2 + b5, b4 + b3, b6 + b1, b6 - b1, b4 - b3, b2 - b5, b0 - b7]
    g = [0] * 64
    for i in range(8):
        g[8 * i:8 * i + 8] = one(d[8 * i:8 * i + 8])
    r = [0] * 64
    for j in range(8):
        col = one([g[8 * i + j] for i in range(8)])
        for i in range(8):
            r[8 * i + j] = (col[i] + 32) >> 6
    return r


def dequant4(coef_scan, qp, ls, dc=None):
    """8.5.12.1: coef_scan in zig-zag order -> scaled raster d. When dc is
    given, it is the already-scaled DC (Intra16x16 or chroma)."""
    d = [0] * 16
    m = ls[qp % 6]
    q6 = qp // 6
    for k in range(16):
        c = coef_scan[k]
        if c:
            pos = T.ZIGZAG4[k]
            if qp >= 24:
                d[pos] = (c * m[pos]) << (q6 - 4)
            else:
                d[pos] = (c * m[pos] + (1 << (3 - q6))) >> (4 - q6)
    if dc is not None:
        d[0] = dc
    return d


def dequant8(coef_scan, qp, ls):
    d = [0] * 64
    m = ls[qp % 6]
    q6 = qp // 6
    for k in range(64):
        c = coef_scan[k]
        if c:
            pos = T.ZIGZAG8[k]
            if qp >= 36:
                d[pos] = (c * m[pos]) << (q6 - 6)
            else:
                d[pos] = (c * m[pos] + (1 << (5 - q6))) >> (6 - q6)
    return d


def luma_dc16(dc_scan, qp, ls):
    """8.5.10: Intra16x16 DC (zig-zag list) -> dcY in raster order."""
    c = [0] * 16
    for k in range(16):
        c[T.ZIGZAG4[k]] = dc_scan[k]
    # f = H c H with H = [[1,1,1,1],[1,1,-1,-1],[1,-1,-1,1],[1,-1,1,-1]]
    t = [0] * 16
    for i in range(4):
        a, b, cc, d = c[4 * i], c[4 * i + 1], c[4 * i + 2], c[4 * i + 3]
        t[4 * i] = a + b + cc + d
        t[4 * i + 1] = a + b - cc - d
        t[4 * i + 2] = a - b - cc + d
        t[4 * i + 3] = a - b + cc - d
    f = [0] * 16
    for j in range(4):
        a, b, cc, d = t[j], t[4 + j], t[8 + j], t[12 + j]
        f[j] = a + b + cc + d
        f[4 + j] = a + b - cc - d
        f[8 + j] = a - b - cc + d
        f[12 + j] = a - b + cc - d
    s = ls[qp % 6][0]
    q6 = qp // 6
    if qp >= 36:
        return [(v * s) << (q6 - 6) for v in f]
    return [(v * s + (1 << (5 - q6))) >> (6 - q6) for v in f]


def chroma_dc420(c, qp, ls):
    """8.5.11 for 4:2:0: c = [c00, c01, c10, c11] -> dcC (same order)."""
    a, b, cc, d = c
    f = [a + b + cc + d, a - b + cc - d, a + b - cc - d, a - b - cc + d]
    s = ls[qp % 6][0]
    return [((v * s) << (qp // 6)) >> 5 for v in f]


# ---------------------------------------------------------------------------
# Intra prediction (8.3)

def intra_avail(sd, n):
    return n is not None and (n.is_intra or not sd.constrained)


def pred_intra4(sd, mb, blk, mode):
    """8.3.1.2: returns a 4x4 list of rows."""
    pic = sd.pic
    W = pic.w
    Y = pic.Y
    x0, y0 = BLK_X[blk], BLK_Y[blk]
    X = mb.mbx * 16 + x0
    Yy = mb.mby * 16 + y0
    top_ok = True if y0 > 0 else intra_avail(sd, mb.mbB)
    left_ok = True if x0 > 0 else intra_avail(sd, mb.mbA)
    if x0 > 0 and y0 > 0:
        tl_ok = True
    elif x0 > 0:
        tl_ok = intra_avail(sd, mb.mbB)
    elif y0 > 0:
        tl_ok = intra_avail(sd, mb.mbA)
    else:
        tl_ok = intra_avail(sd, mb.mbD)
    if y0 == 0:
        tr_ok = intra_avail(sd, mb.mbB) if x0 + 4 < 16 else intra_avail(sd, mb.mbC)
    else:
        tr_ok = x0 + 4 < 16 and blk not in (3, 11)
    top = [0] * 8
    if top_ok:
        base = (Yy - 1) * W + X
        for i in range(4):
            top[i] = Y[base + i]
        if tr_ok:
            for i in range(4, 8):
                top[i] = Y[base + i]
        else:
            for i in range(4, 8):
                top[i] = top[3]
    left = [0] * 4
    if left_ok:
        for j in range(4):
            left[j] = Y[(Yy + j) * W + X - 1]
    tl = Y[(Yy - 1) * W + X - 1] if tl_ok else 0
    return pred4_from(mode, top, left, tl, top_ok, left_ok, tl_ok, tr_ok or top_ok)


def pred4_from(mode, top, left, tl, top_ok, left_ok, tl_ok, tr_ok):
    def P(x, y):
        if y == -1:
            return tl if x == -1 else top[x]
        return left[y]
    out = [[0] * 4 for _ in range(4)]
    if mode == 0:
        if not top_ok:
            raise DecodeError("Intra4x4 vertical without the samples above")
        for y in range(4):
            out[y] = top[:4]
    elif mode == 1:
        if not left_ok:
            raise DecodeError("Intra4x4 horizontal without the samples left")
        for y in range(4):
            out[y] = [left[y]] * 4
    elif mode == 2:
        if top_ok and left_ok:
            v = (sum(top[:4]) + sum(left) + 4) >> 3
        elif left_ok:
            v = (sum(left) + 2) >> 2
        elif top_ok:
            v = (sum(top[:4]) + 2) >> 2
        else:
            v = 128
        for y in range(4):
            out[y] = [v] * 4
    elif mode == 3:
        if not top_ok:
            raise DecodeError("Intra4x4 diagonal down left without the samples above")
        for y in range(4):
            for x in range(4):
                if x == 3 and y == 3:
                    out[y][x] = (top[6] + 3 * top[7] + 2) >> 2
                else:
                    out[y][x] = (top[x + y] + 2 * top[x + y + 1] + top[x + y + 2] + 2) >> 2
    elif mode == 4:
        if not (top_ok and left_ok and tl_ok):
            raise DecodeError("Intra4x4 diagonal down right without its samples")
        for y in range(4):
            for x in range(4):
                if x > y:
                    out[y][x] = (P(x - y - 2, -1) + 2 * P(x - y - 1, -1) + P(x - y, -1) + 2) >> 2
                elif x < y:
                    out[y][x] = (P(-1, y - x - 2) + 2 * P(-1, y - x - 1) + P(-1, y - x) + 2) >> 2
                else:
                    out[y][x] = (P(0, -1) + 2 * P(-1, -1) + P(-1, 0) + 2) >> 2
    elif mode == 5:
        if not (top_ok and left_ok and tl_ok):
            raise DecodeError("Intra4x4 vertical right without its samples")
        for y in range(4):
            for x in range(4):
                z = 2 * x - y
                if z >= 0 and z % 2 == 0:
                    out[y][x] = (P(x - (y >> 1) - 1, -1) + P(x - (y >> 1), -1) + 1) >> 1
                elif z >= 0:
                    out[y][x] = (P(x - (y >> 1) - 2, -1) + 2 * P(x - (y >> 1) - 1, -1)
                                 + P(x - (y >> 1), -1) + 2) >> 2
                elif z == -1:
                    out[y][x] = (P(-1, 0) + 2 * P(-1, -1) + P(0, -1) + 2) >> 2
                else:
                    out[y][x] = (P(-1, y - 1) + 2 * P(-1, y - 2) + P(-1, y - 3) + 2) >> 2
    elif mode == 6:
        if not (top_ok and left_ok and tl_ok):
            raise DecodeError("Intra4x4 horizontal down without its samples")
        for y in range(4):
            for x in range(4):
                z = 2 * y - x
                if z >= 0 and z % 2 == 0:
                    out[y][x] = (P(-1, y - (x >> 1) - 1) + P(-1, y - (x >> 1)) + 1) >> 1
                elif z >= 0:
                    out[y][x] = (P(-1, y - (x >> 1) - 2) + 2 * P(-1, y - (x >> 1) - 1)
                                 + P(-1, y - (x >> 1)) + 2) >> 2
                elif z == -1:
                    out[y][x] = (P(-1, 0) + 2 * P(-1, -1) + P(0, -1) + 2) >> 2
                else:
                    out[y][x] = (P(x - 1, -1) + 2 * P(x - 2, -1) + P(x - 3, -1) + 2) >> 2
    elif mode == 7:
        if not top_ok:
            raise DecodeError("Intra4x4 vertical left without the samples above")
        for y in range(4):
            for x in range(4):
                i = x + (y >> 1)
                if y % 2 == 0:
                    out[y][x] = (top[i] + top[i + 1] + 1) >> 1
                else:
                    out[y][x] = (top[i] + 2 * top[i + 1] + top[i + 2] + 2) >> 2
    elif mode == 8:
        if not left_ok:
            raise DecodeError("Intra4x4 horizontal up without the samples left")
        for y in range(4):
            for x in range(4):
                z = x + 2 * y
                if z < 5 and z % 2 == 0:
                    out[y][x] = (left[y + (x >> 1)] + left[y + (x >> 1) + 1] + 1) >> 1
                elif z < 5:
                    out[y][x] = (left[y + (x >> 1)] + 2 * left[y + (x >> 1) + 1]
                                 + left[y + (x >> 1) + 2] + 2) >> 2
                elif z == 5:
                    out[y][x] = (left[2] + 3 * left[3] + 2) >> 2
                else:
                    out[y][x] = left[3]
    else:
        raise DecodeError("bad Intra4x4PredMode")
    return out


def pred_intra16(sd, mb, mode):
    """8.3.3: returns 16 rows of 16."""
    pic = sd.pic
    W = pic.w
    Y = pic.Y
    X = mb.mbx * 16
    Yy = mb.mby * 16
    top_ok = intra_avail(sd, mb.mbB)
    left_ok = intra_avail(sd, mb.mbA)
    tl_ok = intra_avail(sd, mb.mbD)
    top = list(Y[(Yy - 1) * W + X:(Yy - 1) * W + X + 16]) if top_ok else None
    left = [Y[(Yy + j) * W + X - 1] for j in range(16)] if left_ok else None
    if mode == 0:
        if not top_ok:
            raise DecodeError("Intra16x16 vertical without the samples above")
        return [top[:] for _ in range(16)]
    if mode == 1:
        if not left_ok:
            raise DecodeError("Intra16x16 horizontal without the samples left")
        return [[left[y]] * 16 for y in range(16)]
    if mode == 2:
        if top_ok and left_ok:
            v = (sum(top) + sum(left) + 16) >> 5
        elif left_ok:
            v = (sum(left) + 8) >> 4
        elif top_ok:
            v = (sum(top) + 8) >> 4
        else:
            v = 128
        return [[v] * 16 for _ in range(16)]
    if mode == 3:
        if not (top_ok and left_ok and tl_ok):
            raise DecodeError("Intra16x16 plane without its samples")
        tl = Y[(Yy - 1) * W + X - 1]

        def pt(x):
            return tl if x < 0 else top[x]

        def pl(y):
            return tl if y < 0 else left[y]
        H = sum((x + 1) * (pt(8 + x) - pt(6 - x)) for x in range(8))
        V = sum((y + 1) * (pl(8 + y) - pl(6 - y)) for y in range(8))
        a = 16 * (left[15] + top[15])
        b = (5 * H + 32) >> 6
        c = (5 * V + 32) >> 6
        return [[clip1((a + b * (x - 7) + c * (y - 7) + 16) >> 5) for x in range(16)]
                for y in range(16)]
    raise DecodeError("bad Intra16x16PredMode")


def pred_chroma(sd, mb, mode, plane):
    """8.3.4 for 4:2:0: returns 8 rows of 8."""
    pic = sd.pic
    W = pic.cw
    X = mb.mbx * 8
    Yy = mb.mby * 8
    top_ok = intra_avail(sd, mb.mbB)
    left_ok = intra_avail(sd, mb.mbA)
    tl_ok = intra_avail(sd, mb.mbD)
    top = list(plane[(Yy - 1) * W + X:(Yy - 1) * W + X + 8]) if top_ok else None
    left = [plane[(Yy + j) * W + X - 1] for j in range(8)] if left_ok else None
    if mode == 0:
        out = [[0] * 8 for _ in range(8)]
        for by in range(2):
            for bx in range(2):
                xo, yo = bx * 4, by * 4
                st = sum(top[xo:xo + 4]) if top_ok else 0
                sl = sum(left[yo:yo + 4]) if left_ok else 0
                if (xo == 0 and yo == 0) or (xo > 0 and yo > 0):
                    if top_ok and left_ok:
                        v = (st + sl + 4) >> 3
                    elif left_ok:
                        v = (sl + 2) >> 2
                    elif top_ok:
                        v = (st + 2) >> 2
                    else:
                        v = 128
                elif xo > 0 and yo == 0:
                    if top_ok:
                        v = (st + 2) >> 2
                    elif left_ok:
                        v = (sl + 2) >> 2
                    else:
                        v = 128
                else:
                    if left_ok:
                        v = (sl + 2) >> 2
                    elif top_ok:
                        v = (st + 2) >> 2
                    else:
                        v = 128
                for y in range(4):
                    for x in range(4):
                        out[yo + y][xo + x] = v
        return out
    if mode == 1:
        if not left_ok:
            raise DecodeError("chroma horizontal without the samples left")
        return [[left[y]] * 8 for y in range(8)]
    if mode == 2:
        if not top_ok:
            raise DecodeError("chroma vertical without the samples above")
        return [top[:] for _ in range(8)]
    if mode == 3:
        if not (top_ok and left_ok and tl_ok):
            raise DecodeError("chroma plane without its samples")
        tl = plane[(Yy - 1) * W + X - 1]

        def pt(x):
            return tl if x < 0 else top[x]

        def pl(y):
            return tl if y < 0 else left[y]
        H = sum((x + 1) * (pt(4 + x) - pt(2 - x)) for x in range(4))
        V = sum((y + 1) * (pl(4 + y) - pl(2 - y)) for y in range(4))
        a = 16 * (left[7] + top[7])
        b = (34 * H + 32) >> 6
        c = (34 * V + 32) >> 6
        return [[clip1((a + b * (x - 3) + c * (y - 3) + 16) >> 5) for x in range(8)]
                for y in range(8)]
    raise DecodeError("bad intra_chroma_pred_mode")


def pred_intra8(sd, mb, b8, mode):
    """8.3.2.2: Intra_8x8 with reference sample filtering."""
    pic = sd.pic
    W = pic.w
    Y = pic.Y
    x0, y0 = (b8 & 1) * 8, (b8 >> 1) * 8
    X = mb.mbx * 16 + x0
    Yy = mb.mby * 16 + y0
    top_ok = True if y0 > 0 else intra_avail(sd, mb.mbB)
    left_ok = True if x0 > 0 else intra_avail(sd, mb.mbA)
    if x0 > 0 and y0 > 0:
        tl_ok = True
    elif x0 > 0:
        tl_ok = intra_avail(sd, mb.mbB)
    elif y0 > 0:
        tl_ok = intra_avail(sd, mb.mbA)
    else:
        tl_ok = intra_avail(sd, mb.mbD)
    if y0 == 0:
        tr_ok = intra_avail(sd, mb.mbB) if x0 == 0 else intra_avail(sd, mb.mbC)
    else:
        tr_ok = x0 == 0  # block 2's top right is block 1; block 3 has none
    p_top = [0] * 16
    if top_ok:
        base = (Yy - 1) * W + X
        for i in range(8):
            p_top[i] = Y[base + i]
        for i in range(8, 16):
            p_top[i] = Y[base + i] if tr_ok else p_top[7]
    p_left = [Y[(Yy + j) * W + X - 1] for j in range(8)] if left_ok else [0] * 8
    p_tl = Y[(Yy - 1) * W + X - 1] if tl_ok else 0
    # 8.3.2.2.1 reference sample filtering
    ft = [0] * 16
    if top_ok:
        if tl_ok:
            ft[0] = (p_tl + 2 * p_top[0] + p_top[1] + 2) >> 2
        else:
            ft[0] = (3 * p_top[0] + p_top[1] + 2) >> 2
        for x in range(1, 15):
            ft[x] = (p_top[x - 1] + 2 * p_top[x] + p_top[x + 1] + 2) >> 2
        ft[15] = (p_top[14] + 3 * p_top[15] + 2) >> 2
    ftl = 0
    if tl_ok:
        if top_ok and left_ok:
            ftl = (p_top[0] + 2 * p_tl + p_left[0] + 2) >> 2
        elif top_ok:
            ftl = (3 * p_tl + p_top[0] + 2) >> 2
        elif left_ok:
            ftl = (3 * p_tl + p_left[0] + 2) >> 2
        else:
            ftl = p_tl
    fl = [0] * 8
    if left_ok:
        if tl_ok:
            fl[0] = (p_tl + 2 * p_left[0] + p_left[1] + 2) >> 2
        else:
            fl[0] = (3 * p_left[0] + p_left[1] + 2) >> 2
        for y in range(1, 7):
            fl[y] = (p_left[y - 1] + 2 * p_left[y] + p_left[y + 1] + 2) >> 2
        fl[7] = (p_left[6] + 3 * p_left[7] + 2) >> 2

    def P(x, y):
        if y == -1:
            return ftl if x == -1 else ft[x]
        return fl[y]
    out = [[0] * 8 for _ in range(8)]
    if mode == 0:
        for y in range(8):
            out[y] = ft[:8]
    elif mode == 1:
        for y in range(8):
            out[y] = [fl[y]] * 8
    elif mode == 2:
        if top_ok and left_ok:
            v = (sum(ft[:8]) + sum(fl) + 8) >> 4
        elif left_ok:
            v = (sum(fl) + 4) >> 3
        elif top_ok:
            v = (sum(ft[:8]) + 4) >> 3
        else:
            v = 128
        for y in range(8):
            out[y] = [v] * 8
    elif mode == 3:
        for y in range(8):
            for x in range(8):
                if x == 7 and y == 7:
                    out[y][x] = (ft[14] + 3 * ft[15] + 2) >> 2
                else:
                    out[y][x] = (ft[x + y] + 2 * ft[x + y + 1] + ft[x + y + 2] + 2) >> 2
    elif mode == 4:
        for y in range(8):
            for x in range(8):
                if x > y:
                    out[y][x] = (P(x - y - 2, -1) + 2 * P(x - y - 1, -1) + P(x - y, -1) + 2) >> 2
                elif x < y:
                    out[y][x] = (P(-1, y - x - 2) + 2 * P(-1, y - x - 1) + P(-1, y - x) + 2) >> 2
                else:
                    out[y][x] = (P(0, -1) + 2 * P(-1, -1) + P(-1, 0) + 2) >> 2
    elif mode == 5:
        for y in range(8):
            for x in range(8):
                z = 2 * x - y
                if z >= 0 and z % 2 == 0:
                    out[y][x] = (P(x - (y >> 1) - 1, -1) + P(x - (y >> 1), -1) + 1) >> 1
                elif z >= 0:
                    out[y][x] = (P(x - (y >> 1) - 2, -1) + 2 * P(x - (y >> 1) - 1, -1)
                                 + P(x - (y >> 1), -1) + 2) >> 2
                elif z == -1:
                    out[y][x] = (P(-1, 0) + 2 * P(-1, -1) + P(0, -1) + 2) >> 2
                else:
                    out[y][x] = (P(-1, y - 2 * x - 1) + 2 * P(-1, y - 2 * x - 2)
                                 + P(-1, y - 2 * x - 3) + 2) >> 2
    elif mode == 6:
        for y in range(8):
            for x in range(8):
                z = 2 * y - x
                if z >= 0 and z % 2 == 0:
                    out[y][x] = (P(-1, y - (x >> 1) - 1) + P(-1, y - (x >> 1)) + 1) >> 1
                elif z >= 0:
                    out[y][x] = (P(-1, y - (x >> 1) - 2) + 2 * P(-1, y - (x >> 1) - 1)
                                 + P(-1, y - (x >> 1)) + 2) >> 2
                elif z == -1:
                    out[y][x] = (P(-1, 0) + 2 * P(-1, -1) + P(0, -1) + 2) >> 2
                else:
                    out[y][x] = (P(x - 2 * y - 1, -1) + 2 * P(x - 2 * y - 2, -1)
                                 + P(x - 2 * y - 3, -1) + 2) >> 2
    elif mode == 7:
        for y in range(8):
            for x in range(8):
                i = x + (y >> 1)
                if y % 2 == 0:
                    out[y][x] = (ft[i] + ft[i + 1] + 1) >> 1
                else:
                    out[y][x] = (ft[i] + 2 * ft[i + 1] + ft[i + 2] + 2) >> 2
    elif mode == 8:
        for y in range(8):
            for x in range(8):
                z = x + 2 * y
                if z < 13 and z % 2 == 0:
                    out[y][x] = (fl[y + (x >> 1)] + fl[y + (x >> 1) + 1] + 1) >> 1
                elif z < 13:
                    out[y][x] = (fl[y + (x >> 1)] + 2 * fl[y + (x >> 1) + 1]
                                 + fl[y + (x >> 1) + 2] + 2) >> 2
                elif z == 13:
                    out[y][x] = (fl[6] + 3 * fl[7] + 2) >> 2
                else:
                    out[y][x] = fl[7]
    else:
        raise DecodeError("bad Intra8x8PredMode")
    return out


# ---------------------------------------------------------------------------
# Reconstruction

def write_block(plane, W, X, Y, rows):
    for j, row in enumerate(rows):
        o = (Y + j) * W + X
        plane[o:o + len(row)] = bytes(row)


def write_pcm(sd, mb):
    pic = sd.pic
    s = mb.pcm
    X, Y = mb.mbx * 16, mb.mby * 16
    write_block(pic.Y, pic.w, X, Y, [s[16 * j:16 * j + 16] for j in range(16)])
    write_block(pic.U, pic.cw, X >> 1, Y >> 1, [s[256 + 8 * j:256 + 8 * j + 8] for j in range(8)])
    write_block(pic.V, pic.cw, X >> 1, Y >> 1, [s[320 + 8 * j:320 + 8 * j + 8] for j in range(8)])
    mb.res_y = [[0] * 16 for _ in range(16)]
    mb.res_u = [[0] * 8 for _ in range(8)]
    mb.res_v = [[0] * 8 for _ in range(8)]


def luma_residuals(sd, mb):
    """Residual 16x16 (rows) for a non-Intra16x16 MB."""
    scale = get_scale(sd.sps, sd.pps)
    res = [[0] * 16 for _ in range(16)]
    inter = 0 if mb.is_intra else 1
    if mb.t8:
        ls = scale.ls8[inter]
        for b8 in range(4):
            lv = mb.coef8[b8]
            if lv is None:
                continue
            r = idct8(dequant8(lv, mb.qp, ls))
            x0, y0 = (b8 & 1) * 8, (b8 >> 1) * 8
            for j in range(8):
                res[y0 + j][x0:x0 + 8] = r[8 * j:8 * j + 8]
        return res
    ls = scale.ls4[0 + 3 * inter]
    for rr in range(16):
        c = mb.coef[rr]
        if c is None:
            continue
        r = idct4(dequant4(c, mb.qp, ls))
        x0, y0 = (rr & 3) * 4, (rr >> 2) * 4
        for j in range(4):
            res[y0 + j][x0:x0 + 4] = r[4 * j:4 * j + 4]
    return res


def chroma_residuals(sd, mb):
    scale = get_scale(sd.sps, sd.pps)
    inter = 0 if mb.is_intra else 1
    out = []
    for c in range(2):
        off = sd.pps.chroma_qp_index_offset if c == 0 else sd.pps.second_chroma_qp_index_offset
        qpc = chroma_qp(mb.qp, off)
        ls = scale.ls4[1 + c + 3 * inter]
        res = [[0] * 8 for _ in range(8)]
        dcs = mb.cdc[c] if mb.cbp >> 4 else [0, 0, 0, 0]
        if any(dcs):
            dcc = chroma_dc420(dcs, qpc, ls)
        else:
            dcc = [0, 0, 0, 0]
        for r in range(4):
            ac = mb.cac[c][r] if (mb.cbp >> 4) & 2 else None
            if ac is None and dcc[r] == 0:
                continue
            if ac is None:
                ac = [0] * 16
            rr = idct4(dequant4(ac, qpc, ls, dc=dcc[r]))
            x0, y0 = (r & 1) * 4, (r >> 1) * 4
            for j in range(4):
                res[y0 + j][x0:x0 + 4] = rr[4 * j:4 * j + 4]
        out.append(res)
    return out


def add_clip(pred, res):
    return [[clip1(p + r) for p, r in zip(pr, rr)] for pr, rr in zip(pred, res)]


def intra_reconstruct(sd, mb):
    pic = sd.pic
    X, Y = mb.mbx * 16, mb.mby * 16
    scale = get_scale(sd.sps, sd.pps)
    pred_y = [[0] * 16 for _ in range(16)]
    if mb.kind == "I16":
        pred = pred_intra16(sd, mb, mb.i16_mode)
        ls = scale.ls4[0]
        res = [[0] * 16 for _ in range(16)]
        dc = luma_dc16(mb.dc16, mb.qp, ls) if any(mb.dc16) else [0] * 16
        for rr in range(16):
            ac = mb.coef[rr]
            if ac is None and dc[rr] == 0:
                continue
            if ac is None:
                ac = [0] * 16
            r = idct4(dequant4(ac, mb.qp, ls, dc=dc[rr]))
            x0, y0 = (rr & 3) * 4, (rr >> 2) * 4
            for j in range(4):
                res[y0 + j][x0:x0 + 4] = r[4 * j:4 * j + 4]
        write_block(pic.Y, pic.w, X, Y, add_clip(pred, res))
        pred_y = pred
    elif mb.kind == "I4":
        ls = scale.ls4[0]
        res = [[0] * 16 for _ in range(16)]
        for blk in range(16):
            rr = BLK_RASTER[blk]
            x0, y0 = BLK_X[blk], BLK_Y[blk]
            pred = pred_intra4(sd, mb, blk, mb.modes4[rr])
            c = mb.coef[rr]
            if c is not None:
                r = idct4(dequant4(c, mb.qp, ls))
                rows = [r[4 * j:4 * j + 4] for j in range(4)]
            else:
                rows = [[0] * 4 for _ in range(4)]
            write_block(pic.Y, pic.w, X + x0, Y + y0, add_clip(pred, rows))
            for j in range(4):
                res[y0 + j][x0:x0 + 4] = rows[j]
                pred_y[y0 + j][x0:x0 + 4] = pred[j]
    else:  # I8
        ls = scale.ls8[0]
        res = [[0] * 16 for _ in range(16)]
        for b8 in range(4):
            x0, y0 = (b8 & 1) * 8, (b8 >> 1) * 8
            pred = pred_intra8(sd, mb, b8, mb.modes8[b8])
            lv = mb.coef8[b8]
            if lv is not None:
                r = idct8(dequant8(lv, mb.qp, ls))
                rows = [r[8 * j:8 * j + 8] for j in range(8)]
            else:
                rows = [[0] * 8 for _ in range(8)]
            write_block(pic.Y, pic.w, X + x0, Y + y0, add_clip(pred, rows))
            for j in range(8):
                res[y0 + j][x0:x0 + 8] = rows[j]
                pred_y[y0 + j][x0:x0 + 8] = pred[j]
    mb.res_y = res
    mb.pred_y = pred_y
    pu = pred_chroma(sd, mb, mb.chroma_mode, pic.U)
    pv = pred_chroma(sd, mb, mb.chroma_mode, pic.V)
    ru, rv = chroma_residuals(sd, mb)
    write_block(pic.U, pic.cw, X >> 1, Y >> 1, add_clip(pu, ru))
    write_block(pic.V, pic.cw, X >> 1, Y >> 1, add_clip(pv, rv))
    mb.res_u, mb.res_v = ru, rv
    mb.pred_u, mb.pred_v = pu, pv


def inter_reconstruct(sd, mb):
    pic = sd.pic
    X, Y = mb.mbx * 16, mb.mby * 16
    if mb.skip:
        ry = [[0] * 16 for _ in range(16)]
        ru = [[0] * 8 for _ in range(8)]
        rv = [[0] * 8 for _ in range(8)]
        mb.coef = [None] * 16
        mb.cdc = [[0] * 4, [0] * 4]
        mb.cac = [[None] * 4, [None] * 4]
    else:
        ry = luma_residuals(sd, mb)
        ru, rv = chroma_residuals(sd, mb)
    write_block(pic.Y, pic.w, X, Y, add_clip(mb.pred_y, ry))
    write_block(pic.U, pic.cw, X >> 1, Y >> 1, add_clip(mb.pred_u, ru))
    write_block(pic.V, pic.cw, X >> 1, Y >> 1, add_clip(mb.pred_v, rv))
    mb.res_y, mb.res_u, mb.res_v = ry, ru, rv


# ---------------------------------------------------------------------------
# Deblocking (8.7)

def mv_differs(p, bp, q, bq):
    P = [(p.refpic[lx][bp], p.mv[lx][bp]) for lx in (0, 1) if p.pred_flag[lx][bp]]
    Q = [(q.refpic[lx][bq], q.mv[lx][bq]) for lx in (0, 1) if q.pred_flag[lx][bq]]
    if len(P) != len(Q):
        return True

    def far(a, b):
        return abs(a[0] - b[0]) >= 4 or abs(a[1] - b[1]) >= 4
    if len(P) == 1:
        if P[0][0] is not Q[0][0]:
            return True
        return far(P[0][1], Q[0][1])
    (pa, pma), (pb, pmb) = P
    (qa, qma), (qb, qmb) = Q
    if sorted([id(pa), id(pb)]) != sorted([id(qa), id(qb)]):
        return True
    if pa is not pb:
        if qa is pa:
            return far(pma, qma) or far(pmb, qmb)
        return far(pma, qmb) or far(pmb, qma)
    # Both use the same picture twice.
    return (far(pma, qma) or far(pmb, qmb)) and (far(pma, qmb) or far(pmb, qma))


def bs_for(p, bp, q, bq, mb_edge):
    if p.is_intra or q.is_intra:
        return 4 if mb_edge else 3
    if p.nz[bp] or q.nz[bq]:
        return 2
    return 1 if mv_differs(p, bp, q, bq) else 0


def compute_bs(dec, pic, mb):
    """bS for the 4 vertical and 4 horizontal luma edges of mb:
    bs[dir][edge][k], k = 4-sample segment along the edge. Edges that are
    not filtered get None."""
    h = mb.hdr
    W = pic.w // 16
    bs = [[None] * 4, [None] * 4]
    if h.disable_deblocking == 1:
        return bs
    left = pic.mbs[mb.addr - 1] if mb.mbx > 0 else None
    top = pic.mbs[mb.addr - W] if mb.mby > 0 else None
    if h.disable_deblocking == 2:
        if left is not None and left.slice_num != mb.slice_num:
            left = None
        if top is not None and top.slice_num != mb.slice_num:
            top = None
    for e in range(4):
        if e > 0 and mb.t8 and e % 2:
            continue
        # vertical edge at x = 4e
        if e == 0 and left is None:
            pass
        else:
            row = []
            for k in range(4):
                bq = k * 4 + e
                if e == 0:
                    row.append(bs_for(left, k * 4 + 3, mb, bq, True))
                else:
                    row.append(bs_for(mb, bq - 1, mb, bq, False))
            bs[0][e] = row
        if e == 0 and top is None:
            pass
        else:
            row = []
            for k in range(4):
                bq = e * 4 + k
                if e == 0:
                    row.append(bs_for(top, 12 + k, mb, bq, True))
                else:
                    row.append(bs_for(mb, bq - 4, mb, bq, False))
            bs[1][e] = row
    return bs, left, top


def filter_line(s, idx, step, bS, alpha, beta, tc0, chroma):
    """8.7.2.3 / 8.7.2.4 on samples s[idx + i*step] (q side) and
    s[idx - (i+1)*step] (p side)."""
    p0 = s[idx - step]
    q0 = s[idx]
    if abs(p0 - q0) >= alpha:
        return
    p1 = s[idx - 2 * step]
    q1 = s[idx + step]
    if abs(p1 - p0) >= beta or abs(q1 - q0) >= beta:
        return
    if chroma:
        if bS < 4:
            tc = tc0 + 1
            delta = clip3(-tc, tc, (((q0 - p0) << 2) + (p1 - q1) + 4) >> 3)
            s[idx - step] = clip1(p0 + delta)
            s[idx] = clip1(q0 - delta)
        else:
            s[idx - step] = (2 * p1 + p0 + q1 + 2) >> 2
            s[idx] = (2 * q1 + q0 + p1 + 2) >> 2
        return
    p2 = s[idx - 3 * step]
    q2 = s[idx + 2 * step]
    ap = abs(p2 - p0)
    aq = abs(q2 - q0)
    if bS < 4:
        tc = tc0 + (1 if ap < beta else 0) + (1 if aq < beta else 0)
        delta = clip3(-tc, tc, (((q0 - p0) << 2) + (p1 - q1) + 4) >> 3)
        s[idx - step] = clip1(p0 + delta)
        s[idx] = clip1(q0 - delta)
        if ap < beta:
            s[idx - 2 * step] = p1 + clip3(-tc0, tc0, (p2 + ((p0 + q0 + 1) >> 1) - (p1 << 1)) >> 1)
        if aq < beta:
            s[idx + step] = q1 + clip3(-tc0, tc0, (q2 + ((p0 + q0 + 1) >> 1) - (q1 << 1)) >> 1)
        return
    p3 = s[idx - 4 * step]
    q3 = s[idx + 3 * step]
    small = abs(p0 - q0) < ((alpha >> 2) + 2)
    if ap < beta and small:
        s[idx - step] = (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3
        s[idx - 2 * step] = (p2 + p1 + p0 + q0 + 2) >> 2
        s[idx - 3 * step] = (2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3
    else:
        s[idx - step] = (2 * p1 + p0 + q1 + 2) >> 2
    if aq < beta and small:
        s[idx] = (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3
        s[idx + step] = (p0 + q0 + q1 + q2 + 2) >> 2
        s[idx + 2 * step] = (2 * q3 + 3 * q2 + q1 + q0 + p0 + 4) >> 3
    else:
        s[idx] = (2 * q1 + q0 + p1 + 2) >> 2


def deblock_picture(dec, pic):
    for mb in pic.mbs:
        h = mb.hdr
        res = compute_bs(dec, pic, mb)
        if h.disable_deblocking == 1:
            mb.bs = [[None] * 4, [None] * 4]
            continue
        bs, left, top = res
        mb.bs = bs
        if dec.collect:
            for d in (0, 1):
                for e in range(4):
                    if bs[d][e]:
                        for b in bs[d][e]:
                            dec.stat("bS %d" % b)
        pps = h.pps
        X, Y = mb.mbx * 16, mb.mby * 16
        qp_q = 0 if mb.kind == "IPCM" else mb.qp
        for d in (0, 1):
            for e in range(4):
                seg = bs[d][e]
                if seg is None:
                    continue
                if e == 0:
                    pm = left if d == 0 else top
                    qp_p = 0 if pm.kind == "IPCM" else pm.qp
                else:
                    qp_p = qp_q
                qpav = (qp_p + qp_q + 1) >> 1
                ia = clip3(0, 51, qpav + h.alpha_off)
                ib = clip3(0, 51, qpav + h.beta_off)
                alpha = T.ALPHA[ia]
                beta = T.BETA[ib]
                for k in range(16):
                    b = seg[k >> 2]
                    if b == 0:
                        continue
                    tc0 = T.TC0[b][ia] if b < 4 else 0
                    if d == 0:
                        idx = (Y + k) * pic.w + X + 4 * e
                        filter_line(pic.Y, idx, 1, b, alpha, beta, tc0, False)
                    else:
                        idx = (Y + 4 * e) * pic.w + X + k
                        filter_line(pic.Y, idx, pic.w, b, alpha, beta, tc0, False)
        # chroma
        for c, plane in enumerate((pic.U, pic.V)):
            off = pps.chroma_qp_index_offset if c == 0 else pps.second_chroma_qp_index_offset
            qq = chroma_qp(0 if mb.kind == "IPCM" else mb.qp, off)
            for d in (0, 1):
                for e in (0, 2):
                    seg = bs[d][e]
                    if seg is None:
                        continue
                    if e == 0:
                        pm = left if d == 0 else top
                        qpp = chroma_qp(0 if pm.kind == "IPCM" else pm.qp, off)
                    else:
                        qpp = qq
                    qpav = (qpp + qq + 1) >> 1
                    ia = clip3(0, 51, qpav + h.alpha_off)
                    ib = clip3(0, 51, qpav + h.beta_off)
                    alpha = T.ALPHA[ia]
                    beta = T.BETA[ib]
                    cx, cy = X >> 1, Y >> 1
                    for k in range(8):
                        b = seg[k >> 1]
                        if b == 0:
                            continue
                        tc0 = T.TC0[b][ia] if b < 4 else 0
                        if d == 0:
                            idx = (cy + k) * pic.cw + cx + 2 * e
                            filter_line(plane, idx, 1, b, alpha, beta, tc0, True)
                        else:
                            idx = (cy + 2 * e) * pic.cw + cx + k
                            filter_line(plane, idx, pic.cw, b, alpha, beta, tc0, True)


# ---------------------------------------------------------------------------
# CABAC (9.3)

# Table 9-43, frame coded macroblocks: ctxIdxInc of significant_coeff_flag
# and last_significant_coeff_flag of an 8x8 block, by levelListIdx.
SIG8_FRAME = [0, 1, 2, 3, 4, 5, 5, 4, 4, 3, 3, 4, 4, 4, 5, 5, 4, 4, 4, 4, 3, 3, 6, 7,
              7, 7, 8, 9, 10, 9, 8, 7, 7, 6, 11, 12, 13, 11, 6, 7, 8, 9, 14, 10, 9, 8,
              6, 11, 12, 13, 11, 6, 9, 14, 10, 9, 11, 12, 13, 11, 14, 10, 12]
LAST8 = [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2,
         2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4,
         5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8]
# Table 9-40, by ctxBlockCat 0..5.
CBF_CAT_OFF = [0, 4, 8, 12, 16, 0]
SIG_CAT_OFF = [0, 15, 29, 44, 47, 0]
ABS_CAT_OFF = [0, 10, 20, 30, 39, 0]


class Cabac:
    """The arithmetic decoding engine (9.3.1.2, 9.3.3.2) and the context
    variables (9.3.1.1)."""

    def __init__(self, br, slice_qp, init_idc, is_i):
        import h264_cabac_tables as CT
        self.CT = CT
        self.br = br
        table = CT.CTX_INIT[3 if is_i else init_idc]
        qp = clip3(0, 51, slice_qp)
        self.st = [0] * 1024
        self.mps = [0] * 1024
        for i, (m, n) in enumerate(table):
            pre = clip3(1, 126, ((m * qp) >> 4) + n)
            if pre <= 63:
                self.st[i] = 63 - pre
                self.mps[i] = 0
            else:
                self.st[i] = pre - 64
                self.mps[i] = 1
        self.lps = CT.RANGE_LPS
        self.tl = CT.TRANS_LPS
        self.tm = CT.TRANS_MPS
        self.init_engine()

    def init_engine(self):
        self.range = 510
        self.offset = self.br.u(9)

    def decision(self, ctx):
        s = self.st[ctx]
        rlps = self.lps[s][(self.range >> 6) & 3]
        rng = self.range - rlps
        if self.offset >= rng:
            b = 1 - self.mps[ctx]
            self.offset -= rng
            rng = rlps
            if s == 0:
                self.mps[ctx] = 1 - self.mps[ctx]
            self.st[ctx] = self.tl[s]
        else:
            b = self.mps[ctx]
            self.st[ctx] = self.tm[s]
        if rng < 256:
            br = self.br
            off = self.offset
            while rng < 256:
                rng <<= 1
                off = (off << 1) | br.u1()
            self.offset = off
        self.range = rng
        return b

    def bypass(self):
        self.offset = (self.offset << 1) | self.br.u1()
        if self.offset >= self.range:
            self.offset -= self.range
            return 1
        return 0

    def terminate(self):
        self.range -= 2
        if self.offset >= self.range:
            return 1
        if self.range < 256:
            br = self.br
            while self.range < 256:
                self.range <<= 1
                self.offset = (self.offset << 1) | br.u1()
        return 0


def run_cabac(sd):
    """slice_data() with entropy_coding_mode_flag 1 (7.3.4)."""
    h = sd.h
    br = sd.br
    br.byte_align()  # cabac_alignment_one_bit
    sd.cab = Cabac(br, h.qp, h.cabac_init_idc, h.slice_type == SLICE_I)
    addr = h.first_mb
    total = sd.W * sd.H
    sd.prev_mb = None
    while True:
        if addr >= total:
            raise DecodeError("slice data runs past the picture")
        if h.slice_type != SLICE_I:
            mb = sd.new_mb(addr)
            base = 11 if h.slice_type == SLICE_P else 24
            inc = sum(1 for n in (mb.mbA, mb.mbB) if n is not None and not n.skip)
            if sd.cab.decision(base + inc):
                sd.decode_skip(addr)
                sd.prev_mb = sd.pic.mbs[addr]
                if sd.cab.terminate():
                    break
                addr += 1
                continue
        mb = sd.new_mb(addr)
        cabac_mb(sd, mb)
        sd.finish_mb(mb)
        sd.prev_mb = mb
        if sd.cab.terminate():
            break
        addr += 1


def cabac_mb_type_i(sd, mb, cab, base, prefix_ctx):
    """The I macroblock type bins (Table 9-36). base: ctxIdxOffset of the
    bins after bin 0 (3 in I slices, 17 or 32 as a suffix); prefix_ctx: the
    ctxIdx of bin 0."""
    if not cab.decision(prefix_ctx):
        return 0
    if cab.terminate():
        return 25
    if base == 3:
        c_luma, c_chroma, c_c2, c_p0, c_p1 = 3 + 3, 3 + 4, 3 + 5, 3 + 6, 3 + 7
        luma = cab.decision(c_luma)
        chroma = cab.decision(c_chroma)
        if chroma:
            chroma = 1 + cab.decision(c_c2)
            p0 = cab.decision(3 + 6)
            p1 = cab.decision(3 + 7)
        else:
            p0 = cab.decision(c_p0)
            p1 = cab.decision(c_p1)
    else:
        luma = cab.decision(base + 1)
        chroma = cab.decision(base + 2)
        if chroma:
            chroma = 1 + cab.decision(base + 2)
        p0 = cab.decision(base + 3)
        p1 = cab.decision(base + 3)
    return 1 + (p0 * 2 + p1) + 4 * chroma + 12 * luma


def cabac_mb(sd, mb):
    cab = sd.cab
    h = sd.h
    st = h.slice_type
    if st == SLICE_I:
        inc = sum(1 for n in (mb.mbA, mb.mbB) if n is not None and n.kind not in ("I4", "I8"))
        t = cabac_mb_type_i(sd, mb, cab, 3, 3 + inc)
        mb.mb_type = t
        sd.set_i_type(mb, t)
    elif st == SLICE_P:
        if cab.decision(14):
            t = cabac_mb_type_i(sd, mb, cab, 17, 17)
            mb.mb_type = 5 + t
            sd.set_i_type(mb, t)
        else:
            b1 = cab.decision(15)
            if b1 == 0:
                t = 3 if cab.decision(16) else 0
            else:
                t = 1 if cab.decision(17) else 2
            mb.mb_type = t
            sd.set_p_type(mb, t)
    else:
        inc = sum(1 for n in (mb.mbA, mb.mbB)
                  if n is not None and not n.skip and n.name != "B_Direct_16x16")
        t = cabac_mb_type_b(cab, inc)
        if t >= 23:
            ti = cabac_mb_type_i(sd, mb, cab, 32, 32)
            mb.mb_type = 23 + ti
            sd.set_i_type(mb, ti)
        else:
            mb.mb_type = t
            sd.set_b_type(mb, t)
    if mb.kind == "IPCM":
        br = sd.br
        br.byte_align()
        sd.read_pcm(mb, lambda: br.u(8))
        cab.init_engine()
        mb.qp_delta = 0
        return
    pps = sd.pps
    if mb.kind in ("P", "B") and mb.num_parts == 4:
        cabac_sub_mb_pred(sd, mb)
    else:
        if mb.kind == "I4" and pps.transform_8x8_mode:
            if cab.decision(399 + t8_inc(mb)):
                mb.kind = "I8"
                mb.t8 = 1
        cabac_mb_pred(sd, mb)
    if mb.kind != "I16":
        mb.cbp = cabac_cbp(sd, mb)
        if (mb.cbp & 15) and pps.transform_8x8_mode and not mb.is_intra and sd.no_sub_lt8x8(mb):
            mb.t8 = cab.decision(399 + t8_inc(mb))
    mb.qp_delta = 0
    if (mb.cbp & 15) or (mb.cbp >> 4) or mb.kind == "I16":
        mb.qp_delta = cabac_qp_delta(sd)
        if mb.qp_delta < -26 or mb.qp_delta > 25:
            raise DecodeError("mb_qp_delta out of range")
    sd.qp = (sd.qp + mb.qp_delta + 52) % 52
    mb.qp = sd.qp
    sd.emit_mb_header(mb)
    cabac_residual(sd, mb)


def t8_inc(mb):
    return sum(1 for n in (mb.mbA, mb.mbB) if n is not None and n.t8)


def cabac_mb_type_b(cab, inc):
    """Table 9-37, B slices: returns 0..22, or 23 for the intra prefix."""
    if not cab.decision(27 + inc):
        return 0
    if not cab.decision(27 + 3):
        return 1 + cab.decision(27 + 5)
    b2 = cab.decision(27 + 4)
    b3 = cab.decision(27 + 5)
    b4 = cab.decision(27 + 5)
    b5 = cab.decision(27 + 5)
    if b2 == 0:
        return 3 + (b3 << 2 | b4 << 1 | b5)
    if b3 == 1:
        if b4 == 1:
            return 22 if b5 else 11
        if b5 == 1:
            return 23
        return 20 + cab.decision(27 + 5)
    b6 = cab.decision(27 + 5)
    return 12 + (b4 << 2 | b5 << 1 | b6)


def cabac_sub_type(cab, is_b):
    if not is_b:
        if cab.decision(21):
            return 0
        if not cab.decision(22):
            return 1
        return 2 if cab.decision(23) else 3
    if not cab.decision(36):
        return 0
    if not cab.decision(37):
        return 1 + cab.decision(39)
    if not cab.decision(38):
        b3 = cab.decision(39)
        b4 = cab.decision(39)
        return 3 + 2 * b3 + b4
    if cab.decision(39):
        return 11 + cab.decision(39)
    b4 = cab.decision(39)
    b5 = cab.decision(39)
    return 7 + 2 * b4 + b5


def part_uses(mb, r, lx):
    """Whether the partition holding raster 4x4 block r of mb predicts from
    list lx with a coded ref_idx/mvd (predModeEqualFlag, 9.3.3.1.1.6/7)."""
    if mb.is_intra or mb.skip or mb.name == "B_Direct_16x16":
        return False
    return mb.refsyn[lx][r] is not None


def cabac_ref_idx(sd, mb, x, y, lx):
    cab = sd.cab
    inc = 0
    for (dx, dy, wgt) in ((-1, 0, 1), (0, -1, 2)):
        n, xw, yw = sd.nb4(mb, x + dx, y + dy)
        if n is None:
            continue
        r = (yw >> 2) * 4 + (xw >> 2)
        if part_uses(n, r, lx) and n.refsyn[lx][r] > 0:
            inc += wgt
    v = 0
    ctx = 54 + inc
    while cab.decision(ctx):
        v += 1
        ctx = 54 + (4 if v == 1 else 5)
        if v > 32:
            raise DecodeError("ref_idx too large")
    return v


def cabac_mvd(sd, mb, x, y, lx, comp):
    cab = sd.cab
    base = 40 if comp == 0 else 47
    s = 0
    for (dx, dy) in ((-1, 0), (0, -1)):
        n, xw, yw = sd.nb4(mb, x + dx, y + dy)
        if n is None:
            continue
        r = (yw >> 2) * 4 + (xw >> 2)
        if part_uses(n, r, lx):
            s += abs(n.mvd[lx][r][comp])
    inc = 0 if s < 3 else (1 if s <= 32 else 2)
    v = 0
    if cab.decision(base + inc):
        v = 1
        ctx = base + 3
        while v < 9 and cab.decision(ctx):
            v += 1
            if ctx < base + 6:
                ctx += 1
        if v >= 9:
            k = 3
            while cab.bypass():
                v += 1 << k
                k += 1
                if k > 30:
                    raise DecodeError("bad mvd suffix")
            while k:
                k -= 1
                v += cab.bypass() << k
    if v and cab.bypass():
        v = -v
    return v


def set_syn(mb, x, y, w, h, lx, ref=None, mvd=None):
    for yy in range(y >> 2, (y + h) >> 2):
        for xx in range(x >> 2, (x + w) >> 2):
            r = yy * 4 + xx
            if ref is not None:
                mb.refsyn[lx][r] = ref
            if mvd is not None:
                mb.mvd[lx][r] = mvd


def cabac_mb_pred(sd, mb):
    cab = sd.cab
    h = sd.h
    if mb.kind == "I4":
        mb.modes4 = [0] * 16
        for blk in range(16):
            if cab.decision(68):
                sd.resolve_intra4(mb, blk, 1, None)
            else:
                rem = cab.decision(69) | (cab.decision(69) << 1) | (cab.decision(69) << 2)
                sd.resolve_intra4(mb, blk, 0, rem)
    elif mb.kind == "I8":
        mb.modes8 = [0] * 4
        for b8 in range(4):
            if cab.decision(68):
                sd.resolve_intra8(mb, b8, 1, None)
            else:
                rem = cab.decision(69) | (cab.decision(69) << 1) | (cab.decision(69) << 2)
                sd.resolve_intra8(mb, b8, 0, rem)
    if mb.is_intra:
        inc = sum(1 for n in (mb.mbA, mb.mbB)
                  if n is not None and n.is_intra and n.kind != "IPCM" and n.chroma_mode != 0)
        v = 0
        if cab.decision(64 + inc):
            v = 1
            if cab.decision(67):
                v = 2
                if cab.decision(67):
                    v = 3
        mb.chroma_mode = v
        return
    n = mb.num_parts
    mb.part_ref = [[0] * n, [0] * n]
    mb.part_mvd = [[None] * n, [None] * n]
    w, hh = mb.part_w, mb.part_h
    geo = [(0 if w == 16 else 8 * i, 0 if hh == 16 else 8 * i) for i in range(n)]
    for lx in (0, 1):
        for i in range(n):
            if mb.part_pred[i] in ("L" + str(lx), "Bi"):
                x, y = geo[i]
                if h.num_ref_idx[lx] > 1:
                    mb.part_ref[lx][i] = cabac_ref_idx(sd, mb, x, y, lx)
                set_syn(mb, x, y, w, hh, lx, ref=mb.part_ref[lx][i])
    for lx in (0, 1):
        for i in range(n):
            if mb.part_pred[i] in ("L" + str(lx), "Bi"):
                x, y = geo[i]
                dx = cabac_mvd(sd, mb, x, y, lx, 0)
                dy = cabac_mvd(sd, mb, x, y, lx, 1)
                mb.part_mvd[lx][i] = (dx, dy)
                set_syn(mb, x, y, w, hh, lx, mvd=(dx, dy))


def sub_geo(i, s, j):
    x0, y0 = (i & 1) * 8, (i >> 1) * 8
    sw, sh = s[2], s[3]
    if sw == 8 and sh == 8:
        return x0, y0
    if sw == 8:
        return x0, y0 + 4 * j
    if sh == 8:
        return x0 + 4 * j, y0
    return x0 + 4 * (j & 1), y0 + 4 * (j >> 1)


def cabac_sub_mb_pred(sd, mb):
    cab = sd.cab
    h = sd.h
    is_b = mb.kind == "B"
    mb.sub = []
    for i in range(4):
        t = cabac_sub_type(cab, is_b)
        mb.sub.append(B_SUB[t] if is_b else P_SUB[t] + ("L0",))
    mb.sub_ref = [[0] * 4, [0] * 4]
    for lx in (0, 1):
        for i in range(4):
            s = mb.sub[i]
            if s[4] in ("L" + str(lx), "Bi"):
                x0, y0 = (i & 1) * 8, (i >> 1) * 8
                if h.num_ref_idx[lx] > 1:
                    mb.sub_ref[lx][i] = cabac_ref_idx(sd, mb, x0, y0, lx)
                set_syn(mb, x0, y0, 8, 8, lx, ref=mb.sub_ref[lx][i])
    mb.sub_mvd = [[[None] * 4 for _ in range(4)], [[None] * 4 for _ in range(4)]]
    for lx in (0, 1):
        for i in range(4):
            s = mb.sub[i]
            if s[4] in ("L" + str(lx), "Bi"):
                for j in range(s[1]):
                    x, y = sub_geo(i, s, j)
                    dx = cabac_mvd(sd, mb, x, y, lx, 0)
                    dy = cabac_mvd(sd, mb, x, y, lx, 1)
                    mb.sub_mvd[lx][i][j] = (dx, dy)
                    set_syn(mb, x, y, s[2], s[3], lx, mvd=(dx, dy))


def cabac_cbp(sd, mb):
    cab = sd.cab
    luma = 0
    for b8 in range(4):
        x, y = (b8 & 1) * 8, (b8 >> 1) * 8
        inc = 0
        for (dx, dy, wgt) in ((-1, 0, 1), (0, -1, 2)):
            n, xw, yw = sd.nb4(mb, x + dx, y + dy)
            k = (yw >> 3) * 2 + (xw >> 3)
            if n is None or n.kind == "IPCM":
                cond = 0
            elif n is not mb:
                cond = 0 if (not n.skip and (n.cbp >> k) & 1) else 1
            else:
                cond = 0 if (luma >> k) & 1 else 1
            inc += wgt * cond
        luma |= cab.decision(73 + inc) << b8
    chroma = 0
    for b in range(2):
        inc = 0
        for n, wgt in ((mb.mbA, 1), (mb.mbB, 2)):
            if n is not None and n.kind == "IPCM":
                cond = 1
            elif n is None or n.skip:
                cond = 0
            elif b == 0:
                cond = 1 if (n.cbp >> 4) != 0 else 0
            else:
                cond = 1 if (n.cbp >> 4) == 2 else 0
            inc += wgt * cond
        if not cab.decision(77 + inc + (4 if b else 0)):
            break
        chroma += 1
    return luma | (chroma << 4)


def cabac_qp_delta(sd):
    cab = sd.cab
    p = sd.prev_mb
    if p is None or p.skip or p.kind == "IPCM" or \
            (p.kind != "I16" and (p.cbp & 15) == 0 and (p.cbp >> 4) == 0) or p.qp_delta == 0:
        inc = 0
    else:
        inc = 1
    k = 0
    ctx = 60 + inc
    while cab.decision(ctx):
        k += 1
        ctx = 62 if k == 1 else 63
        if k > 104:
            raise DecodeError("mb_qp_delta too large")
    return (k + 1) >> 1 if k & 1 else -(k >> 1)


def cbf_cond(sd, mb, n, flag):
    """condTermFlagN of 9.3.3.1.1.9 given neighbour macroblock n and, when
    the neighbour's transform block exists, its coded_block_flag (else None)."""
    if n is None:
        return 1 if mb.is_intra else 0
    if n.kind == "IPCM":
        return 1
    if flag is None:
        return 0
    return flag


def cabac_residual(sd, mb):
    mb.coef = [None] * 16
    mb.coef8 = [None] * 4
    mb.cbf = {}
    cbp_l = mb.cbp & 15
    if mb.kind == "I16":
        inc = 0
        for n, wgt in ((mb.mbA, 1), (mb.mbB, 2)):
            f = n.cbf.get("dc") if n is not None and n.kind == "I16" else None
            inc += wgt * cbf_cond(sd, mb, n, f)
        mb.dc16, cnt = residual_block_cabac(sd, mb, 0, 16, inc)
        mb.cbf["dc"] = 1 if cnt else 0
        sd.emit_coef(mb, "Intra16x16DC", 0, mb.dc16, cnt)
    for b8 in range(4):
        if mb.t8:
            if cbp_l & (1 << b8):
                lv, cnt = residual_block_cabac(sd, mb, 5, 64, None)
                mb.coef8[b8] = lv
                sd.emit_coef(mb, "Luma8x8", b8, lv, cnt)
                for b4 in range(4):
                    r = BLK_RASTER[b8 * 4 + b4]
                    mb.cbf[("y", r)] = 1
                    mb.nz[r] = 1 if cnt else 0
            continue
        for b4 in range(4):
            blk = b8 * 4 + b4
            r = BLK_RASTER[blk]
            if not cbp_l & (1 << b8):
                continue
            x, y = BLK_X[blk], BLK_Y[blk]
            inc = 0
            for (dx, dy, wgt) in ((-1, 0, 1), (0, -1, 2)):
                n, xw, yw = sd.nb4(mb, x + dx, y + dy)
                f = None
                if n is not None and not n.skip and n.kind != "IPCM":
                    rn = (yw >> 2) * 4 + (xw >> 2)
                    k = (yw >> 3) * 2 + (xw >> 3)
                    if (n.cbp >> k) & 1:
                        f = n.cbf.get(("y", rn), 0)
                inc += wgt * cbf_cond(sd, mb, n, f)
            if mb.kind == "I16":
                ac, cnt = residual_block_cabac(sd, mb, 1, 15, inc)
                mb.coef[r] = [0] + ac
                sd.emit_coef(mb, "Intra16x16AC", blk, mb.coef[r], cnt)
            else:
                mb.coef[r], cnt = residual_block_cabac(sd, mb, 2, 16, inc)
                sd.emit_coef(mb, "Luma4x4", blk, mb.coef[r], cnt)
            mb.cbf[("y", r)] = 1 if cnt else 0
            mb.nz[r] = 1 if cnt else 0
    cbp_c = mb.cbp >> 4
    mb.cdc = [[0] * 4, [0] * 4]
    mb.cac = [[None] * 4, [None] * 4]
    if cbp_c & 3:
        for c in range(2):
            inc = 0
            for n, wgt in ((mb.mbA, 1), (mb.mbB, 2)):
                f = None
                if n is not None and not n.skip and n.kind != "IPCM" and (n.cbp >> 4):
                    f = n.cbf.get(("cdc", c), 0)
                inc += wgt * cbf_cond(sd, mb, n, f)
            mb.cdc[c], cnt = residual_block_cabac(sd, mb, 3, 4, inc)
            mb.cbf[("cdc", c)] = 1 if cnt else 0
            sd.emit_coef(mb, "ChromaDC" + "BR"[c], 0, mb.cdc[c], cnt)
    if cbp_c & 2:
        for c in range(2):
            for r in range(4):
                bx, by = r & 1, r >> 1
                inc = 0
                for (n, rn, wgt) in (
                        (mb if bx else mb.mbA, r - 1 if bx else r + 1, 1),
                        (mb if by else mb.mbB, r - 2 if by else r + 2, 2)):
                    f = None
                    if n is not None and not n.skip and n.kind != "IPCM" and (n.cbp >> 4) == 2:
                        f = n.cbf.get(("cac", c, rn), 0)
                    inc += wgt * cbf_cond(sd, mb, n, f)
                ac, cnt = residual_block_cabac(sd, mb, 4, 15, inc)
                mb.cac[c][r] = [0] + ac
                mb.cbf[("cac", c, r)] = 1 if cnt else 0
                sd.emit_coef(mb, "ChromaAC" + "BR"[c], r, mb.cac[c][r], cnt)


def residual_block_cabac(sd, mb, cat, maxnum, cbf_inc):
    """residual_block_cabac (7.3.5.3.3) for startIdx 0, endIdx maxnum - 1.
    Returns (levels in scan order, number of nonzero levels)."""
    cab = sd.cab
    coeff = [0] * maxnum
    if cat != 5:
        if not cab.decision(85 + CBF_CAT_OFF[cat] + cbf_inc):
            return coeff, 0
    if cat == 5:
        sig_base, last_base, abs_base = 402, 417, 426
    else:
        sig_base = 105 + SIG_CAT_OFF[cat]
        last_base = 166 + SIG_CAT_OFF[cat]
        abs_base = 227 + ABS_CAT_OFF[cat]
    num = maxnum
    sig = [0] * maxnum
    i = 0
    while i < num - 1:
        if cat == 5:
            si, li = SIG8_FRAME[i], LAST8[i]
        elif cat == 3:
            si = li = min(i, 2)
        else:
            si = li = i
        if cab.decision(sig_base + si):
            sig[i] = 1
            if cab.decision(last_base + li):
                num = i + 1
                break
        i += 1
    sig[num - 1] = 1
    eq1 = 0
    gt1 = 0
    count = 0
    gt1_max = 4 - (1 if cat == 3 else 0)
    for i in range(num - 1, -1, -1):
        if not sig[i]:
            continue
        ctx = abs_base + (0 if gt1 else min(4, 1 + eq1))
        if not cab.decision(ctx):
            a = 0
        else:
            a = 1
            ctx = abs_base + 5 + min(gt1_max, gt1)
            while a < 14 and cab.decision(ctx):
                a += 1
            if a >= 14:
                k = 0
                while cab.bypass():
                    a += 1 << k
                    k += 1
                    if k > 30:
                        raise DecodeError("bad coeff_abs_level_minus1 suffix")
                while k:
                    k -= 1
                    a += cab.bypass() << k
        level = a + 1
        if level == 1:
            eq1 += 1
        else:
            gt1 += 1
        coeff[i] = -level if cab.bypass() else level
        count += 1
    return coeff, count


SliceDecoder.run_cabac = run_cabac


# ---------------------------------------------------------------------------
# Command line

def decode_file(path, trace=None):
    with open(path, "rb") as f:
        data = f.read()
    dec = Decoder(trace)
    pics = dec.decode_stream(data)
    return dec, [dec.cropped_frame(p) for p in pics]


def read_md5(path):
    out = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            out.append(line.split()[1])
    return out


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd = argv[1]
    if cmd == "decode":
        path = argv[2]
        dec, frames = decode_file(path)
        if "-o" in argv:
            with open(argv[argv.index("-o") + 1], "wb") as f:
                for fr in frames:
                    f.write(fr)
        if "--md5" in argv:
            for i, fr in enumerate(frames):
                print(i, hashlib.md5(fr).hexdigest())
        return 0
    if cmd == "check":
        dec, frames = decode_file(argv[2])
        ref = read_md5(argv[3])
        got = [hashlib.md5(fr).hexdigest() for fr in frames]
        bad = 0
        if len(got) != len(ref):
            print("%s: %d frames decoded, %d expected" % (argv[2], len(got), len(ref)))
            bad = 1
        for i, (g, r) in enumerate(zip(got, ref)):
            if g != r:
                print("%s: frame %d differs" % (argv[2], i))
                bad = 1
        if not bad:
            print("%s: %d frames bit-exact" % (argv[2], len(got)))
        return bad
    if cmd == "stats":
        with open(argv[2], "rb") as f:
            data = f.read()
        dec = Decoder()
        dec.collect = True
        dec.decode_stream(data)
        for k in sorted(dec.stats):
            print("%-24s %d" % (k, dec.stats[k]))
        return 0
    if cmd == "dump":
        import h264_dump
        return h264_dump.main(argv)
    if cmd == "selftest":
        for t in T.COEFF_TOKEN + list(T.TOTAL_ZEROS_4X4.values()) + \
                list(T.TOTAL_ZEROS_DC420.values()) + list(T.TOTAL_ZEROS_DC422.values()) + \
                list(T.RUN_BEFORE.values()):
            VLC(t)
            assert sum(2.0 ** -len(c) for c in t) <= 1.0
        print("tables ok")
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
