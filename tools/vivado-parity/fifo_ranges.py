#!/usr/bin/env python3
"""Keeps what Vivado said of each design of `gen_fifo.py --ranges`, whose flag
offsets are past any depth, so that its refusal names the range it allows:

    fifo_ranges.py <vivado outputs> > testdata/fpga/vivado/fifo/ranges.txt

Each `<name>.log` beside a build is `lutram_probe.tcl`'s log. A line of the
answer is `<cell> <width> <mode> empty <least> <most> full <least> <most>`, the
mode `async` (`EN_SYN` false), `fwft` (and the first word falling through) or
`sync`; the rule names are `FIFO_AlmostRange_empty` and `FIFO_AlmostRange_full`.
"""
import os
import re
import sys


def main():
    builds = sys.argv[1]
    print("# The flag offsets Vivado 2026.1 accepts, by its refusal of offsets past any")
    print("# depth: tools/vivado-parity/fifo_ranges.py says how to read a line.")
    for log in sorted(os.listdir(builds)):
        m = re.match(r"fr_(18|36)_w(\d+)_(async|fwft|sync)\.log$", log)
        if not m:
            continue
        text = open(os.path.join(builds, log)).read()
        found = {}
        for which, rule in (("empty", "FIFO_AlmostRange_empty"), ("full", "FIFO_AlmostRange_full")):
            r = re.search(rule + r": dut: \w+ value of \d+ is out of range (\d+) to (\d+)", text)
            found[which] = f"{r.group(1)} {r.group(2)}" if r else "-"
        print(f"FIFO{m.group(1)}E1 {m.group(2)} {m.group(3)} empty {found['empty']} full {found['full']}")


main()
