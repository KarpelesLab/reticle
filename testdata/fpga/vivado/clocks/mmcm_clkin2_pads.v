// MMCME2_ADV choosing between two clock pads with a signal, with a reset.
module mmcm_clkin2_pads (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2,
    input wire srcc
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire fb, fbi, locked;
  assign fbi = fb;
  wire c0, g0;
  BUFG u_g0 (.O(g0), .I(c0));
  MMCME2_ADV #(.CLKFBOUT_MULT_F(8.0), .CLKIN1_PERIOD(10.0), .CLKOUT0_DIVIDE_F(8.0), .CLKIN2_PERIOD(10.0)) dut (.CLKFBOUT(fb), .CLKFBIN(fbi), .CLKIN1(clk2), .LOCKED(locked), .PWRDWN(1'b0), .DADDR(7'b0), .DCLK(1'b0), .DEN(1'b0), .DI(16'b0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0), .CLKOUT0(c0), .CLKIN2(srcc), .CLKINSEL(sr[0]), .RST(sr[1]));
  reg t0;
  always @(posedge g0) t0 <= locked;
  assign y = t0;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
