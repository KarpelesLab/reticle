// Conditional drivers become tri-state cells.
module tristate(
    input  wire a,
    input  wire en,
    output wire y0,
    output wire y1,
    output wire y2,
    output wire y3
);
    bufif1 t0 (y0, a, en);
    bufif0 t1 (y1, a, en);
    notif1 t2 (y2, a, en);
    notif0 t3 (y3, a, en);
endmodule
