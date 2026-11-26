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

FORMAT = "h264-model-dump 1"

# Token tags (the contract's v0 set; see docs/h264-decoder.md).
SPS_DIMS, SLICE_HDR, SLICE_QP, SLICE_DBLK, SLICE_REF, SLICE_FIRST = 0x01, 0x02, 0x03, 0x04, 0x05, 0x06
MB_START, MB_TYPE, MB_QP, I4_MODE, MVD, MV = 0x10, 0x11, 0x12, 0x13, 0x14, 0x16
COEF_BLK, COEF, PCM, MB_END, SLICE_END, PIC_END = 0x20, 0x21, 0x30, 0x3F, 0x40, 0x41
I8_MODE = 0x15

KIND_CODE = {
    "I_NxN": 0, "I8x8": 1, "I_16x16": 2, "I_PCM": 3,
    "P_L0_16x16": 4, "P_L0_L0_16x8": 5, "P_L0_L0_8x16": 6, "P_8x8": 7,
    "P_8x8ref0": 8, "P_Skip": 9, "B_Skip": 10, "B_Direct_16x16": 11,
    "B_8x8": 12,
}
COEF_CAT = {
    "Luma4x4": 0, "Intra16x16DC": 1, "Intra16x16AC": 2, "ChromaDCB": 3,
    "ChromaDCR": 4, "ChromaACB": 5, "ChromaACR": 6, "Luma8x8part": 7,
    "Luma8x8": 8,
}


def s_bits(v, n):
    return v & ((1 << n) - 1)


def tok(tag, data):
    return (tag << 24) | (data & 0xFFFFFF)


def hexrow(row):
    return "".join("%02x" % v for v in row)


def kind_code(mb):
    if mb.kind == "I8":
        return KIND_CODE["I8x8"]
    if mb.name.startswith("I_16x16"):
        return KIND_CODE["I_16x16"]
    if mb.name in KIND_CODE:
        return KIND_CODE[mb.name]
    if mb.name.startswith("B_"):
        return 16 + mb.mb_type  # B macroblock types by their mb_type value
    raise ValueError(mb.name)


