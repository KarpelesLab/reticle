// A warm boot between the images of a multi-image flash, for an iCE40
// board with an FT2232H (115200 baud, 8N1, the 12 MHz oscillator on pin 20,
// pin 10 out): `IMAGE` says which image this is and `NEXT` which one it
// boots.
//
// It prints `WB<IMAGE> <n>` eight times, n from 0 to 7, about three times
// a second, and then raises SB_WARMBOOT's BOOT with S1 S0 = NEXT, which
// makes the part load the image the flash's header number NEXT + 1 points
// at (`fpga::icestorm::image::multiboot`). Built twice, `IMAGE 0 NEXT 1`
// and `IMAGE 1 NEXT 0`, and laid out as images 0 and 1 of one flash, the
// line says `WB0 0` .. `WB0 7`, `WB1 0` .. `WB1 7`, `WB0 0`, ... for as long
// as the board runs: each line a configuration the other image asked for.
module warmboot_demo #(
    parameter [1:0] IMAGE = 2'd0,
    parameter [1:0] NEXT = 2'd1
) (
    input  wire clk,
    output wire tx
);
    reg [21:0] tick = 22'd0;
    reg [3:0]  lines = 4'd0;
    reg        boot = 1'b0;
    reg        go = 1'b0;
    always @(posedge clk) begin
        tick <= tick + 22'd1;
        go <= 1'b0;
        if (&tick) begin
            if (lines[3]) boot <= 1'b1;
            else begin
                go <= 1'b1;
                lines <= lines + 4'd1;
            end
        end
    end
    SB_WARMBOOT wb (.BOOT(boot), .S1(NEXT[1]), .S0(NEXT[0]));

    // "WB<i> <n>\r\n": seven characters.
    reg [2:0] ch = 3'd0;
    reg       sending = 1'b0;
    reg [7:0] char;
    always @* begin
        case (ch)
            3'd0: char = "W";
            3'd1: char = "B";
            3'd2: char = 8'h30 + {6'd0, IMAGE};
            3'd3: char = " ";
            3'd4: char = 8'h30 + {5'd0, lines[2:0] - 3'd1};
            3'd5: char = 8'h0D;
            default: char = 8'h0A;
        endcase
    end
    reg [9:0] out_n = 10'd0;
    reg [3:0] bits = 4'd0;
    reg [6:0] div = 7'd0;
    always @(posedge clk) begin
        if (go) begin
            sending <= 1'b1;
            ch <= 3'd0;
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
            if (ch == 3'd6) sending <= 1'b0;
            else ch <= ch + 3'd1;
        end
    end
    assign tx = ~out_n[0];
endmodule
