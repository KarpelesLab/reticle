#!/usr/bin/env python3
"""What Vivado's timer keeps of the clock's pessimism at a setup path's capture,
from clock_pessimism.tcl's dumps.

    clock_pessimism.py <dump>...
        testdata/fpga/vivado/clock/pessimism.txt: for each part (speed grade
        included) and each way a path's two ends relate (`same_site`,
        `one_region`, `other_half` of the region row, `other_row`), over every
        path whose two clock pins the dump timed:
        `pessimism <part> <kind> paths <n> kept <ps> sd <ps> credit <ps>
        late_less_early <ps> late_skew <ps> check <ps>`.

A setup path's launch is timed on the clock's late delay, L (the slow corner's
maximum), and its capture on its early one, E (the slow corner's minimum); the
clock pessimism removal gives back the difference on the part of the two
clock paths they share. With the pad's buffer, its net and the global buffer,
which every path shares, taken out (the source's clock delay less L at the
start, against the destination's less E at the end), what is left of the
removal is the credit on the clock net itself, and what the capture keeps is
its own L less E less that credit. `late_less_early` is the capture's L less E
alone; `late_skew` the mean of L at the end less L at the start, which
`xray::timing::clock_arrival`'s table is for; `check` the most any path's skew
is from the capture's E less the launch's L plus the credit, which is the
decomposition's own check (a picosecond, the dump's rounding).
"""
import collections
import statistics
import sys


def load(path):
    pins, paths, part = {}, [], None
    for line in open(path):
        w = line.split()
        if not w:
            continue
        if w[0] == "part":
            part = w[1]
        elif w[0] == "pin":
            pins[w[2].rsplit("/", 1)[0]] = (w[3], int(w[5]), int(w[6]))
        elif w[0] == "path":
            f = dict(zip(w[8::2], w[9::2]))
            f.update(start=w[2], ss=w[3], sr=w[4], end=w[5], es=w[6], er=w[7])
            paths.append(f)
    return part, pins, paths


def kind(p):
    if p["ss"] == p["es"]:
        return "same_site"
    if p["sr"] == p["er"]:
        return "one_region"
    # A region is X<half>Y<row>.
    if p["sr"].split("Y")[1] == p["er"].split("Y")[1]:
        return "other_half"
    return "other_row"


def main(files):
    by = collections.defaultdict(list)
    uncertainty = {}
    for path in files:
        part, pins, paths = load(path)
        for p in paths:
            s = pins.get(p["start"].rsplit("/", 1)[0])
            d = pins.get(p["end"].rsplit("/", 1)[0])
            if not s or not d:
                continue
            scd, dcd, cpr, skew = (float(p[k]) * 1000 for k in ("scd", "dcd", "cpr", "skew"))
            uncertainty[part] = max(uncertainty.get(part, 0.0), float(p["unc"]) * 1000)
            (_, ls, _), (_, ld, ed) = s, d
            credit = cpr - ((scd - ls) - (dcd - ed))
            by[(part, kind(p))].append(
                ((ld - ed) - credit, credit, ld - ed, ld - ls, skew - ((ed - ls) + credit))
            )
    print("# What Vivado 2026.1's timer keeps of the clock's pessimism at a setup path's")
    print("# capture, ps, by how the path's two ends relate, over each clock's 400 worst")
    print("# paths (one an endpoint) of Reticle's routes of the QoR corpus, each timed at")
    print("# grades 3, 2 and 1: seeds 1 to 3 at xc7a100tftg256 (c73761b1's builds) and")
    print("# seeds 1 and 2 at xc7a35tftg256 (7f45c0f7's).")
    print("# tools/vivado-parity/clock_pessimism.py; see docs/fpga-vivado-parity.md,")
    print("# \"The clock's pessimism between regions\".")
    for part in sorted(uncertainty):
        print(f"uncertainty {part} {uncertainty[part]:.0f}")
    order = ["same_site", "one_region", "other_half", "other_row"]
    for part, k in sorted(by, key=lambda x: (x[0], order.index(x[1]))):
        v = by[(part, k)]
        mean = lambda i: statistics.mean(x[i] for x in v)
        sd = statistics.pstdev(x[0] for x in v)
        check = max(abs(x[4]) for x in v)
        print(
            f"pessimism {part} {k} paths {len(v)} kept {mean(0):.1f} sd {sd:.1f} credit {mean(1):.1f} "
            f"late_less_early {mean(2):.1f} late_skew {mean(3):+.1f} check {check:.0f}"
        )


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
