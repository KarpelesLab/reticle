module counter (
  input wire clk,
  input wire rst,
  input wire en,
  output wire [3:0] q,
  output wire carry
);
  wire [3:0] u_reg$rst;
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
  wire u_reg$q0;
  wire u_reg$q1;
  wire u_reg$q2;
  wire u_reg$q3;
  assign u_reg$rst = {\$g3 , \$g6 , \$g8 , \$g9 };
  assign q = {u_reg$q3, u_reg$q2, u_reg$q1, u_reg$q0};
  AND2_X1 \$g10  (
    .A(q[0]),
    .B(q[1]),
    .Y(\$g0 )
  );
  AOI21_X1 \$g11  (
    .A1(q[2]),
    .A2(\$g0 ),
    .B(q[3]),
    .Y(\$g1 )
  );
  AND2_X1 \$g12  (
    .A(q[2]),
    .B(q[3]),
    .Y(\$g2 )
  );
  AND2_X1 \$g13  (
    .A(\$g0 ),
    .B(\$g2 ),
    .Y(carry)
  );
  NOR3_X1 \$g14  (
    .A(rst),
    .B(carry),
    .C(\$g1 ),
    .Y(\$g3 )
  );
  AND2_X1 \$g15  (
    .A(q[2]),
    .B(\$g0 ),
    .Y(\$g4 )
  );
  AOI21_X1 \$g16  (
    .A1(q[0]),
    .A2(q[1]),
    .B(q[2]),
    .Y(\$g5 )
  );
  NOR3_X1 \$g17  (
    .A(rst),
    .B(\$g5 ),
    .C(\$g4 ),
    .Y(\$g6 )
  );
  NOR2_X1 \$g18  (
    .A(q[0]),
    .B(q[1]),
    .Y(\$g7 )
  );
  NOR3_X1 \$g19  (
    .A(rst),
    .B(\$g0 ),
    .C(\$g7 ),
    .Y(\$g8 )
  );
  NOR2_X1 \$g20  (
    .A(q[0]),
    .B(rst),
    .Y(\$g9 )
  );
  DFFE_X1 u_reg$ff0 (
    .CLK(clk),
    .D(u_reg$rst[0]),
    .E(en),
    .Q(u_reg$q0)
  );
  DFFE_X1 u_reg$ff1 (
    .CLK(clk),
    .D(u_reg$rst[1]),
    .E(en),
    .Q(u_reg$q1)
  );
  DFFE_X1 u_reg$ff2 (
    .CLK(clk),
    .D(u_reg$rst[2]),
    .E(en),
    .Q(u_reg$q2)
  );
  DFFE_X1 u_reg$ff3 (
    .CLK(clk),
    .D(u_reg$rst[3]),
    .E(en),
    .Q(u_reg$q3)
  );
endmodule
