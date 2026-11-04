module unread_pair(input en, input dn, output r, input a0p, input a0n);
  IBUFDS u0(.I(a0p), .IB(a0n), .O());
  assign r = 1'b0;
endmodule
