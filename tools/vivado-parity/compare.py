#!/usr/bin/env python3
"""A table of two output directories: run_all.sh's and run_reticle.sh's.

    tools/vivado-parity/compare.py <vivado out> <reticle out>
"""
import glob, os, re, sys

head = "design", "tool", "LUT", "FF", "CARRY", "SRL", "MUXF", "LUTRAM", "BRAM18", "DSP", "ns", "s"
print("| " + " | ".join(head) + " |")
print("|" + "---|" * len(head))
for path in sorted(glob.glob(f"{sys.argv[1]}/*/*.result")):
    name = os.path.basename(path)[:-7]
    text = open(path).read()
    cells = {m[1]: int(m[2]) for m in re.finditer(r"^cell (\S+) (\d+)", text, re.M)}
    count = lambda pat: sum(v for k, v in cells.items() if re.match(pat, k))
    clock = re.search(r"^clock clk period \S+ slack \S+ delay (\S+)", text, re.M)
    seconds = sum(float(x) for x in re.findall(r"^seconds \w+ (\S+)", text, re.M))
    print(f"| {name} | Vivado | {count('LUT')} | {count('FD')} | {count('CARRY')} | {count('SRL')} | {count('MUXF')} | "
          f"{count('RAM[0-9DS]')} | {2 * count('RAMB36') + count('RAMB18')} | {count('DSP')} | "
          f"{clock[1] if clock else '-'} | {seconds:.0f} |")
    log = f"{sys.argv[2]}/{name}.log"
    if not os.path.exists(log):
        continue
    text = open(log, errors="replace").read()
    cells = {}
    for m in re.finditer(r"^  (\d+) x (\S+)$", text, re.M):
        cells[m[2]] = cells.get(m[2], 0) + int(m[1])
    count = lambda pat: sum(v for k, v in cells.items() if re.match(pat, k))
    late = re.findall(r"routed timing estimate:? .*?clk ([\d.]+) ns", text) or re.findall(r"round \d+: clk ([\d.]+) ns", text)
    total = re.findall(r"\(total\s+([\d.]+) s\)", text)
    lut = re.search(r"(\d+) of \d+ lut sites", text)
    print(f"| | Reticle | {lut[1] if lut else count('LUT')} | {count('FD')} | {count('CARRY')} | {count('SRL')} | "
          f"{count('MUXF')} | {count('RAM[0-9DS]')} | {count('RAMB18')} | {count('DSP')} | "
          f"{late[-1] if late else '-'} | {float(total[-1]) if total else 0:.0f} |")
