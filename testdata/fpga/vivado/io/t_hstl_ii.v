module t_hstl_ii(input en, input dn, output r, output a0p, output a0n, output a1p, output a1n, output a2p, output a2n, output a3p, output a3n, output a4p, output a4n, output a5p, output a5n);
  OBUFT t0p(.O(a0p), .I(dn), .T(en)); OBUFT t0n(.O(a0n), .I(dn), .T(en));
  OBUFT t1p(.O(a1p), .I(dn), .T(en)); OBUFT t1n(.O(a1n), .I(dn), .T(en));
  OBUFT t2p(.O(a2p), .I(dn), .T(en)); OBUFT t2n(.O(a2n), .I(dn), .T(en));
  OBUFT t3p(.O(a3p), .I(dn), .T(en)); OBUFT t3n(.O(a3n), .I(dn), .T(en));
  OBUFT t4p(.O(a4p), .I(dn), .T(en)); OBUFT t4n(.O(a4n), .I(dn), .T(en));
  OBUFT t5p(.O(a5p), .I(dn), .T(en)); OBUFT t5n(.O(a5n), .I(dn), .T(en));
  assign r = 1'b0;
endmodule
