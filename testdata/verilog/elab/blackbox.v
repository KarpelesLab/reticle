// An instance of a module the source does not declare becomes a black
// box with a warning.
module blackbox_user(
    input  wire       clk,
    input  wire [7:0] d,
    output wire [7:0] q
);
    vendor_ram #(.WIDTH(8)) u_ram (
        .clk (clk),
        .d   (d),
        .q   (q)
    );
endmodule
