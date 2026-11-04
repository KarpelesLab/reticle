module bad_bidir(input en, input dn, output r, inout a0);
  wire x0; IOBUF u0(.IO(a0), .I(dn), .T(en), .O(x0));
  assign r = x0;
endmodule
