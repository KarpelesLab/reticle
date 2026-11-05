module serdes_feedback(input clk, input clkd, input c0, input c1, input c2, input c3, output reg r, output p0, output p1);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire gdiv; BUFG bd(.I(clkd), .O(gdiv));
  reg [15:0] sh; always @(posedge gdiv) sh <= {sh[14:0], c3};
  wire ofb_1;
  wire ofb_0;
  wire q1_0, q2_0, q3_0, q4_0, q5_0, q6_0, q7_0, q8_0;
  ISERDESE2 #(.OFB_USED("TRUE")) i0(.Q1(q1_0), .Q2(q2_0), .Q3(q3_0), .Q4(q4_0), .Q5(q5_0), .Q6(q6_0), .Q7(q7_0), .Q8(q8_0), .D(), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(1'b0), .OCLK(gclk), .OCLKB(~gclk), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0), .OFB(ofb_0));
  OSERDESE2 u0(.OQ(p0), .OFB(ofb_0), .CLK(gclk), .CLKDIV(gdiv), .OCE(c0), .RST(c1), .D1(sh[0]), .D2(sh[1]), .D3(sh[2]), .D4(sh[3]), .D5(sh[4]), .D6(sh[5]), .D7(sh[6]), .D8(sh[7]));
  wire q1_1, q2_1, q3_1, q4_1, q5_1, q6_1, q7_1, q8_1;
  ISERDESE2 #(.DATA_WIDTH(8), .INTERFACE_TYPE("NETWORKING"), .OFB_USED("TRUE")) i1(.Q1(q1_1), .Q2(q2_1), .Q3(q3_1), .Q4(q4_1), .Q5(q5_1), .Q6(q6_1), .Q7(q7_1), .Q8(q8_1), .D(), .CLK(gclk), .CLKB(~gclk), .CLKDIV(gdiv), .CLKDIVP(1'b0), .OCLK(), .OCLKB(), .CE1(c0), .CE2(c1), .RST(c2), .BITSLIP(c3), .DYNCLKSEL(1'b0), .DYNCLKDIVSEL(1'b0), .OFB(ofb_1));
  OSERDESE2 #(.DATA_WIDTH(8), .TRISTATE_WIDTH(1)) u1(.OQ(p1), .OFB(ofb_1), .CLK(gclk), .CLKDIV(gdiv), .OCE(c0), .RST(c1), .D1(sh[0]), .D2(sh[1]), .D3(sh[2]), .D4(sh[3]), .D5(sh[4]), .D6(sh[5]), .D7(sh[6]), .D8(sh[7]));
  always @(posedge gdiv) r <= q1_0 ^ q2_0 ^ q3_0 ^ q4_0 ^ q5_0 ^ q6_0 ^ q7_0 ^ q8_0 ^ q1_1 ^ q2_1 ^ q3_1 ^ q4_1 ^ q5_1 ^ q6_1 ^ q7_1 ^ q8_1;
endmodule
