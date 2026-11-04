module bank14_lvds(input en, input dn, output r, output a0p, output a0n);
  OBUFDS u0(.I(dn), .O(a0p), .OB(a0n));
  assign r = 1'b0;
endmodule
