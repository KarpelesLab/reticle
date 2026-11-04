module bias_lvds_io_mini_io(input en, input dn, output r, inout a0p, inout a0n, inout a1p, inout a1n);
  wire x0; IOBUFDS u0(.I(dn), .T(en), .O(x0), .IO(a0p), .IOB(a0n));
  wire x1; IOBUFDS u1(.I(dn), .T(en), .O(x1), .IO(a1p), .IOB(a1n));
  assign r = x0 ^ x1;
endmodule
