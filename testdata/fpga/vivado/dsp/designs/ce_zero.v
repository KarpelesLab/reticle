module ce_zero (input wire clk, input wire si, output reg so);
  reg [161:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[160:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1)) dut (.CLK(clk), .P(p), .A(sr[29:0]), .B(sr[47:30]), .C(sr[95:48]), .D(sr[120:96]), .ALUMODE(sr[124:121]), .CARRYINSEL(sr[127:125]), .INMODE(sr[132:128]), .OPMODE(sr[139:133]), .CARRYIN(sr[140]), .CEA1(1'h0), .CEA2(sr[141]), .CEAD(sr[142]), .CEALUMODE(sr[143]), .CEB1(1'h0), .CEB2(sr[144]), .CEC(sr[145]), .CECARRYIN(sr[146]), .CECTRL(sr[147]), .CED(sr[148]), .CEINMODE(sr[149]), .CEM(sr[150]), .CEP(sr[151]), .RSTA(sr[152]), .RSTALLCARRYIN(sr[153]), .RSTALUMODE(sr[154]), .RSTB(sr[155]), .RSTC(sr[156]), .RSTCTRL(sr[157]), .RSTD(sr[158]), .RSTINMODE(sr[159]), .RSTM(sr[160]), .RSTP(sr[161]));
endmodule
