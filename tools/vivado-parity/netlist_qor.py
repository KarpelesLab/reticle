#!/usr/bin/env python3
"""The table of netlist_qor.sh.

    netlist_qor.py table [--fixture] <label>=<dir>...
        Per design, each run's lookup tables, CARRY4s and flip-flops, and per
        clock the most levels of logic on its paths and the levels of its worst
        path as tables + CARRY4s, from <dir>/<design>/<design>.qor. With
        --fixture, the lines testdata/fpga/vivado/qor/netlist.txt holds:
          cells <label> <design> <LUT> <CARRY4> <FF> <MUXF7> <MUXF8>
          levels <label> <design> <clock> <max> <worst> <worst luts> <worst carries>
"""
import os, re, sys


def read(path):
    cells, levels = {}, {}
    if not os.path.exists(path):
        return None
    for line in open(path):
        w = line.split()
        if w[:1] == ["cells"]:
            cells[w[1]] = int(w[2])
        elif w[:1] == ["levels"]:
            levels[w[1]] = (int(w[3]), int(w[5]), int(w[7]), int(w[9]))
    if not cells:
        return None
    return cells, levels


def table(args):
    fixture = "--fixture" in args
    runs = [a.split("=", 1) for a in args if "=" in a]
    designs = sorted({d for _, root in runs for d in os.listdir(root)
                      if os.path.isdir(f"{root}/{d}")})
    got = {(label, d): read(f"{root}/{d}/{d}.qor") for label, root in runs for d in designs}
    if fixture:
        for d in designs:
            for label, _ in runs:
                r = got[(label, d)]
                if r is None:
                    continue
                c = r[0]
                print("cells", label, d, *(c.get(k, 0) for k in ("LUT", "CARRY4", "FF", "MUXF7", "MUXF8")))
                for clock, l in sorted(r[1].items()):
                    print("levels", label, d, clock, *l)
        return
    head = "| design | clock |" + "".join(f" {label} |" for label, _ in runs)
    print(head)
    print("|---|---|" + "---|" * len(runs))
    for d in designs:
        clocks = sorted({c for label, _ in runs if got[(label, d)] for c in got[(label, d)][1]})
        row = f"| {d} | cells |"
        for label, _ in runs:
            r = got[(label, d)]
            row += " - |" if r is None else " {} / {} / {} |".format(
                r[0].get("LUT", 0), r[0].get("CARRY4", 0), r[0].get("FF", 0))
        print(row)
        for clock in clocks:
            row = f"| | {clock} |"
            for label, _ in runs:
                r = got[(label, d)]
                l = r and r[1].get(clock)
                row += " - |" if not l else f" {l[0]} ({l[1]}: {l[2]}+{l[3]}) |"
            print(row)


if __name__ == "__main__":
    if sys.argv[1] == "table":
        table(sys.argv[2:])
    else:
        sys.exit(__doc__)
