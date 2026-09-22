// ANSI port lists in every form: inherited direction and type,
// attributes, net and variable kinds, defaults, unpacked dimensions,
// signed, interface and user-typed ports.
typedef logic [7:0] byte_t;

module ansi #(parameter N = 4) (
    input a, b,
    input wire c,
    input wire [3:0] d, e,
    input logic signed [7:0] s,
    output reg [N-1:0] r,
    output logic [N-1:0] l = '0,
    output var logic v,
    (* mark_debug = "true" *) output tri t,
    inout wire [1:0] io,
    input byte_t bytes [0:3],
    input int unsigned u,
    ref logic rf,
    input bus_if.master m,
    input pkg::word_t w,
    input logic [1:0][3:0] packed2d,
    input logic arr [2][3]
);
endmodule

module lonely (input clk);
endmodule

module typed_first (byte_t x, output y);
endmodule
