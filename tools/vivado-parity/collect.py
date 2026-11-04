#!/usr/bin/env python3
"""Reduces a run_all.sh output directory to one committed file: each design's
cells, sites and timing as Vivado reported them. Seconds are kept as a record
of this machine and never compared by a test.

    tools/vivado-parity/collect.py <out> > <fixtures>/vivado.txt
"""
import glob, os, re, sys

print("# What Vivado 2026.1 made of each design here; tools/vivado-parity/collect.py.")
for path in sorted(glob.glob(f"{sys.argv[1]}/*/*.result")):
    text = open(path).read()
    keep = [line for line in text.splitlines() if re.match(r"(part|failed|message|cell|site|clock|wns|routed_pips|seconds) ", line)]
    print(f"design {os.path.basename(path)[:-7]}")
    print("\n".join(f"  {line}" for line in keep))
