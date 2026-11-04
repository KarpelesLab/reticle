module left_tmds(input en, input dn, output r, input a0p, input a0n, output a1p, output a1n);
  wire x0; IBUFDS u0(.I(a0p), .IB(a0n), .O(x0));
  OBUFDS u1(.I(dn), .O(a1p), .OB(a1n));
  assign r = x0;
endmodule
