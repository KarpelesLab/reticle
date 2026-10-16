// Conditional drivers become tri-state cells.
//
// Two spellings reach the same cell: the four `bufif`/`notif` gates, and a
// continuous assignment whose alternative is all `z`. The second is the one
// a bus is usually written with, and it is what lets an `inout` port become
// a bidirectional pad — `fpga::primitives` looks for this cell on the port's
// net. Both ways round are taken, because a ULPI data bus is normally
// written "release while `dir`" rather than "drive while `oe`".
//
// The `z` may sit anywhere in a tree of choices (`io_tree`), which is how
// NewAE's Husky writes every pin it shares between functions: the enable is
// the tree with each `z` leaf a zero and every other leaf a one, the data
// the tree with each `z` leaf replaced by the other side of its choice. And
// the target may be one bit of a net (`io_bits[i]` from a `generate` loop):
// the cell then drives a net of its own, assigned into that bit.
//
// An alternative that is not all `z` is **not** this: `x` is a value and not
// a released net, and a partial `z` has no IR form, so both stay ordinary
// assignments.
module tristate(
    input  wire a,
    input  wire en,
    input  wire [7:0] d,
    output wire y0,
    output wire y1,
    output wire y2,
    output wire y3,
    inout  wire [7:0] bus0,
    inout  wire [7:0] bus1,
    inout  wire [7:0] bus2,
    output wire [7:0] not_a_bus,
    output wire [7:0] partly,
    input  wire b,
    input  wire c,
    input  wire e,
    inout  wire io_tree,
    inout  wire [1:0] io_bits
);
    bufif1 t0 (y0, a, en);
    bufif0 t1 (y1, a, en);
    notif1 t2 (y2, a, en);
    notif0 t3 (y3, a, en);

    // Drive while `en`, two ways of writing the same released value.
    assign bus0 = en ? d : 8'bz;
    assign bus1 = en ? d : {8{1'bz}};

    // Release while `en`, so the enable is the inverse.
    assign bus2 = en ? 8'bz : d;

    // Released while `a`; otherwise `e` under `c` when `b`, else `d[0]`.
    assign io_tree = a ? 1'bz : b ? (c ? e : 1'bz) : d[0];

    // One driver per bit, each with its own enable.
    genvar i;
    for (i = 0; i < 2; i = i + 1) begin : g
        assign io_bits[i] = (i == 0) ? (b ? c : 1'bz) : (c ? 1'bz : e);
    end

    // Neither of these is a tri-state driver.
    assign not_a_bus = en ? d : 8'bx;
    assign partly = en ? d : {7'bz, 1'b0};
endmodule
