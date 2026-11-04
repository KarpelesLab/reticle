module mix_tmds_lvcmos33(input en, input dn, output r, output a0p, output a0n, output a1);
  OBUFDS u0(.I(dn), .O(a0p), .OB(a0n));
  assign a1 = dn;
  assign r = 1'b0;
endmodule
