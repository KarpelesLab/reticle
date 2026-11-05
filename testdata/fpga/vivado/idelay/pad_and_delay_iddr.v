module pad_and_delay_iddr(input clk, input c0, input c1, input c2, input c3, output reg r, input p0);
  wire gclk; BUFG bg(.I(clk), .O(gclk));
  wire d0;
  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));
  wire q1_0, q2_0;
  IDDR v0(.Q1(q1_0), .Q2(q2_0), .C(gclk), .CE(1'b1), .D(d0), .R(1'b0), .S(1'b0));
  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));
  always @(posedge gclk) r <= q1_0 ^ q2_0 ^ p0 ^ rdy;
endmodule
