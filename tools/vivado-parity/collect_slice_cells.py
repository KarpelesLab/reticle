#!/usr/bin/env python3
"""Reduces a run_all.sh output directory of testdata/fpga/vivado/slice_cells to
the one file the tests read: what each bitstream decodes to, in the slices that
hold a latch or a shift register and the interconnect beside them.

    tools/vivado-parity/collect_slice_cells.py <out> <reticle> > testdata/fpga/vivado/slice_cells/vivado.features

`<out>` is run_all.sh's directory and `<reticle>` the binary whose `fpga --decode`
names the bits. For each design it writes where each cell landed and which site pin
each of its pins is (Vivado's `get_bel_pins`), every feature of the tiles that
hold one, the unnamed bits there, and how many bits the whole image leaves
unnamed. A tile's truth tables are kept as one hexadecimal number each.
"""
import glob, os, re, subprocess, sys

out, reticle = sys.argv[1], sys.argv[2]
print("# What Vivado 2026.1 wrote for each design here, through Reticle's decode.")
print("# tools/vivado-parity/gen_slice_cells.py, run_all.sh, collect_slice_cells.py.")
for path in sorted(glob.glob(f"{out}/*/*.result")):
    name = os.path.basename(path)[:-7]
    result = open(path).read()
    print(f"design {name}")
    for line in result.splitlines():
        if line.startswith(("cell ", "placed ")) or (line.startswith("pin ") and "/" in line):
            print(f"  {line}")
    bit = path[:-7] + ".bit"
    if not os.path.exists(bit):
        print("  no-bitstream")
        continue
    run = subprocess.run([reticle, "fpga", "--device", "xc7a100t-ftg256", "--decode", bit],
                         capture_output=True, text=True, check=True)
    decoded = run.stdout.splitlines()
    unnamed = re.search(r"(\d+) bit\(s\) unexplained", run.stderr)
    print(f"  unexplained {unnamed[1]}")
    held = set()
    for line in decoded:
        words = line.split()
        if len(words) == 2 and re.match(r"SLICE[LM]_X[01]\.(LATCH$|.LUT\.SRL$)", words[1]):
            held.add(words[0])
    near = set()
    for tile in held:
        m = re.match(r"CLBL[LM]_([LR])_(X\d+Y\d+)", tile)
        near.add(f"INT_{m[1]}_{m[2]}")
    tables = {}
    for line in decoded:
        words = line.split()
        if words[0] == "?":
            if words[1] in held:
                print(f"  unnamed {words[1]} {words[2]}")
        elif words[0] in held:
            m = re.match(r"(SLICE[LM]_X[01]\..LUT)\.INIT\[(\d+)\]", words[1])
            if m:
                key = (words[0], m[1])
                tables[key] = tables.get(key, 0) | (1 << int(m[2]))
            else:
                print(f"  feature {words[0]} {words[1]}")
        elif words[0] in near:
            print(f"  pip {words[0]} {words[1]}")
    for (tile, lut), value in sorted(tables.items()):
        print(f"  table {tile} {lut} {value:016X}")
