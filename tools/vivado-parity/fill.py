#!/usr/bin/env python3
"""What fill.tcl's runs say: critical paths, flip-flop setups, connection delays.

    fill.py paths [-n N] <dir>...
        Per design directory (fill.tcl `placed` or `netlist`) and clock, over its N
        worst paths (50): the worst slack and data path, the mean logic levels and
        hops, the share of hops inside one slice and inside one tile (a tile is two
        slices, SLICE_X pairs), the mean hop's distance in tiles (|dx| / 2 + |dy|)
        and the mean path's span, the sum of its hops' distances.
    fill.py setups <root>...
        Each root holds design directories of fill.tcl `placed`; every flip-flop of
        all of them, classed by its slice (SLICEL, SLICEM) and how its data reaches
        it by the site pin Vivado names (none: inside the slice; <L>X: the bypass;
        <L>6: the upper half's way through the lane's table; <L>1 to <L>5: the lower
        half's), with how many and the mean of Vivado's setup in each.
    fill.py distance --tilegrid <tilegrid.json> [--fanout N] <connections.txt>...
        A line fitted by least squares to the interconnect delay of every
        connection of fanout N or less (all), against four measures of its length:
        grid tiles, grid columns and rows apart, interconnect columns and rows apart
        (the rank of the INT_L/INT_R column each slice's tile uses, and of its row),
        each with its residual. And on one row, the delay by interconnect columns.
"""
import collections
import json
import math
import os
import re
import statistics
import sys


def xy(site):
    m = re.match(r"SLICE_X(\d+)Y(\d+)", site or "")
    return (int(m[1]), int(m[2])) if m else None


