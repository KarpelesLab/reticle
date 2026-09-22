// Blocking and non-blocking assignments in the wrong kind of block.
// lint-config: off:all warn:blocking-in-sequential warn:nonblocking-in-comb warn:mixed-assignment-styles
module styles (
    input  logic clk,
    input  logic a,
    input  logic b,
    output logic q,
    output logic y,
    output logic z
);
    always_ff @(posedge clk) q = a;      // `=` in an edge-triggered block

    always_comb y <= a & b;              // `<=` in a combinational block

    always_ff @(posedge clk) z <= a;     // `z` is written both ways
    always_comb                z  = b;
endmodule
