`timescale 1ns / 1ps
// 7-series clock managers measured from outside, written by
// tools/vivado-parity/gen_sim_clock_managers.py: each probe's first three
// rising edges, the first three after a second reset (at 2600.3 ns, where
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

module tb;
  reg clk0 = 1'b0;
  always #5 clk0 = ~clk0;
  reg rst0 = 1'b1;
  initial #123.3 rst0 = 1'b0;
  initial begin #2600.3 rst0 = 1'b1; #50.4 rst0 = 1'b0; end
  wire [6:0] o0;
  wire locked0;
  PLLE2_ADV #(.CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE(4), .CLKOUT1_DIVIDE(10), .CLKOUT1_PHASE(90.0), .CLKOUT2_DIVIDE(8), .CLKOUT2_DUTY_CYCLE(0.25), .CLKOUT3_DIVIDE(5), .CLKOUT4_DIVIDE(20), .CLKOUT4_PHASE(180.0), .CLKOUT5_DIVIDE(3)) u0 (.CLKOUT0(o0[0]), .CLKOUT1(o0[1]), .CLKOUT2(o0[2]), .CLKOUT3(o0[3]), .CLKOUT4(o0[4]), .CLKOUT5(o0[5]), .CLKFBOUT(o0[6]), .LOCKED(locked0), .CLKIN1(clk0), .CLKFBIN(o0[6]), .PWRDWN(1'b0), .RST(rst0), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), .DI(16'h0), .DWE(1'b0));
  probe #(.NAME("u0.LOCKED"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.000)) p0 (.c(locked0), .ref_clk(clk0));
  probe #(.NAME("u0.CLKOUT0"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.001)) p1 (.c(o0[0]), .ref_clk(clk0));
  probe #(.NAME("u0.CLKOUT1"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.002)) p2 (.c(o0[1]), .ref_clk(clk0));
  probe #(.NAME("u0.CLKOUT2"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.003)) p3 (.c(o0[2]), .ref_clk(clk0));
  probe #(.NAME("u0.CLKOUT3"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.004)) p4 (.c(o0[3]), .ref_clk(clk0));
  probe #(.NAME("u0.CLKOUT4"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.005)) p5 (.c(o0[4]), .ref_clk(clk0));
  probe #(.NAME("u0.CLKOUT5"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.006)) p6 (.c(o0[5]), .ref_clk(clk0));
  probe #(.NAME("u0.CLKFBOUT"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.007)) p7 (.c(o0[6]), .ref_clk(clk0));
  reg clk1 = 1'b0;
  always #4 clk1 = ~clk1;
  reg rst1 = 1'b1;
  initial #141.7 rst1 = 1'b0;
  wire [6:0] o1;
  wire locked1;
  PLLE2_ADV #(.CLKIN1_PERIOD(8.0), .CLKFBOUT_MULT(8), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE(2), .CLKOUT1_DIVIDE(5), .CLKOUT1_PHASE(45.0), .CLKOUT2_DIVIDE(7), .CLKOUT3_DIVIDE(16), .CLKOUT3_PHASE(270.0), .CLKOUT4_DIVIDE(1), .CLKOUT5_DIVIDE(9)) u1 (.CLKOUT0(o1[0]), .CLKOUT1(o1[1]), .CLKOUT2(o1[2]), .CLKOUT3(o1[3]), .CLKOUT4(o1[4]), .CLKOUT5(o1[5]), .CLKFBOUT(o1[6]), .LOCKED(locked1), .CLKIN1(clk1), .CLKFBIN(o1[6]), .PWRDWN(1'b0), .RST(rst1), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), .DI(16'h0), .DWE(1'b0));
  probe #(.NAME("u1.LOCKED"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.008)) p8 (.c(locked1), .ref_clk(clk1));
  probe #(.NAME("u1.CLKOUT0"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.009)) p9 (.c(o1[0]), .ref_clk(clk1));
  probe #(.NAME("u1.CLKOUT1"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.010)) p10 (.c(o1[1]), .ref_clk(clk1));
  probe #(.NAME("u1.CLKOUT2"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.011)) p11 (.c(o1[2]), .ref_clk(clk1));
  probe #(.NAME("u1.CLKOUT3"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.012)) p12 (.c(o1[3]), .ref_clk(clk1));
  probe #(.NAME("u1.CLKOUT4"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.013)) p13 (.c(o1[4]), .ref_clk(clk1));
  probe #(.NAME("u1.CLKOUT5"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.014)) p14 (.c(o1[5]), .ref_clk(clk1));
  probe #(.NAME("u1.CLKFBOUT"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.015)) p15 (.c(o1[6]), .ref_clk(clk1));
  reg clk2 = 1'b0;
  always #10 clk2 = ~clk2;
  reg rst2 = 1'b1;
  initial #150.1 rst2 = 1'b0;
  wire [6:0] o2;
  wire locked2;
  PLLE2_ADV #(.CLKIN1_PERIOD(20.0), .CLKFBOUT_MULT(32), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE(16), .CLKOUT1_DIVIDE(32), .CLKFBOUT_PHASE(90.0), .CLKOUT2_DIVIDE(6)) u2 (.CLKOUT0(o2[0]), .CLKOUT1(o2[1]), .CLKOUT2(o2[2]), .CLKOUT3(o2[3]), .CLKOUT4(o2[4]), .CLKOUT5(o2[5]), .CLKFBOUT(o2[6]), .LOCKED(locked2), .CLKIN1(clk2), .CLKFBIN(o2[6]), .PWRDWN(1'b0), .RST(rst2), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), .DI(16'h0), .DWE(1'b0));
  probe #(.NAME("u2.LOCKED"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.016)) p16 (.c(locked2), .ref_clk(clk2));
  probe #(.NAME("u2.CLKOUT0"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.017)) p17 (.c(o2[0]), .ref_clk(clk2));
  probe #(.NAME("u2.CLKOUT1"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.018)) p18 (.c(o2[1]), .ref_clk(clk2));
  probe #(.NAME("u2.CLKOUT2"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.019)) p19 (.c(o2[2]), .ref_clk(clk2));
  probe #(.NAME("u2.CLKOUT3"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.020)) p20 (.c(o2[3]), .ref_clk(clk2));
  probe #(.NAME("u2.CLKOUT4"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.021)) p21 (.c(o2[4]), .ref_clk(clk2));
  probe #(.NAME("u2.CLKOUT5"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.022)) p22 (.c(o2[5]), .ref_clk(clk2));
  probe #(.NAME("u2.CLKFBOUT"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.023)) p23 (.c(o2[6]), .ref_clk(clk2));
  reg clk3 = 1'b0;
  always #5 clk3 = ~clk3;
  reg rst3 = 1'b1;
  initial #111.1 rst3 = 1'b0;
  wire [6:0] o3;
  wire locked3;
  PLLE2_ADV #(.CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(16), .DIVCLK_DIVIDE(2), .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(4), .CLKOUT1_PHASE(22.5), .CLKOUT2_DIVIDE(10), .CLKOUT2_DUTY_CYCLE(0.3)) u3 (.CLKOUT0(o3[0]), .CLKOUT1(o3[1]), .CLKOUT2(o3[2]), .CLKOUT3(o3[3]), .CLKOUT4(o3[4]), .CLKOUT5(o3[5]), .CLKFBOUT(o3[6]), .LOCKED(locked3), .CLKIN1(clk3), .CLKFBIN(o3[6]), .PWRDWN(1'b0), .RST(rst3), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), .DI(16'h0), .DWE(1'b0));
  probe #(.NAME("u3.LOCKED"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.024)) p24 (.c(locked3), .ref_clk(clk3));
  probe #(.NAME("u3.CLKOUT0"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.025)) p25 (.c(o3[0]), .ref_clk(clk3));
  probe #(.NAME("u3.CLKOUT1"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.026)) p26 (.c(o3[1]), .ref_clk(clk3));
  probe #(.NAME("u3.CLKOUT2"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.027)) p27 (.c(o3[2]), .ref_clk(clk3));
  probe #(.NAME("u3.CLKOUT3"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.028)) p28 (.c(o3[3]), .ref_clk(clk3));
  probe #(.NAME("u3.CLKOUT4"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.029)) p29 (.c(o3[4]), .ref_clk(clk3));
  probe #(.NAME("u3.CLKOUT5"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.030)) p30 (.c(o3[5]), .ref_clk(clk3));
  probe #(.NAME("u3.CLKFBOUT"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.031)) p31 (.c(o3[6]), .ref_clk(clk3));
  reg clk4 = 1'b0;
  always #5 clk4 = ~clk4;
  reg rst4 = 1'b1;
  initial #123.3 rst4 = 1'b0;
  initial begin #2600.3 rst4 = 1'b1; #50.4 rst4 = 1'b0; end
  wire [12:0] o4;
  wire locked4;
  MMCME2_ADV #(.CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT_F(8.0), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE_F(2.5), .CLKOUT1_DIVIDE(8), .CLKOUT1_PHASE(45.0), .CLKOUT2_DIVIDE(4), .CLKOUT2_DUTY_CYCLE(0.75), .CLKOUT3_DIVIDE(16), .CLKOUT5_DIVIDE(3), .CLKOUT6_DIVIDE(10), .CLKOUT6_PHASE(180.0)) u4 (.CLKOUT0(o4[0]), .CLKOUT0B(o4[1]), .CLKOUT1(o4[2]), .CLKOUT1B(o4[3]), .CLKOUT2(o4[4]), .CLKOUT2B(o4[5]), .CLKOUT3(o4[6]), .CLKOUT3B(o4[7]), .CLKOUT4(o4[8]), .CLKOUT5(o4[9]), .CLKOUT6(o4[10]), .CLKFBOUT(o4[11]), .CLKFBOUTB(o4[12]), .LOCKED(locked4), .CLKIN1(clk4), .CLKFBIN(o4[11]), .PWRDWN(1'b0), .RST(rst4), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), .DI(16'h0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0));
  probe #(.NAME("u4.LOCKED"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.032)) p32 (.c(locked4), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT0"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.033)) p33 (.c(o4[0]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT0B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.034)) p34 (.c(o4[1]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT1"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.035)) p35 (.c(o4[2]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT1B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.036)) p36 (.c(o4[3]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT2"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.037)) p37 (.c(o4[4]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT2B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.038)) p38 (.c(o4[5]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT3"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.039)) p39 (.c(o4[6]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT3B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.040)) p40 (.c(o4[7]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT4"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.041)) p41 (.c(o4[8]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT5"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.042)) p42 (.c(o4[9]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKOUT6"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.043)) p43 (.c(o4[10]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKFBOUT"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.044)) p44 (.c(o4[11]), .ref_clk(clk4));
  probe #(.NAME("u4.CLKFBOUTB"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.045)) p45 (.c(o4[12]), .ref_clk(clk4));
  reg clk5 = 1'b0;
  always #6 clk5 = ~clk5;
  reg rst5 = 1'b1;
  initial #157.3 rst5 = 1'b0;
  wire [12:0] o5;
  wire locked5;
  MMCME2_ADV #(.CLKIN1_PERIOD(12.0), .CLKFBOUT_MULT_F(20.0), .DIVCLK_DIVIDE(2), .CLKOUT0_DIVIDE_F(10.0), .CLKOUT1_DIVIDE(20), .CLKOUT1_DUTY_CYCLE(0.25), .CLKFBOUT_PHASE(45.0)) u5 (.CLKOUT0(o5[0]), .CLKOUT0B(o5[1]), .CLKOUT1(o5[2]), .CLKOUT1B(o5[3]), .CLKOUT2(o5[4]), .CLKOUT2B(o5[5]), .CLKOUT3(o5[6]), .CLKOUT3B(o5[7]), .CLKOUT4(o5[8]), .CLKOUT5(o5[9]), .CLKOUT6(o5[10]), .CLKFBOUT(o5[11]), .CLKFBOUTB(o5[12]), .LOCKED(locked5), .CLKIN1(clk5), .CLKFBIN(o5[11]), .PWRDWN(1'b0), .RST(rst5), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), .DI(16'h0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0));
  probe #(.NAME("u5.LOCKED"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.046)) p46 (.c(locked5), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT0"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.047)) p47 (.c(o5[0]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT0B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.048)) p48 (.c(o5[1]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT1"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.049)) p49 (.c(o5[2]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT1B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.050)) p50 (.c(o5[3]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT2"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.051)) p51 (.c(o5[4]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT2B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.052)) p52 (.c(o5[5]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT3"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.053)) p53 (.c(o5[6]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT3B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.054)) p54 (.c(o5[7]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT4"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.055)) p55 (.c(o5[8]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT5"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.056)) p56 (.c(o5[9]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKOUT6"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.057)) p57 (.c(o5[10]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKFBOUT"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.058)) p58 (.c(o5[11]), .ref_clk(clk5));
  probe #(.NAME("u5.CLKFBOUTB"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.059)) p59 (.c(o5[12]), .ref_clk(clk5));
  reg clk6 = 1'b0;
  always #5 clk6 = ~clk6;
  reg rst6 = 1'b1;
  initial #97.9 rst6 = 1'b0;
  wire [12:0] o6;
  wire locked6;
  MMCME2_ADV #(.CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT_F(10.0), .DIVCLK_DIVIDE(1), .CLKOUT0_DIVIDE_F(5.0), .CLKOUT1_DIVIDE(10), .CLKOUT1_PHASE(135.0), .CLKOUT2_DIVIDE(20), .CLKOUT2_PHASE(270.0), .CLKOUT3_DIVIDE(3), .CLKOUT4_DIVIDE(8), .CLKOUT4_DUTY_CYCLE(0.375)) u6 (.CLKOUT0(o6[0]), .CLKOUT0B(o6[1]), .CLKOUT1(o6[2]), .CLKOUT1B(o6[3]), .CLKOUT2(o6[4]), .CLKOUT2B(o6[5]), .CLKOUT3(o6[6]), .CLKOUT3B(o6[7]), .CLKOUT4(o6[8]), .CLKOUT5(o6[9]), .CLKOUT6(o6[10]), .CLKFBOUT(o6[11]), .CLKFBOUTB(o6[12]), .LOCKED(locked6), .CLKIN1(clk6), .CLKFBIN(o6[11]), .PWRDWN(1'b0), .RST(rst6), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'h0), .DCLK(1'b0), .DEN(1'b0), .DI(16'h0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0));
  probe #(.NAME("u6.LOCKED"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.060)) p60 (.c(locked6), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT0"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.061)) p61 (.c(o6[0]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT0B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.062)) p62 (.c(o6[1]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT1"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.063)) p63 (.c(o6[2]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT1B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.064)) p64 (.c(o6[3]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT2"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.065)) p65 (.c(o6[4]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT2B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.066)) p66 (.c(o6[5]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT3"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.067)) p67 (.c(o6[6]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT3B"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.068)) p68 (.c(o6[7]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT4"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.069)) p69 (.c(o6[8]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT5"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.070)) p70 (.c(o6[9]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKOUT6"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.071)) p71 (.c(o6[10]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKFBOUT"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.072)) p72 (.c(o6[11]), .ref_clk(clk6));
  probe #(.NAME("u6.CLKFBOUTB"), .SETTLE(2500.0), .SECOND(4500.0), .REPORT(4500.073)) p73 (.c(o6[12]), .ref_clk(clk6));
  reg clk7 = 1'b0;
  always #5 clk7 = ~clk7;
  reg rst7 = 1'b1;
  initial #23.3 rst7 = 1'b0;
  initial begin #2600.3 rst7 = 1'b1; #50.4 rst7 = 1'b0; end
  wire [6:0] o7;
  wire locked7;
  PLLE2_BASE #(.CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(10), .CLKOUT0_DIVIDE(5), .CLKOUT1_DIVIDE(10), .CLKOUT1_PHASE(90.0)) u7 (.CLKOUT0(o7[0]), .CLKOUT1(o7[1]), .CLKOUT2(o7[2]), .CLKOUT3(o7[3]), .CLKOUT4(o7[4]), .CLKOUT5(o7[5]), .CLKFBOUT(o7[6]), .LOCKED(locked7), .CLKIN1(clk7), .CLKFBIN(o7[6]), .PWRDWN(1'b0), .RST(rst7));
  probe #(.NAME("u7.LOCKED"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.074)) p74 (.c(locked7), .ref_clk(clk7));
  probe #(.NAME("u7.CLKOUT0"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.075)) p75 (.c(o7[0]), .ref_clk(clk7));
  probe #(.NAME("u7.CLKOUT1"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.076)) p76 (.c(o7[1]), .ref_clk(clk7));
  probe #(.NAME("u7.CLKOUT2"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.077)) p77 (.c(o7[2]), .ref_clk(clk7));
  probe #(.NAME("u7.CLKOUT3"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.078)) p78 (.c(o7[3]), .ref_clk(clk7));
  probe #(.NAME("u7.CLKOUT4"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.079)) p79 (.c(o7[4]), .ref_clk(clk7));
  probe #(.NAME("u7.CLKOUT5"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.080)) p80 (.c(o7[5]), .ref_clk(clk7));
  probe #(.NAME("u7.CLKFBOUT"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.081)) p81 (.c(o7[6]), .ref_clk(clk7));
  reg clk8 = 1'b0;
  always #5 clk8 = ~clk8;
  reg rst8 = 1'b1;
  initial #23.3 rst8 = 1'b0;
  initial begin #2600.3 rst8 = 1'b1; #50.4 rst8 = 1'b0; end
  wire [12:0] o8;
  wire locked8;
  MMCME2_BASE #(.CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT_F(10.0), .CLKOUT0_DIVIDE_F(5.0), .CLKOUT1_DIVIDE(10), .CLKOUT1_PHASE(90.0)) u8 (.CLKOUT0(o8[0]), .CLKOUT0B(o8[1]), .CLKOUT1(o8[2]), .CLKOUT1B(o8[3]), .CLKOUT2(o8[4]), .CLKOUT2B(o8[5]), .CLKOUT3(o8[6]), .CLKOUT3B(o8[7]), .CLKOUT4(o8[8]), .CLKOUT5(o8[9]), .CLKOUT6(o8[10]), .CLKFBOUT(o8[11]), .CLKFBOUTB(o8[12]), .LOCKED(locked8), .CLKIN1(clk8), .CLKFBIN(o8[11]), .PWRDWN(1'b0), .RST(rst8));
  probe #(.NAME("u8.LOCKED"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.082)) p82 (.c(locked8), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT0"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.083)) p83 (.c(o8[0]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT0B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.084)) p84 (.c(o8[1]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT1"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.085)) p85 (.c(o8[2]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT1B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.086)) p86 (.c(o8[3]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT2"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.087)) p87 (.c(o8[4]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT2B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.088)) p88 (.c(o8[5]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT3"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.089)) p89 (.c(o8[6]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT3B"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.090)) p90 (.c(o8[7]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT4"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.091)) p91 (.c(o8[8]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT5"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.092)) p92 (.c(o8[9]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKOUT6"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.093)) p93 (.c(o8[10]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKFBOUT"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.094)) p94 (.c(o8[11]), .ref_clk(clk8));
  probe #(.NAME("u8.CLKFBOUTB"), .SETTLE(2500.0), .SECOND(2600.3), .REPORT(4500.095)) p95 (.c(o8[12]), .ref_clk(clk8));
  initial #4501 $finish;
endmodule
