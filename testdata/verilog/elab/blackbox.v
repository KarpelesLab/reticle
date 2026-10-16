// An instance of a module the source does not declare becomes a black
// box with a warning, keeping every parameter it was given — a string as
// its text — for the tool that does know the module.
module blackbox_user(
    input  wire       clk,
    input  wire [7:0] d,
    output wire [7:0] q
);
    vendor_ram #(.WIDTH(8), .MODE("FAST")) u_ram (
        .clk (clk),
        .d   (d),
        .q   (q)
    );
endmodule
