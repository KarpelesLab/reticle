// An output port connected to a net of a different width.
module eow_leaf(input wire [7:0] a, output wire [7:0] y);
    assign y = a;
endmodule

module error_output_width(input wire [7:0] a, output wire [3:0] y);
    eow_leaf u0 (.a(a), .y(y));
endmodule
