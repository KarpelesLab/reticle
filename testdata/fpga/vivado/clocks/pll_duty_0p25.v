// PLLE2_ADV, CLKOUT1_DUTY_CYCLE 0.25.
module pll_duty_0p25 (
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
  PLLE2_ADV #(.CLKFBOUT_MULT(8), .CLKIN1_PERIOD(10.0), .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(8), .CLKOUT1_DUTY_CYCLE(0.25)) dut (.CLKFBOUT(fb), .CLKFBIN(fbi), .CLKIN1(clk2), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0), .DADDR(7'b0), .DCLK(1'b0), .DEN(1'b0), .DI(16'b0), .DWE(1'b0), .CLKIN2(1'b0), .CLKINSEL(1'b1), .CLKOUT0(c0), .CLKOUT1(c1));
  reg t0;
  always @(posedge g0) t0 <= locked;
  reg t1;
  always @(posedge g1) t1 <= locked;
  assign y = t0 ^ t1;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
