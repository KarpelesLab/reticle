module bcasc (input wire clk, input wire si, output reg so);
  reg [163:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[162:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("CASCADE"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1)) dut (.CLK(clk), .P(p), .A(sr[29:0]), .B(sr[47:30]), .C(sr[95:48]), .D(sr[120:96]), .ALUMODE(sr[124:121]), .CARRYINSEL(sr[127:125]), .INMODE(sr[132:128]), .OPMODE(sr[139:133]), .CARRYIN(sr[140]), .CEA1(sr[141]), .CEA2(sr[142]), .CEAD(sr[143]), .CEALUMODE(sr[144]), .CEB1(sr[145]), .CEB2(sr[146]), .CEC(sr[147]), .CECARRYIN(sr[148]), .CECTRL(sr[149]), .CED(sr[150]), .CEINMODE(sr[151]), .CEM(sr[152]), .CEP(sr[153]), .RSTA(sr[154]), .RSTALLCARRYIN(sr[155]), .RSTALUMODE(sr[156]), .RSTB(sr[157]), .RSTC(sr[158]), .RSTCTRL(sr[159]), .RSTD(sr[160]), .RSTINMODE(sr[161]), .RSTM(sr[162]), .RSTP(sr[163]), .BCIN(18'd0));
endmodule
