// An input port left unconnected warns.
module uc_leaf(input wire a, input wire b, output wire y);
    assign y = a & b;
endmodule

module unconnected(input wire a, output wire y);
    uc_leaf u0 (.a(a), .y(y));
endmodule
