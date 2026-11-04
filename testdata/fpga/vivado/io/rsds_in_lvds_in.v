module rsds_in_lvds_in(input en, input dn, output r, input a0p, input a0n, input a1p, input a1n);
  wire x0; IBUFDS u0(.I(a0p), .IB(a0n), .O(x0));
  wire x1; IBUFDS u1(.I(a1p), .IB(a1n), .O(x1));
  assign r = x0 ^ x1;
endmodule
