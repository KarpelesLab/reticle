module bias_lvds_rsds_mini_ppds(input en, input dn, output r, output a0p, output a0n, output a1p, output a1n, output a2p, output a2n, output a3p, output a3n);
  OBUFDS u0(.I(dn), .O(a0p), .OB(a0n));
  OBUFDS u1(.I(dn), .O(a1p), .OB(a1n));
  OBUFDS u2(.I(dn), .O(a2p), .OB(a2n));
  OBUFDS u3(.I(dn), .O(a3p), .OB(a3n));
  assign r = 1'b0;
endmodule
