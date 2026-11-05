module ddly_wide(input clk, input clkd, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire gdiv; BUFG bd(.I(clkd), .O(gdiv));
  wire d0;
  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  wire q1_0, q2_0, q3_0, q4_0, q5_0, q6_0, q7_0, q8_0, so1_0, so2_0;
  ISERDESE2 #(.IOBDELAY("IFD"), .INTERFACE_TYPE("NETWORKING"), .DATA_WIDTH(10)) v0(.Q1(q1_0), .Q2(q2_0), .Q3(q3_0), .Q4(q4_0), .Q5(q5_0), .Q6(q6_0), .Q7(q7_0), .Q8(q8_0), .SHIFTOUT1(so1_0), .SHIFTOUT2(so2_0), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(1'b0), .OCLK(), .OCLKB(), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0), .D(), .DDLY(d0));
  wire t3_0, t4_0, t5_0, t6_0, t7_0, t8_0;
  ISERDESE2 #(.IOBDELAY("IFD"), .INTERFACE_TYPE("NETWORKING"), .DATA_WIDTH(10), .SERDES_MODE("SLAVE")) w0(.Q3(t3_0), .Q4(t4_0), .Q5(t5_0), .Q6(t6_0), .Q7(t7_0), .Q8(t8_0), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(1'b0), .OCLK(), .OCLKB(), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0), .SHIFTIN1(so1_0), .SHIFTIN2(so2_0));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gdiv) r <= q1_0 ^ q2_0 ^ q3_0 ^ q4_0 ^ q5_0 ^ q6_0 ^ q7_0 ^ q8_0 ^ t3_0 ^ t4_0 ^ t5_0 ^ t6_0 ^ t7_0 ^ t8_0 ^ rdy;
endmodule
