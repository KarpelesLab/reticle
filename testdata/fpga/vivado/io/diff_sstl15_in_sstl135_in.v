module diff_sstl15_in_sstl135_in(input en, input dn, output r, input a0p, input a0n, input a1);
  wire x0; IBUFDS u0(.I(a0p), .IB(a0n), .O(x0));
  assign r = x0 ^ a1;
endmodule
