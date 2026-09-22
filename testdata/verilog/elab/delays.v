// Delays on continuous assignments and inside a self-timed block.
`timescale 1ns / 1ps
module delays(
    input  wire a,
    output wire y,
    output reg  clk
);
    assign #3 y = ~a;

    initial clk = 1'b0;
    always #5 clk = ~clk;
endmodule
