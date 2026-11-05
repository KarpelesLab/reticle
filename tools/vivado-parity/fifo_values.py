#!/usr/bin/env python3
"""Solves which bit of a FIFO's `INIT` and `SRVAL` each of its tile's
`ZINIT_A`, `ZINIT_B`, `ZSRVAL_A` and `ZSRVAL_B` bits is, from Vivado's builds of
`gen_fifo.py --values` decoded through `reticle fpga --decode`:

    fifo_values.py <designs> <vivado outputs> > testdata/fpga/vivado/fifo/values.txt

Each design's `<name>/<name>.dec` beside its `.bit` is what `--decode` printed.
A line of the answer is `<cell> <width> <half>.<port>[<j>] <source>`: bit `j` of
the half's `INIT_<port>` and `SRVAL_<port>` registers (`ZINIT_A[17 - j]` and
`ZSRVAL_A[17 - j]` in the tile) is bit `<source>` of the cell's `INIT` and of
its `SRVAL`, solved for both at once, or `=0` or `=1` for a bit no source bit
explains and that is that value in every build. A bit that two source bits
explain alike says `ambiguous`, and one nothing explains `none`: either is a
reason to build more.
"""
import os
import re
import sys


def main():
    designs, builds = sys.argv[1], sys.argv[2]
    samples = {}
    for name in sorted(os.listdir(designs)):
        if not name.endswith(".v"):
            continue
        top = name[:-2]
        dec = os.path.join(builds, top, top + ".dec")
        if not os.path.isfile(dec):
            continue
        src = open(os.path.join(designs, name)).read()
        cell = "FIFO36E1" if "FIFO36E1" in src else "FIFO18E1"
        width = int(re.search(r"DATA_WIDTH\((\d+)\)", src).group(1))
        values = {k: int(h, 16) for k, _, h in re.findall(r"\.(INIT|SRVAL)\((\d+)'h([0-9a-f]+)\)", src)}
        zeros = {}
        for line in open(dec):
            m = re.match(r"BRAM_\S+ (RAMB18_Y[01])\.(ZINIT|ZSRVAL)_([AB])\[(\d+)\]", line)
            if m:
                zeros.setdefault((m.group(1), m.group(2)[1:], m.group(3)), set()).add(int(m.group(4)))
        halves = ["RAMB18_Y0", "RAMB18_Y1"] if cell == "FIFO36E1" else ["RAMB18_Y0"]
        for param in ("INIT", "SRVAL"):
            for half in halves:
                for port in "AB":
                    z = zeros.get((half, param, port), set())
                    for j in range(18):
                        # The register's bit 17 - j, as `ZINIT_A[j]` names it.
                        bit = 0 if j in z else 1
                        # INIT and SRVAL are solved together: a source both agree on.
                        samples.setdefault((cell, width, half, port, j), []).append((values[param], bit))
    bits = {"FIFO18E1": 36, "FIFO36E1": 72}
    print("# Which bit of INIT and SRVAL each ZINIT and ZSRVAL bit of a FIFO's tile is,")
    print("# solved from Vivado 2026.1's builds of `gen_fifo.py --values` by")
    print("# tools/vivado-parity/fifo_values.py: see it for how to read a line.")
    for (cell, width, half, port, j), seen in sorted(samples.items()):
        fits = [k for k in range(bits[cell]) if all(((v >> k) & 1) == b for v, b in seen)]
        consts = [c for c in (0, 1) if all(b == c for _, b in seen)]
        if len(fits) == 1 and not consts:
            source = f"{fits[0]}"
        elif not fits and len(consts) == 1:
            source = f"={consts[0]}"
        elif not fits:
            source = "none"
        else:
            source = "ambiguous"
        print(f"{cell} {width} {half}.{port}[{17 - j}] {source}")


main()
