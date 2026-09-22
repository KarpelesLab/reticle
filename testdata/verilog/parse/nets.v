// Net declarations: every net type, strengths, delays, vectored/scalared,
// arrays, implicit continuous assignments and signed vectors.
module nets;
    wire w;
    wire [7:0] bus, bus2;
    wire signed [15:0] sw;
    wire unsigned [3:0] uw;
    tri t;
    tri0 t0;
    tri1 t1;
    triand ta;
    trior to;
    trireg (large) cap;
    trireg (small) [3:0] caps;
    wand wa;
    wor wo;
    supply0 gnd;
    supply1 vcc;
    uwire u;
    wire (strong0, weak1) [1:0] drv = 2'b01;
    wire (pull1, supply0) p = 1'b1;
    wire #1 delayed = w;
    wire #(1, 2) rf = w;
    wire #(1:2:3, 2:3:4) mtm = w;
    wire [7:0] #(1, 2, 3) turnoff;
    wire vectored [7:0] vbus;
    wire scalared [7:0] sbus;
    wire [7:0] arr [0:3];
    wire [7:0] arr2 [0:3][0:1];
    wire a = 1'b0, b = a, c;
    wire [3:0] x = 4'd1, y = x + 1;
    wire \escaped-name = w;
    wire [7:0] \bus[0] ;
    assign bus2 = \bus[0] ;
endmodule
