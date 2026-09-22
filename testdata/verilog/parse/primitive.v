// User-defined primitives: a combinational and a sequential one; the
// table is kept as raw rows.
primitive mux2(out, sel, a, b);
    output out;
    input sel, a, b;
    table
        // sel a b : out
        0 0 ? : 0;
        0 1 ? : 1;
        1 ? 0 : 0;
        1 ? 1 : 1;
        ? 0 0 : 0;
        ? 1 1 : 1;
    endtable
endprimitive

primitive dff_udp(q, clk, d);
    output q;
    reg q;
    input clk, d;
    initial q = 1'b0;
    table
        // clk d : q : q+
        (01) 0 : ? : 0;
        (01) 1 : ? : 1;
        (0?) 1 : 1 : 1;
        (?0) ? : ? : -;
        ?    (??) : ? : -;
    endtable
endprimitive

module uses_udp(input clk, d, sel, a, b, output q, y);
    dff_udp ff (q, clk, d);
    mux2 m (y, sel, a, b);
endmodule
