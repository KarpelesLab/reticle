module order_rsds115_lvds109(input en, input dn, output r, output a0p, output a0n, output a1p, output a1n);
  OBUFDS u0(.I(dn), .O(a0p), .OB(a0n));
  OBUFDS u1(.I(dn), .O(a1p), .OB(a1n));
  assign r = 1'b0;
endmodule
