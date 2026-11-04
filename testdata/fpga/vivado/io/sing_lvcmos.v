module sing_lvcmos(input en, input dn, output r, output a0, input a1);
  assign a0 = dn;
  assign r = a1;
endmodule
