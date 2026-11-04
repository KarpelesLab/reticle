// MMCME2_ADV, COMPENSATION ZHOLD, feedback pad.
module mmcm_comp_zhold_pad (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2,
    output wire fb_out,
    input wire fb_in
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire fb, fbi, locked;
  OBUF u_fbo (.O(fb_out), .I(fb));
  IBUF u_fbi (.O(fbi), .I(fb_in));
  wire c0, g0;
  BUFG u_g0 (.O(g0), .I(c0));
  MMCME2_ADV #(.CLKFBOUT_MULT_F(8.0), .CLKIN1_PERIOD(10.0), .CLKOUT0_DIVIDE_F(8.0), .COMPENSATION("ZHOLD")) dut (.CLKFBOUT(fb), .CLKFBIN(fbi), .CLKIN1(clk2), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0), .DADDR(7'b0), .DCLK(1'b0), .DEN(1'b0), .DI(16'b0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0), .CLKIN2(1'b0), .CLKINSEL(1'b1), .CLKOUT0(c0));
  reg t0;
  always @(posedge g0) t0 <= locked;
  assign y = t0;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
