// An old-style port list, a name a later standard reserves, and a file
// that never turns implicit nets off.
// lint-config: off:all note:non-ansi-ports warn:missing-default-nettype warn:keyword-as-identifier
module adder(a, b, sum);
    input  [3:0] a, b;
    output [4:0] sum;
    wire   [3:0] bit;          // `bit` is a SystemVerilog keyword

    assign bit = a;
    assign sum = bit + b;
endmodule
