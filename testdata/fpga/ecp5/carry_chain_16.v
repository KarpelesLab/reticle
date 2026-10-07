// A 16-bit accumulator and nothing else, so a test can take a carry chain
// through place, route and a bitstream on its own.
//
// Why this shape:
//
//   * **one adder, 16 bits wide.** Seventeen lanes — one spent entering
//     the chain, which this fabric has no pin for — so nine `CCU2C` in
//     one chain, which crosses three logic tiles: four slices of a tile
//     and then the tile one column east. A chain that long cannot be
//     placed by accident.
//   * **the sum is registered**, so every one of the chain's sums has to
//     leave its slice and come back in on a flip-flop's `M<z>` input.
//     That exercises the bit `CCU2.INJECT1_<n> = NO` shares with the pip
//     that carries a sum out, in both of its states: the sixteen sums
//     that are routed and the two lanes that are not.
//   * **`step` is an input, not a constant**, so both operands are real
//     wires and every lane is a two-operand add rather than an
//     increment.
//   * nothing else in the design, so nothing else can absorb a cell the
//     test is counting.
//
// The width is sixteen and not thirty-two because every port is on a
// top-edge ball and that edge has 56 of them.
module carry_chain_16 (
    input  wire        clk,
    input  wire        rst_n,
    input  wire [15:0] step,
    output wire [15:0] total
);
    reg [15:0] acc;

    always @(posedge clk) begin
        if (!rst_n) acc <= 16'd0;
        else acc <= acc + step;
    end

    assign total = acc;
endmodule
