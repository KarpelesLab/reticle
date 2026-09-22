// Signals no process assigns.
// lint-config: off:all warn:undriven-signal warn:undriven-output
module undriven (
    input  wire a,
    output wire y,
    output wire z    // never driven
);
    wire mid;        // never driven
    supply0 gnd;     // a supply drives itself

    assign y = a & mid & ~gnd;
endmodule
