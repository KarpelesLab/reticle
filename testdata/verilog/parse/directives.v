// Directives that reach the parser, at unit and module level, with
// macros expanded before parsing.
`timescale 1ns / 1ps
`default_nettype none
`define WIDTH 8
`define REG(name, w) reg [w-1:0] name
`resetall
`timescale 10 ps / 1 ps

module dir(input wire clk, output reg [`WIDTH-1:0] q);
    `default_nettype wire
    `REG(r, `WIDTH);
    `ifdef SYNTHESIS
    wire synth = 1'b1;
    `else
    wire synth = 1'b0;
    `endif
    always @(posedge clk) q <= r;
    `celldefine
    `endcelldefine
endmodule

`default_nettype wire
