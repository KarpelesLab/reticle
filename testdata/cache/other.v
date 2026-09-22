// Instantiated by `top` but sharing nothing with `mid` or `leaf`, so a
// change under `mid` must leave this module's cache entry untouched.
module other (
    input  wire a,
    output wire y
);
    assign y = a;
endmodule
