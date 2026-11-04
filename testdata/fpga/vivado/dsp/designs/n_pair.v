// Two blocks by name, joined by every cascade there is.
module n_pair (input wire clk, input wire si, output reg so);
  reg [47:0] in;
  wire [29:0] ac;
  wire [17:0] bc;
  wire [47:0] pc;
  wire cc, ms;
  wire [47:0] p;
  DSP48E1 #(.AREG(1), .ACASCREG(1), .BREG(1), .BCASCREG(1), .CREG(0), .ADREG(0), .DREG(0), .MREG(1), .PREG(1),
            .INMODEREG(0), .OPMODEREG(0), .ALUMODEREG(0), .CARRYINREG(0), .CARRYINSELREG(0))
    low (.CLK(clk), .A({{12{in[17]}}, in[17:0]}), .B(in[35:18]), .C(48'd0), .D(25'd0), .CARRYIN(1'b0),
         .CARRYINSEL(3'd0), .INMODE(5'd0), .OPMODE(7'b0000101), .ALUMODE(4'd0),
         .CEA1(1'b0), .CEA2(1'b1), .CEB1(1'b0), .CEB2(1'b1), .CEM(1'b1), .CEP(1'b1), .CEC(1'b0), .CEAD(1'b0), .CED(1'b0),
         .CECTRL(1'b0), .CEALUMODE(1'b0), .CECARRYIN(1'b0), .CEINMODE(1'b0),
         .RSTA(1'b0), .RSTB(1'b0), .RSTC(1'b0), .RSTD(1'b0), .RSTM(1'b0), .RSTP(1'b0), .RSTCTRL(1'b0),
         .RSTALLCARRYIN(1'b0), .RSTALUMODE(1'b0), .RSTINMODE(1'b0),
         .ACOUT(ac), .BCOUT(bc), .PCOUT(pc), .CARRYCASCOUT(cc), .MULTSIGNOUT(ms));
  DSP48E1 #(.AREG(1), .ACASCREG(1), .BREG(1), .BCASCREG(1), .CREG(0), .ADREG(0), .DREG(0), .MREG(1), .PREG(1),
            .INMODEREG(0), .OPMODEREG(0), .ALUMODEREG(0), .CARRYINREG(0), .CARRYINSELREG(0),
            .A_INPUT("CASCADE"), .B_INPUT("CASCADE"))
    high (.CLK(clk), .A(30'd0), .B(18'd0), .ACIN(ac), .BCIN(bc), .PCIN(pc), .CARRYCASCIN(cc), .MULTSIGNIN(ms),
          .C(48'd0), .D(25'd0), .CARRYIN(1'b0), .CARRYINSEL(3'd0), .INMODE(5'd0), .OPMODE(7'b1010101), .ALUMODE(4'd0),
          .CEA1(1'b0), .CEA2(1'b1), .CEB1(1'b0), .CEB2(1'b1), .CEM(1'b1), .CEP(1'b1), .CEC(1'b0), .CEAD(1'b0), .CED(1'b0),
          .CECTRL(1'b0), .CEALUMODE(1'b0), .CECARRYIN(1'b0), .CEINMODE(1'b0),
          .RSTA(1'b0), .RSTB(1'b0), .RSTC(1'b0), .RSTD(1'b0), .RSTM(1'b0), .RSTP(1'b0), .RSTCTRL(1'b0),
          .RSTALLCARRYIN(1'b0), .RSTALUMODE(1'b0), .RSTINMODE(1'b0), .P(p));
  always @(posedge clk) begin in <= {in[46:0], si}; so <= ^p; end
endmodule
