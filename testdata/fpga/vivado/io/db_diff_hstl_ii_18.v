module db_diff_hstl_ii_18(input en, input dn, output r, inout a0p, inout a0n, inout a1p, inout a1n, inout a2p, inout a2n, inout a3p, inout a3n, inout a4p, inout a4n, inout a5p, inout a5n, inout a6p, inout a6n, inout a7p, inout a7n);
  wire x0; IOBUFDS u0(.I(dn), .T(en), .O(x0), .IO(a0p), .IOB(a0n));
  wire x1; IOBUFDS u1(.I(dn), .T(en), .O(x1), .IO(a1p), .IOB(a1n));
  wire x2; IOBUFDS u2(.I(dn), .T(en), .O(x2), .IO(a2p), .IOB(a2n));
  wire x3; IOBUFDS u3(.I(dn), .T(en), .O(x3), .IO(a3p), .IOB(a3n));
  wire x4; IOBUFDS u4(.I(dn), .T(en), .O(x4), .IO(a4p), .IOB(a4n));
  wire x5; IOBUFDS u5(.I(dn), .T(en), .O(x5), .IO(a5p), .IOB(a5n));
  wire x6; IOBUFDS u6(.I(dn), .T(en), .O(x6), .IO(a6p), .IOB(a6n));
  wire x7; IOBUFDS u7(.I(dn), .T(en), .O(x7), .IO(a7p), .IOB(a7n));
  assign r = x0 ^ x1 ^ x2 ^ x3 ^ x4 ^ x5 ^ x6 ^ x7;
endmodule
