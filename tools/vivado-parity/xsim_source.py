#!/usr/bin/env python3
"""What a design's own source does under Vivado's simulator, for a design
Vivado did not build: `xsim_source.py <Vivado dir> <fixture dir> <top>
<object>...`.

The stimulus is `gen_lutram_vectors.py`'s (a sixteen-bit LFSR into `si` from
200 ns, `so` one nanosecond after each rising edge), and each `<object>` the
testbench zeroes at time zero through a hierarchical name, `sr` for a
register and `mem:1024` for a memory of 1,024 words: a part's flip-flops and
block RAMs start at zero where the source's variables start at `x`, and a
netlist built from it is compared with the answer bit for bit. The line
`<top> <bits>` replaces the design's line in `xsim.txt` there, or is added.
Nothing of the vendor's is kept; only what came back.
"""
import os, re, subprocess, sys, tempfile

CYCLES = 2000


def main():
    vivado, fixture, top = sys.argv[1:4]
    zero = ""
    for item in sys.argv[4:]:
        if ":" in item:
            name, depth = item.split(":")
            zero += (f"  integer z_{name};\n  initial for (z_{name} = 0; z_{name} < {depth}; "
                     f"z_{name} = z_{name} + 1) dut.{name}[z_{name}] = 0;\n")
        else:
            zero += f"  initial dut.{item} = 0;\n"
    tb = ("`timescale 1ns / 1ps\nmodule tb;\n  reg clk = 0;\n  reg si = 0;\n"
          "  reg [15:0] lfsr = 16'hACE1;\n  wire so;\n"
          f"  {top} dut (.clk(clk), .si(si), .so(so));\n" + zero +
          "  integer cycle;\n  initial begin\n    #200;\n    $write(\"R \");\n"
          f"    for (cycle = 0; cycle < {CYCLES}; cycle = cycle + 1) begin\n"
          "      #5 clk = 1;\n      #1 $write(\"%b\", so);\n      #4 clk = 0;\n"
          "      lfsr = {lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]};\n"
          "      si = lfsr[0];\n    end\n    $display(\"\");\n    $finish;\n  end\nendmodule\n")
    source = os.path.abspath(os.path.join(fixture, top + ".v"))
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, "tb.v"), "w") as f:
            f.write(tb)
        script = (f". {vivado}/settings64.sh >/dev/null 2>&1; cd {d}; "
                  f"nice -n 19 xvlog {source} tb.v >x.log 2>&1 && "
                  f"nice -n 19 xelab -debug off tb -s s >>x.log 2>&1 && "
                  f"nice -n 19 xsim s -R >out.log 2>&1")
        subprocess.run(["sh", "-c", script], check=False)
        text = open(os.path.join(d, "out.log")).read()
    found = re.search(r"(?:^|\n)R ", text)
    bits = text[found.end():].split("\n", 1)[0].strip().lower() if found else ""
    if len(bits) != CYCLES:
        sys.exit(f"{top}: no answer")
    path = os.path.join(fixture, "xsim.txt")
    lines = [l for l in open(path).read().splitlines() if not l.startswith(top + " ")]
    lines.append(f"{top} {bits}")
    head = [l for l in lines if l.startswith("#")]
    body = sorted(l for l in lines if not l.startswith("#"))
    with open(path, "w") as f:
        f.write("\n".join(head + body) + "\n")
    print(top, "written")


main()
