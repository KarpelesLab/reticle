// TODO: drop the defparam once the instance carries the parameter.
// lint-config: off:all note:deprecated-construct note:todo-comment
`ifdef USE_EXTERNAL
`include "other.v"
`endif
module leaf (input wire a, output wire y);
    parameter W = 1;
    assign y = a;
endmodule

module top (input wire clk, input wire a, output reg y);
    wire mid;

    leaf u0 (.a(a), .y(mid));
    defparam u0.W = 4;

    always @(posedge clk) begin
        wait (mid) y <= mid;
    end
endmodule
