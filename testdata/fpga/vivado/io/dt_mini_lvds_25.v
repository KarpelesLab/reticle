module dt_mini_lvds_25(input en, input dn, output r, output a0p, output a0n, output a1p, output a1n, output a2p, output a2n, output a3p, output a3n, output a4p, output a4n, output a5p, output a5n);
  OBUFTDS u0(.I(dn), .T(en), .O(a0p), .OB(a0n));
  OBUFTDS u1(.I(dn), .T(en), .O(a1p), .OB(a1n));
  OBUFTDS u2(.I(dn), .T(en), .O(a2p), .OB(a2n));
  OBUFTDS u3(.I(dn), .T(en), .O(a3p), .OB(a3n));
  OBUFTDS u4(.I(dn), .T(en), .O(a4p), .OB(a4n));
  OBUFTDS u5(.I(dn), .T(en), .O(a5p), .OB(a5n));
  assign r = 1'b0;
endmodule
