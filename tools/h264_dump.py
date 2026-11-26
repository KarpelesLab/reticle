"""Per-macroblock dumps of tools/h264_model.py, the corpora of the H.264 IP.

Usage (through the model):
  h264_model.py dump IN.264 OUTDIR [--name NAME] [--frames N]

writes three files into OUTDIR, NAME defaulting to the stream's base name:

  h264_tokens_NAME.txt   the NAL units and the syntax token stream
  h264_recon_NAME.txt    prediction, residual, samples before and after
                         deblocking, boundary strengths, per macroblock
  h264_inter_NAME.txt    final motion vectors and inter prediction samples
                         per inter macroblock, and every decoded picture

The format is documented in docs/h264-decoder.md, section "Model dumps".
"""

import os
import sys

FORMAT = "h264-model-dump 2"

# Token tags: docs/h264-decoder.md section 1, the parse agent's layout.
SPS_DIMS, SLICE_HDR, SLICE_QP, SLICE_DBLK, SLICE_REF, SLICE_FIRST = 0x01, 0x02, 0x03, 0x04, 0x05, 0x06
SPS_INFO, SPS_POC, SPS_CROP, SLICE_POC, RPLM = 0x07, 0x08, 0x09, 0x0A, 0x0B
PWT_DENOM, PWT, DRPM, MMCO = 0x0C, 0x0D, 0x0E, 0x0F
MB_START, MB_TYPE, MB_QP, I4_MODE, MVD, SUB_MB_TYPE = 0x10, 0x11, 0x12, 0x13, 0x14, 0x15
I8_MODE, REF_IDX, MB_CBP = 0x17, 0x18, 0x19
COEF_BLK, COEF, PCM, MB_END, SLICE_END, PIC_END = 0x20, 0x21, 0x30, 0x3F, 0x40, 0x41

# MB_TYPE kind (section 1.4).
KIND_CODE = {
    "I_NxN": 0, "I_16x16": 1, "I_PCM": 2,
    "P_L0_16x16": 4, "P_L0_L0_16x8": 5, "P_L0_L0_8x16": 6, "P_8x8": 7,
    "P_8x8ref0": 8, "P_Skip": 9, "B_Direct_16x16": 10, "B_8x8": 14, "B_Skip": 15,
}
# COEF_BLK category (section 1.6). CAVLC's four 4x4 parts of an 8x8 block
# are category 2, as the syntax codes them.
COEF_CAT = {
    "Intra16x16DC": 0, "Intra16x16AC": 1, "Luma4x4": 2, "Luma8x8part": 2,
    "ChromaDCB": 3, "ChromaDCR": 4, "ChromaACB": 5, "ChromaACR": 6, "Luma8x8": 7,
}
PRED_BITS = {"L0": 1, "L1": 2, "Bi": 3}


def s_bits(v, n):
    return v & ((1 << n) - 1)


def tok(tag, data):
    return (tag << 24) | (data & 0xFFFFFF)


def hexrow(row):
    return "".join("%02x" % v for v in row)


def kind_code(mb):
    if mb.name.startswith("I_16x16"):
        return KIND_CODE["I_16x16"]
    if mb.name in KIND_CODE:
        return KIND_CODE[mb.name]
    if mb.name.startswith("B_"):
        return {16: 11, 8: 12}[mb.part_h] if mb.part_w == 16 else 13
    raise ValueError(mb.name)


def sub_raw(mb):
    import h264_model as M
    table = M.B_SUB if mb.kind == "B" else M.P_SUB
    names = [s[0] for s in table]
    return [names.index(s[0]) for s in mb.sub]


