// Names that Verilog would turn into one-bit wires.
// lint-config: off:all warn:implicit-net
module leaf (input wire i, output wire o);
    assign o = i;
endmodule

module top (input wire a, output wire y);
    leaf u0 (.i(a), .o(mid));     // `mid` is never declared
    assign stray = a;             // so is `stray`
    assign y = a;
endmodule