def paths(args):
    n = 50
    if args[:1] == ["-n"]:
        n, args = int(args[1]), args[2:]
    for d in args:
        per = collections.defaultdict(list)
        pins = []
        for line in open(os.path.join(d, "paths.txt")):
            p = line.split()
            if p[0] == "path":
                pins = []
                per[p[1]].append((float(p[2]), float(p[3]), int(p[4]), pins))
            elif p[0] == "pin":
                pins.append((p[1].rsplit("/", 1)[0], p[2]))
        for clock, ps in sorted(per.items()):
            ps = ps[:n]
            hops = slice_ = tile = 0
            dist = span = levels = 0.0
            for _, _, lv, pins in ps:
                levels += lv
                for (ca, sa), (cb, sb) in zip(pins, pins[1:]):
                    if ca == cb:
                        continue
                    hops += 1
                    slice_ += sa == sb
                    a, b = xy(sa), xy(sb)
                    if a and b:
                        tile += a[0] // 2 == b[0] // 2 and a[1] == b[1]
                        d_ = abs(a[0] // 2 - b[0] // 2) + abs(a[1] - b[1])
                        dist += d_
                        span += d_
            k = len(ps)
            print(f"{os.path.basename(d.rstrip('/')):14s} {clock:14s} worst {ps[0][0]:7.3f} "
                  f"delay {ps[0][1]:6.3f} levels {levels / k:4.1f} hops {hops / k:4.1f} "
                  f"slice {100 * slice_ / max(1, hops):5.1f}% tile {100 * tile / max(1, hops):5.1f}% "
                  f"distance {dist / max(1, hops):5.2f} span {span / k:5.1f}")


def setups(roots):
    rows = collections.defaultdict(list)
    for root in roots:
        for d in sorted(os.listdir(root)):
            f = os.path.join(root, d, "ffs.txt")
            if not os.path.exists(f):
                continue
            for line in open(f):
                p = line.split()
                if len(p) != 9 or p[6] == "-":
                    continue
                pin = p[4].split("/")[-1]
                how = ("inside" if p[4] == "-" else "bypass" if pin.endswith("X")
                       else "way, upper half" if pin.endswith("6") else "way, lower half")
                rows[(p[2].split(".")[0], how)].append(float(p[6]))
    print("| slice | data | flip-flops | setup, mean (ns) |")
    print("|---|---|---|---|")
    for (kind, how), v in sorted(rows.items()):
        print(f"| {kind} | {how} | {len(v)} | {statistics.mean(v):.3f} |")


def fit(xs, ys):
    """Least squares y = c0 + c1 x1 + ...: the coefficients and the residual's rms."""
    n = len(xs[0]) + 1
    a = [[0.0] * n for _ in range(n)]
    b = [0.0] * n
    for x, y in zip(xs, ys):
        v = (1.0,) + tuple(x)
        for i in range(n):
            b[i] += v[i] * y
            for j in range(n):
                a[i][j] += v[i] * v[j]
    for i in range(n):
        for j in range(i + 1, n):
            r = a[j][i] / a[i][i]
            for k in range(i, n):
                a[j][k] -= r * a[i][k]
            b[j] -= r * b[i]
    c = [0.0] * n
    for i in reversed(range(n)):
        c[i] = (b[i] - sum(a[i][k] * c[k] for k in range(i + 1, n))) / a[i][i]
    res = [y - sum(ci * vi for ci, vi in zip(c, (1.0,) + tuple(x))) for x, y in zip(xs, ys)]
    return c, math.sqrt(statistics.mean(r * r for r in res))


def distance(args):
    grid, fanout, files = None, math.inf, []
    while args:
        if args[0] == "--tilegrid":
            grid, args = args[1], args[2:]
        elif args[0] == "--fanout":
            fanout, args = int(args[1]), args[2:]
        else:
            files.append(args.pop(0))
    tiles = json.load(open(grid))
    site, columns, rows_ = {}, set(), set()
    for t in tiles.values():
        if t["type"] in ("INT_L", "INT_R"):
            columns.add(t["grid_x"])
            rows_.add(t["grid_y"])
        for s in t.get("sites", {}):
            if s.startswith("SLICE_X"):
                # A CLB tile's interconnect is the column beside it, on its side.
                side = -1 if t["type"].endswith("_R") else 1
                site[s] = (t["grid_x"], t["grid_y"], t["grid_x"] + side)
    col = {x: i for i, x in enumerate(sorted(columns))}
    row = {y: i for i, y in enumerate(sorted(rows_))}
    data = []
    for f in files:
        for line in open(f):
            p = line.split()
            if p[0] != "c" or p[1] not in site or p[2] not in site or int(p[3]) > fanout:
                continue
            a, b = site[p[1]], site[p[2]]
            data.append((float(p[4]), abs(a[0] - b[0]), abs(a[1] - b[1]),
                         abs(col[a[2]] - col[b[2]]), abs(row[a[1]] - row[b[1]])))
    ys = [d[0] for d in data]
    print(f"{len(data)} connections")
    for name, xs in (("grid tiles", [(d[1] + d[2],) for d in data]),
                     ("grid columns, rows", [(d[1], d[2]) for d in data]),
                     ("interconnect tiles", [(d[3] + d[4],) for d in data]),
                     ("interconnect columns, rows", [(d[3], d[4]) for d in data])):
        c, rms = fit(xs, ys)
        print(f"{name:27s} {' + '.join(f'{v:.1f}' for v in c)} ps, residual {rms:.1f} ps")
    by = collections.defaultdict(list)
    for d in data:
        if d[2] == 0:
            by[d[3]].append(d[0])
    print("one row, by interconnect columns: " + " ".join(
        f"{k}:{statistics.median(v):.0f} ({len(v)})" for k, v in sorted(by.items()) if k < 10))


if __name__ == "__main__":
    what, rest = (sys.argv[1], sys.argv[2:]) if len(sys.argv) > 1 else (None, [])
    if what == "paths":
        paths(rest)
    elif what == "setups":
        setups(rest)
    elif what == "distance":
        distance(rest)
    else:
        sys.exit(__doc__)
