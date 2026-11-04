module tie_zero (input wire clk, input wire si, output reg so);
  reg [1:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {sr[0:0], si}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #(.A_INPUT("DIRECT"), .B_INPUT("DIRECT"), .USE_DPORT("FALSE"), .USE_MULT("MULTIPLY"), .USE_SIMD("ONE48"), .AUTORESET_PATDET("NO_RESET"), .MASK(48'h3fffffffffff), .PATTERN(48'h000000000000), .SEL_MASK("MASK"), .SEL_PATTERN("PATTERN"), .USE_PATTERN_DETECT("NO_PATDET"), .ACASCREG(1), .ADREG(1), .ALUMODEREG(1), .AREG(1), .BCASCREG(1), .BREG(1), .CARRYINREG(1), .CARRYINSELREG(1), .CREG(1), .DREG(1), .INMODEREG(1), .MREG(1), .OPMODEREG(1), .PREG(1)) dut (.CLK(clk), .P(p), .A(30'h0), .B(18'h0), .C(48'h0), .D(25'h0), .ALUMODE(4'h0), .CARRYINSEL(3'h0), .INMODE(5'h0), .OPMODE(7'h0), .CARRYIN(1'h0), .CEA1(1'h0), .CEA2(1'h0), .CEAD(1'h0), .CEALUMODE(1'h0), .CEB1(1'h0), .CEB2(1'h0), .CEC(1'h0), .CECARRYIN(1'h0), .CECTRL(1'h0), .CED(1'h0), .CEINMODE(1'h0), .CEM(1'h0), .CEP(1'h0), .RSTA(1'h0), .RSTALLCARRYIN(1'h0), .RSTALUMODE(1'h0), .RSTB(1'h0), .RSTC(1'h0), .RSTCTRL(1'h0), .RSTD(1'h0), .RSTINMODE(1'h0), .RSTM(1'h0), .RSTP(1'h0));
endmodule
