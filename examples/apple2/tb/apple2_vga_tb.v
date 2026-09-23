// apple2_vga_tb — the machine behind `vga_out`, watched at the pins.
//
// `tb/apple2_tb.v` next to this one proves the whole session and reads
// the picture off the *colour bus* the video generator hands to
// `dvi_tx`. This one proves the last step of the other path: that the
// picture reaches the five pins of a VGA socket. So the machine is wired
// to `vga_out` — four bits a channel, as a Basys 3's resistor ladder
// takes them — and `tests/apple2.rs` decodes `vga_rgb`, which is those
// twelve pins and nothing inside the design.
//
// It is deliberately the cheaper of the two. The long session belongs to
// the other testbench and is not typed again here; this one boots,
// types `T`, which paints every glyph of the character generator over
// rows 3 to 23 shifted seven per row, and draws one frame. That still
// gives 24 rows to decode, of which 21 are all different from each
// other, so nothing about the interleaved line order or the character
// generator can be wrong and still pass.
//
// Everything else follows `apple2_tb`: one pixel per clock rather than
// the one in five `apple2_top` runs at and the one in four
// `apple2_basys3` runs at — `apple2` moves on `pix_en`, so the frames
// are the same — and the raster is held in reset until the monitor has
// finished drawing, so the frame the test decodes is frame 0 and the run
// spends nothing looking for a boundary.
//
// The keyboard is the serial port, so the testbench types by driving
// `uart_rx` and waits for the monitor to echo on `uart_tx` before
// typing again, which is what stops it typing faster than a 6502 reads.
`timescale 1ns / 1ns

module apple2_vga_tb;
    // One pixel per clock. 20 ns is 25 MHz, which is the pixel rate a
    // Basys 3 makes by dividing its 100 MHz oscillator by four.
    localparam HALF     = 20;
    // Clocks per serial bit. Nothing here is a real terminal, so the
    // rate is chosen to keep the run short.
    localparam CLK_DIV  = 16;
    localparam BIT_TIME = CLK_DIV * 2 * HALF;
    // The line after the last one of the picture.
    localparam PIC_END  = 432;
    // The last pixel and the last line of the 640 x 480 raster, which is
    // as far as the frame has to be drawn: the two lines of vertical
    // sync are at 490 and 491, below the picture.
    localparam H_LAST   = 799;
    localparam V_LAST   = 524;
    // Long enough for the whole run with room to spare; reaching it
    // means something has stopped.
    localparam TIMEOUT  = 4000000 * HALF;

    reg clk = 1'b0;
    always #HALF clk = ~clk;

    reg rst_n = 1'b0;
    // The raster's own reset, held while the machine boots and draws.
    reg vid_rst_n = 1'b0;

    // -----------------------------------------------------------------
    // The video output, pins and all
    // -----------------------------------------------------------------
    wire        de;
    wire [11:0] x;
    wire [11:0] y;
    wire [7:0]  r;
    wire [7:0]  g;
    wire [7:0]  b;
    wire [3:0]  vga_r;
    wire [3:0]  vga_g;
    wire [3:0]  vga_b;
    wire        vga_hsync;
    wire        vga_vsync;

    vga_out #(
        .MODE      (0),
        .BPC       (4)
    ) u_vga (
        .clk       (clk),
        .rst_n     (vid_rst_n),
        .pix_en    (1'b1),
        .de        (de),
        .frame     (),
        .x         (x),
        .y         (y),
        .r         (r),
        .g         (g),
        .b         (b),
        .vga_r     (vga_r),
        .vga_g     (vga_g),
        .vga_b     (vga_b),
        .vga_hsync (vga_hsync),
        .vga_vsync (vga_vsync)
    );

    // The twelve colour pins as one word, which is what the test
    // samples. Nothing inside the design is watched.
    wire [11:0] vga_rgb = {vga_r, vga_g, vga_b};

    // -----------------------------------------------------------------
    // The machine
    // -----------------------------------------------------------------
    wire       uart_tx;
    reg        uart_rx = 1'b1;
    wire       speaker;

    apple2 #(
        .ROM_FILE  ("sw/monitor.hex"),
        .FONT_FILE ("sw/font.hex"),
        .CLK_DIV   (CLK_DIV)
    ) dut (
        .clk     (clk),
        .rst_n   (rst_n),
        .pix_en  (1'b1),
        .de      (de),
        .x       (x),
        .y       (y),
        .r       (r),
        .g       (g),
        .b       (b),
        .uart_tx (uart_tx),
        .uart_rx (uart_rx),
        .speaker (speaker)
    );

    // -----------------------------------------------------------------
    // The terminal: enough of a receiver to know when the monitor has
    // echoed a character and when it has printed a prompt, which is all
    // the handshaking needs. `apple2_tb` reads the whole session back;
    // this one does not, because that is proved there.
    // -----------------------------------------------------------------
    reg [7:0] rx_byte;
    integer   rx_count;
    reg       prompt;
    // Whether the next byte received starts a line. A `]` is only a
    // prompt when it does: the machine's own title has one in it.
    reg       line_start;
    integer   i;

    initial begin
        rx_count   = 0;
        prompt     = 1'b0;
        line_start = 1'b1;
        forever begin
            @(negedge uart_tx);
            // Half a bit into the start bit, which must still be low.
            #(BIT_TIME / 2);
            if (uart_tx !== 1'b0) begin
                $display("apple2_vga_tb: glitch on uart_tx at %0t", $time);
            end else begin
                for (i = 0; i < 8; i = i + 1) begin
                    #(BIT_TIME);
                    rx_byte[i] = uart_tx;
                end
                #(BIT_TIME);
                if (uart_tx !== 1'b1) begin
                    $display("apple2_vga_tb: framing error at %0t", $time);
                    $finish;
                end
                rx_count = rx_count + 1;
                if (rx_byte == 8'h0a) begin
                    line_start = 1'b1;
                end else if (rx_byte != 8'h0d) begin
                    if (line_start & (rx_byte == 8'h5d)) prompt = 1'b1;
                    line_start = 1'b0;
                end
            end
        end
    end

    task sendbyte(input [7:0] value);
        integer k;
        begin
            uart_rx = 1'b0;                     // start bit
            #(BIT_TIME);
            for (k = 0; k < 8; k = k + 1) begin
                uart_rx = value[k];
                #(BIT_TIME);
            end
            uart_rx = 1'b1;                     // stop bit
            #(BIT_TIME);
        end
    endtask

    // One character, then wait for the monitor to echo it.
    task sendchar(input [7:0] value);
        integer mark;
        begin
            mark = rx_count;
            sendbyte(value);
            wait (rx_count != mark);
        end
    endtask

    // -----------------------------------------------------------------
    // The session
    // -----------------------------------------------------------------
    initial begin
        repeat (8) @(posedge clk);
        rst_n = 1'b1;

        // The monitor clears the screen, prints its banner and prompts.
        wait (prompt);

        // `T` paints the character test card over rows 3 to 23, which is
        // everything below the banner and the prompt it is typed at.
        sendchar("T");
        prompt = 1'b0;
        sendbyte(8'h0d);
        wait (prompt);

        // Nothing writes to the screen from here, so let the raster go
        // and draw the whole of frame 0 — all 525 lines and not just the
        // 432 the picture ends on, because the two lines of vertical
        // sync are at 490 and the test reads them off the pin. The last
        // few clocks cover the block's one pixel of output latency
        // several times over.
        vid_rst_n = 1'b1;
        wait (y == PIC_END);
        $display("apple2_vga_tb: the picture is drawn at %0t", $time);
        wait (y == V_LAST);
        wait (x == H_LAST);
        repeat (4) @(posedge clk);
        $display("apple2_vga_tb: frame 0 recorded at %0t", $time);
        $finish;
    end

    initial begin
        #(TIMEOUT);
        $display("apple2_vga_tb: timed out after %0d byte(s) received", rx_count);
        $finish;
    end
endmodule