class Trace:
    def __init__(self, max_frames=None):
        self.max_frames = max_frames
        self.tokens = []        # output lines
        self.recon = []
        self.inter = []
        self.pic_index = -1
        self.active = True

    # -- hooks called by the decoder ---------------------------------------
    def picture_start(self, dec, h, pic):
        self.pic_index += 1
        self.active = self.max_frames is None or self.pic_index < self.max_frames
        pic.dump_index = self.pic_index
        if not self.active:
            return
        t = "IPB"[[2, 0, 1].index(h.slice_type)] if h.slice_type in (0, 1, 2) else "?"
        line = "pic %d %s poc %d frame_num %d ref %d idr %d" % (
            self.pic_index, t, pic.poc, h.frame_num, 1 if h.nal_ref_idc else 0,
            1 if h.idr else 0)
        for out in (self.tokens, self.recon, self.inter):
            out.append("")
            out.append(line)

    def slice(self, dec, h):
        if not self.active:
            return
        pps = h.pps
        sps = h.sps
        lines = self.tokens
        lines.append("slice %d type %d qp %d dbk %d %d %d nref %d %d" % (
            h.first_mb, h.slice_type, h.qp, h.disable_deblocking, h.alpha_off,
            h.beta_off, h.num_ref_idx[0], h.num_ref_idx[1]))
        e = self.emit_tok
        if h.first_mb == 0:
            e(lines, tok(SPS_DIMS, (sps.width_mbs << 12) | sps.height_mbs),
              "SPS_DIMS %dx%d MBs" % (sps.width_mbs, sps.height_mbs))
            e(lines, tok(SPS_INFO, (sps.profile_idc << 16) | (sps.level_idc << 8)
                         | (sps.max_num_ref_frames << 3) | (sps.frame_mbs_only << 2)
                         | (sps.direct_8x8_inference << 1) | sps.gaps_allowed),
              "SPS_INFO profile %d level %d refs %d" % (sps.profile_idc, sps.level_idc,
                                                         sps.max_num_ref_frames))
            lsb = sps.log2_max_poc_lsb if sps.poc_type == 0 else 0
            always0 = getattr(sps, "delta_pic_order_always_zero", 0) if sps.poc_type == 1 else 0
            e(lines, tok(SPS_POC, (sps.poc_type << 22) | (sps.log2_max_frame_num << 17)
                         | (lsb << 12) | (sps.chroma_format_idc << 10)
                         | ((sps.bit_depth_luma - 8) << 7) | ((sps.bit_depth_chroma - 8) << 4)
                         | (always0 << 3)),
              "SPS_POC type %d log2_max_frame_num %d log2_max_poc_lsb %d" % (
                  sps.poc_type, sps.log2_max_frame_num, lsb))
            cl, cr, ct, cb = sps.crop
            e(lines, tok(SPS_CROP, (cl << 18) | (cr << 12) | (ct << 6) | cb),
              "SPS_CROP %d %d %d %d" % sps.crop)
        e(lines, tok(SLICE_HDR, (h.slice_type << 20) | (h.idr << 19) | (h.nal_ref_idc << 17)
                     | (pps.entropy_coding_mode << 16) | (pps.constrained_intra_pred << 15)
                     | (pps.transform_8x8_mode << 14) | (pps.weighted_pred << 13)
                     | (pps.weighted_bipred_idc << 11) | (h.direct_spatial << 10)
                     | (h.cabac_init_idc << 8) | pps.pps_id),
          "SLICE_HDR type %d idr %d nal_ref_idc %d cabac %d" % (
              h.slice_type, h.idr, h.nal_ref_idc, pps.entropy_coding_mode))
        e(lines, tok(SLICE_FIRST, h.first_mb), "SLICE_FIRST %d" % h.first_mb)
        if sps.poc_type == 0:
            e(lines, tok(SLICE_POC, (0 << 22) | h.poc_lsb), "SLICE_POC lsb %d" % h.poc_lsb)
            if pps.bottom_field_pic_order_in_frame_present:
                e(lines, tok(SLICE_POC, (1 << 22) | s_bits(h.delta_poc_bottom, 22)),
                  "SLICE_POC delta_bottom %d" % h.delta_poc_bottom)
        if sps.poc_type == 1 and not sps.delta_pic_order_always_zero:
            e(lines, tok(SLICE_POC, (2 << 22) | s_bits(h.delta_poc[0], 22)),
              "SLICE_POC delta0 %d" % h.delta_poc[0])
            if pps.bottom_field_pic_order_in_frame_present:
                e(lines, tok(SLICE_POC, (3 << 22) | s_bits(h.delta_poc[1], 22)),
                  "SLICE_POC delta1 %d" % h.delta_poc[1])
        n0 = max(h.num_ref_idx[0], 1) - 1
        n1 = max(h.num_ref_idx[1], 1) - 1
        e(lines, tok(SLICE_REF, (n0 << 20) | (n1 << 16) | h.frame_num),
          "SLICE_REF l0 %d l1 %d frame_num %d" % (n0, n1, h.frame_num))
        for lx in (0, 1):
            for idc, v in h.rplm[lx]:
                e(lines, tok(RPLM, (lx << 23) | (idc << 21) | v), "RPLM L%d idc %d %d" % (lx, idc, v))
        if h.wp is not None:
            wp = h.wp
            e(lines, tok(PWT_DENOM, (wp.chroma_log2 << 3) | wp.luma_log2),
              "PWT_DENOM luma %d chroma %d" % (wp.luma_log2, wp.chroma_log2))
            for lx in (0, 1):
                for i, (lw, lo, cws, lf, cf) in enumerate(wp.w[lx]):
                    ents = []
                    if lf:
                        ents.append((0, lw, lo))
                    if cf:
                        ents += [(1, cws[0][0], cws[0][1]), (2, cws[1][0], cws[1][1])]
                    for comp, w, o in ents:
                        e(lines, tok(PWT, (lx << 23) | (i << 18) | (comp << 16)
                                     | (s_bits(w, 8) << 8) | s_bits(o, 8)),
                          "PWT L%d ref %d comp %d w %d o %d" % (lx, i, comp, w, o))
        if h.nal_ref_idc:
            adaptive = 0 if h.idr else (1 if h.mmco is not None else 0)
            e(lines, tok(DRPM, (h.no_output_of_prior_pics << 2) | (h.long_term_reference << 1)
                         | adaptive), "DRPM adaptive %d" % adaptive)
            for op, a, b in (h.mmco or []):
                idx = b if op in (3, 6) else (a if op == 4 else 0)
                num = a if op in (1, 2, 3) else 0
                e(lines, tok(MMCO, (op << 21) | (idx << 16) | num),
                  "MMCO %d %d %d" % (op, idx, num))
        e(lines, tok(SLICE_QP, (s_bits(h.qp, 8) << 16) | (s_bits(pps.chroma_qp_index_offset, 8) << 8)
                     | s_bits(pps.second_chroma_qp_index_offset, 8)),
          "SLICE_QP %d chroma offsets %d %d" % (
              h.qp, pps.chroma_qp_index_offset, pps.second_chroma_qp_index_offset))
        e(lines, tok(SLICE_DBLK, (h.disable_deblocking << 22) | (s_bits(h.alpha_off, 8) << 8)
                     | s_bits(h.beta_off, 8)),
          "SLICE_DBLK idc %d A %d B %d" % (h.disable_deblocking, h.alpha_off, h.beta_off))

    def mb_header(self, sd, mb):
        """Called once mb_type, the prediction, coded_block_pattern and QPY
        are known (and the I_PCM samples read): the tokens up to MB_QP."""
        from h264_model import BLK_RASTER
        t = mb.tokens
        t.append((tok(MB_START, (mb.mbx << 12) | mb.mby), "MB_START %d %d" % (mb.mbx, mb.mby)))
        kc = kind_code(mb)
        p0 = p1 = 0
        if mb.kind in ("P", "B") and mb.num_parts in (1, 2):
            p0 = PRED_BITS[mb.part_pred[0]]
            if mb.num_parts == 2:
                p1 = PRED_BITS[mb.part_pred[1]]
        i16 = mb.i16_mode if mb.kind == "I16" else 0
        t.append((tok(MB_TYPE, (kc << 19) | (p0 << 17) | (p1 << 15) | (i16 << 13)),
                  "MB_TYPE %s" % mb.name))
        if mb.kind in ("P", "B") and mb.num_parts == 4:
            raw = sub_raw(mb)
            t.append((tok(SUB_MB_TYPE, (raw[0] << 12) | (raw[1] << 8) | (raw[2] << 4) | raw[3]),
                      "SUB_MB_TYPE %s" % " ".join(s[0] for s in mb.sub)))
        if mb.kind == "I4":
            for blk in range(16):
                m = mb.modes4[BLK_RASTER[blk]]
                t.append((tok(I4_MODE, (blk << 4) | m), "I4_MODE blk %d mode %d" % (blk, m)))
        if mb.kind == "I8":
            for b8 in range(4):
                m = mb.modes8[b8]
                t.append((tok(I8_MODE, (b8 << 4) | m), "I8_MODE blk %d mode %d" % (b8, m)))
        if mb.kind in ("P", "B"):
            self.inter_tokens(mb)
        chroma = mb.chroma_mode if mb.is_intra and mb.kind != "IPCM" else 0
        cbp = 0 if mb.kind == "IPCM" else mb.cbp
        t.append((tok(MB_CBP, (mb.t8 << 8) | (chroma << 6) | cbp),
                  "MB_CBP cbp %d chroma_pred %d t8 %d" % (cbp, chroma, mb.t8)))
        t.append((tok(MB_QP, s_bits(mb.qp, 8)), "MB_QP %d" % mb.qp))
        if mb.kind == "IPCM":
            s = mb.pcm
            for i in range(0, 384, 3):
                t.append((tok(PCM, s[i] | (s[i + 1] << 8) | (s[i + 2] << 16)), "PCM"))

    def inter_tokens(self, mb):
        t = mb.tokens

        def ref(lx, part, v):
            t.append((tok(REF_IDX, (lx << 23) | (part << 21) | v), "REF_IDX L%d part %d %d" % (lx, part, v)))

        def mvd(lx, part, sub, d):
            for comp in (0, 1):
                data = (lx << 23) | (part << 21) | (sub << 19) | (comp << 18) | s_bits(d[comp], 16)
                t.append((tok(MVD, data), "MVD L%d part %d sub %d %s %d" % (
                    lx, part, sub, "xy"[comp], d[comp])))
        if mb.num_parts == 4:
            for lx in (0, 1):
                for i in range(4):
                    if mb.sub[i][4] in ("L%d" % lx, "Bi"):
                        ref(lx, i, mb.sub_ref[lx][i])
            for lx in (0, 1):
                for i in range(4):
                    s = mb.sub[i]
                    if s[4] in ("L%d" % lx, "Bi"):
                        for j in range(s[1]):
                            mvd(lx, i, j, mb.sub_mvd[lx][i][j])
        elif mb.num_parts:
            for lx in (0, 1):
                for i in range(mb.num_parts):
                    if mb.part_pred[i] in ("L%d" % lx, "Bi"):
                        ref(lx, i, mb.part_ref[lx][i])
            for lx in (0, 1):
                for i in range(mb.num_parts):
                    if mb.part_pred[i] in ("L%d" % lx, "Bi"):
                        mvd(lx, i, 0, mb.part_mvd[lx][i])

    def coef(self, sd, mb, cat, idx, coeffs, n):
        t = mb.tokens
        c = COEF_CAT[cat]
        t.append((tok(COEF_BLK, (c << 20) | (idx << 16) | min(n, 31)),
                  "COEF_BLK %s %d total %d" % (cat, idx, n)))
        for pos in range(len(coeffs) - 1, -1, -1):
            v = coeffs[pos]
            if v:
                t.append((tok(COEF, (pos << 18) | s_bits(v, 16)), "COEF %d %d" % (pos, v)))

    def mb_done(self, sd, mb):
        if mb.skip:
            mb.tokens = [(tok(MB_START, (mb.mbx << 12) | mb.mby), "MB_START %d %d" % (mb.mbx, mb.mby)),
                         (tok(MB_TYPE, kind_code(mb) << 19), "MB_TYPE %s" % mb.name),
                         (tok(MB_CBP, 0), "MB_CBP 0"),
                         (tok(MB_QP, s_bits(mb.qp, 8)), "MB_QP %d" % mb.qp)]
        mb.tokens.append((tok(MB_END, 0), "MB_END"))
        if not self.active:
            return
        lines = self.tokens
        lines.append("mb %d %d %d %s" % (mb.addr, mb.mbx, mb.mby, mb.name))
        for word, text in mb.tokens:
            # COEF and PCM words carry no comment: they are the bulk of the
            # file and their fields are plain (docs/h264-decoder.md).
            if (word >> 24) in (COEF, PCM):
                lines.append("tok %08x" % word)
            else:
                lines.append("tok %08x %s" % (word, text))


    def end_slice(self):
        if self.active:
            self.emit_tok(self.tokens, tok(SLICE_END, 0), "SLICE_END")

    def pre_deblock(self, dec, pic):
        pic.pre = (bytes(pic.Y), bytes(pic.U), bytes(pic.V))

    def picture_end(self, dec, pic):
        if not self.active:
            return
        self.emit_tok(self.tokens, tok(PIC_END, 0), "PIC_END")
        W = pic.w // 16
        pre_y, pre_u, pre_v = pic.pre
        for mb in pic.mbs:
            self.recon_mb(pic, mb, pre_y, pre_u, pre_v)
            if mb.kind in ("P", "PSKIP", "B", "BSKIP"):
                self.inter_mb(pic, mb)
        _ = W
        for name, plane, w, h in (("y", pic.Y, pic.w, pic.h), ("u", pic.U, pic.cw, pic.ch),
                                  ("v", pic.V, pic.cw, pic.ch)):
            self.inter.append("frame %s %s" % (name, " ".join(
                hexrow(plane[y * w:(y + 1) * w]) for y in range(h))))

    # -- per macroblock output ----------------------------------------------
    def recon_mb(self, pic, mb, pre_y, pre_u, pre_v):
        out = self.recon
        out.append("mb %d %d %d %s qp %d" % (mb.addr, mb.mbx, mb.mby, mb.name,
                                             0 if mb.kind == "IPCM" else mb.qp))
        X, Y = mb.mbx * 16, mb.mby * 16
        if mb.kind != "IPCM":
            out.append("pred y %s" % " ".join(hexrow(r) for r in mb.pred_y))
            out.append("pred u %s" % " ".join(hexrow(r) for r in mb.pred_u))
            out.append("pred v %s" % " ".join(hexrow(r) for r in mb.pred_v))
            out.append("res y %s" % " ".join(str(v) for r in mb.res_y for v in r))
            out.append("res u %s" % " ".join(str(v) for r in mb.res_u for v in r))
            out.append("res v %s" % " ".join(str(v) for r in mb.res_v for v in r))
        for tag, (py, pu, pv) in (("pre", (pre_y, pre_u, pre_v)), ("post", (pic.Y, pic.U, pic.V))):
            out.append("%s y %s" % (tag, " ".join(
                hexrow(py[(Y + j) * pic.w + X:(Y + j) * pic.w + X + 16]) for j in range(16))))
            cx, cy = X >> 1, Y >> 1
            out.append("%s u %s" % (tag, " ".join(
                hexrow(pu[(cy + j) * pic.cw + cx:(cy + j) * pic.cw + cx + 8]) for j in range(8))))
            out.append("%s v %s" % (tag, " ".join(
                hexrow(pv[(cy + j) * pic.cw + cx:(cy + j) * pic.cw + cx + 8]) for j in range(8))))
        for d, name in ((0, "v"), (1, "h")):
            edges = []
            for e in range(4):
                seg = mb.bs[d][e]
                edges.append("-" if seg is None else "".join(str(b) for b in seg))
            out.append("bs %s %s" % (name, " ".join(edges)))

    def inter_mb(self, pic, mb):
        out = self.inter
        out.append("mb %d %d %d %s" % (mb.addr, mb.mbx, mb.mby, mb.name))
        for lx in (0, 1):
            if not any(mb.pred_flag[lx]):
                continue
            ent = []
            for r in range(16):
                if mb.pred_flag[lx][r]:
                    ent.append("%d:%d:%d" % (mb.ref[lx][r], mb.mv[lx][r][0], mb.mv[lx][r][1]))
                else:
                    ent.append("-")
            out.append("mv l%d %s" % (lx, " ".join(ent)))
            refs = sorted({mb.ref[lx][r] for r in range(16) if mb.pred_flag[lx][r]})
            out.append("refpic l%d %s" % (lx, " ".join(
                "%d=%d" % (i, mb.refpic[lx][[r for r in range(16) if mb.ref[lx][r] == i
                                              and mb.pred_flag[lx][r]][0]].dump_index)
                for i in refs)))
        words = []
        for rows in (mb.pred_y, mb.pred_u, mb.pred_v):
            for row in rows:
                for i in range(0, len(row), 4):
                    words.append("%08x" % (row[i] | (row[i + 1] << 8) | (row[i + 2] << 16)
                                           | (row[i + 3] << 24)))
        out.append("pred %s" % " ".join(words))

    # -- helpers -------------------------------------------------------------
    def emit_tok(self, lines, word, text):
        lines.append("tok %08x %s" % (word, text))


