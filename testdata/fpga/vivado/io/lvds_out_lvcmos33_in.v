module lvds_out_lvcmos33_in(input en, input dn, output r, output a0p, output a0n, input a1);
  OBUFDS u0(.I(dn), .O(a0p), .OB(a0n));
  assign r = a1;
endmodule
