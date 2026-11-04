module t_lvcmos33(input en, input dn, output r, output a0p, output a0n, output a1p, output a1n, output a2p, output a2n, output a3p, output a3n, output a4p, output a4n, output a5p, output a5n, output a6p, output a6n, output a7p, output a7n, output a8p, output a8n, output a9p, output a9n, output a10p, output a10n, output a11p, output a11n, output a12p, output a12n, output a13p, output a13n);
  OBUFT t0p(.O(a0p), .I(dn), .T(en)); OBUFT t0n(.O(a0n), .I(dn), .T(en));
  OBUFT t1p(.O(a1p), .I(dn), .T(en)); OBUFT t1n(.O(a1n), .I(dn), .T(en));
  OBUFT t2p(.O(a2p), .I(dn), .T(en)); OBUFT t2n(.O(a2n), .I(dn), .T(en));
  OBUFT t3p(.O(a3p), .I(dn), .T(en)); OBUFT t3n(.O(a3n), .I(dn), .T(en));
  OBUFT t4p(.O(a4p), .I(dn), .T(en)); OBUFT t4n(.O(a4n), .I(dn), .T(en));
  OBUFT t5p(.O(a5p), .I(dn), .T(en)); OBUFT t5n(.O(a5n), .I(dn), .T(en));
  OBUFT t6p(.O(a6p), .I(dn), .T(en)); OBUFT t6n(.O(a6n), .I(dn), .T(en));
  OBUFT t7p(.O(a7p), .I(dn), .T(en)); OBUFT t7n(.O(a7n), .I(dn), .T(en));
  OBUFT t8p(.O(a8p), .I(dn), .T(en)); OBUFT t8n(.O(a8n), .I(dn), .T(en));
  OBUFT t9p(.O(a9p), .I(dn), .T(en)); OBUFT t9n(.O(a9n), .I(dn), .T(en));
  OBUFT t10p(.O(a10p), .I(dn), .T(en)); OBUFT t10n(.O(a10n), .I(dn), .T(en));
  OBUFT t11p(.O(a11p), .I(dn), .T(en)); OBUFT t11n(.O(a11n), .I(dn), .T(en));
  OBUFT t12p(.O(a12p), .I(dn), .T(en)); OBUFT t12n(.O(a12n), .I(dn), .T(en));
  OBUFT t13p(.O(a13p), .I(dn), .T(en)); OBUFT t13n(.O(a13n), .I(dn), .T(en));
  assign r = 1'b0;
endmodule
