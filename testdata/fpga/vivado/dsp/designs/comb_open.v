module comb_open (input wire clk, input wire si, output reg so);
  reg [47:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[46:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(0), .ADREG(1), .ALUMODEREG(0), .AREG(0), .BCASCREG(0), .BREG(0), .CARRYINREG(0), .CARRYINSELREG(0), .CREG(0), .DREG(1), .INMODEREG(0), .MREG(0), .OPMODEREG(0), .PREG(0)) dut (.P(p), .A(sr[29:0]), .B(sr[47:30]), .ALUMODE(4'h0), .INMODE(5'h0), .OPMODE(7'b0000101));
endmodule
