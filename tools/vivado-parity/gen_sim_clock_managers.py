#!/usr/bin/env python3
"""Writes testdata/sim/xc7/clock_managers.v: 7-series clock managers measured
from outside.

Each PLLE2_ADV, MMCME2_ADV, PLLE2_BASE or MMCME2_BASE gets its own input
clock and a reset released at an awkward moment, some a second reset later,
and a probe on LOCKED and on every output. A probe keeps the times of the
first three rising edges, and of the first three after the second reset, and
in the steady state between the two the output's period, high time and the
offset of its rising edge from the input's, all in picoseconds; each prints
at its own moment, so that the lines come in one order.

Every multiplier here is whole and every VCO period a whole number of
picoseconds: xsim takes a fractional multiplier a good deal longer to lock,
and rounds a fractional VCO period its own way, neither of which the model
has (src/fpga/devices/xc7_sim.v says so).

Usage: gen_sim_clock_managers.py > testdata/sim/xc7/clock_managers.v
"""

# (kind, input period ns, reset release ns, second reset, params)
CONFIGS = [
    ('PLLE2_ADV', 10.0, 123.3, True,
     'CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE(4), .CLKOUT1_DIVIDE(10), '
     '.CLKOUT1_PHASE(90.0), .CLKOUT2_DIVIDE(8), .CLKOUT2_DUTY_CYCLE(0.25), .CLKOUT3_DIVIDE(5), '
     '.CLKOUT4_DIVIDE(20), .CLKOUT4_PHASE(180.0), .CLKOUT5_DIVIDE(3)'),
    ('PLLE2_ADV', 8.0, 141.7, False,
     'CLKFBOUT_MULT(8), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE(2), .CLKOUT1_DIVIDE(5), '
     '.CLKOUT1_PHASE(45.0), .CLKOUT2_DIVIDE(7), .CLKOUT3_DIVIDE(16), .CLKOUT3_PHASE(270.0), '
     '.CLKOUT4_DIVIDE(1), .CLKOUT5_DIVIDE(9)'),
    ('PLLE2_ADV', 20.0, 150.1, False,
     'CLKFBOUT_MULT(32), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE(16), .CLKOUT1_DIVIDE(32), '
     '.CLKFBOUT_PHASE(90.0), .CLKOUT2_DIVIDE(6)'),
    ('PLLE2_ADV', 10.0, 111.1, False,
     'CLKFBOUT_MULT(16), .DIVCLK_DIVIDE(2), .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(4), '
     '.CLKOUT1_PHASE(22.5), .CLKOUT2_DIVIDE(10), .CLKOUT2_DUTY_CYCLE(0.3)'),
    ('MMCME2_ADV', 10.0, 123.3, True,
     'CLKFBOUT_MULT_F(8.0), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE_F(2.5), .CLKOUT1_DIVIDE(8), '
     '.CLKOUT1_PHASE(45.0), .CLKOUT2_DIVIDE(4), .CLKOUT2_DUTY_CYCLE(0.75), .CLKOUT3_DIVIDE(16), '
     '.CLKOUT5_DIVIDE(3), .CLKOUT6_DIVIDE(10), .CLKOUT6_PHASE(180.0)'),
    ('MMCME2_ADV', 12.0, 157.3, False,
     'CLKFBOUT_MULT_F(20.0), .DIVCLK_DIVIDE(2), .CLKOUT0_DIVIDE_F(10.0), .CLKOUT1_DIVIDE(20), '
     '.CLKOUT1_DUTY_CYCLE(0.25), .CLKFBOUT_PHASE(45.0)'),
    ('MMCME2_ADV', 10.0, 97.9, False,
     'CLKFBOUT_MULT_F(10.0), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE_F(5.0), .CLKOUT1_DIVIDE(10), '
     '.CLKOUT1_PHASE(135.0), .CLKOUT2_DIVIDE(20), .CLKOUT2_PHASE(270.0), .CLKOUT3_DIVIDE(3), '
     '.CLKOUT4_DIVIDE(8), .CLKOUT4_DUTY_CYCLE(0.375)'),
    ('PLLE2_BASE', 10.0, 23.3, True,
     'CLKFBOUT_MULT(10), .CLKOUT0_DIVIDE(5), .CLKOUT1_DIVIDE(10), .CLKOUT1_PHASE(90.0)'),
    ('MMCME2_BASE', 10.0, 23.3, True,
     'CLKFBOUT_MULT_F(10.0), .CLKOUT0_DIVIDE_F(5.0), .CLKOUT1_DIVIDE(10), .CLKOUT1_PHASE(90.0)'),
]
OUTS = {
    'PLLE2_ADV': ['CLKOUT0', 'CLKOUT1', 'CLKOUT2', 'CLKOUT3', 'CLKOUT4', 'CLKOUT5', 'CLKFBOUT'],
    'MMCME2_ADV': ['CLKOUT0', 'CLKOUT0B', 'CLKOUT1', 'CLKOUT1B', 'CLKOUT2', 'CLKOUT2B',
                   'CLKOUT3', 'CLKOUT3B', 'CLKOUT4', 'CLKOUT5', 'CLKOUT6', 'CLKFBOUT',
                   'CLKFBOUTB'],
}
OUTS['PLLE2_BASE'] = OUTS['PLLE2_ADV']
OUTS['MMCME2_BASE'] = OUTS['MMCME2_ADV']
SETTLE, SECOND, SECOND_END, REPORT = 2500.0, 2600.3, 2650.7, 4500.0

