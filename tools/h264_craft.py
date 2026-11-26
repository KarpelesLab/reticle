#!/usr/bin/env python3
"""Writes small H.264 streams that libx264 never produces.

libx264 never writes an I_PCM macroblock, a memory_management_control_operation,
a reference picture list modification, a long-term reference, picture order
count types 0 or 1, or disable_deblocking_filter_idc 2. A decoder still has to
handle all of them, and so does the model. This writer makes streams that use
them, from the syntax of Rec. ITU-T H.264 clause 7.3 and nothing else; ffmpeg
then decodes them as a black box (tools/h264_streams.sh) and the model is held
to that like every other stream.

It writes only what it needs: I_PCM, Intra_4x4 macroblocks with every mode
predicted and no residual (so every mode is DC), P_L0_16x16 macroblocks with
a chosen ref_idx and mvd and no residual, I_PCM inside P slices, and P_Skip
runs. With no residual there is no CAVLC to get right here.

Usage: h264_craft.py NAME OUT.264      NAME is one of the STREAMS below
"""

import sys


class BitWriter:
    def __init__(self):
        self.bits = []

    def u(self, n, v):
        for i in range(n - 1, -1, -1):
            self.bits.append((v >> i) & 1)

    def ue(self, v):
        v += 1
        n = v.bit_length()
        self.u(n - 1, 0)
        self.u(n, v)

    def se(self, v):
        self.ue(2 * v - 1 if v > 0 else -2 * v)

    def te(self, cmax, v):
        if cmax > 1:
            self.ue(v)
        else:
            self.u(1, 1 - v)

    def align_zero(self):
        while len(self.bits) % 8:
            self.bits.append(0)

    def trailing(self):
        self.bits.append(1)
        self.align_zero()

    def bytes(self):
        out = bytearray()
        for i in range(0, len(self.bits), 8):
            b = 0
            for bit in self.bits[i:i + 8]:
                b = (b << 1) | bit
            out.append(b)
        return bytes(out)


def escape(rbsp):
    out = bytearray()
    zeros = 0
    for b in rbsp:
        if zeros >= 2 and b <= 3:
            out.append(3)
            zeros = 0
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
    return bytes(out)


def nal(ref_idc, typ, bw):
    return b"\x00\x00\x00\x01" + bytes([(ref_idc << 5) | typ]) + escape(bw.bytes())


class Rng:
    """A fixed linear congruential generator, so the streams are reproducible."""

    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF

    def next(self, n):
        self.s = (self.s * 1103515245 + 12345) & 0xFFFFFFFF
        return (self.s >> 8) % n


