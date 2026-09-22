// Sensitivity lists that do not say what the block reads.
// lint-config: off:all warn:sensitivity-list
module sens (
    input  wire a,
    input  wire b,
    input  wire clk,
    output reg  y,
    output reg  z,
    output reg  w
);
    always @(a) y = a & b;               // `b` is missing

    always @(posedge clk or b) z <= a;   // an edge next to a level

    always @* w = 1'b0;                  // reads nothing at all
endmodule
