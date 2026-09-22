// Gate and switch instantiations with names, strengths, delays and
// instance arrays.
module gates(input a, b, c, en, output y0, y1, y2, y3, y4, y5, y6, y7, inout p, q);
    wire [3:0] va, vb, vy;
    and g0 (y0, a, b);
    nand (y1, a, b, c);
    or #2 g2 (y2, a, b);
    nor #(1, 2) g3 (y3, a, b);
    xor (strong0, weak1) g4 (y4, a, b);
    xnor (weak1, strong0) #(1:2:3) g5 (y5, a, b);
    buf b0 (y6, a), b1 (y7, b);
    not #1 n0 (y7, a);
    bufif1 t0 (y0, a, en);
    bufif0 t1 (y1, a, en);
    notif1 t2 (y2, a, en);
    notif0 t3 (y3, a, en);
    and ga [3:0] (vy, va, vb);
    nmos m0 (y4, a, en);
    pmos m1 (y5, a, en);
    cmos m2 (y6, a, en, ~en);
    rnmos m3 (y4, a, en);
    rpmos m4 (y5, a, en);
    rcmos m5 (y6, a, en, ~en);
    tran tr0 (p, q);
    rtran tr1 (p, q);
    tranif1 tr2 (p, q, en);
    tranif0 tr3 (p, q, en);
    rtranif1 tr4 (p, q, en);
    rtranif0 tr5 (p, q, en);
    pullup (strong1) pu (p);
    pulldown pd (q);
    pullup pu2 (p), pu3 (q);
endmodule
