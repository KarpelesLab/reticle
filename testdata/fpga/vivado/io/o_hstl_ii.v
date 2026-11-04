module o_hstl_ii(input en, input dn, output r, output a0p, output a0n, output a1p, output a1n, output a2p, output a2n, output a3p, output a3n, output a4p, output a4n, output a5p, output a5n);
  assign a0p = dn; assign a0n = dn;
  assign a1p = dn; assign a1n = dn;
  assign a2p = dn; assign a2n = dn;
  assign a3p = dn; assign a3n = dn;
  assign a4p = dn; assign a4n = dn;
  assign a5p = dn; assign a5n = dn;
  assign r = 1'b0;
endmodule
