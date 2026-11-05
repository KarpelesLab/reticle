module iserdes_iobdelay(input clk, input clkd, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire gdiv; BUFG bd(.I(clkd), .O(gdiv));
  reg [15:0] sh; always @(posedge gdiv) sh <= {sh[14:0], c3};
  wire q1_0, q2_0, q3_0, q4_0, q5_0, q6_0, q7_0, q8_0;
  ISERDESE2 #(.IOBDELAY("BOTH")) u0(.Q1(q1_0), .Q2(q2_0), .Q3(q3_0), .Q4(q4_0), .Q5(q5_0), .Q6(q6_0), .Q7(q7_0), .Q8(q8_0), .D(p0), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(1'b0), .OCLK(gclk), .OCLKB(~gclk), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0));
  always @(posedge gdiv) r <= q1_0 ^ q2_0 ^ q3_0 ^ q4_0 ^ q5_0 ^ q6_0 ^ q7_0 ^ q8_0;
endmodule
