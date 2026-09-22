// Signed arithmetic, sign extension and the $signed / $unsigned casts.
module signed_arith(
    input  wire signed [7:0]  a,
    input  wire signed [7:0]  b,
    input  wire        [7:0]  u,
    output wire signed [15:0] mul,
    output wire signed [8:0]  sum,
    output wire               lt,
    output wire               ult,
    output wire signed [7:0]  neg
);
    assign mul = a * b;
    assign sum = a + b;
    assign lt  = a < b;
    assign ult = $unsigned(a) < u;
    assign neg = -a;
endmodule
