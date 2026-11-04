module i_mobile_ddr(input en, input dn, output r, input a0p, input a0n, input a1p, input a1n, input a2p, input a2n, input a3p, input a3n, input a4p, input a4n, input a5p, input a5n, input a6p, input a6n, input a7p, input a7n);
  assign r = a0p ^ a0n ^ a1p ^ a1n ^ a2p ^ a2n ^ a3p ^ a3n ^ a4p ^ a4n ^ a5p ^ a5n ^ a6p ^ a6n ^ a7p ^ a7n;
endmodule
