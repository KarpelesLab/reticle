module in_zero_odd (input wire clk, input wire si, output reg so);
  reg [158:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[157:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1)) dut (.CLK(clk), .P(p), .A(sr[29:0]), .B(sr[47:30]), .C(sr[95:48]), .D(sr[120:96]), .ALUMODE(sr[124:121]), .CARRYINSEL(sr[127:125]), .INMODE(5'h0), .OPMODE(sr[134:128]), .CARRYIN(sr[135]), .CEA1(sr[136]), .CEA2(sr[137]), .CEAD(sr[138]), .CEALUMODE(sr[139]), .CEB1(sr[140]), .CEB2(sr[141]), .CEC(sr[142]), .CECARRYIN(sr[143]), .CECTRL(sr[144]), .CED(sr[145]), .CEINMODE(sr[146]), .CEM(sr[147]), .CEP(sr[148]), .RSTA(sr[149]), .RSTALLCARRYIN(sr[150]), .RSTALUMODE(sr[151]), .RSTB(sr[152]), .RSTC(sr[153]), .RSTCTRL(sr[154]), .RSTD(sr[155]), .RSTINMODE(sr[156]), .RSTM(sr[157]), .RSTP(sr[158]));
endmodule
