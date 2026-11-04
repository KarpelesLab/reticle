#!/usr/bin/env python3
"""Asks Vivado's simulator what each design of a fixture directory does, as
Vivado built it: `gen_lutram_vectors.py <Vivado dir> <lutram.sh output> <fixture dir>`.

`lutram_probe.tcl` leaves each design's routed netlist on Vivado's simulation
models (`<top>.funcsim.v`). This drives each with the stimulus
`tests/fpga_xray_lutram_vivado.rs` uses, a sixteen-bit LFSR into `si` from
200 ns, once the models' global reset is over, and keeps `so` one nanosecond
after each rising edge, as one character per cycle, in `xsim.txt` there.
Nothing of the vendor's models or of its netlist is kept; only what came back.
"""
import os, re, subprocess, sys
from concurrent.futures import ThreadPoolExecutor

CYCLES = 2000

TB = """`timescale 1ns / 1ps
module tb;
  reg clk = 0;
  reg si = 0;
  reg [15:0] lfsr = 16'hACE1;
  wire so;
  {top} dut (.clk(clk), .si(si), .so(so));
  integer cycle;
  initial begin
    #200;
    $write("R ");
    for (cycle = 0; cycle < {cycles}; cycle = cycle + 1) begin
      #5 clk = 1;
      #1 $write("%b", so);
      #4 clk = 0;
      lfsr = {{lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]}};
      si = lfsr[0];
    end
    $display("");
    $finish;
  end
endmodule
"""


def run(job):
    vivado, d, top = job
    with open(os.path.join(d, "tb.v"), "w") as f:
        f.write(TB.format(top=top, cycles=CYCLES))
    glbl = f"{vivado}/data/verilog/src/glbl.v"
    script = (f". {vivado}/settings64.sh >/dev/null 2>&1; cd {d}; "
              f"nice -n 19 ionice -c3 xvlog {top}.funcsim.v tb.v {glbl} >x.log 2>&1 && "
              f"nice -n 19 ionice -c3 xelab -L unisims_ver -debug off tb glbl -s s >>x.log 2>&1 && "
              f"nice -n 19 ionice -c3 xsim s -R >out.log 2>&1")
    subprocess.run(["sh", "-c", script], check=False)
    try:
        text = open(os.path.join(d, "out.log")).read()
    except OSError:
        return top, None
    found = re.search(r"(?:^|\n)R ", text)
    if found is None:
        return top, None
    # A block RAM's model reports a collision of its two ports in the middle
    # of the line, a heading and a sentence; what it reads then is an `x`,
    # and stays in the answer.
    rest = re.sub(r"Memory Collision Error on [^\n]*\n(?:[A-Z][^\n]*\n)?", "", text[found.end():])
    bits = rest.split("\n", 1)[0].strip().lower()
    if len(bits) == CYCLES:
        return top, bits
    return top, None


def main():
    vivado, out, fixture = sys.argv[1:4]
    jobs = []
    for top in sorted(os.listdir(out)):
        d = os.path.join(out, top)
        if os.path.isfile(os.path.join(d, top + ".funcsim.v")):
            jobs.append((vivado, d, top))
    with ThreadPoolExecutor(int(os.environ.get("JOBS", "24"))) as pool:
        results = list(pool.map(run, jobs))
    missing = [t for t, bits in results if bits is None]
    with open(os.path.join(fixture, "xsim.txt"), "w") as f:
        f.write("# What Vivado 2026.1's xsim says each design here does, as Vivado built it:\n"
                "# `so` one nanosecond after each rising edge, from 200 ns, under the stimulus\n"
                "# tools/vivado-parity/gen_lutram_vectors.py writes. One line per design; an `x`\n"
                "# is a block RAM's model reading a collision of its two ports.\n")
        for top, bits in results:
            if bits is not None:
                f.write(f"{top} {bits}\n")
    print(len(results) - len(missing), "written;", len(missing), "would not run:", " ".join(missing))


main()
