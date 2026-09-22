module alu (
  input wire [3:0] a,
  input wire [3:0] b,
  input wire [1:0] op,
  output wire [3:0] y
);
  wire \$g0 ;
  wire \$g1 ;
  wire \$g2 ;
  wire \$g3 ;
  wire \$g4 ;
  wire \$g5 ;
  wire \$g6 ;
  wire \$g7 ;
  wire \$g8 ;
  wire \$g9 ;
  wire \$g10 ;
  wire \$g11 ;
  wire \$g12 ;
  wire \$g13 ;
  wire \$g14 ;
  wire \$g15 ;
  wire \$g16 ;
  wire \$g17 ;
  wire \$g18 ;
  wire \$g19 ;
  wire \$g20 ;
  wire \$g21 ;
  wire \$g22 ;
  wire \$g23 ;
  wire \$g24 ;
  wire \$g25 ;
  wire \$g26 ;
  wire \$g27 ;
  wire \$g28 ;
  wire \$g29 ;
  wire \$g30 ;
  wire \$g31 ;
  wire \$g32 ;
  wire \$g33 ;
  wire \$g34 ;
  wire \$g35 ;
  wire \$g36 ;
  wire \$g37 ;
  wire \$g38 ;
  wire \$g39 ;
  wire \$g40 ;
  wire \$g41 ;
  wire \$g42 ;
  wire \$g43 ;
  wire \$g44 ;
  wire \$g45 ;
  wire \$g46 ;
  wire \$g47 ;
  wire \$g48 ;
  wire \$g49 ;
  wire \$g50 ;
  wire \$g51 ;
  wire \$g52 ;
  wire \$g53 ;
  wire \$g54 ;
  wire \$g55 ;
  wire \$g56 ;
  wire \$g57 ;
  wire \$g58 ;
  wire \$g59 ;
  wire \$g60 ;
  wire \$g61 ;
  wire \$g62 ;
  wire \$g63 ;
  wire \$g64 ;
  wire \$g65 ;
  wire \$g66 ;
  wire \$g67 ;
  assign y = {\$g21 , \$g44 , \$g61 , \$g67 };
  NOR2_X1 \$g68  (
    .A(a[1]),
    .B(b[1]),
    .Y(\$g0 )
  );
  AND2_X1 \$g69  (
    .A(a[1]),
    .B(b[1]),
    .Y(\$g1 )
  );
  AND2_X1 \$g70  (
    .A(a[0]),
    .B(b[0]),
    .Y(\$g2 )
  );
  INV_X1 \$g71  (
    .A(\$g0 ),
    .Y(\$g3 )
  );
  AOI21_X1 \$g72  (
    .A1(\$g2 ),
    .A2(\$g3 ),
    .B(\$g1 ),
    .Y(\$g4 )
  );
  NOR2_X1 \$g73  (
    .A(a[2]),
    .B(b[2]),
    .Y(\$g5 )
  );
  NOR2_X1 \$g74  (
    .A(\$g5 ),
    .B(\$g4 ),
    .Y(\$g6 )
  );
  AND2_X1 \$g75  (
    .A(a[2]),
    .B(b[2]),
    .Y(\$g7 )
  );
  INV_X1 \$g76  (
    .A(op[0]),
    .Y(\$g8 )
  );
  INV_X1 \$g77  (
    .A(\$g7 ),
    .Y(\$g9 )
  );
  INV_X1 \$g78  (
    .A(\$g6 ),
    .Y(\$g10 )
  );
  AOI21_X1 \$g79  (
    .A1(\$g9 ),
    .A2(\$g10 ),
    .B(\$g8 ),
    .Y(\$g11 )
  );
  XOR2_X1 \$g80  (
    .A(a[3]),
    .B(b[3]),
    .Y(\$g12 )
  );
  XNOR2_X1 \$g81  (
    .A(\$g12 ),
    .B(\$g11 ),
    .Y(\$g13 )
  );
  INV_X1 \$g82  (
    .A(a[3]),
    .Y(\$g14 )
  );
  INV_X1 \$g83  (
    .A(b[3]),
    .Y(\$g15 )
  );
  INV_X1 \$g84  (
    .A(op[0]),
    .Y(\$g16 )
  );
  AOI21_X1 \$g85  (
    .A1(\$g14 ),
    .A2(\$g15 ),
    .B(\$g16 ),
    .Y(\$g17 )
  );
  AOI21_X1 \$g86  (
    .A1(a[3]),
    .A2(b[3]),
    .B(\$g17 ),
    .Y(\$g18 )
  );
  INV_X1 \$g87  (
    .A(\$g18 ),
    .Y(\$g19 )
  );
  INV_X1 \$g88  (
    .A(\$g13 ),
    .Y(\$g20 )
  );
  MUX2_X1 \$g89  (
    .A0(\$g19 ),
    .A1(\$g20 ),
    .S(op[1]),
    .Y(\$g21 )
  );
  INV_X1 \$g90  (
    .A(a[1]),
    .Y(\$g22 )
  );
  INV_X1 \$g91  (
    .A(b[1]),
    .Y(\$g23 )
  );
  INV_X1 \$g92  (
    .A(\$g2 ),
    .Y(\$g24 )
  );
  AOI21_X1 \$g93  (
    .A1(\$g22 ),
    .A2(\$g23 ),
    .B(\$g24 ),
    .Y(\$g25 )
  );
  INV_X1 \$g94  (
    .A(op[0]),
    .Y(\$g26 )
  );
  INV_X1 \$g95  (
    .A(\$g1 ),
    .Y(\$g27 )
  );
  INV_X1 \$g96  (
    .A(\$g25 ),
    .Y(\$g28 )
  );
  AOI21_X1 \$g97  (
    .A1(\$g27 ),
    .A2(\$g28 ),
    .B(\$g26 ),
    .Y(\$g29 )
  );
  XOR2_X1 \$g98  (
    .A(a[2]),
    .B(b[2]),
    .Y(\$g30 )
  );
  INV_X1 \$g99  (
    .A(op[1]),
    .Y(\$g31 )
  );
  AOI21_X1 \$g100  (
    .A1(\$g30 ),
    .A2(\$g29 ),
    .B(\$g31 ),
    .Y(\$g32 )
  );
  INV_X1 \$g101  (
    .A(\$g4 ),
    .Y(\$g33 )
  );
  AOI21_X1 \$g102  (
    .A1(op[0]),
    .A2(\$g33 ),
    .B(\$g30 ),
    .Y(\$g34 )
  );
  INV_X1 \$g103  (
    .A(a[2]),
    .Y(\$g35 )
  );
  INV_X1 \$g104  (
    .A(b[2]),
    .Y(\$g36 )
  );
  INV_X1 \$g105  (
    .A(op[0]),
    .Y(\$g37 )
  );
  AOI21_X1 \$g106  (
    .A1(\$g35 ),
    .A2(\$g36 ),
    .B(\$g37 ),
    .Y(\$g38 )
  );
  INV_X1 \$g107  (
    .A(\$g7 ),
    .Y(\$g39 )
  );
  INV_X1 \$g108  (
    .A(\$g38 ),
    .Y(\$g40 )
  );
  AOI21_X1 \$g109  (
    .A1(\$g39 ),
    .A2(\$g40 ),
    .B(op[1]),
    .Y(\$g41 )
  );
  INV_X1 \$g110  (
    .A(\$g41 ),
    .Y(\$g42 )
  );
  INV_X1 \$g111  (
    .A(\$g32 ),
    .Y(\$g43 )
  );
  OAI21_X1 \$g112  (
    .A1(\$g34 ),
    .A2(\$g43 ),
    .B(\$g42 ),
    .Y(\$g44 )
  );
  AND2_X1 \$g113  (
    .A(op[0]),
    .B(\$g2 ),
    .Y(\$g45 )
  );
  XOR2_X1 \$g114  (
    .A(a[1]),
    .B(b[1]),
    .Y(\$g46 )
  );
  INV_X1 \$g115  (
    .A(op[1]),
    .Y(\$g47 )
  );
  INV_X1 \$g116  (
    .A(\$g46 ),
    .Y(\$g48 )
  );
  INV_X1 \$g117  (
    .A(\$g45 ),
    .Y(\$g49 )
  );
  AOI21_X1 \$g118  (
    .A1(\$g48 ),
    .A2(\$g49 ),
    .B(\$g47 ),
    .Y(\$g50 )
  );
  AND2_X1 \$g119  (
    .A(\$g46 ),
    .B(\$g45 ),
    .Y(\$g51 )
  );
  INV_X1 \$g120  (
    .A(a[1]),
    .Y(\$g52 )
  );
  INV_X1 \$g121  (
    .A(b[1]),
    .Y(\$g53 )
  );
  INV_X1 \$g122  (
    .A(op[0]),
    .Y(\$g54 )
  );
  AOI21_X1 \$g123  (
    .A1(\$g52 ),
    .A2(\$g53 ),
    .B(\$g54 ),
    .Y(\$g55 )
  );
  INV_X1 \$g124  (
    .A(\$g1 ),
    .Y(\$g56 )
  );
  INV_X1 \$g125  (
    .A(\$g55 ),
    .Y(\$g57 )
  );
  AOI21_X1 \$g126  (
    .A1(\$g56 ),
    .A2(\$g57 ),
    .B(op[1]),
    .Y(\$g58 )
  );
  INV_X1 \$g127  (
    .A(\$g58 ),
    .Y(\$g59 )
  );
  INV_X1 \$g128  (
    .A(\$g50 ),
    .Y(\$g60 )
  );
  OAI21_X1 \$g129  (
    .A1(\$g51 ),
    .A2(\$g60 ),
    .B(\$g59 ),
    .Y(\$g61 )
  );
  NOR2_X1 \$g130  (
    .A(op[0]),
    .B(op[1]),
    .Y(\$g62 )
  );
  NOR2_X1 \$g131  (
    .A(a[0]),
    .B(b[0]),
    .Y(\$g63 )
  );
  INV_X1 \$g132  (
    .A(\$g63 ),
    .Y(\$g64 )
  );
  INV_X1 \$g133  (
    .A(\$g62 ),
    .Y(\$g65 )
  );
  AOI21_X1 \$g134  (
    .A1(\$g64 ),
    .A2(\$g65 ),
    .B(\$g2 ),
    .Y(\$g66 )
  );
  AOI21_X1 \$g135  (
    .A1(op[1]),
    .A2(\$g2 ),
    .B(\$g66 ),
    .Y(\$g67 )
  );
endmodule
