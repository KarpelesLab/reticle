// An output port connected to a narrower net drives its low bits, with a
// warning, as xsim builds it; it used to be refused. An inout connected so
// is still refused (`error_inout_width.v`).
module eow_leaf(input wire [7:0] a, output wire [7:0] y);
    assign y = a;
endmodule

module output_width(input wire [7:0] a, output wire [3:0] y);
    eow_leaf u0 (.a(a), .y(y));
endmodule
