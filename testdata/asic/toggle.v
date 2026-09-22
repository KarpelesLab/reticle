module toggle (
  input wire clk,
  input wire rst,
  output wire q
);
  wire n;
  wire rst$n;
  DFFR_X1 u_ff (
    .CLK(clk),
    .D(n),
    .RN(rst$n),
    .Q(q)
  );
  INV_X1 \$g0  (
    .A(q),
    .Y(n)
  );
  INV_X1 rst$n$inv (
    .A(rst),
    .Y(rst$n)
  );
endmodule
