// apple2_tb — types at the machine and watches its screen.
//
// The testbench drives `apple2` from a bare `video_timing` at **one pixel
// per clock**, where `apple2_top` drives it from `dvi_tx` at one pixel in
// five. It is the same raster block with the same MODE, and everything in
// the machine that moves with the picture moves on `pix_en`, so the
// frames are identical; what is left out is the TMDS encoders and the
// 10:1 serialisers, which are `dvi_tx`'s and are tested in
// `tests/ip_library.rs` against the DVI specification. Leaving them out is
// worth four fifths of the run time, and it is the difference between a
// test that looks at a whole frame and one that cannot afford to.
//
// The keyboard is the serial port, so the testbench types by driving
// `uart_rx`, and the monitor echoes everything it prints to `uart_tx`, so
// the testbench can read the session back. Both the transmitter and the
// receiver here are written from the 8N1 frame rather than taken from the
// library, so they check `uart` instead of agreeing with it.
//
// The typing is handshaked rather than timed: `sendchar` waits for the
// monitor to echo each character and `command` waits for the next `]`
// prompt, which is what stops the testbench typing faster than a 6502
// reads and makes the run the same length however the timing parameters
// are set.
//
// The raster is held in reset until the monitor has finished drawing, so
// the frame `tests/apple2.rs` decodes back into 40 columns of 24 rows is
// frame 0 and the run spends nothing looking for a boundary. A second,
// shorter frame follows, far enough down the screen to show the flashing
// words in their other state.
`timescale 1ns / 1ns

module apple2_tb;
    // One pixel per clock. 20 ns is 25 MHz, near enough the mode's
    // 25.175 MHz pixel clock, and it keeps the arithmetic whole.
    localparam HALF        = 20;
    // Clocks per serial bit. Nothing here is a real terminal, so the
    // rate is chosen to keep the run short: at 16 clocks a bit, a
    // character costs 160 pixels rather than a quarter of a frame.
    localparam CLK_DIV     = 16;
    // Which bit of the frame counter drives the flashing attribute. The
    // machine's default is 3, which is eight frames in each state; 0
    // makes a flashing cell change every frame, so the testbench can see
    // both states one frame apart instead of eight and can afford to.
    localparam FLASH_SHIFT = 0;
    localparam BIT_TIME    = CLK_DIV * 2 * HALF;
    // The last line of the picture and the line after the row the
    // flashing words are on, which is as far as the second frame has to
    // be drawn.
    localparam PIC_END     = 432;
    localparam FLASH_END   = 80;
    // Long enough for the whole session with room to spare; reaching it
    // means something has stopped.
    localparam TIMEOUT     = 4000000 * HALF;

    reg clk = 1'b0;
    always #HALF clk = ~clk;

    reg rst_n = 1'b0;
    // The raster has a reset of its own, and it is held while the machine
    // boots. Nothing is being displayed that anyone wants to see until
    // the monitor has finished drawing, and starting the raster
    // afterwards means the frame the test decodes is the first one rather
    // than one found by waiting for a boundary — which is a third of the
    // run. The machine cannot tell: `apple2` is given `x`, `y` and `de`,
    // and while they are held it simply draws the same border pixel.
    reg vid_rst_n = 1'b0;

    // -----------------------------------------------------------------
    // The raster: `dvi_tx`'s, without the serialisers after it.
    // -----------------------------------------------------------------
    wire        de;
    wire        frame;
    wire [11:0] x;
    wire [11:0] y;

    video_timing #(.MODE(0)) u_timing (
        .clk   (clk),
        .rst_n (vid_rst_n),
        .en    (1'b1),
        .de    (de),
        .hsync (),
        .vsync (),
        .frame (frame),
        .x     (x),
        .y     (y)
    );

    // -----------------------------------------------------------------
    // The machine
    // -----------------------------------------------------------------
    wire [7:0] r;
    wire [7:0] g;
    wire [7:0] b;
    wire       uart_tx;
    reg        uart_rx = 1'b1;
    wire       speaker;

    apple2 #(
        .ROM_FILE    ("sw/monitor.hex"),
        .FONT_FILE   ("sw/font.hex"),
        .CLK_DIV     (CLK_DIV),
        .FLASH_SHIFT (FLASH_SHIFT)
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

    // The video signal, as one word, which is what the test samples.
    wire [23:0] rgb = {r, g, b};

    // -----------------------------------------------------------------
    // The terminal: a receiver, so the session can be read, and a
    // transmitter, so it can be typed.
    // -----------------------------------------------------------------
    reg [7:0]      rx_byte;
    reg [8*48-1:0] line;
    integer        rx_count;
    reg            prompt;
    // Whether the next byte received starts a line. The prompt is only a
    // prompt when it does: the machine's own title is
    // `RETICLE APPLE ][ COMPATIBLE 6502`, and a `]` in the middle of that
    // is not an invitation to type.
    reg            line_start;
    integer        i;

    initial begin
        line       = 0;
        rx_count   = 0;
        prompt     = 1'b0;
        line_start = 1'b1;
        forever begin
            @(negedge uart_tx);
            // Half a bit into the start bit, which must still be low.
            #(BIT_TIME / 2);
            if (uart_tx !== 1'b0) begin
                $display("apple2_tb: glitch on uart_tx at %0t", $time);
            end else begin
                for (i = 0; i < 8; i = i + 1) begin
                    #(BIT_TIME);
                    rx_byte[i] = uart_tx;
                end
                #(BIT_TIME);
                if (uart_tx !== 1'b1) begin
                    $display("apple2_tb: framing error at %0t", $time);
                    $finish;
                end
                rx_count = rx_count + 1;
                if (rx_byte == 8'h0a) begin
                    $display("%s", line);
                    line       = 0;
                    line_start = 1'b1;
                end else if (rx_byte != 8'h0d) begin
                    if (line_start & (rx_byte == 8'h5d)) prompt = 1'b1;
                    line       = {line[8*47-1:0], rx_byte};
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

    // A whole command line: the text in `cmdbuf`, then the return, then
    // wait for the prompt the monitor prints when it has finished obeying
    // it. The text goes through a register rather than an argument
    // because a string literal is not a vector until something stores it.
    reg [8*32-1:0] cmdbuf;

    task command(input integer n);
        integer        k;
        reg [8*32-1:0] rest;
        begin
            for (k = n; k > 0; k = k - 1) begin
                rest = cmdbuf >> (8 * (k - 1));
                sendchar(rest[7:0]);
            end
            prompt = 1'b0;
            sendbyte(8'h0d);
            wait (prompt);
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

        // Eight bytes of the ROM: the first instructions of this very
        // monitor, which the test knows from the assembled source.
        cmdbuf = "E F800";
        command(6);
        // Eight bytes into RAM, and then the same eight back out.
        cmdbuf = "D 0300 11 22 33 44 55 66 77 88";
        command(30);
        cmdbuf = "E 0300";
        command(6);
        // Lower case, to show that the monitor folds it: the echo, and
        // the screen, say `T`.
        cmdbuf = "t";
        command(1);

        // Nothing writes to the screen from here, so let the raster go.
        // Frame 0 is drawn in full; frame 1 is drawn as far as the row
        // the flashing words are on, which is all the second phase of the
        // flashing attribute needs.
        vid_rst_n = 1'b1;
        wait (y == PIC_END);
        $display("apple2_tb: frame 0 recorded at %0t", $time);
        wait (y == 12'd0);
        wait (y == FLASH_END);
        $display("apple2_tb: frame 1 recorded to line %0d at %0t", FLASH_END, $time);
        $finish;
    end

    initial begin
        #(TIMEOUT);
        $display("apple2_tb: timed out after %0d byte(s) received", rx_count);
        $finish;
    end
endmodule
