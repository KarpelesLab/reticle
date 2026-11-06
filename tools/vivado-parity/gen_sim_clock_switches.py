#!/usr/bin/env python3
"""Writes testdata/sim/xc7/bufg_switches.v: directed clock multiplexer and
enable switches.

Each instance has its own select (or enable), raised once and lowered once,
each time at a chosen phase of the two clocks and half a nanosecond away from
any edge of either, so that no answer depends on the order of events within a
time step. Every cell type shares one schedule.

Usage: gen_sim_clock_switches.py > testdata/sim/xc7/bufg_switches.v
"""
def i0(t):
    return int(t // 5) % 2
def i1(t):
    return int((t - 1.5) // 7) % 2 if t >= 1.5 else 0
def find(after, a, b):
    t = after
    while True:
        t += 0.25
        if t % 0.5 == 0:
            continue
        if all(i0(t + d) == a and i1(t + d) == b for d in (-0.5, 0.5)):
            return t
cells = ['BUFGMUX', 'BUFGMUX_CTRL', 'BUFGMUX_1', 'BUFGCE', 'BUFGCE_1']
phases = ((0, 0), (0, 1), (1, 0), (1, 1))
slots = []
t = 150.0
for on in phases:
    for off in ((0, 0), (1, 1), (0, 1), (1, 0)):
        t_on = find(t, *on)
        t_off = find(t_on + 60, *off)
        slots.append((t_on, t_off))
        t = t_off + 40
inst, ev = [], []
k = 0
for cell in cells:
    for (t_on, t_off) in slots:
        if cell.startswith('BUFGCE'):
            inst.append(f"  {cell} u{k} (.O(o[{k}]), .CE(~sel[{k}]), .I(i0));")
        else:
            inst.append(f"  {cell} u{k} (.O(o[{k}]), .I0(i0), .I1(i1), .S(sel[{k}]));")
        ev.append((t_on, k, 1))
        ev.append((t_off, k, 0))
        k += 1
n = k
ev.sort()
body, now = [], 0.0
for (tt, j, v) in ev:
    body.append(f"    #{tt - now:g} sel[{j}] = 1'b{v};")
    now = tt
print(f"""`timescale 1ns / 1ps
// Directed switching of the clock multiplexers and enables, written by
// tools/vivado-parity/gen_sim_clock_switches.py: each instance's select is
// raised once and lowered once, each time at a chosen phase of the two
// clocks and half a nanosecond away from any edge. Every output change is
// printed with the quarter nanosecond it fell in.
module tb;
  reg i0 = 1'b0, i1 = 1'b0;
  always #5 i0 = ~i0;
  initial begin
    #1.5;
    forever #7 i1 = ~i1;
  end
  reg [{n-1}:0] sel = 0;
  wire [{n-1}:0] o;
""" + '\n'.join(inst) + f"""
  integer tick = 0;
  reg [{n-1}:0] last = {{{n}{{1'bx}}}};
  initial begin
    #0.125;
    forever begin
      if (o !== last) $display("%0d %b", tick, o);
      last = o;
      #0.25 tick = tick + 1;
    end
  end
  initial begin
""" + '\n'.join(body) + f"""
    #80 $finish;
  end
endmodule""")
