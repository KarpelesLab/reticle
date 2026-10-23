// The behavioural models of xilinx_primitives.v against what UG472, UG953
// and UG480 say they do: an MMCM's output frequencies and quarter-period
// phase from a 100 MHz input, its relock at new divides after a DRP write
// under reset, a PLL's output, BUFGMUX's select, ODDR's two edges, IDDR's
// two captures, and the XADC's temperature read over its DRP. Each check is
// the testbench's own.
//
// Would catch a divide or multiply read the wrong way round, a phase not
// in degrees of the output's own period, a DRP write that does not reach
// the output, an output that never starts, or a DDR edge swapped; would
// not catch the models being wrong about the part where the guides are,
// and is no substitute for Vivado's own models.
`timescale 1ps / 1ps
module xilinx_primitives_tb;
  reg clk = 0;
  always #5000 clk = ~clk;  // 100 MHz
  reg rst = 1;
  integer errors = 0;
  real a, b, c;

  // --- MMCME2_ADV: 100 MHz * 10 / 5 = 200 MHz; / 20 = 50 MHz at 90 degrees.
  wire o0, o1, fb, locked;
  reg dclk = 0, den = 0, dwe = 0;
  reg [6:0] daddr = 0;
  reg [15:0] di = 0;
  wire [15:0] dout;
  wire drdy;
  always #2000 dclk = ~dclk;
  MMCME2_ADV #(.CLKFBOUT_MULT_F(10.0), .CLKOUT0_DIVIDE_F(5.0), .CLKOUT1_DIVIDE(20),
               .CLKOUT1_PHASE(90.0), .CLKIN1_PERIOD(10.0)) mmcm (
    .CLKIN1(clk), .CLKIN2(1'b0), .CLKINSEL(1'b1), .CLKFBIN(fb), .CLKFBOUT(fb),
    .CLKOUT0(o0), .CLKOUT1(o1), .LOCKED(locked), .RST(rst), .PWRDWN(1'b0),
    .DCLK(dclk), .DEN(den), .DWE(dwe), .DADDR(daddr), .DI(di), .DO(dout), .DRDY(drdy),
    .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0));

  // --- PLLE2_ADV: 100 MHz * 8 / 4 = 200 MHz.
  wire p0, pfb, plocked;
  PLLE2_ADV #(.CLKFBOUT_MULT(8), .CLKOUT0_DIVIDE(4), .CLKIN1_PERIOD(10.0)) pll (
    .CLKIN1(clk), .CLKIN2(1'b0), .CLKINSEL(1'b1), .CLKFBIN(pfb), .CLKFBOUT(pfb),
    .CLKOUT0(p0), .LOCKED(plocked), .RST(rst), .PWRDWN(1'b0),
    .DCLK(1'b0), .DEN(1'b0), .DWE(1'b0), .DADDR(7'd0), .DI(16'd0));

  // --- BUFGMUX, ODDR, IDDR.
  reg sel = 0, d1 = 1, d2 = 0, din = 0;
  wire muxed, q, q1, q2;
  BUFGMUX bm (.O(muxed), .I0(o0), .I1(o1), .S(sel));
  ODDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE")) oddr (
    .Q(q), .C(clk), .CE(1'b1), .D1(d1), .D2(d2), .R(1'b0), .S(1'b0));
  IDDR #(.DDR_CLK_EDGE("OPPOSITE_EDGE")) iddr (
    .Q1(q1), .Q2(q2), .C(clk), .CE(1'b1), .D(din), .R(1'b0), .S(1'b0));

  // --- XADC: register 0 is the temperature.
  reg xden = 0;
  wire [15:0] xdo;
  wire xdrdy;
  XADC xadc (.DCLK(dclk), .DEN(xden), .DWE(1'b0), .DADDR(7'h00), .DI(16'd0), .DO(xdo),
             .DRDY(xdrdy), .RESET(1'b0), .CONVST(1'b0), .CONVSTCLK(1'b0),
             .VAUXP(16'd0), .VAUXN(16'd0), .VP(1'b0), .VN(1'b0));

  task check(input ok, input [8*24-1:0] what);
    if (!ok) begin
      errors = errors + 1;
      $display("FAIL: %0s", what);
    end
  endtask

  // A DRP write of one register, as XAPP888 sequences it.
  task drp_write(input [6:0] addr, input [15:0] value);
    begin
      @(posedge dclk);
      daddr = addr;
      di = value;
      den = 1;
      dwe = 1;
      @(posedge dclk);
      den = 0;
      dwe = 0;
      wait (drdy);
    end
  endtask

  initial begin
    #20000 rst = 0;
    wait (locked && plocked);
    @(posedge o0); a = $realtime; @(posedge o0); b = $realtime;
    check(b - a > 4999.0 && b - a < 5001.0, "mmcm 200 MHz");
    @(posedge o1); a = $realtime; @(posedge o1); b = $realtime;
    check(b - a > 19999.0 && b - a < 20001.0, "mmcm 50 MHz");
    // 90 degrees of 20 ns: the output's edge 5 ns after every other input
    // edge (and so 15 ns after the others), plus the picosecond every
    // output starts late by.
    @(posedge clk); c = $realtime; @(posedge o1); a = $realtime;
    check((a - c > 5000.5 && a - c < 5001.5) || (a - c > 15000.5 && a - c < 15001.5),
          "mmcm phase");
    @(posedge p0); a = $realtime; @(posedge p0); b = $realtime;
    check(b - a > 4999.0 && b - a < 5001.0, "pll 200 MHz");

    // CLKOUT0 to a divide of 10 (HIGH_TIME 5, LOW_TIME 5): 100 MHz.
    rst = 1;
    drp_write(7'h08, {4'b0000, 6'd5, 6'd5});
    drp_write(7'h09, 16'h0000);
    rst = 0;
    wait (locked);
    @(posedge o0); a = $realtime; @(posedge o0); b = $realtime;
    check(b - a > 9999.0 && b - a < 10001.0, "drp to 100 MHz");

    // BUFGMUX follows its select.
    sel = 1;
    #1;
    check(muxed === o1, "bufgmux select");

    // ODDR: D1 shown after the rising edge, D2 after the falling.
    @(posedge clk); #100;
    check(q === 1'b1, "oddr rising");
    @(negedge clk); #100;
    check(q === 1'b0, "oddr falling");

    // IDDR: 1 before the rising edge, 0 before the falling.
    @(negedge clk); din = 1;
    @(posedge clk); #100 din = 0;
    @(negedge clk); #100;
    check(q1 === 1'b1 && q2 === 1'b0, "iddr captures");

    // XADC: 25 C reads as about 0x976 left-justified.
    @(posedge dclk); xden = 1;
    @(posedge dclk); xden = 0;
    wait (xdrdy);
    check(xdo[15:4] > 12'h960 && xdo[15:4] < 12'h990, "xadc temperature");

    if (errors == 0) $display("PASS: the primitive models do what the guides say");
    else $display("FAIL: %0d check(s)", errors);
    $finish;
  end
  initial begin
    #5000000;
    $display("FAIL: timeout, mmcm locked %b pll locked %b", locked, plocked);
    $finish;
  end
endmodule
