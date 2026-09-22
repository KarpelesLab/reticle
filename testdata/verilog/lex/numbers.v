module numbers;
    // Unsized decimal, with underscores.
    localparam A = 42;
    localparam B = 1_000_000;
    // Sized and based, all bases, both cases, signed, blanks allowed.
    localparam C = 8'hff;
    localparam D = 4'b10x1;
    localparam E = 12'o7_7zz;
    localparam F = 32'd1000;
    localparam G = 8'sh80;
    localparam H = 4'B1?01;
    localparam I = 16 'h dead;
    localparam J = 'hFACE;
    localparam K = 'd10;
    // Unsized single-bit fill.
    localparam L = '0;
    localparam M = '1;
    localparam N = 'x;
    localparam O = 'Z;
    // Reals.
    localparam real P = 1.5;
    localparam real Q = 1e10;
    localparam real R = 2.5E-3;
    localparam real S = 3_000.000_1e+2;
    // Time.
    initial #10ns $display("t");
    initial #1.5us;
    initial #1step;
    initial #1 sec;
    // A number followed by a range operator, not a real.
    wire [7:0] w = x[3:0];
    wire y = 1.e;
endmodule
