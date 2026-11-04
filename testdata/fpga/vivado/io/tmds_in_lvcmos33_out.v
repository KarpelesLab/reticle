module tmds_in_lvcmos33_out(input en, input dn, output r, input a0p, input a0n, output a1);
  wire x0; IBUFDS u0(.I(a0p), .IB(a0n), .O(x0));
  assign a1 = dn;
  assign r = x0;
endmodule
