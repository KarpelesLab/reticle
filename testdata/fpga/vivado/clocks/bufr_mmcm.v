// BUFR on an MMCM's output.
module bufr_mmcm (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire fb, c0, o;
  MMCME2_ADV #(.CLKFBOUT_MULT_F(8.0), .CLKIN1_PERIOD(10.0), .CLKOUT0_DIVIDE_F(8.0), .COMPENSATION("INTERNAL")) m (.CLKFBOUT(fb), .CLKOUT0(c0), .CLKFBIN(fb), .CLKIN1(clk2), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'b0), .DCLK(1'b0), .DEN(1'b0), .DI(16'b0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0), .PWRDWN(1'b0), .RST(1'b0));
  BUFR dut (.O(o), .CE(1'b1), .CLR(1'b0), .I(c0));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
