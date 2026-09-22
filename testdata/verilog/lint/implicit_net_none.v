// The same mistake with implicit nets switched off: an error, not a warning.
// lint-config: off:all warn:implicit-net
`default_nettype none
module leaf (input wire i, output wire o);
    assign o = i;
endmodule

module top (input wire a, output wire y);
    leaf u0 (.i(a), .o(mid));
    assign y = a;
endmodule
`default_nettype wire
