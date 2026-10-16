// An indexed part-select wider than the vector it reads: NewAE's `esad`
// reads a byte of a five-bit register this way (`triggerer_init[reg_bytecnt*8
// +: 8]`) when built small. The bits past the top read `x`; the vector is
// padded with them before the select, because the IR's selection must fit
// its operand, and an unpadded one was an internal error.
module part_select_wide(
    input  wire [4:0] r,
    input  wire [1:0] i,
    output wire [7:0] y
);
    assign y = r[i*8 +: 8];
endmodule
