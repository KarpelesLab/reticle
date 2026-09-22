module errors;
    // Bad numbers: keep lexing after each.
    wire a = 4'b102;
    wire b = 8'h;
    wire c = 3'd1x;
    wire d = 8'o9;
    // Unterminated string ends at the newline.
    wire e = "open
    wire f = "bad \q escape";
    wire g = "octal too big \777";
    wire h = "hex needs digits \x";
    // Stray characters become error tokens.
    wire i = a § b;
    wire j = \ ;
    // Macros.
    wire k = `UNDEFINED;
    `MACRO_WITH_ARGS(1)
    `define F(a, b) a + b
    wire l = `F(1);
    wire m = `F(1, 2, 3);
    wire n = `F;
    `define R `R
    wire o = `R;
    `define S `T
    `define T `S
    wire p = `S;
    `endif
    `else
    `include "missing.vh"
    `include nope
    `define
    `define 9x
    `ifdef
    `endif
    `ifdef X `else `else `endif
    `ifndef X `elsif Y `else `elsif Z `endif
    wire q = "unterminated string, then an unterminated conditional
    `ifdef ALSO_UNTERMINATED
    wire skipped;
