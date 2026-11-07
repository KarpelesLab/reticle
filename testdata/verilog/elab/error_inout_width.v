// An inout port connected to a net of another width: the bits would have to
// go both ways, which a narrowing or a widening does not say how to do.
module eiw_leaf(inout wire [7:0] y);
endmodule

module error_inout_width(inout wire [3:0] y);
    eiw_leaf u0 (.y(y));
endmodule