HEADER = {
    "tokens": """# H.264 model dump: NAL units and the syntax token stream ({fmt}).
# Stream {name}: {desc}
# Made by tools/h264_model.py (clean-room, checked bit-exact against ffmpeg)
# from testdata/ip/h264/{name}.264; the format is in docs/h264-decoder.md,
# section "Model dumps". Do not edit: regenerate with
#   python3 tools/h264_model.py dump testdata/ip/h264/{name}.264 testdata/ip{frames}
""",
    "recon": """# H.264 model dump: reconstruction per macroblock ({fmt}).
# Stream {name}: {desc}
# Made by tools/h264_model.py from testdata/ip/h264/{name}.264; the format
# is in docs/h264-decoder.md, section "Model dumps". Regenerate with
#   python3 tools/h264_model.py dump testdata/ip/h264/{name}.264 testdata/ip{frames}
""",
    "inter": """# H.264 model dump: motion vectors and inter prediction ({fmt}).
# Stream {name}: {desc}
# Made by tools/h264_model.py from testdata/ip/h264/{name}.264; the format
# is in docs/h264-decoder.md, section "Model dumps". Regenerate with
#   python3 tools/h264_model.py dump testdata/ip/h264/{name}.264 testdata/ip{frames}
""",
}


def main(argv):
    import h264_model as M
    src = argv[2]
    outdir = argv[3]
    name = os.path.basename(src)
    if name.endswith(".264"):
        name = name[:-4]
    if "--name" in argv:
        name = argv[argv.index("--name") + 1]
    frames = None
    if "--frames" in argv:
        frames = int(argv[argv.index("--frames") + 1])
    desc = ""
    if "--desc" in argv:
        desc = argv[argv.index("--desc") + 1]
    os.makedirs(outdir, exist_ok=True)
    tr = Trace(frames)
    with open(src, "rb") as f:
        data = f.read()
    dec = M.Decoder(tr)
    nals = M.nal_units(data)
    nal_lines = []
    for nal in nals:
        dec.decode_nal(nal)
        if (nal[0] & 31) in (1, 5):
            tr.end_slice()
        if tr.active:
            nal_lines.append("nal %s" % nal.hex())
    dec.finish_picture()
    tr.tokens = nal_lines + tr.tokens
    fr = " --frames %d" % frames if frames is not None else ""
    for kind, lines in (("tokens", tr.tokens), ("recon", tr.recon), ("inter", tr.inter)):
        if kind == "inter" and not any(l.startswith("mv ") for l in lines):
            continue  # no inter macroblock: nothing for the inter agent
        path = os.path.join(outdir, "h264_%s_%s.txt" % (kind, name))
        with open(path, "w", newline="\n") as f:
            f.write(HEADER[kind].format(fmt=FORMAT, name=name, desc=desc, frames=fr))
            f.write("stream %s %d %d\n" % (name, dec.sps[min(dec.sps)].width_mbs,
                                           dec.sps[min(dec.sps)].height_mbs))
            f.write("\n".join(lines).lstrip("\n") + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
