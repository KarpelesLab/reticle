// The UltraPlus's three open-drain pads as plain IO (SB_IO_OD, Lattice's
// iCE40 LED Driver Usage Guide, Appendix B), on an iCE40UP5K-SG48 board
// whose RGB LED sinks into pins 39 (red), 40 (green) and 41 (blue), with
// the 12 MHz oscillator on pin 20.
//
// The LED driver is not used, so nothing limits the current but the pad:
// each pad pulls its LED low for one clock in 64 (a 1.3 us pulse at 190 kHz,
// about 1.6 % of the time), which bounds what the LED sees, and the colour
// steps red, green, blue, about 1.4 s each. A pad not pulling is released,
// not driven high. Nothing but the eye checks it.
module io_od_blink (
    input  wire clk,
    inout  wire led_r,
    inout  wire led_g,
    inout  wire led_b
);
    reg [25:0] count = 26'd0;
    always @(posedge clk) count <= count + 26'd1;
    wire pulse = count[5:0] == 6'd0;
    wire [1:0] colour = count[25:24];
    // A pad pulls low when its DOUT0 is zero, and is released when one.
    wire r_n = ~(pulse && colour == 2'd0);
    wire g_n = ~(pulse && colour == 2'd1);
    wire b_n = ~(pulse && colour == 2'd2);
    SB_IO_OD #(.PIN_TYPE(6'b011001)) r_pad (.PACKAGEPIN(led_r), .DOUT0(r_n));
    SB_IO_OD #(.PIN_TYPE(6'b011001)) g_pad (.PACKAGEPIN(led_g), .DOUT0(g_n));
    SB_IO_OD #(.PIN_TYPE(6'b011001)) b_pad (.PACKAGEPIN(led_b), .DOUT0(b_n));
endmodule
