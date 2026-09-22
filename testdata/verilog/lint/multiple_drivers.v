// One net driven from two places, and the cases that are fine.
// lint-config: off:all warn:multiple-drivers
module multi (
    input  wire       a,
    input  wire       b,
    output wire       y,
    output wire [3:0] bus,
    output wor        shared
);
    assign y = a;
    assign y = b;                 // second driver of the same bit

    assign bus[1:0] = {2{a}};     // disjoint constant slices are fine
    assign bus[3:2] = {2{b}};

    assign shared = a;            // a `wor` resolves its drivers
    assign shared = b;
endmodule