HEADER = f"""`timescale 1ns / 1ps
// 7-series clock managers measured from outside, written by
// tools/vivado-parity/gen_sim_clock_managers.py: each probe's first three
// rising edges, the first three after a second reset (at {SECOND} ns, where
// there is one), and its period, high time and phase in the steady state
// between, all in picoseconds.

module probe #(parameter NAME = "", parameter real SETTLE = 0.0, parameter real SECOND = 0.0,
    parameter real REPORT = 0.0)
  (input c, input ref_clk);
  real t_ref, t_rise, period, high, offset, r0, r1, r2, s0, s1, s2, f0;
  integer edges = 0, later = 0;
  initial begin
    t_ref = 0.0;
    t_rise = -1.0;
    period = 0.0;
    high = 0.0;
    offset = 0.0;
    r0 = -1.0;
    r1 = -1.0;
    r2 = -1.0;
    s0 = -1.0;
    s1 = -1.0;
    s2 = -1.0;
    f0 = -1.0;
  end
  always @(posedge ref_clk) t_ref = $realtime;
  always @(posedge c) begin
    if (edges == 0) r0 = $realtime;
    if (edges == 1) r1 = $realtime;
    if (edges == 2) r2 = $realtime;
    edges = edges + 1;
    if ($realtime > SECOND) begin
      if (later == 0) s0 = $realtime;
      if (later == 1) s1 = $realtime;
      if (later == 2) s2 = $realtime;
      later = later + 1;
    end
    if ($realtime > SETTLE && $realtime < SECOND && t_rise > 0.0) begin
      period = $realtime - t_rise;
      offset = $realtime - t_ref;
    end
    t_rise = $realtime;
  end
  always @(negedge c) begin
    if ($realtime > SETTLE && $realtime < SECOND && t_rise > 0.0) high = $realtime - t_rise;
    if ($realtime > SECOND && f0 < 0.0) f0 = $realtime;
  end
  function integer ps(input real t);
    ps = t < 0.0 ? -1 : $rtoi(t * 1000.0 + 0.5);
  endfunction
  initial begin
    #(REPORT);
    $display("%0s rises %0d %0d %0d period %0d high %0d offset %0d then %0d falls %0d rises %0d %0d %0d",
      NAME, ps(r0), ps(r1), ps(r2), ps(period), ps(high), ps(offset), ps(SECOND), ps(f0), ps(s0),
      ps(s1), ps(s2));
  end
endmodule

module tb;"""


def main():
    out = [HEADER]
    n_probe = 0
    for i, (kind, per, rel, again, params) in enumerate(CONFIGS):
        outs = OUTS[kind]
        out.append(f"  reg clk{i} = 1'b0;")
        out.append(f"  always #{per / 2:g} clk{i} = ~clk{i};")
        out.append(f"  reg rst{i} = 1'b1;")
        out.append(f"  initial #{rel:g} rst{i} = 1'b0;")
        if again:
            out.append(f"  initial begin #{SECOND:g} rst{i} = 1'b1; "
                       f"#{SECOND_END - SECOND:g} rst{i} = 1'b0; end")
        out.append(f"  wire [{len(outs) - 1}:0] o{i};")
        out.append(f"  wire locked{i};")
        conns = ', '.join(f".{n}(o{i}[{j}])" for j, n in enumerate(outs))
        fb = outs.index('CLKFBOUT')
        extra = ''
        if kind.endswith('_ADV'):
            extra = (", .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), "
                     ".DI(16'h0), .DWE(1'b0)")
        if kind == 'MMCME2_ADV':
            extra += ", .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0)"
        out.append(f"  {kind} #(.CLKIN1_PERIOD({per}), .{params}) u{i} ({conns}, "
                   f".LOCKED(locked{i}), .CLKIN1(clk{i}), .CLKFBIN(o{i}[{fb}]), .PWRDWN(1'b0), "
                   f".RST(rst{i}){extra});")
        second = SECOND if again else REPORT
        for j, n in [(-1, 'LOCKED')] + list(enumerate(outs)):
            sig = f"locked{i}" if j < 0 else f"o{i}[{j}]"
            out.append(f"  probe #(.NAME(\"u{i}.{n}\"), .SETTLE({SETTLE}), .SECOND({second}), "
                       f".REPORT({REPORT + n_probe * 0.001:.3f})) p{n_probe} (.c({sig}), "
                       f".ref_clk(clk{i}));")
            n_probe += 1
    out.append(f"  initial #{REPORT + 1:g} $finish;\nendmodule")
    print('\n'.join(out))


if __name__ == '__main__':
    main()
