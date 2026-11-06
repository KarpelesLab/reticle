`timescale 1ns / 1ps
// DSP48E1 through the global reset: every register on, a clock from time
// zero, the operands moving each cycle, and P printed after each edge.
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg [29:0] a = 0;
  reg [17:0] b = 0;
  reg [47:0] c = 0;
  wire [47:0] p, p2;
  DSP48E1 #(.AREG(1), .BREG(1), .CREG(1), .MREG(1), .PREG(1), .OPMODEREG(1), .ALUMODEREG(1))
    u0 (.CLK(clk), .A(a), .B(b), .C(c), .D(25'h0), .PCIN(48'h0), .ACIN(30'h0), .BCIN(18'h0),
    .CARRYIN(1'b0), .CARRYCASCIN(1'b0), .MULTSIGNIN(1'b0), .CEA1(1'b1), .CEA2(1'b1),
    .CEAD(1'b1), .CEALUMODE(1'b1), .CEB1(1'b1), .CEB2(1'b1), .CEC(1'b1), .CECARRYIN(1'b1),
    .CECTRL(1'b1), .CED(1'b1), .CEINMODE(1'b1), .CEM(1'b1), .CEP(1'b1), .RSTA(1'b0),
    .RSTALLCARRYIN(1'b0), .RSTALUMODE(1'b0), .RSTB(1'b0), .RSTC(1'b0), .RSTCTRL(1'b0),
    .RSTD(1'b0), .RSTINMODE(1'b0), .RSTM(1'b0), .RSTP(1'b0), .P(p), .OPMODE(7'b0110101),
    .ALUMODE(4'b0000), .CARRYINSEL(3'b000), .INMODE(5'b00000));
  DSP48E1 #(.AREG(0), .BREG(0), .CREG(0), .MREG(0), .PREG(1), .OPMODEREG(0), .ALUMODEREG(0),
    .INMODEREG(0), .CARRYINREG(0), .CARRYINSELREG(0), .ACASCREG(0), .BCASCREG(0))
    u1 (.CLK(clk), .A(a), .B(b), .C(c), .D(25'h0), .PCIN(48'h0), .ACIN(30'h0), .BCIN(18'h0),
    .CARRYIN(1'b0), .CARRYCASCIN(1'b0), .MULTSIGNIN(1'b0), .CEA1(1'b1), .CEA2(1'b1),
    .CEAD(1'b1), .CEALUMODE(1'b1), .CEB1(1'b1), .CEB2(1'b1), .CEC(1'b1), .CECARRYIN(1'b1),
    .CECTRL(1'b1), .CED(1'b1), .CEINMODE(1'b1), .CEM(1'b1), .CEP(1'b1), .RSTA(1'b0),
    .RSTALLCARRYIN(1'b0), .RSTALUMODE(1'b0), .RSTB(1'b0), .RSTC(1'b0), .RSTCTRL(1'b0),
    .RSTD(1'b0), .RSTINMODE(1'b0), .RSTM(1'b0), .RSTP(1'b0), .P(p2), .OPMODE(7'b0110101),
    .ALUMODE(4'b0000), .CARRYINSEL(3'b000), .INMODE(5'b00000));
  integer i, ta, tb_, tc;
  initial begin
    for (i = 0; i < 24; i = i + 1) begin
      ta = i + 3;
      tb_ = 2 * i + 1;
      tc = 1000 * i;
      #2 a = ta[29:0];
      b = tb_[17:0];
      c = {16'h0, tc};
      #6 $display("%0d %h %h", i, p, p2);
      #2;
    end
    $finish;
  end
endmodule
