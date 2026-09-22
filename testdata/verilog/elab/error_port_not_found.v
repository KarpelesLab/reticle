// An instance connecting a port the target does not have.
module epn_leaf(input wire a, output wire y);
    assign y = ~a;
endmodule

module error_port_not_found(input wire a, output wire y);
    epn_leaf u0 (.a(a), .y(y), .missing(a));
endmodule
