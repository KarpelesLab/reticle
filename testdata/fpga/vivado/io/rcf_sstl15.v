module rcf_sstl15(input en, input dn, output r, input a0, output a1, input a2);
  assign a1 = dn;
  assign r = a0 ^ a2;
endmodule
