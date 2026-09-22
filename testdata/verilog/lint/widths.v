// Widths that do not match, where the source says what they are.
// lint-config: off:all warn:width-mismatch warn:unsized-literal-in-concat
module widths (
    input  wire [3:0] b,
    input  wire [1:0] c,
    output wire [7:0] wide,
    output wire [3:0] narrow,
    output wire [5:0] exact
);
    reg [7:0] seed = 9'h1ff;      // the literal needs nine bits

    assign wide   = {b, c};       // six bits into eight
    assign exact  = {b, c};       // six into six: fine
    assign narrow = {b, 1};       // an unsized literal is 32 bits here
endmodule
