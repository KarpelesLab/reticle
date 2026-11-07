#!/usr/bin/env python3
"""The arrangement of each memory of testdata/fpga/vivado/memarr/, Vivado's
beside Reticle's: `memarr_table.py <lutram.sh output> [<label>=<run_reticle output>...]`.

Vivado's is read off `lutram_probe.tcl`'s `<top>.cells` (its routed cells and
each block's `params`), Reticle's off the log of a build with `--report`: the
mapping report's block RAM lines and the netlist's cell counts. A block is
written `<count> <primitive> <mode> <read width>[/<write width>][ reg]`, where
the width is Vivado's parameter (`READ_WIDTH_A` in `SDP`, `READ_WIDTH_B`
otherwise) and `reg` an output register in the block; distributed RAM is its
cells; `LUT` and `FF` are every lookup table and flip-flop of the design,
`MUXF` the slice's multiplexers.
"""
import os
import re
import sys
from collections import Counter

# A depth's width, by primitive, as the parameters name it.
WIDTH_36 = {512: 72, 1024: 36, 2048: 18, 4096: 9, 8192: 4, 16384: 2, 32768: 1}
WIDTH_18 = {512: 36, 1024: 18, 2048: 9, 4096: 4, 8192: 2, 16384: 1}
LUTRAM = ("RAM32M", "RAM64M", "RAM32X1D", "RAM64X1D", "RAM128X1D", "RAM32X1S", "RAM64X1S",
          "RAM128X1S", "RAM256X1S")


def vivado(path):
    try:
        text = open(path).read()
    except OSError:
        return None
    routed = Counter()
    blocks = Counter()
    for line in text.splitlines():
        words = line.split()
        if not words:
            continue
        if words[0] == "routed":
            for k, v in zip(words[1::2], words[2::2]):
                routed[k] += int(v)
        elif words[0] == "params":
            p = dict(w.split("=", 1) for w in words[3:])
            mode = p.get("RAM_MODE", "TDP")
            if mode == "SDP":
                width = f"{p['READ_WIDTH_A']}/{p['WRITE_WIDTH_B']}"
            else:
                width = p.get("READ_WIDTH_B")
                if width == "0" and p.get("WRITE_WIDTH_B") == "0":
                    # One port, A.
                    width = f"{p.get('READ_WIDTH_A')} A"
                elif p.get("READ_WIDTH_A") != width or p.get("WRITE_WIDTH_A") != width:
                    width = (f"{p.get('WRITE_WIDTH_A')}w{p.get('READ_WIDTH_A')}r/"
                             f"{p.get('WRITE_WIDTH_B')}w{width}r")
            reg = " reg" if p.get("DOA_REG") == "1" or p.get("DOB_REG") == "1" else ""
            ext = p.get("RAM_EXTENSION_A", "NONE")
            ext = "" if ext == "NONE" else f" cascade {ext}"
            blocks[f"{words[2]} {mode} {width}{reg}{ext}"] += 1
    if not routed:
        return None
    return routed, blocks


def reticle(path):
    try:
        text = open(path).read()
    except OSError:
        return None
    if "netlist:" not in text:
        return None
    cells = Counter()
    blocks = Counter()
    section = None
    for line in text.splitlines():
        if not line.startswith("  "):
            section = line.rstrip(":")
            continue
        if section == "netlist":
            m = re.match(r"\s+(\d+) x (\S+)", line)
            if m:
                cells[m.group(2)] += int(m.group(1))
        elif section == "block RAM" and " -> " in line and " mode" in line:
            sdp = ", simple dual port" in line
            reg = " A" if "1 read(s) sharing a write's port" in line else ""
            reg += " reg" if "output register(s) in the block" in line else ""
            first = True
            for m in re.finditer(r"(\d+) x (\S+) in (\d+)x(\d+) mode", line):
                count, prim, width, depth = (int(m.group(1)), m.group(2), int(m.group(3)),
                                             int(m.group(4)))
                if first and sdp:
                    blocks[f"{prim} SDP {width}/{width}{reg}"] += count
                else:
                    table = WIDTH_36 if prim == "RAMB36E1" else WIDTH_18
                    blocks[f"{prim} TDP {table.get(depth, '?')}{reg}"] += count
                first = False
    return cells, blocks


def summary(cells, blocks):
    luts = sum(v for k, v in cells.items() if re.fullmatch(r"LUT\d", k))
    ffs = sum(v for k, v in cells.items() if re.fullmatch(r"FD[A-Z]{2}", k))
    muxf = sum(v for k, v in cells.items() if k.startswith("MUXF"))
    parts = [f"{n} {b}" for b, n in sorted(blocks.items())]
    parts += [f"{v} {k}" for k, v in sorted(cells.items()) if k in LUTRAM]
    srl = sum(v for k, v in cells.items() if k.startswith("SRL"))
    arrangement = ", ".join(parts) or "logic"
    extra = f" MUXF {muxf}" if muxf else ""
    extra += f" SRL {srl}" if srl else ""
    return arrangement, f"LUT {luts} FF {ffs}{extra}"


def main():
    viv = sys.argv[1]
    runs = [a.split("=", 1) for a in sys.argv[2:]]
    names = sorted(d for d in os.listdir(viv) if os.path.isdir(os.path.join(viv, d))
                   and not d.endswith(".tmp"))
    for name in names:
        v = vivado(os.path.join(viv, name, f"{name}.cells"))
        line = [name]
        line.append("Vivado: " + (" | ".join(summary(*v)) if v else "none"))
        for label, out in runs:
            r = reticle(os.path.join(out, f"{name}.log"))
            line.append(f"{label}: " + (" | ".join(summary(*r)) if r else "refused"))
        print("\n  ".join(line))


main()
