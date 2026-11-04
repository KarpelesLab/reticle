module unconnected (input wire clk, input wire si, output reg so);
  reg [47:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[46:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1)) dut (.CLK(clk), .P(p), .A(sr[29:0]), .B(sr[47:30]));
endmodule
