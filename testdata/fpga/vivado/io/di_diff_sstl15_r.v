module di_diff_sstl15_r(input en, input dn, output r, input a0p, input a0n, input a1p, input a1n, input a2p, input a2n, input a3p, input a3n, input a4p, input a4n, input a5p, input a5n, input a6p, input a6n, input a7p, input a7n, input a8p, input a8n, input a9p, input a9n, input a10p, input a10n, input a11p, input a11n);
  wire x0; IBUFDS u0(.I(a0p), .IB(a0n), .O(x0));
  wire x1; IBUFDS u1(.I(a1p), .IB(a1n), .O(x1));
  wire x2; IBUFDS u2(.I(a2p), .IB(a2n), .O(x2));
  wire x3; IBUFDS u3(.I(a3p), .IB(a3n), .O(x3));
  wire x4; IBUFDS u4(.I(a4p), .IB(a4n), .O(x4));
  wire x5; IBUFDS u5(.I(a5p), .IB(a5n), .O(x5));
  wire x6; IBUFDS u6(.I(a6p), .IB(a6n), .O(x6));
  wire x7; IBUFDS u7(.I(a7p), .IB(a7n), .O(x7));
  wire x8; IBUFDS u8(.I(a8p), .IB(a8n), .O(x8));
  wire x9; IBUFDS u9(.I(a9p), .IB(a9n), .O(x9));
  wire x10; IBUFDS u10(.I(a10p), .IB(a10n), .O(x10));
  wire x11; IBUFDS u11(.I(a11p), .IB(a11n), .O(x11));
  assign r = x0 ^ x1 ^ x2 ^ x3 ^ x4 ^ x5 ^ x6 ^ x7 ^ x8 ^ x9 ^ x10 ^ x11;
endmodule
