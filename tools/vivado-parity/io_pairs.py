#!/usr/bin/env python3
"""io_pairs.tcl's lines as one line per first item of each pair: `.` allowed,
`C` BIVC-1, `R` BIVR-1, `B` both."""
import sys

rows = {}
order = []
for path in sys.argv[1:]:
    for line in open(path):
        w = line.split()
        if len(w) < 5:
            continue
        a, b = (w[0], w[1]), (w[2], w[3])
        rows[(a, b)] = {"ok": ".", "BIVC-1": "C", "BIVR-1": "R", "BIVC-1,BIVR-1": "B"}[w[4]]
        for item in (a, b):
            if item not in order:
                order.append(item)
order.sort()
print("# What Vivado 2026.1's design rule check says of two pads in one bank of an")
print("# XC7A100T: the items, then a line per first item of a pair, by io_pairs.py.")
for item in order:
    print(*item)
for i, a in enumerate(order):
    print(f"{i}: " + "".join(rows[(a, b)] for b in order))
