// A parameterised up/down counter with synchronous load and enable.
`timescale 1ns / 1ps
`default_nettype none

module counter #(
    parameter WIDTH = 8,
    parameter [WIDTH-1:0] RESET_VALUE = {WIDTH{1'b0}}
) (
    input  wire             clk,
    input  wire             rst_n,
    input  wire             en,
    input  wire             up,
    input  wire             load,
    input  wire [WIDTH-1:0] d,
    output reg  [WIDTH-1:0] q,
    output wire             wrap
);

    // Wrap flag: all ones counting up, or zero counting down.
    assign wrap = up ? &q : ~|q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            q <= RESET_VALUE;
        else if (load)
            q <= d;
        else if (en) begin
            if (up)
                q <= q + 1'b1;
            else
                q <= q - 1'b1;
        end
    end

endmodule

`default_nettype wire
