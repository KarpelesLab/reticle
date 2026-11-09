#!/usr/bin/env python3
"""When a global clock reaches each slice of a part, from clock_arrival.tcl.

    clock_arrival.py fixture --tilegrid <tilegrid.json> <out stem .txt>...
        The runs, one per global buffer, as testdata/fpga/vivado/clock/arrival.txt:
        `hclk <grid y>` for each HCLK row, then `arrival <buffer> <site> <grid x>
        <grid y> <ps>` per flip-flop, the clock net's slow maximum. A run that
        names its part (clock_arrival.tcl writes `part <part>` now) has its
        buffer written `<part>/<buffer>`, so one fixture can hold two packages
        of a die; the xc7a100t's runs predate that line.
    clock_arrival.py table <arrival.txt> [<part prefix>]
        The table xray::timing keeps, of the runs whose buffer starts with
        <part prefix> if one is given: the growth per row away from a region row's
        HCLK row (the median over pairs of one column and region row), and per
        region row and slice column the median arrival at the HCLK row, less the
        earliest, a region row with no slice in a column taking its nearest
        column's; as Rust constants, with the largest residual of any flip-flop.
"""
import collections
import json
import re
import statistics
import sys


def fixture(args):
    grid = args[args.index("--tilegrid") + 1]
    files = [a for a in args if a != "--tilegrid" and a != grid]
    tiles = json.load(open(grid))
    site, hclk = {}, set()
    for t in tiles.values():
        if t["type"] in ("HCLK_L", "HCLK_R"):
            hclk.add(t["grid_y"])
        for s in t.get("sites", {}):
            if s.startswith("SLICE_X"):
                site[s] = (t["grid_x"], t["grid_y"])
    parts = []
    for f in files:
        for line in open(f):
            p = line.split()
            if p[:1] == ["part"] and p[1] not in parts:
                parts.append(p[1])
    if parts:
        die = parts[0][:parts[0].index("t") + 1]
        at = " and ".join(parts)
        print("# When a global clock reaches the clock pin of a flip-flop of each slice of")
        print(f"# the {die}, by Vivado 2026.1 at {at}: the clock net's")
    else:
        print("# When a global clock reaches the clock pin of a flip-flop of each slice of")
        print("# the xc7a100t, by Vivado 2026.1 at xc7a100tftg256-3: the clock net's")
    print("# interconnect delay, the slow corner's maximum, in ps, from one global buffer")
    print("# per run. tools/vivado-parity/clock_arrival.tcl and `clock_arrival.py fixture`;")
    print("# docs/fpga-vivado-parity.md, \"Filling slices\".")
    print("#   hclk <grid y>                                     an HCLK row")
    if parts:
        print("#   arrival <part>/<buffer> <site> <grid x> <grid y> <ps>")
    else:
        print("#   arrival <buffer> <site> <grid x> <grid y> <ps>")
    for y in sorted(hclk):
        print(f"hclk {y}")
    for f in files:
        buffer = part = None
        for line in open(f):
            p = line.split()
            if p[0] in ("buffer", "bufg"):
                buffer = p[1]
            elif p[0] == "part":
                part = p[1]
            elif p[0] == "clk" and p[1] in site:
                if part:
                    buffer = f"{part}/{buffer.split('/')[-1]}"
                x, y = site[p[1]]
                print(f"arrival {buffer} {p[1]} {x} {y} {p[3]}")


def table(path, prefix=""):
    hclk, points = [], []
    for line in open(path):
        p = line.split()
        if not p or p[0].startswith("#"):
            continue
        if p[0] == "hclk":
            hclk.append(int(p[1]))
        elif p[0] == "arrival" and p[1].startswith(prefix):
            points.append((p[1], int(p[3]), int(p[4]), float(p[5])))
    hclk.sort()

    def band(y):
        return min(range(len(hclk)), key=lambda b: (abs(hclk[b] - y), b))

    by = collections.defaultdict(dict)
    for buf, x, y, ps in points:
        by[(buf, x, band(y))][y] = ps
    slopes = []
    for (_, _, b), d in by.items():
        for y in d:
            for y2 in d:
                dy = abs(y - hclk[b]) - abs(y2 - hclk[b])
                if dy > 5:
                    slopes.append((d[y] - d[y2]) / dy)
    k = round(statistics.median(slopes), 2)
    cells = collections.defaultdict(list)
    for _, x, y, ps in points:
        cells[(band(y), x)].append(ps - k * abs(y - hclk[band(y)]))
    cells = {c: statistics.median(v) for c, v in cells.items()}
    base = min(cells.values())
    columns = sorted({x for _, x in cells})
    full = {}
    for b in range(len(hclk)):
        have = [x for x in columns if (b, x) in cells]
        for x in columns:
            near = min(have, key=lambda h: (abs(h - x), h))
            full[(b, x)] = round(cells[(b, near)] - base)
    worst = max(abs(ps - (base + full[(band(y), x)] + k * abs(y - hclk[band(y)])))
                for _, x, y, ps in points)
    print(f"// {len(points)} flip-flops, the largest residual {worst:.1f} ps")
    print(f"const CLOCK_COLUMNS: [u32; {len(columns)}] = {columns};")
    print(f"const CLOCK_ROWS: [u32; {len(hclk)}] = {hclk};")
    print(f"const CLOCK_ARRIVAL_PS: [[u8; {len(columns)}]; {len(hclk)}] = [")
    for b in range(len(hclk)):
        print("    [" + ", ".join(str(full[(b, x)]) for x in columns) + "],")
    print("];")
    print(f"const CLOCK_PS_PER_ROW: f64 = {k};")


if __name__ == "__main__":
    if sys.argv[1:2] == ["fixture"]:
        fixture(sys.argv[2:])
    elif sys.argv[1:2] == ["table"] and len(sys.argv) in (3, 4):
        table(*sys.argv[2:])
    else:
        sys.exit(__doc__)
