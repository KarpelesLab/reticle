module left_rsds(input en, input dn, output r, output a0p, output a0n, input a1p, input a1n, input a2p, input a2n);
  OBUFDS u0(.I(dn), .O(a0p), .OB(a0n));
  wire x1; IBUFDS u1(.I(a1p), .IB(a1n), .O(x1));
  wire x2; IBUFDS u2(.I(a2p), .IB(a2n), .O(x2));
  assign r = x1 ^ x2;
endmodule
