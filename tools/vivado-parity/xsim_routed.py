#!/usr/bin/env python3
"""A netlist Reticle routed, under Vivado's simulator and the vendor's own models.

For each design, simulates the source and `<dir>/<name>/<name>.routed.v` (what
`reticle fpga --routed-netlist` wrote) under `xsim` with the unisim library, from
the same testbench as tests/fpga_dsp_vivado.rs, and says whether `so` agrees on
every cycle after the first few hundred. Nothing is committed from this: it is the
vendor's model of each cell against the source, for the record in
docs/fpga-vivado-parity.md, and needs Vivado where the tests do not.

    tools/vivado-parity/xsim_routed.py ~/Xilinx/2026.1/Vivado <designs> <dir> <work> <name>...
"""
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TEST = open(os.path.join(ROOT, "tests/fpga_dsp_vivado.rs")).read()
TB = TEST.split('const SHIFT_TB: &str = r#"')[1].split('"#;')[0]


def run(vivado, work, label, top, files, pads=False):
    d = os.path.join(work, f"{top}_{label}")
    os.makedirs(d, exist_ok=True)
    with open(f"{d}/tb.v", "w") as f:
        bench = TB.replace("TOP", top)
        if pads:
            bench = bench.replace(".clk(clk), .si(si), .so(so)", ".pad_clk(clk), .pad_si(si), .pad_so(so)")
        f.write("`timescale 1ns / 1ps\n" + bench)
    glbl = f"{vivado}/data/verilog/src/glbl.v"
    sources = " ".join(files)
    script = (f". {vivado}/settings64.sh >/dev/null 2>&1; cd {d}; "
              f"nice -n 19 ionice -c3 xvlog -sv {sources} tb.v {glbl} >x.log 2>&1 && "
              f"nice -n 19 ionice -c3 xelab -L unisims_ver -debug off tb glbl -s s >>x.log 2>&1 && "
              f"nice -n 19 ionice -c3 xsim s -R >out.log 2>&1")
    subprocess.run(["sh", "-c", script], check=False)
    try:
        return [l for l in open(f"{d}/out.log").read().splitlines() if re.fullmatch(r"[01xz]", l)]
    except OSError:
        return []


def one(args):
    vivado, designs, routed, work, top = args
    want = run(vivado, work, "source", top, [f"{designs}/{top}.v"])
    # `xsim` leaves an explicit port, `.so(so$pad)`, undriven from outside: `z` on
    # every cycle where Reticle's own simulator sees the net. That is the header's
    # spelling and no part of the design, so it is written plainly, with the outside
    # names under `pad_` since a netlist has inside nets of those names already.
    text = open(f"{routed}/{top}/{top}.routed.v").read()
    head, rest = text.split(");", 1)
    pairs = re.findall(r"\.(\w+)\((\S+?)\)", head)
    plain = head.split("(", 1)[0] + "(" + ", ".join(f"pad_{p}" for p, _ in pairs) + ");\n"
    joins = ""
    for port, inner in pairs:
        direction = re.search(rf"(input|output)\s+{re.escape(inner)}\s*;", rest).group(1)
        plain += f"  {direction} pad_{port};\n"
        joins += f"  assign pad_{port} = {inner};\n" if direction == "output" else f"  assign {inner} = pad_{port};\n"
    rest = re.sub(r"  (input|output) \S+;\n", "", rest)
    rest = rest.replace("endmodule", joins + "endmodule")
    os.makedirs(f"{work}/{top}_routed", exist_ok=True)
    plainly = f"{work}/{top}_routed/routed.v"
    with open(plainly, "w") as f:
        f.write(plain + rest)
    got = run(vivado, work, "routed", top, [plainly], pads=True)
    if not want or not got:
        return f"{top}: the simulator would not run"
    moved = len(set(want)) > 1
    same = want == got
    return f"{top}: {len(want)} cycles, {'the same' if same else 'DIFFERENT'}{'' if moved else ' (never moved)'}"


def main():
    vivado, designs, routed, work = [os.path.abspath(p) for p in sys.argv[1:5]]
    jobs = [(vivado, designs, routed, work, top) for top in sys.argv[5:]]
    with ThreadPoolExecutor(int(os.environ.get("JOBS", "8"))) as pool:
        for line in pool.map(one, jobs):
            print(line, flush=True)


main()
