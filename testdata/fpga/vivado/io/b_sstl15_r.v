module b_sstl15_r(input en, input dn, output r, inout a0p, inout a0n, inout a1p, inout a1n, inout a2p, inout a2n, inout a3p, inout a3n, inout a4p, inout a4n, inout a5p, inout a5n, inout a6p, inout a6n, inout a7p, inout a7n, inout a8p, inout a8n);
  wire x0p, x0n; IOBUF u0p(.IO(a0p), .I(dn), .T(en), .O(x0p)); IOBUF u0n(.IO(a0n), .I(dn), .T(en), .O(x0n));
  wire x1p, x1n; IOBUF u1p(.IO(a1p), .I(dn), .T(en), .O(x1p)); IOBUF u1n(.IO(a1n), .I(dn), .T(en), .O(x1n));
  wire x2p, x2n; IOBUF u2p(.IO(a2p), .I(dn), .T(en), .O(x2p)); IOBUF u2n(.IO(a2n), .I(dn), .T(en), .O(x2n));
  wire x3p, x3n; IOBUF u3p(.IO(a3p), .I(dn), .T(en), .O(x3p)); IOBUF u3n(.IO(a3n), .I(dn), .T(en), .O(x3n));
  wire x4p, x4n; IOBUF u4p(.IO(a4p), .I(dn), .T(en), .O(x4p)); IOBUF u4n(.IO(a4n), .I(dn), .T(en), .O(x4n));
  wire x5p, x5n; IOBUF u5p(.IO(a5p), .I(dn), .T(en), .O(x5p)); IOBUF u5n(.IO(a5n), .I(dn), .T(en), .O(x5n));
  wire x6p, x6n; IOBUF u6p(.IO(a6p), .I(dn), .T(en), .O(x6p)); IOBUF u6n(.IO(a6n), .I(dn), .T(en), .O(x6n));
  wire x7p, x7n; IOBUF u7p(.IO(a7p), .I(dn), .T(en), .O(x7p)); IOBUF u7n(.IO(a7n), .I(dn), .T(en), .O(x7n));
  wire x8p, x8n; IOBUF u8p(.IO(a8p), .I(dn), .T(en), .O(x8p)); IOBUF u8n(.IO(a8n), .I(dn), .T(en), .O(x8n));
  assign r = x0p ^ x0n ^ x1p ^ x1n ^ x2p ^ x2n ^ x3p ^ x3n ^ x4p ^ x4n ^ x5p ^ x5n ^ x6p ^ x6n ^ x7p ^ x7n ^ x8p ^ x8n;
endmodule
