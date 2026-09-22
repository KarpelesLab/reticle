// A parameter overridden with defparam rather than with #().
module dp_leaf #(parameter W = 4) (
    input  wire [W-1:0] a,
    output wire [W-1:0] y
);
    assign y = a;
endmodule

module defparam_top(
    input  wire [7:0] a,
    output wire [7:0] y
);
    dp_leaf u0 (.a(a), .y(y));
    defparam u0.W = 8;
endmodule
