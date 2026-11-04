module in0_one (input wire clk, input wire si, output reg so);
  reg [162:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[161:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1)) dut (.CLK(clk), .P(p), .A(sr[29:0]), .B(sr[47:30]), .C(sr[95:48]), .D(sr[120:96]), .ALUMODE(sr[124:121]), .CARRYINSEL(sr[127:125]), .INMODE({sr[128], sr[129], sr[130], sr[131], 1'b1}), .OPMODE(sr[138:132]), .CARRYIN(sr[139]), .CEA1(sr[140]), .CEA2(sr[141]), .CEAD(sr[142]), .CEALUMODE(sr[143]), .CEB1(sr[144]), .CEB2(sr[145]), .CEC(sr[146]), .CECARRYIN(sr[147]), .CECTRL(sr[148]), .CED(sr[149]), .CEINMODE(sr[150]), .CEM(sr[151]), .CEP(sr[152]), .RSTA(sr[153]), .RSTALLCARRYIN(sr[154]), .RSTALUMODE(sr[155]), .RSTB(sr[156]), .RSTC(sr[157]), .RSTCTRL(sr[158]), .RSTD(sr[159]), .RSTINMODE(sr[160]), .RSTM(sr[161]), .RSTP(sr[162]));
endmodule
