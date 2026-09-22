`define WIDTH 8
`define MAX(a, b) ((a) > (b) ? (a) : (b))
`define REG(name, w = `WIDTH) logic [w-1:0] name
`define CAT(a, b) a``b
`define STR(x) `"x`"
`define QUOTED(x) `"say `\`"x`\`"`"
`define EMPTY
`define MULTI(x) \
    x``_a, \
    x``_b
`define ALWAYS_FF(clk) always_ff @(posedge clk) // trailing comment is dropped

module macros;
    `REG(a);
    `REG(b, 16);
    `REG(c, );
    logic [`WIDTH-1:0] d;
    logic [`MAX(`WIDTH, 4)-1:0] e;
    logic [`WIDTH'd7:0] f;
    initial $display(`STR(hello world), `QUOTED(hi));
    wire `MULTI(x);
    `EMPTY wire g;
    `ALWAYS_FF(clk) a <= b;
    `MAX (1, {2, 3});
    wire `CAT(pre, fix);
    // The formal name inside an ordinary string is not substituted.
    `define MSG(x) $display("x", x)
    `MSG(1);
    `undef WIDTH
    `ifdef WIDTH bad `else logic [7:0] h; `endif
endmodule
