// Expressions long enough to need breaking, and a port list too wide for
// one line.
module long_exprs (
    input wire [31:0] a, b, c, d, e, f,
    input wire [31:0] g, h,
    output wire [31:0] y,
    output wire [31:0] w,
    output wire z
);
    assign y = a + b * c - d / e + f * a + b * c - d / e + f + a + b + c + d + e + f + a + b + g;
    assign z = (a > b) && (c < d) || (e == f) && (a != b) || (c >= d) && (e <= f) || (a === b);
    assign w = a ? b : c ? d : e;

    wire [31:0] mixed = ((a + b) * (c - d)) >> 2 | (e & ~f) ^ (g[15:0] * h[15:0]) + {a[3:0], 4'h5};

    adder #(.WIDTH(32), .REGISTERED(1), .PIPELINE_STAGES(3)) u_adder (
        .clk(a[0]), .rst(b[0]), .lhs(c), .rhs(d), .sum(w), .carry(z)
    );
endmodule
