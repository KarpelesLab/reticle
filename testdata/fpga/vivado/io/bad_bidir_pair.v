module bad_bidir_pair(input en, input dn, output r, inout a0p, inout a0n);
  wire x0; IOBUFDS u0(.I(dn), .T(en), .O(x0), .IO(a0p), .IOB(a0n));
  assign r = x0;
endmodule
