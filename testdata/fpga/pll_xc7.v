module pllshift (.clk(clk$pad), .d(d$pad), .q(q$pad));
  input clk$pad;
  input d$pad;
  output q$pad;
  wire clk;
  (* clock_mhz = 50 *)
  wire sys;
  wire q;
  wire [7:0] sr;
  wire [7:0] next;
  wire clk$pad;
  wire clk$in0;
  wire d$pad;
  wire d$in0;
  wire q$pad;
  wire q$pin0;
  wire sys$fb;
  wire sys$gb;
  wire sr$q0;
  wire sr$q1;
  wire sr$q2;
  wire sr$q3;
  wire sr$q4;
  wire sr$q5;
  wire sr$q6;
  wire sr$q7;
  assign clk = clk$in0;
  assign q = sr[7];
  assign next = {sr[6], sr[5], sr[4], sr[3], sr[2], sr[1], sr[0], d$in0};
  assign q$pad = q$pin0;
  assign sr = {sr$q7, sr$q6, sr$q5, sr$q4, sr$q3, sr$q2, sr$q1, sr$q0};
  (* port = "clk", pin = "W5", io_standard = "LVCMOS33" *)
  IBUF clk$io0 (
    .I(clk$pad),
    .O(clk$in0)
  );
  (* port = "d", pin = "V17", io_standard = "LVCMOS33" *)
  IBUF d$io0 (
    .I(d$pad),
    .O(d$in0)
  );
  (* port = "q", pin = "U16", io_standard = "LVCMOS33" *)
  OBUF q$io0 (
    .I(q),
    .O(q$pin0)
  );
  (* frequency_mhz = "50.000000" *)
  PLLE2_BASE #(.STARTUP_WAIT("FALSE"), .BANDWIDTH("OPTIMIZED"), .DIVCLK_DIVIDE(1), .CLKFBOUT_MULT(8), .CLKOUT0_DIVIDE(16)) sys$pll (
    .CLKIN1(clk),
    .CLKFBIN(sys$fb),
    .RST(1'b0),
    .PWRDWN(1'b0),
    .CLKOUT0(sys),
    .CLKFBOUT(sys$fb)
  );
  BUFG sys$gbuf (
    .I(sys),
    .O(sys$gb)
  );
  FDRE #(.INIT(1'b0)) sr$ff0 (
    .C(sys$gb),
    .D(next[0]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q0)
  );
  FDRE #(.INIT(1'b0)) sr$ff1 (
    .C(sys$gb),
    .D(next[1]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q1)
  );
  FDRE #(.INIT(1'b0)) sr$ff2 (
    .C(sys$gb),
    .D(next[2]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q2)
  );
  FDRE #(.INIT(1'b0)) sr$ff3 (
    .C(sys$gb),
    .D(next[3]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q3)
  );
  FDRE #(.INIT(1'b0)) sr$ff4 (
    .C(sys$gb),
    .D(next[4]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q4)
  );
  FDRE #(.INIT(1'b0)) sr$ff5 (
    .C(sys$gb),
    .D(next[5]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q5)
  );
  FDRE #(.INIT(1'b0)) sr$ff6 (
    .C(sys$gb),
    .D(next[6]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q6)
  );
  FDRE #(.INIT(1'b0)) sr$ff7 (
    .C(sys$gb),
    .D(next[7]),
    .CE(1'b1),
    .R(1'b0),
    .Q(sr$q7)
  );
endmodule
