// PLLE2_ADV with its dynamic reconfiguration port in use.
module pll_drp (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [24:0] sr;
  always @(posedge clk) sr <= {sr[23:0], si};
  wire y;
  wire fb, fbi, locked;
  assign fbi = fb;
  wire c0, g0;
  BUFG u_g0 (.O(g0), .I(c0));
  PLLE2_ADV #(.CLKFBOUT_MULT(8), .CLKIN1_PERIOD(10.0), .CLKOUT0_DIVIDE(8)) dut (.CLKFBOUT(fb), .CLKFBIN(fbi), .CLKIN1(clk2), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0), .CLKIN2(1'b0), .CLKINSEL(1'b1), .CLKOUT0(c0), .DADDR(sr[6:0]), .DCLK(clk), .DEN(sr[7]), .DI(sr[23:8]), .DWE(sr[24]));
  reg t0;
  always @(posedge g0) t0 <= locked;
  assign y = t0;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
