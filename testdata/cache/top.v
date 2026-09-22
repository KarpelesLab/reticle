// The root of the hierarchy: `top` -> `mid` -> `leaf`, and `top` -> `other`.
module top (
    input  wire a,
    output wire y,
    output wire z
);
    mid u0 (
        .a(a),
        .y(y)
    );
    other u1 (
        .a(a),
        .y(z)
    );
endmodule
