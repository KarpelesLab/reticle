#!/usr/bin/env python3
"""Writes a small hand-built H.264 stream that no encoder at hand produces:
Exp-Golomb codes longer than 32 bits, I_PCM macroblocks in an I and a P slice, an Intra_16x16 macroblock whose nC
comes from an I_PCM neighbour, memory_management_control_operations and
explicit deblocking offsets.

    python3 ip/video/h264_parse/tools/make_pcm.py out.264
    python3 ip/video/h264_parse/tools/make_corpus.py pcm file:out.264 > testdata/ip/h264_parse_pcm.txt

Written from the syntax tables of ITU-T H.264 7.3; ffmpeg's decoder is the
check that it is a valid stream (make_corpus.py records what it decodes).
"""
import sys


class Bits:
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

    def align0(self):
        while len(self.bits) % 8:
            self.bits.append(0)

    def trailing(self):
        self.bits.append(1)
        self.align0()

    def bytes(self):
        assert len(self.bits) % 8 == 0
        return bytes(int(''.join(map(str, self.bits[i:i + 8])), 2) for i in range(0, len(self.bits), 8))


def nal(header, payload):
    out = bytearray(b'\x00\x00\x00\x01')
    out.append(header)
    zeros = 0
    for b in payload:
        if zeros >= 2 and b <= 3:
            out.append(3)
            zeros = 0
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
    return bytes(out)


def pcm(b, seed):
    b.align0()
    for i in range(384):
        # Zeros included on purpose: runs of them need emulation prevention.
        b.u(8, 0 if (i + seed) % 7 < 2 else (i * 37 + seed * 11) & 0xFF)


def main():
    out = bytearray()
    # SPS: Baseline, 2x2 macroblocks, pic_order_cnt_type 2.
    b = Bits()
    b.u(8, 66); b.u(8, 0xC0); b.u(8, 10); b.ue(0)
    b.ue(0)        # log2_max_frame_num_minus4
    # pic_order_cnt_type 1, with values whose codes are longer than 32
    # bits, so that the parser's two-cycle Exp-Golomb path is taken.
    b.ue(1)        # pic_order_cnt_type
    b.u(1, 0)      # delta_pic_order_always_zero_flag
    b.se(-100000)  # offset_for_non_ref_pic
    b.se(0)        # offset_for_top_to_bottom_field
    b.ue(1)        # num_ref_frames_in_pic_order_cnt_cycle
    b.se(2)        # offset_for_ref_frame[0]
    b.ue(2)        # max_num_ref_frames: the P picture marks adaptively, so both stay
    b.u(1, 0)      # gaps
    b.ue(1); b.ue(1)
    b.u(1, 1); b.u(1, 1); b.u(1, 0); b.u(1, 0)
    b.trailing()
    out += nal(0x67, b.bytes())
    # PPS
    b = Bits()
    b.ue(0); b.ue(0); b.u(1, 0); b.u(1, 0); b.ue(0); b.ue(0); b.ue(0)
    b.u(1, 0); b.u(2, 0); b.se(0); b.se(0); b.se(0)
    b.u(1, 1)      # deblocking_filter_control_present_flag
    b.u(1, 0); b.u(1, 0)
    b.trailing()
    out += nal(0x68, b.bytes())
    # IDR: I_PCM, I_PCM, I_16x16 (DC, no residual but its DC block), I_PCM.
    b = Bits()
    b.ue(0); b.ue(7); b.ue(0); b.u(4, 0)
    b.ue(65535)                   # idr_pic_id, a 33-bit code
    b.se(0)                       # delta_pic_order_cnt[0]
    b.u(1, 0); b.u(1, 0)          # dec_ref_pic_marking
    b.se(0)                       # slice_qp_delta
    b.ue(1)                       # disable_deblocking_filter_idc
    b.ue(25); pcm(b, 1)
    b.ue(25); pcm(b, 2)
    b.ue(3)                       # I_16x16_2_0_0
    b.ue(0)                       # intra_chroma_pred_mode
    b.se(0)                       # mb_qp_delta
    b.u(6, 0b000011)              # coeff_token, nC = 16 (an I_PCM above), TotalCoeff 0
    b.ue(25); pcm(b, 3)
    b.trailing()
    out += nal(0x65, b.bytes())
    # P: skip, I_PCM, P_L0_16x16 with no residual, skip; MMCOs and offsets.
    b = Bits()
    b.ue(0); b.ue(5); b.ue(0); b.u(4, 1)
    b.se(70000)                   # delta_pic_order_cnt[0], a 35-bit code
    b.u(1, 0)                     # num_ref_idx_active_override_flag
    b.u(1, 0)                     # ref_pic_list_modification_flag_l0
    b.u(1, 1)                     # adaptive_ref_pic_marking_mode_flag
    b.ue(4); b.ue(1)              # max_long_term_frame_idx_plus1 = 1
    b.ue(0)
    b.se(-3)                      # slice_qp_delta
    b.ue(0); b.se(1); b.se(-2)    # deblocking: idc 0, alpha +2, beta -4
    b.ue(1)                       # mb_skip_run
    b.ue(30); pcm(b, 4)           # I_PCM in a P slice
    b.ue(0)
    b.ue(0)                       # P_L0_16x16
    b.se(3); b.se(-1)             # mvd
    b.ue(0)                       # coded_block_pattern 0
    b.ue(1)                       # mb_skip_run
    b.trailing()
    out += nal(0x41, b.bytes())
    open(sys.argv[1], 'wb').write(out)


if __name__ == '__main__':
    main()