class Stream:
    def __init__(self, w_mbs, h_mbs, poc_type=2, max_refs=4, log2_fn=4,
                 log2_poc=6, chroma_qp_offset=0, crop=None, poc_cycle=None):
        self.w = w_mbs
        self.h = h_mbs
        self.poc_type = poc_type
        self.max_refs = max_refs
        self.log2_fn = log2_fn
        self.log2_poc = log2_poc
        self.cqo = chroma_qp_offset
        self.crop = crop
        self.poc_cycle = poc_cycle or [2]
        self.out = bytearray()
        self.headers()

    def headers(self):
        b = BitWriter()
        b.u(8, 66)          # profile_idc: Baseline
        b.u(8, 0xC0)        # constraint_set0/1
        b.u(8, 30)          # level 3.0
        b.ue(0)             # seq_parameter_set_id
        b.ue(self.log2_fn - 4)
        b.ue(self.poc_type)
        if self.poc_type == 0:
            b.ue(self.log2_poc - 4)
        elif self.poc_type == 1:
            b.u(1, 0)       # delta_pic_order_always_zero_flag
            b.se(1)         # offset_for_non_ref_pic
            b.se(0)         # offset_for_top_to_bottom_field
            b.ue(len(self.poc_cycle))
            for v in self.poc_cycle:
                b.se(v)
        b.ue(self.max_refs)
        b.u(1, 0)           # gaps_in_frame_num_value_allowed_flag
        b.ue(self.w - 1)
        b.ue(self.h - 1)
        b.u(1, 1)           # frame_mbs_only_flag
        b.u(1, 1)           # direct_8x8_inference_flag
        if self.crop:
            b.u(1, 1)
            for v in self.crop:
                b.ue(v)
        else:
            b.u(1, 0)
        b.u(1, 0)           # vui_parameters_present_flag
        b.trailing()
        self.out += nal(3, 7, b)
        b = BitWriter()
        b.ue(0)             # pic_parameter_set_id
        b.ue(0)             # seq_parameter_set_id
        b.u(1, 0)           # entropy_coding_mode_flag
        b.u(1, 0)           # bottom_field_pic_order_in_frame_present_flag
        b.ue(0)             # num_slice_groups_minus1
        b.ue(0)             # num_ref_idx_l0_default_active_minus1
        b.ue(0)             # num_ref_idx_l1_default_active_minus1
        b.u(1, 0)           # weighted_pred_flag
        b.u(2, 0)           # weighted_bipred_idc
        b.se(0)             # pic_init_qp_minus26
        b.se(0)             # pic_init_qs_minus26
        b.se(self.cqo)      # chroma_qp_index_offset
        b.u(1, 1)           # deblocking_filter_control_present_flag
        b.u(1, 0)           # constrained_intra_pred_flag
        b.u(1, 0)           # redundant_pic_cnt_present_flag
        b.trailing()
        self.out += nal(3, 8, b)

    def slice(self, kind, mbs, frame_num, ref_idc=1, idr=False, idr_id=0,
              poc_lsb=0, nref=1, rplm=None, mmco=None, long_term=False,
              qp=26, dbk=(0, 0, 0), first_mb=0, delta_poc=0):
        """kind 'I' or 'P'. mbs: list of macroblocks for addresses first_mb..,
        each ('pcm', rng) / ('i4',) / ('p', ref_idx, mvdx, mvdy) / ('skip',)."""
        b = BitWriter()
        b.ue(first_mb)
        b.ue(7 if kind == "I" else 5)    # slice_type: all slices of the picture alike
        b.ue(0)                          # pic_parameter_set_id
        b.u(self.log2_fn, frame_num)
        if idr:
            b.ue(idr_id)
        if self.poc_type == 0:
            b.u(self.log2_poc, poc_lsb)
        if self.poc_type == 1:
            b.se(delta_poc)              # delta_pic_order_cnt[0]
        if kind == "P":
            if nref != 1:
                b.u(1, 1)                # num_ref_idx_active_override_flag
                b.ue(nref - 1)
            else:
                b.u(1, 0)
            if rplm:
                b.u(1, 1)
                for idc, v in rplm:
                    b.ue(idc)
                    b.ue(v)
                b.ue(3)
            else:
                b.u(1, 0)
        if ref_idc:
            if idr:
                b.u(1, 0)                # no_output_of_prior_pics_flag
                b.u(1, 1 if long_term else 0)
            elif mmco:
                b.u(1, 1)
                for op in mmco:
                    b.ue(op[0])
                    for v in op[1:]:
                        b.ue(v)
                b.ue(0)
            else:
                b.u(1, 0)
        b.se(qp - 26)                    # slice_qp_delta
        b.ue(dbk[0])                     # disable_deblocking_filter_idc
        if dbk[0] != 1:
            b.se(dbk[1])
            b.se(dbk[2])
        skip = 0
        for m in mbs:
            if m[0] == "skip":
                skip += 1
                continue
            if kind == "P":
                b.ue(skip)
                skip = 0
            if m[0] == "pcm":
                b.ue(25 if kind == "I" else 30)
                b.align_zero()
                rng = m[1]
                for i in range(384):
                    b.u(8, rng.next(256))
            elif m[0] == "i4":
                b.ue(0 if kind == "I" else 5)   # I_NxN
                for i in range(16):
                    b.u(1, 1)                    # prev_intra4x4_pred_mode_flag
                b.ue(0)                          # intra_chroma_pred_mode: DC
                b.ue(3)                          # coded_block_pattern 0 (Table 9-4)
            elif m[0] == "p":
                _, ref, mx, my = m
                b.ue(0)                          # P_L0_16x16
                if nref > 1:
                    b.te(nref - 1, ref)
                b.se(mx)
                b.se(my)
                b.ue(0)                          # coded_block_pattern 0
            else:
                raise ValueError(m)
        if kind == "P" and skip:
            b.ue(skip)
        b.trailing()
        self.out += nal(ref_idc, 5 if idr else 1, b)


def all_mbs(s, f):
    return [f(i) for i in range(s.w * s.h)]


def pcm_stream():
    """I_PCM: a picture of nothing else, then I_PCM beside Intra_4x4, then
    I_PCM inside P slices, at QPs where deblocking across a PCM edge
    filters (qPp is 0 for the I_PCM side)."""
    s = Stream(2, 2, max_refs=1)
    r = Rng(1)
    s.slice("I", all_mbs(s, lambda i: ("pcm", r)), 0, idr=True, qp=30)
    s.slice("I", all_mbs(s, lambda i: ("pcm", r) if i in (0, 3) else ("i4",)), 1, qp=44)
    s.slice("P", [("p", 0, 3, -2), ("pcm", r), ("skip",), ("p", 0, -5, 7)], 2, qp=40)
    s.slice("P", [("pcm", r), ("skip",), ("p", 0, 1, 1), ("pcm", r)], 3, qp=51)
    return bytes(s.out)


