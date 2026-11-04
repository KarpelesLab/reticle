// Two of every kind of pad, for io_pairs.tcl. Written here.
module pairs(input dn, input en, output r,
  input in1, input in2, output out1, output out2, inout io1, inout io2,
  input din1p, input din1n, input din2p, input din2n,
  output dout1p, output dout1n, output dout2p, output dout2n,
  inout dio1p, inout dio1n, inout dio2p, inout dio2n);
  wire a,b,c,d,e,f;
  assign out1 = dn; assign out2 = dn;
  IOBUF u1(.IO(io1),.I(dn),.T(en),.O(a));
  IOBUF u2(.IO(io2),.I(dn),.T(en),.O(b));
  IBUFDS v1(.I(din1p),.IB(din1n),.O(c));
  IBUFDS v2(.I(din2p),.IB(din2n),.O(d));
  OBUFDS w1(.I(dn),.O(dout1p),.OB(dout1n));
  OBUFDS w2(.I(dn),.O(dout2p),.OB(dout2n));
  IOBUFDS x1(.I(dn),.T(en),.O(e),.IO(dio1p),.IOB(dio1n));
  IOBUFDS x2(.I(dn),.T(en),.O(f),.IO(dio2p),.IOB(dio2n));
  assign r = a^b^c^d^e^f^in1^in2;
endmodule
