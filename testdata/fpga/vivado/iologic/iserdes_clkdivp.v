module iserdes_clkdivp(input clk, input clkd, input c0, input c1, input c2, input c3, output reg r, input p0, input p1, input p2);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire gdiv; BUFG bd(.I(clkd), .O(gdiv));
  reg [15:0] sh; always @(posedge gdiv) sh <= {sh[14:0], c3};
  wire q1_0, q2_0, q3_0, q4_0, q5_0, q6_0, q7_0, q8_0;
  ISERDESE2 u0(.Q1(q1_0), .Q2(q2_0), .Q3(q3_0), .Q4(q4_0), .Q5(q5_0), .Q6(q6_0), .Q7(q7_0), .Q8(q8_0), .D(p0), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(gdiv), .OCLK(gclk), .OCLKB(~gclk), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0));
  wire q1_1, q2_1, q3_1, q4_1, q5_1, q6_1, q7_1, q8_1;
  ISERDESE2 u1(.Q1(q1_1), .Q2(q2_1), .Q3(q3_1), .Q4(q4_1), .Q5(q5_1), .Q6(q6_1), .Q7(q7_1), .Q8(q8_1), .D(p1), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(gdiv), .OCLK(gclk), .OCLKB(~gclk), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0));
  wire q1_2, q2_2, q3_2, q4_2, q5_2, q6_2, q7_2, q8_2;
  ISERDESE2 #(.IS_CLKDIVP_INVERTED(1'b1)) u2(.Q1(q1_2), .Q2(q2_2), .Q3(q3_2), .Q4(q4_2), .Q5(q5_2), .Q6(q6_2), .Q7(q7_2), .Q8(q8_2), .D(p2), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(gdiv), .OCLK(gclk), .OCLKB(~gclk), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0));
  always @(posedge gdiv) r <= q1_0 ^ q2_0 ^ q3_0 ^ q4_0 ^ q5_0 ^ q6_0 ^ q7_0 ^ q8_0 ^ q1_1 ^ q2_1 ^ q3_1 ^ q4_1 ^ q5_1 ^ q6_1 ^ q7_1 ^ q8_1 ^ q1_2 ^ q2_2 ^ q3_2 ^ q4_2 ^ q5_2 ^ q6_2 ^ q7_2 ^ q8_2;
endmodule
