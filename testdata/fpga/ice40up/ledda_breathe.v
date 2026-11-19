// The UltraPlus's LED dimmer (SB_LEDDA_IP) driving the RGB LED through the
// RGB driver, breathing: on an iCE40UP5K-SG48 board whose RGB LED is on
// pins 39 (red), 40 (green) and 41 (blue), with the 12 MHz oscillator on
// pin 20.
//
// After configuration a sequencer writes the dimmer's registers once over
// its control bus (one register a clock, LEDDCS and LEDDDEN high with the
// address and data), as Lattice's iCE40 LED Driver Usage Guide (TN1288,
// Appendix D) lays them out, then holds LEDDEXE high:
//
//   LEDDCR0  (1000) 8'hC0  the dimmer enabled, 250 Hz flicker
//   LEDDBR   (1001) 8'd186 12 MHz / 64 kHz - 1
//   LEDDONR  (1010) 8'h20  on for 1.024 s
//   LEDDOFR  (1011) 8'h20  off for 1.024 s
//   LEDDBCRR (0101) 8'hE7  breathe, on both edges, 1.024 s ramps
//   LEDDBCFR (0110) 8'h00  (overridden by LEDDBCRR's edge bit)
//   LEDDPWRR (0001) 8'h60  red
//   LEDDPWRG (0010) 8'h20  green
//   LEDDPWRB (0011) 8'hC0  blue
//
// So the LED fades up to a violet over a second, holds, fades out, and is
// dark for a second, about every three seconds.
//
// And the fabric watches the dimmer: in each window of 2^18 clocks (about
// 22 ms) it counts the clocks the red PWM output is high and prints
//
//   P<count / 16 as 4 hex digits> O<LEDDON>
//
// on pin 10 at 115200 baud, so the breathing is a ramp of numbers on the
// serial line as well as a light.
module ledda_breathe (
    input  wire clk,
    output wire led_r,
    output wire led_g,
    output wire led_b,
    output wire tx
);
    reg [3:0] step = 4'd0;
    reg       cs = 1'b0;
    reg [3:0] addr = 4'd0;
    reg [7:0] data = 8'd0;
    reg       exe = 1'b0;
    always @(posedge clk) begin
        cs <= 1'b0;
        if (step != 4'd10) begin
            step <= step + 4'd1;
            cs <= step != 4'd9;
            case (step)
                4'd0: begin addr <= 4'b1000; data <= 8'hC0; end
                4'd1: begin addr <= 4'b1001; data <= 8'd186; end
                4'd2: begin addr <= 4'b1010; data <= 8'h20; end
                4'd3: begin addr <= 4'b1011; data <= 8'h20; end
                4'd4: begin addr <= 4'b0101; data <= 8'hE7; end
                4'd5: begin addr <= 4'b0110; data <= 8'h00; end
                4'd6: begin addr <= 4'b0001; data <= 8'h60; end
                4'd7: begin addr <= 4'b0010; data <= 8'h20; end
                4'd8: begin addr <= 4'b0011; data <= 8'hC0; end
                default: ;
            endcase
        end else begin
            exe <= 1'b1;
        end
    end

    wire pwm_r, pwm_g, pwm_b, on;
    SB_LEDDA_IP dimmer (
        .LEDDCS(cs), .LEDDCLK(clk), .LEDDDEN(cs), .LEDDEXE(exe),
        .LEDDDAT7(data[7]), .LEDDDAT6(data[6]), .LEDDDAT5(data[5]), .LEDDDAT4(data[4]),
        .LEDDDAT3(data[3]), .LEDDDAT2(data[2]), .LEDDDAT1(data[1]), .LEDDDAT0(data[0]),
        .LEDDADDR3(addr[3]), .LEDDADDR2(addr[2]), .LEDDADDR1(addr[1]), .LEDDADDR0(addr[0]),
        .PWMOUT0(pwm_r), .PWMOUT1(pwm_g), .PWMOUT2(pwm_b), .LEDDON(on)
    );
    SB_RGBA_DRV #(
        .CURRENT_MODE("0b1"),
        .RGB0_CURRENT("0b000011"),
        .RGB1_CURRENT("0b000011"),
        .RGB2_CURRENT("0b000011")
    ) rgb (
        .CURREN(1'b1), .RGBLEDEN(1'b1),
        .RGB0PWM(pwm_r), .RGB1PWM(pwm_g), .RGB2PWM(pwm_b),
        .RGB0(led_r), .RGB1(led_g), .RGB2(led_b)
    );

    // The duty of the red output, per window.
    reg [17:0] window = 18'd0;
    reg [17:0] high = 18'd0;
    reg [13:0] duty = 14'd0;
    reg        on_seen = 1'b0;
    reg        go = 1'b0;
    always @(posedge clk) begin
        window <= window + 18'd1;
        go <= 1'b0;
        if (&window) begin
            duty <= high[17:4];
            on_seen <= on;
            high <= 18'd0;
            go <= 1'b1;
        end else if (pwm_r) begin
            high <= high + 18'd1;
        end
    end
    function [7:0] hex;
        input [3:0] v;
        hex = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h41 + {4'd0, v} - 8'd10;
    endfunction
    reg [3:0] ch = 4'd0;
    reg       sending = 1'b0;
    reg [7:0] char;
    always @* begin
        case (ch)
            4'd0: char = "P";
            4'd1: char = hex({2'b00, duty[13:12]});
            4'd2: char = hex(duty[11:8]);
            4'd3: char = hex(duty[7:4]);
            4'd4: char = hex(duty[3:0]);
            4'd5: char = " ";
            4'd6: char = "O";
            4'd7: char = 8'h30 + {7'd0, on_seen};
            4'd8: char = 8'h0D;
            default: char = 8'h0A;
        endcase
    end
    reg [9:0] out_n = 10'd0;
    reg [3:0] bits = 4'd0;
    reg [6:0] div = 7'd0;
    always @(posedge clk) begin
        if (go) begin
            sending <= 1'b1;
            ch <= 4'd0;
        end
        if (bits != 4'd0) begin
            div <= div + 7'd1;
            if (div == 7'd103) begin
                div <= 7'd0;
                out_n <= {1'b0, out_n[9:1]};
                bits <= bits - 4'd1;
            end
        end else if (sending) begin
            out_n <= ~{1'b1, char, 1'b0};
            bits <= 4'd10;
            div <= 7'd0;
            if (ch == 4'd9) sending <= 1'b0;
            else ch <= ch + 4'd1;
        end
    end
    assign tx = ~out_n[0];
endmodule
