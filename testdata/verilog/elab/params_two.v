// The same module instantiated twice with different parameters, which
// produces two uniquified variants.
module scaler #(
    parameter WIDTH = 8,
    parameter STEP  = 1
) (
    input  wire             clk,
    input  wire [WIDTH-1:0] d,
    output reg  [WIDTH-1:0] q
);
    always @(posedge clk)
        q <= d + STEP;
endmodule

module params_two(
    input  wire        clk,
    input  wire [7:0]  small_in,
    input  wire [15:0] big_in,
    output wire [7:0]  small_out,
    output wire [15:0] big_out
);
    scaler #(.WIDTH(8),  .STEP(1)) u_small (.clk(clk), .d(small_in), .q(small_out));
    scaler #(.WIDTH(16), .STEP(4)) u_big   (.clk(clk), .d(big_in),   .q(big_out));
endmodule
