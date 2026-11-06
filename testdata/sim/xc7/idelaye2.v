`timescale 1ns / 1ps
// IDELAYE2: a pulse train through FIXED delays of several values at two
// reference frequencies, from DATAIN and inverted, and through a VARIABLE
// and a VAR_LOAD delay stepped by INC and CE and loaded by LD, the steps
// falling while pulses are in the line. Each output's first 64 changes after
// 100 ns are kept with CNTVALUEOUT at each, in picoseconds, and printed at
// the end a probe at a time.
module edges #(parameter NAME = "", parameter real REPORT = 0.0) (input s, input [4:0] cnt);
  reg [63:0] t [0:63];
  reg [4:0] c [0:63];
  integer n = 0, k;
  always @(s) if (n < 64 && $realtime > 100.0) begin
    t[n] = $realtime * 1000.0;
    c[n] = cnt;
    n = n + 1;
  end
  initial begin
    #(REPORT);
    for (k = 0; k < n; k = k + 1)
      $display("%0s %0d %0d %b %0d", NAME, k, t[k], c[k], cnt);
  end
endmodule

module tb;
  reg clk = 1'b0;
  always #5 clk = ~clk;
  reg din = 1'b0;
  // A pulse train from 150 ns: high 3 ns, low 7.
  initial begin
    #150.3;
    forever begin
      din = 1'b1;
      #3 din = 1'b0;
      #7;
    end
  end
  reg ce = 1'b0, inc = 1'b0, ld = 1'b0, rst = 1'b0;
  reg [4:0] cin = 5'd0;
  initial begin
    #201;
    // Four increments, two decrements, a load of 20, then three increments.
    repeat (4) begin
      @(negedge clk) ce = 1'b1; inc = 1'b1;
      @(negedge clk) ce = 1'b0; inc = 1'b0;
    end
    repeat (2) begin
      @(negedge clk) ce = 1'b1; inc = 1'b0;
      @(negedge clk) ce = 1'b0;
    end
    @(negedge clk) cin = 5'd20; ld = 1'b1;
    @(negedge clk) ld = 1'b0;
    repeat (3) begin
      @(negedge clk) ce = 1'b1; inc = 1'b1;
      @(negedge clk) ce = 1'b0; inc = 1'b0;
    end
    @(negedge clk) cin = 5'd31; ld = 1'b1;
    @(negedge clk) ld = 1'b0;
    @(negedge clk) ce = 1'b1; inc = 1'b1;
    @(negedge clk) ce = 1'b0; inc = 1'b0;
  end
  wire [7:0] o;
  wire [4:0] c0, c1, c2, c3, c4, c5, c6, c7;
  IDELAYE2 #(.IDELAY_TYPE("FIXED"), .IDELAY_VALUE(0)) u0 (.DATAOUT(o[0]), .CNTVALUEOUT(c0),
    .C(clk), .CE(1'b0), .CINVCTRL(1'b0), .CNTVALUEIN(5'd0), .DATAIN(1'b0), .IDATAIN(din),
    .INC(1'b0), .LD(1'b0), .LDPIPEEN(1'b0), .REGRST(1'b0));
  IDELAYE2 #(.IDELAY_TYPE("FIXED"), .IDELAY_VALUE(1)) u1 (.DATAOUT(o[1]), .CNTVALUEOUT(c1),
    .C(clk), .CE(1'b0), .CINVCTRL(1'b0), .CNTVALUEIN(5'd0), .DATAIN(1'b0), .IDATAIN(din),
    .INC(1'b0), .LD(1'b0), .LDPIPEEN(1'b0), .REGRST(1'b0));
  IDELAYE2 #(.IDELAY_TYPE("FIXED"), .IDELAY_VALUE(13)) u2 (.DATAOUT(o[2]), .CNTVALUEOUT(c2),
    .C(clk), .CE(1'b0), .CINVCTRL(1'b0), .CNTVALUEIN(5'd0), .DATAIN(1'b0), .IDATAIN(din),
    .INC(1'b0), .LD(1'b0), .LDPIPEEN(1'b0), .REGRST(1'b0));
  IDELAYE2 #(.IDELAY_TYPE("FIXED"), .IDELAY_VALUE(31), .REFCLK_FREQUENCY(300.0)) u3
    (.DATAOUT(o[3]), .CNTVALUEOUT(c3), .C(clk), .CE(1'b0), .CINVCTRL(1'b0),
    .CNTVALUEIN(5'd0), .DATAIN(1'b0), .IDATAIN(din), .INC(1'b0), .LD(1'b0),
    .LDPIPEEN(1'b0), .REGRST(1'b0));
  IDELAYE2 #(.IDELAY_TYPE("VARIABLE"), .IDELAY_VALUE(3)) u4 (.DATAOUT(o[4]),
    .CNTVALUEOUT(c4), .C(clk), .CE(ce), .CINVCTRL(1'b0), .CNTVALUEIN(cin), .DATAIN(1'b0),
    .IDATAIN(din), .INC(inc), .LD(ld), .LDPIPEEN(1'b0), .REGRST(1'b0));
  IDELAYE2 #(.IDELAY_TYPE("VAR_LOAD"), .IDELAY_VALUE(7)) u5 (.DATAOUT(o[5]),
    .CNTVALUEOUT(c5), .C(clk), .CE(ce), .CINVCTRL(1'b0), .CNTVALUEIN(cin), .DATAIN(1'b0),
    .IDATAIN(din), .INC(inc), .LD(ld), .LDPIPEEN(1'b0), .REGRST(1'b0));
  IDELAYE2 #(.IDELAY_TYPE("FIXED"), .IDELAY_VALUE(5), .IS_IDATAIN_INVERTED(1'b1)) u6
    (.DATAOUT(o[6]), .CNTVALUEOUT(c6), .C(clk), .CE(1'b0), .CINVCTRL(1'b0),
    .CNTVALUEIN(5'd0), .DATAIN(1'b0), .IDATAIN(din), .INC(1'b0), .LD(1'b0),
    .LDPIPEEN(1'b0), .REGRST(1'b0));
  IDELAYE2 #(.IDELAY_TYPE("FIXED"), .IDELAY_VALUE(9), .DELAY_SRC("DATAIN")) u7
    (.DATAOUT(o[7]), .CNTVALUEOUT(c7), .C(clk), .CE(1'b0), .CINVCTRL(1'b0),
    .CNTVALUEIN(5'd0), .DATAIN(din), .IDATAIN(1'b0), .INC(1'b0), .LD(1'b0),
    .LDPIPEEN(1'b0), .REGRST(1'b0));
  edges #(.NAME("in"), .REPORT(1000.0)) e_in (.s(din), .cnt(5'd0));
  edges #(.NAME("fixed0"), .REPORT(1000.001)) e0 (.s(o[0]), .cnt(c0));
  edges #(.NAME("fixed1"), .REPORT(1000.002)) e1 (.s(o[1]), .cnt(c1));
  edges #(.NAME("fixed13"), .REPORT(1000.003)) e2 (.s(o[2]), .cnt(c2));
  edges #(.NAME("fixed31at300"), .REPORT(1000.004)) e3 (.s(o[3]), .cnt(c3));
  edges #(.NAME("inverted5"), .REPORT(1000.006)) e6 (.s(o[6]), .cnt(c6));
  edges #(.NAME("datain9"), .REPORT(1000.007)) e7 (.s(o[7]), .cnt(c7));
  // The variable ones: an edge of each pulse from when the steps begin.
  reg go = 1'b0;
  initial #195 go = 1'b1;
  edges #(.NAME("variable"), .REPORT(1000.008)) e4 (.s(o[4] & go), .cnt(c4));
  edges #(.NAME("var_load"), .REPORT(1000.009)) e5 (.s(o[5] & go), .cnt(c5));
  initial #1001 $finish;
endmodule
