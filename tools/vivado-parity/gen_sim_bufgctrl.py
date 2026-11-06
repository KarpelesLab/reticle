#!/usr/bin/env python3
"""Writes testdata/sim/xc7/bufgctrl.v: directed BUFGCTRL experiments.

Five variants (the default, INIT_OUT 1, each PRESELECT, and the inversions)
through seven scenarios (a select swap, an enable swap, a swap with either
IGNORE or both, an enable or a select dropped alone), each at four phases of
the two clocks: one control changes from a settled state and changes back,
half a nanosecond away from any edge, and is left to finish before the next.

Usage: gen_sim_bufgctrl.py > testdata/sim/xc7/bufgctrl.v
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
variants = [
    '',
    '#(.INIT_OUT(1))',
    '#(.PRESELECT_I0("TRUE"))',
    '#(.PRESELECT_I1("TRUE"), .INIT_OUT(1))',
    '#(.IS_CE0_INVERTED(1), .IS_S1_INVERTED(1), .IS_IGNORE1_INVERTED(1), .IS_I0_INVERTED(1))',
]
# scenario: initial (s0 s1 ce0 ce1 ig0 ig1), and the changed one
scenarios = [
    ((1, 0, 1, 1, 0, 0), (0, 1, 1, 1, 0, 0)),  # S swap
    ((1, 1, 1, 0, 0, 0), (1, 1, 0, 1, 0, 0)),  # CE swap
    ((1, 0, 1, 1, 1, 0), (0, 1, 1, 1, 1, 0)),  # S swap, ignore 0
    ((1, 0, 1, 1, 0, 1), (0, 1, 1, 1, 0, 1)),  # S swap, ignore 1
    ((1, 0, 1, 1, 1, 1), (0, 1, 1, 1, 1, 1)),  # S swap, both ignored
    ((1, 0, 1, 1, 0, 0), (1, 0, 0, 1, 0, 0)),  # CE0 off alone
    ((1, 0, 1, 0, 0, 0), (0, 0, 1, 0, 0, 0)),  # S0 off, nothing else
]
phases = [(0, 0), (0, 1), (1, 0), (1, 1)]
names = ['s0', 's1', 'ce0', 'ce1', 'ig0', 'ig1']
inst, init, ev = [], [], []
k = 0
t = 160.0
# One schedule per scenario and phase, which every variant shares.
slots = {}
for si in range(len(scenarios)):
    for pi, (p, q) in enumerate(phases):
        back = phases[(pi + si + 1) % 4]
        t_on = find(t, p, q)
        t_off = find(t_on + 45, *back)
        slots[(si, pi)] = (t_on, t_off)
        t = t_off + 35
for vi, v in enumerate(variants):
    for si, (a, b) in enumerate(scenarios):
        for pi, (p, q) in enumerate(phases):
            t_on, t_off = slots[(si, pi)]
            inv = v.startswith('#(.IS')
            # the inverted variant gets the complement of the inverted pins
            def pin(n, val):
                if inv and n in ('ce0', 's1', 'ig1'):
                    return 1 - val
                return val
            for j, n in enumerate(names):
                init.append(f"  initial {n}[{k}] = 1'b{pin(n, a[j])};")
                if a[j] != b[j]:
                    ev.append((t_on, f"{n}[{k}] = 1'b{pin(n, b[j])};"))
                    ev.append((t_off, f"{n}[{k}] = 1'b{pin(n, a[j])};"))
            inst.append(f"  BUFGCTRL {v} u{k} (.O(o[{k}]), .I0(i0), .I1(i1), .CE0(ce0[{k}]), "
                        f".CE1(ce1[{k}]), .S0(s0[{k}]), .S1(s1[{k}]), .IGNORE0(ig0[{k}]), "
                        f".IGNORE1(ig1[{k}]));")
            k += 1

n = k
ev.sort()
body, now = [], 0.0
for (tt, s) in ev:
    body.append(f"    #{tt - now:g} {s}")
    now = tt
print(f"""`timescale 1ns / 1ps
// Directed BUFGCTRL experiments, written by
// tools/vivado-parity/gen_sim_bufgctrl.py: each instance settles, then one
// control (a select, an enable, with or without IGNORE) changes and changes
// back, each time at a chosen phase of the two clocks and half a nanosecond
// away from any edge; every output change is printed with its quarter
// nanosecond.
module tb;
  reg i0 = 1'b0, i1 = 1'b0;
  always #5 i0 = ~i0;
  initial begin
    #1.5;
    forever #7 i1 = ~i1;
  end
  reg [{n-1}:0] s0, s1, ce0, ce1, ig0, ig1;
""" + '\n'.join(init) + f"""
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
    #{80:g} $finish;
  end
endmodule""")
