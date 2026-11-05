module idelay_iddr(input clk, input c0, input c1, input c2, input c3, output reg r, input p0, input p1, input p2);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  wire q1_0, q2_0;
  IDDR v0(.Q1(q1_0), .Q2(q2_0), .C(gclk), .CE(1'b1), .D(d0), .R(1'b0), .S(1'b0));
  wire d1;
  IDELAYE2 #(.IDELAY_VALUE(3)) u1(.IDATAIN(p1), .DATAOUT(d1));
  wire q1_1, q2_1;
  IDDR v1(.Q1(q1_1), .Q2(q2_1), .C(gclk), .CE(1'b1), .D(d1), .R(1'b0), .S(1'b0));
  wire d2;
  IDELAYE2 u2(.IDATAIN(p2), .DATAOUT(d2));
  wire q1_2, q2_2;
  IDDR v2(.Q1(q1_2), .Q2(q2_2), .C(gclk), .CE(1'b1), .D(d2), .R(1'b0), .S(1'b0));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= q1_0 ^ q2_0 ^ q1_1 ^ q2_1 ^ q1_2 ^ q2_2 ^ d2 ^ rdy;
endmodule
