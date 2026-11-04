module inst_term(input en, input dn, output r, input a0p, input a0n);
  wire x0; IBUFDS #(.DIFF_TERM("TRUE")) u0(.I(a0p), .IB(a0n), .O(x0));
  assign r = x0;
endmodule
