module lvttl_lvcmos33(input en, input dn, output r, output a0, output a1);
  assign a0 = dn;
  assign a1 = dn;
  assign r = 1'b0;
endmodule
