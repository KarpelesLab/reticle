module sstl15_out_lvcmos33_in(input en, input dn, output r, output a0, input a1);
  assign a0 = dn;
  assign r = a1;
endmodule
