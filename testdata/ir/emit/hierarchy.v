module adder (
  input wire [3:0] a,
  input wire [3:0] b,
  output wire [3:0] y
);
  assign y = a + b;
endmodule

module top (
  input wire [3:0] x,
  input wire [3:0] y,
  input wire [3:0] z,
  output wire [3:0] s
);
  wire [3:0] t;
  adder u0 (
    .a(x),
    .b(y),
    .y(t)
  );
  adder u1 (
    .a(t),
    .b(z),
    .y(s)
  );
endmodule
