#!/usr/bin/env python3
"""Gathers what decode_placed.sh left, for committing.

    tools/vivado-parity/bits_table.py <out> bits  > placed/bits.txt
    tools/vivado-parity/bits_table.py <out> cells > placed/cells.txt
"""
import os, sys

out, which = sys.argv[1:3]
print(f"# Reticle's bitstream against Vivado's; tools/vivado-parity/compare_bits.py, {which}.")
for design in sorted(os.listdir(out)):
    path = f"{out}/{design}/{which}.txt"
    if os.path.exists(path):
        print(f"design {design}")
        print(open(path).read(), end="")
