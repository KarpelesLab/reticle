// The bottom of the hierarchy. Editing this file must invalidate `mid`
// and `top` as well, and leave `other` alone.
module leaf (
    input  wire a,
    output wire y
);
    assign y = ~a;
endmodule
