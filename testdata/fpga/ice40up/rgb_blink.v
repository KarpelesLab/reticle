// The RGB LED of an iCE40UP5K-SG48 board stepping through red, green and
// blue, one colour a second, from the 12 MHz oscillator on pin 20.
//
// The three LED pins (39, 40, 41) are the part's constant-current sinks,
// reached only through SB_RGBA_DRV: its RGB0..RGB2 outputs *are* those pads,
// so they get no IO buffer. The current is the smallest the driver has
// (2 mA in half-current mode), which is plenty for an indicator.
//
// The colour counter is two bits holding three values; `phase` is declared
// that wide on purpose (CLAUDE.md, "never declare a register wider than the
// values it holds").
module rgb_blink (
    input  wire clk,
    output wire led_r,
    output wire led_g,
    output wire led_b
);
    reg [23:0] tick = 24'd0;
    reg [1:0] phase = 2'd0;
    always @(posedge clk) begin
        if (tick == 24'd11_999_999) begin
            tick <= 24'd0;
            phase <= (phase == 2'd2) ? 2'd0 : phase + 2'd1;
        end else begin
            tick <= tick + 24'd1;
        end
    end

    SB_RGBA_DRV #(
        .CURRENT_MODE("0b1"),
        .RGB0_CURRENT("0b000001"),
        .RGB1_CURRENT("0b000001"),
        .RGB2_CURRENT("0b000001")
    ) rgb (
        .CURREN(1'b1),
        .RGBLEDEN(1'b1),
        .RGB0PWM(phase == 2'd0),
        .RGB1PWM(phase == 2'd1),
        .RGB2PWM(phase == 2'd2),
        .RGB0(led_r),
        .RGB1(led_g),
        .RGB2(led_b)
    );
endmodule
