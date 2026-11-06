#!/usr/bin/env python3
"""How two tools packed the same designs into slices, from slices.tcl's output.

    slices.py table <vivado dir> <reticle dir>
        Per design (a subdirectory of both): slices used, flip-flops in a
        slice that holds any, how many slices hold more than four, and Vivado's
        `Register driven from within the Slice` against its registers.
    slices.py connections <dir>...
        Per design, each kind of connection (lookup table, flip-flop, carry,
        shift register, RAM, driver to load): how many, the share inside one
        slice and inside one tile, and the mean distance in slices.
"""
import collections
import os
import re
import sys


def load(d):
    cells, site = {}, None
    kinds = {}
    for line in open(os.path.join(d, "sites.txt")):
        p = line.split()
        if line.startswith("site "):
            site = p[1]
            kinds[site] = (p[2], p[3])
        elif len(p) >= 4 and p[0] == "cell":
            cells[p[2]] = (site, p[1].split(".")[-1], p[3], kinds[site][1])
    return cells, kinds


def kind(ref):
    for prefix, name in (("LUT", "lut"), ("FD", "ff"), ("LD", "ff"), ("CARRY4", "carry"),
                         ("SRL", "srl"), ("RAM", "ram")):
        if ref.startswith(prefix):
            return name
    return None


def table(viv, ret):
    def stats(d):
        cells, kinds = load(d)
        slices = [s for s, (t, _) in kinds.items() if t.startswith("SLICE")]
        ffs = collections.Counter(s for (s, _, r, _) in cells.values() if r.startswith("FD"))
        within = None
        for line in open(os.path.join(d, "util.rpt")):
            m = re.search(r"Register driven from within the Slice\s*\|\s*(\d+)", line)
            if m:
                within = int(m.group(1))
        held = [n for n in ffs.values() if n]
        return (len(slices), sum(held) / max(1, len(held)), sum(1 for n in held if n > 4),
                within, sum(held))
    print("| design | slices, Vivado | Reticle | flip-flops a slice, Vivado | Reticle "
          "| slices of more than four, Vivado | Reticle | fed inside the slice, Vivado | Reticle |")
    print("|---|---|---|---|---|---|---|---|---|")
    for name in sorted(os.listdir(viv)):
        if not os.path.isfile(os.path.join(ret, name, "sites.txt")):
            continue
        a, b = stats(os.path.join(viv, name)), stats(os.path.join(ret, name))
        print(f"| {name} | {a[0]} | {b[0]} | {a[1]:.1f} | {b[1]:.1f} | {a[2]} | {b[2]} "
              f"| {a[3]} of {a[4]} | {b[3]} of {b[4]} |")


def connections(d):
    cells, _ = load(d)
    xy = lambda s: tuple(map(int, re.match(r"SLICE_X(\d+)Y(\d+)", s).groups())) \
        if s and s.startswith("SLICE_X") else None
    conn = collections.defaultdict(lambda: [0, 0, 0, 0])
    for line in open(os.path.join(d, "nets.txt")):
        p = line.split()
        if len(p) < 4:
            continue
        dc = p[3].rsplit("/", 1)[0]
        if dc not in cells or kind(cells[dc][2]) is None:
            continue
        for load_pin in p[4:]:
            lc = load_pin.rsplit("/", 1)[0]
            if lc not in cells or kind(cells[lc][2]) is None:
                continue
            k = conn[kind(cells[dc][2]) + ">" + kind(cells[lc][2])]
            k[0] += 1
            k[1] += cells[lc][0] == cells[dc][0]
            k[2] += cells[lc][3] == cells[dc][3]
            a, b = xy(cells[dc][0]), xy(cells[lc][0])
            if a and b:
                k[3] += abs(a[0] - b[0]) // 2 + abs(a[1] - b[1])
    print(f"== {d}")
    for name in sorted(conn):
        n, site, tile, dist = conn[name]
        if n >= 4:
            print(f"  {name:12s} {n:6d}  slice {100 * site / n:5.1f}%  tile {100 * tile / n:5.1f}%"
                  f"  distance {dist / n:5.2f}")


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "table":
        table(sys.argv[2], sys.argv[3])
    elif len(sys.argv) >= 3 and sys.argv[1] == "connections":
        for d in sys.argv[2:]:
            connections(d)
    else:
        sys.exit(__doc__)
