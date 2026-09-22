// One level above `leaf`, one level below `top`.
module mid (
    input  wire a,
    output wire y
);
    wire n;
    leaf u0 (
        .a(a),
        .y(n)
    );
    assign y = n;
endmodule