def mmco_stream(poc_type):
    """Long-term references, every memory_management_control_operation, list
    modification of both kinds, frame_num wrapping (MaxFrameNum 16), a
    non-reference picture, and the picture order count type given."""
    s = Stream(2, 2, poc_type=poc_type, max_refs=4, log2_fn=4, log2_poc=5,
               chroma_qp_offset=5, poc_cycle=[2, 2])
    r = Rng(7)
    pcm = lambda i: ("pcm", r)  # noqa: E731
    poc = [0]

    def emit(kind, mbs, fn, ref_idc=1, **kw):
        if s.poc_type == 0:
            kw.setdefault("poc_lsb", poc[0] % (1 << s.log2_poc))
        poc[0] += 2
        s.slice(kind, mbs, fn, ref_idc=ref_idc, **kw)
    # 0..3: four distinct reference pictures, all I_PCM.
    emit("I", all_mbs(s, pcm), 0, idr=True)
    emit("I", all_mbs(s, pcm), 1)
    emit("I", all_mbs(s, pcm), 2)
    # MaxLongTermFrameIdx = 1, then frame 0 (picNum 3 - 2 - 1 = 0) long-term 0.
    emit("I", all_mbs(s, pcm), 3, mmco=[(4, 2), (3, 2, 0)])
    # 4: default list [f3, f2, f1, L0]; the sliding window then drops f1.
    emit("P", [("p", 0, 1, 2), ("p", 1, -3, 0), ("p", 2, 0, 5), ("p", 3, 2, -2)], 4, nref=4)
    # 5: long-term 0 first, then f2 (picNum 5 - 3 = 2); f3 unmarked by
    # operation 1 (5 - 1 - 1 = 3); the current picture becomes long-term 1.
    emit("P", [("p", 0, 0, 0), ("p", 1, 6, 1), ("p", 2, -1, -1), ("p", 3, 1, 3)], 5, nref=4,
         rplm=[(2, 0), (0, 2)], mmco=[(1, 1), (6, 1)])
    # 6: idc 1 wrapping round MaxPicNum: 6 + 14 = 20 - 16 = 4, then f2, then
    # long-term 1; operation 2 drops long-term 0 afterwards.
    emit("P", [("p", 3, 2, 2), ("p", 2, 0, -4), ("p", 1, 1, 1), ("p", 0, -2, 0)], 6, nref=4,
         rplm=[(1, 13), (0, 1), (2, 1)], mmco=[(2, 0)])
    # 7: a non-reference picture between references.
    emit("P", [("p", 0, 1, 1), ("skip",), ("p", 1, 0, 2), ("p", 2, -1, 0)], 7, ref_idc=0, nref=3)
    # 7..17: references with the sliding window, frame_num wrapping at 16.
    fn = 7
    for k in range(10):
        mbs = [("p", (k + i) % 3, (k * 3 + i) % 7 - 3, (k + 2 * i) % 5 - 2) for i in range(4)]
        if k == 4:
            mbs[1] = ("pcm", r)
        emit("P", mbs, fn % 16, nref=3)
        fn += 1
    # Operation 4 to 0 drops every long-term reference; then operation 5.
    emit("P", [("p", 0, 1, 0), ("p", 1, 0, 1), ("skip",), ("p", 2, 2, 2)], fn % 16, nref=3,
         mmco=[(4, 0)])
    fn += 1
    emit("P", [("p", 0, 0, 0), ("p", 1, 1, 1), ("p", 2, 2, 2), ("p", 0, 3, 3)], fn % 16, nref=3,
         mmco=[(5,)])
    # After operation 5 the picture counts as frame_num 0 and POC 0.
    poc[0] = 2
    emit("P", [("p", 0, -1, 0), ("p", 0, 0, -1), ("skip",), ("p", 0, 1, 1)], 1, nref=1)
    emit("P", [("p", 1, 2, 0), ("p", 0, 0, 2), ("p", 1, -2, 1), ("p", 0, 1, -1)], 2, nref=2)
    return bytes(s.out)


def dbk2_stream():
    """Two slices per picture with disable_deblocking_filter_idc 2: edges
    inside each slice are filtered, the edge between them is not."""
    s = Stream(3, 2, max_refs=1, crop=(0, 2, 1, 3))
    r = Rng(3)
    for pic in range(3):
        kind = "I" if pic == 0 else "P"
        for first, n in ((0, 4), (4, 2)):
            if kind == "I":
                mbs = [("pcm", r) if (first + i) % 2 else ("i4",) for i in range(n)]
            else:
                mbs = [("p", 0, (pic * 5 + i) % 9 - 4, (pic + i) % 5 - 2) for i in range(n)]
            s.slice(kind, mbs, pic, idr=(pic == 0), qp=38 + pic, first_mb=first,
                    dbk=(2, 2 * pic - 2, 1))
    return bytes(s.out)


STREAMS = {
    "craft_pcm": pcm_stream,
    "craft_mmco_poc0": lambda: mmco_stream(0),
    "craft_mmco_poc1": lambda: mmco_stream(1),
    "craft_mmco_poc2": lambda: mmco_stream(2),
    "craft_dbk2_crop": dbk2_stream,
}


def main(argv):
    if len(argv) != 3 or argv[1] not in STREAMS:
        sys.stderr.write(__doc__)
        return 2
    with open(argv[2], "wb") as f:
        f.write(STREAMS[argv[1]]())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
