module shifter (
  input wire clk,
  input wire rst_n,
  input wire en,
  input wire d,
  output wire [3:0] q
);
  wire [3:0] u_reg$en;
  wire \$g0 ;
  wire \$g1 ;
  wire \$g2 ;
  wire \$g3 ;
  wire u_reg$q0;
  wire u_reg$q1;
  wire u_reg$q2;
  wire u_reg$q3;
  assign u_reg$en = {\$g0 , \$g1 , \$g2 , \$g3 };
  assign q = {u_reg$q3, u_reg$q2, u_reg$q1, u_reg$q0};
  MUX2_X1 \$g4  (
    .A0(q[3]),
    .A1(q[2]),
    .S(en),
    .Y(\$g0 )
  );
  MUX2_X1 \$g5  (
    .A0(q[2]),
    .A1(q[1]),
    .S(en),
    .Y(\$g1 )
  );
  MUX2_X1 \$g6  (
    .A0(q[1]),
    .A1(q[0]),
    .S(en),
    .Y(\$g2 )
  );
  MUX2_X1 \$g7  (
    .A0(q[0]),
    .A1(d),
    .S(en),
    .Y(\$g3 )
  );
  DFFR_X1 u_reg$ff0 (
    .CLK(clk),
    .D(u_reg$en[0]),
    .RN(rst_n),
    .Q(u_reg$q0)
  );
  DFFR_X1 u_reg$ff1 (
    .CLK(clk),
    .D(u_reg$en[1]),
    .RN(rst_n),
    .Q(u_reg$q1)
  );
  DFFR_X1 u_reg$ff2 (
    .CLK(clk),
    .D(u_reg$en[2]),
    .RN(rst_n),
    .Q(u_reg$q2)
  );
  DFFR_X1 u_reg$ff3 (
    .CLK(clk),
    .D(u_reg$en[3]),
    .RN(rst_n),
    .Q(u_reg$q3)
  );
endmodule
