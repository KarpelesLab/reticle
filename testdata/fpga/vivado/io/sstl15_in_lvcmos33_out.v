module sstl15_in_lvcmos33_out(input en, input dn, output r, input a0, output a1);
  assign a1 = dn;
  assign r = a0;
endmodule
