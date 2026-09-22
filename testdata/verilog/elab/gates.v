// Gate primitives lowered to continuous assignments.
module gate_shapes(
    input  wire a, b, c,
    output wire y_and, y_or, y_xor, y_nand, y_nor, y_xnor, y_buf, y_not
);
    and  g0 (y_and,  a, b, c);
    or   g1 (y_or,   a, b);
    xor  g2 (y_xor,  a, b);
    nand g3 (y_nand, a, b);
    nor  g4 (y_nor,  a, b);
    xnor g5 (y_xnor, a, b);
    buf  g6 (y_buf,  a);
    not  g7 (y_not,  a);
endmodule
