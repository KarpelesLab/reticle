// PLLE2_ADV with all its plain outputs in use, each a division of its own.
module pll_all_outs (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire fb, fbi, locked;
  assign fbi = fb;
  wire c0, g0;
  BUFG u_g0 (.O(g0), .I(c0));
  wire c1, g1;
  BUFG u_g1 (.O(g1), .I(c1));
  wire c2, g2;
  BUFG u_g2 (.O(g2), .I(c2));
  wire c3, g3;
  BUFG u_g3 (.O(g3), .I(c3));
  wire c4, g4;
  BUFG u_g4 (.O(g4), .I(c4));
  wire c5, g5;
  BUFG u_g5 (.O(g5), .I(c5));
  PLLE2_ADV #(.CLKFBOUT_MULT(8), .CLKIN1_PERIOD(10.0), .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(9), .CLKOUT2_DIVIDE(10), .CLKOUT3_DIVIDE(11), .CLKOUT4_DIVIDE(12), .CLKOUT5_DIVIDE(13)) dut (.CLKFBOUT(fb), .CLKFBIN(fbi), .CLKIN1(clk2), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0), .DADDR(7'b0), .DCLK(1'b0), .DEN(1'b0), .DI(16'b0), .DWE(1'b0), .CLKIN2(1'b0), .CLKINSEL(1'b1), .CLKOUT0(c0), .CLKOUT1(c1), .CLKOUT2(c2), .CLKOUT3(c3), .CLKOUT4(c4), .CLKOUT5(c5));
  reg t0;
  always @(posedge g0) t0 <= locked;
  reg t1;
  always @(posedge g1) t1 <= locked;
  reg t2;
  always @(posedge g2) t2 <= locked;
  reg t3;
  always @(posedge g3) t3 <= locked;
  reg t4;
  always @(posedge g4) t4 <= locked;
  reg t5;
  always @(posedge g5) t5 <= locked;
  assign y = t0 ^ t1 ^ t2 ^ t3 ^ t4 ^ t5;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
