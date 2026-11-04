module i_sstl15_r(input en, input dn, output r, input a0p, input a0n, input a1p, input a1n, input a2p, input a2n, input a3p, input a3n, input a4p, input a4n, input a5p, input a5n, input a6p, input a6n, input a7p, input a7n, input a8p, input a8n, input a9p, input a9n, input a10p, input a10n, input a11p, input a11n);
  assign r = a0p ^ a0n ^ a1p ^ a1n ^ a2p ^ a2n ^ a3p ^ a3n ^ a4p ^ a4n ^ a5p ^ a5n ^ a6p ^ a6n ^ a7p ^ a7n ^ a8p ^ a8n ^ a9p ^ a9n ^ a10p ^ a10n ^ a11p ^ a11n;
endmodule
