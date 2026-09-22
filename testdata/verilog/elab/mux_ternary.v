// Multiplexers written with ?: and with concatenation and replication.
module mux_expr(
    input  wire       s,
    input  wire [3:0] a, b,
    output wire [3:0] y,
    output wire [7:0] wide,
    output wire [3:0] rep
);
    assign y    = s ? a : b;
    assign wide = {a, b};
    assign rep  = {4{s}};
endmodule
