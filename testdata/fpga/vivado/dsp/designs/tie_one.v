module tie_one (input wire clk, input wire si, output reg so);
  reg [1:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[0:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1)) dut (.CLK(clk), .P(p), .A(30'h3fffffff), .B(18'h3ffff), .C(48'hffffffffffff), .D(25'h1ffffff), .ALUMODE(4'hf), .CARRYINSEL(3'h7), .INMODE(5'h1f), .OPMODE(7'h7f), .CARRYIN(1'h1), .CEA1(1'h1), .CEA2(1'h1), .CEAD(1'h1), .CEALUMODE(1'h1), .CEB1(1'h1), .CEB2(1'h1), .CEC(1'h1), .CECARRYIN(1'h1), .CECTRL(1'h1), .CED(1'h1), .CEINMODE(1'h1), .CEM(1'h1), .CEP(1'h1), .RSTA(1'h1), .RSTALLCARRYIN(1'h1), .RSTALUMODE(1'h1), .RSTB(1'h1), .RSTC(1'h1), .RSTCTRL(1'h1), .RSTD(1'h1), .RSTINMODE(1'h1), .RSTM(1'h1), .RSTP(1'h1));
endmodule
