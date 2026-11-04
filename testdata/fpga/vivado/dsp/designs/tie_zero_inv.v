module tie_zero_inv (input wire clk, input wire si, output reg so);
  reg [146:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[145:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1), .IS_CLK_INVERTED(1'b0), .IS_CARRYIN_INVERTED(1'b1), .IS_ALUMODE_INVERTED(4'b1111), .IS_INMODE_INVERTED(5'b11111), .IS_OPMODE_INVERTED(7'b1111111)) dut (.CLK(clk), .P(p), .A(sr[29:0]), .B(sr[47:30]), .C(sr[95:48]), .D(sr[120:96]), .ALUMODE(4'h0), .CARRYINSEL(sr[123:121]), .INMODE(5'h0), .OPMODE(7'h0), .CARRYIN(1'h0), .CEA1(sr[124]), .CEA2(sr[125]), .CEAD(sr[126]), .CEALUMODE(sr[127]), .CEB1(sr[128]), .CEB2(sr[129]), .CEC(sr[130]), .CECARRYIN(sr[131]), .CECTRL(sr[132]), .CED(sr[133]), .CEINMODE(sr[134]), .CEM(sr[135]), .CEP(sr[136]), .RSTA(sr[137]), .RSTALLCARRYIN(sr[138]), .RSTALUMODE(sr[139]), .RSTB(sr[140]), .RSTC(sr[141]), .RSTCTRL(sr[142]), .RSTD(sr[143]), .RSTINMODE(sr[144]), .RSTM(sr[145]), .RSTP(sr[146]));
endmodule