class Trace:
    def __init__(self, max_frames=None):
        self.max_frames = max_frames
        self.tokens = []        # output lines
        self.recon = []
        self.inter = []
        self.pic_index = -1
        self.nals = []
        self.pending_slice_tokens = []
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
        sps = h.sps
        self.emit_tok(self.tokens, tok(SPS_DIMS, (sps.width_mbs << 12) | sps.height_mbs),
                      "SPS_DIMS %dx%d MBs" % (sps.width_mbs, sps.height_mbs))

    def slice(self, dec, h):
        if not self.active:
            return
        pps = h.pps
        lines = self.tokens
        lines.append("slice %d type %d qp %d dbk %d %d %d nref %d" % (
            h.first_mb, h.slice_type, h.qp, h.disable_deblocking, h.alpha_off,
            h.beta_off, h.num_ref_idx[0]))
        self.emit_tok(lines, tok(SLICE_HDR, h.slice_type | (h.idr << 4) | (h.nal_ref_idc << 5)),
                      "SLICE_HDR type %d idr %d nal_ref_idc %d" % (h.slice_type, h.idr, h.nal_ref_idc))
        self.emit_tok(lines, tok(SLICE_QP, h.qp | (s_bits(pps.chroma_qp_index_offset, 5) << 8)
                                 | (s_bits(pps.second_chroma_qp_index_offset, 5) << 16)),
                      "SLICE_QP %d chroma offsets %d %d" % (
                          h.qp, pps.chroma_qp_index_offset, pps.second_chroma_qp_index_offset))
        self.emit_tok(lines, tok(SLICE_DBLK, h.disable_deblocking | (s_bits(h.alpha_off, 5) << 8)
                                 | (s_bits(h.beta_off, 5) << 16)),
                      "SLICE_DBLK idc %d A %d B %d" % (h.disable_deblocking, h.alpha_off, h.beta_off))
        nref = max(h.num_ref_idx[0], 1) - 1
        self.emit_tok(lines, tok(SLICE_REF, nref | (h.frame_num << 8)),
                      "SLICE_REF num_ref_idx_l0_active_minus1 %d frame_num %d" % (nref, h.frame_num))
        self.emit_tok(lines, tok(SLICE_FIRST, h.first_mb), "SLICE_FIRST %d" % h.first_mb)
        self.last_slice_h = h

    def mb_header(self, sd, mb):
        """Called after mb_type/mb_pred/cbp/mb_qp_delta (and PCM samples)."""
        t = mb.tokens
        t.append((tok(MB_START, (mb.mbx << 12) | mb.mby), "MB_START %d %d" % (mb.mbx, mb.mby)))
        kc = kind_code(mb)
        d = kc | (mb.i16_mode << 6) | (mb.chroma_mode << 8) | (mb.cbp << 10) | (mb.t8 << 16)
        t.append((tok(MB_TYPE, d), "MB_TYPE %s cbp %d i16 %d chroma %d t8 %d" % (
            mb.name, mb.cbp, mb.i16_mode, mb.chroma_mode, mb.t8)))
        t.append((tok(MB_QP, mb.qp), "MB_QP %d" % mb.qp))
        if mb.kind == "IPCM":
            s = mb.pcm
            for i in range(0, 384, 3):
                t.append((tok(PCM, s[i] | (s[i + 1] << 8) | (s[i + 2] << 16)), "PCM"))
        if mb.kind == "I4":
            from h264_model import BLK_RASTER
            for blk in range(16):
                m = mb.modes4[BLK_RASTER[blk]]
                t.append((tok(I4_MODE, (blk << 4) | m), "I4_MODE blk %d mode %d" % (blk, m)))
        if mb.kind == "I8":
            for b8 in range(4):
                m = mb.modes8[b8]
                t.append((tok(I8_MODE, (b8 << 4) | m), "I8_MODE blk %d mode %d" % (b8, m)))
        if mb.kind in ("P", "B"):
            self.mvd_tokens(mb)

    def mvd_tokens(self, mb):
        t = mb.tokens

        def one(lx, part, sub, ref, d):
            for comp in (0, 1):
                data = (part << 22) | (sub << 20) | (ref << 16) | (comp << 15) | s_bits(d[comp], 15)
                t.append((tok(MVD, data), "MVD L%d part %d sub %d ref %d %s %d" % (
                    lx, part, sub, ref, "xy"[comp], d[comp])))
        if mb.num_parts == 4:
            for lx in (0, 1):
                for i in range(4):
                    s = mb.sub[i]
                    if s[4] in ("L%d" % lx, "Bi"):
                        for j in range(s[1]):
                            one(lx, i, j, mb.sub_ref[lx][i], mb.sub_mvd[lx][i][j])
        elif mb.num_parts:
            for lx in (0, 1):
                for i in range(mb.num_parts):
                    if mb.part_pred[i] in ("L%d" % lx, "Bi"):
                        one(lx, i, 0, mb.part_ref[lx][i], mb.part_mvd[lx][i])

    def coef(self, sd, mb, cat, idx, coeffs, n):
        t = mb.tokens
        c = COEF_CAT[cat]
        t.append((tok(COEF_BLK, (c << 16) | (idx << 8) | n), "COEF_BLK %s %d total %d" % (cat, idx, n)))
        for pos, v in enumerate(coeffs):
            if v:
                t.append((tok(COEF, (pos << 16) | s_bits(v, 16)), "COEF %d %d" % (pos, v)))

    def mb_done(self, sd, mb):
        if mb.skip:
            mb.tokens = [(tok(MB_START, (mb.mbx << 12) | mb.mby), "MB_START %d %d" % (mb.mbx, mb.mby)),
                         (tok(MB_TYPE, kind_code(mb)), "MB_TYPE %s" % mb.name),
                         (tok(MB_QP, mb.qp), "MB_QP %d" % mb.qp)]
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
