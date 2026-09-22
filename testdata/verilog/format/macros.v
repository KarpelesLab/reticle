`timescale 1ns / 1ps
`default_nettype none

`define TAP_COUNT 8
`define SHIFT(reg_name, width) \
    reg [width-1:0] reg_name

module macros (input wire clk, input wire rst, output wire done);
`ifdef FAST
    localparam LIMIT = 4;
`else
    localparam LIMIT = 16;
`endif
    reg [7:0] count;

    always @(posedge clk) begin
        if (rst)
            count <= 8'd0;
        else
            count <= count + 8'd1;
    end

    assign done = (count == LIMIT);
endmodule

`default_nettype wire
