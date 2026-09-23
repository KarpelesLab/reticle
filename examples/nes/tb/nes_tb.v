// nes_tb — runs the console for two frames and says what came out of
// its video port.
//
// The testbench watches the pins and nothing else: `vid_de` says a pixel
// is on the port, `vid_color` is its palette index, and `vid_frame`
// marks the top left corner of a frame. It folds every visible pixel of
// the *second* frame into a hash and prints it — the first frame is the
// one sw/demo.s spends setting the machine up, and the second is the
// first one drawn from a complete picture.
//
// `the_console_agrees_with_reticle_sim` in tests/nes.rs computes the
// same hash from a frame buffer it works out from the nametable, the
// pattern table and the palette by the documented rules, and the two
// have to agree. The hash is deliberately a poor one — a shift and a
// polynomial — because it is not protecting anything, it is only
// carrying one number across two implementations of the same picture.
//
// One dot per clock here. `nes_console` moves on `en` and on nothing
// else, so a testbench holding `en` high runs it at one dot a cycle,
// where `nes_top` gives it one dot in 24 of a 126 MHz clock.
`timescale 1ns / 1ns

module nes_tb;
    localparam HALF = 5;

    // 341 x 262 dots a frame, and three frames' worth of slack.
    localparam TIMEOUT = 3 * 341 * 262 * 2 * HALF + 1000;

    reg clk   = 1'b0;
    reg rst_n = 1'b0;

    wire        vid_de;
    wire [7:0]  vid_x;
    wire [7:0]  vid_y;
    wire [5:0]  vid_color;
    wire        vid_frame;

    nes_console dut (
        .clk        (clk),
        .rst_n      (rst_n),
        .en         (1'b1),
        .vid_de     (vid_de),
        .vid_x      (vid_x),
        .vid_y      (vid_y),
        .vid_color  (vid_color),
        .vid_frame  (vid_frame),
        .dbg_dot    (),
        .dbg_line   (),
        .dbg_pc     (),
        .dbg_retire (),
        .dbg_dma    ()
    );

    always #HALF clk = ~clk;

    initial begin
        repeat (8) @(posedge clk);
        rst_n <= 1'b1;
    end

    integer    frames;
    integer    pixels;
    reg [31:0] hash;

    always @(posedge clk) begin
        if (!rst_n) begin
            frames <= 0;
            pixels <= 0;
            hash   <= 32'h811c_9dc5;
        end else begin
            if (vid_frame) begin
                if (frames == 2) begin
                    $display("nes_tb: frame 2: %0d pixels, hash %h", pixels, hash);
                    $finish;
                end
                frames <= frames + 1;
            end
            if ((frames == 2) && vid_de) begin
                pixels <= pixels + 1;
                hash   <= {hash[30:0], 1'b0}
                        ^ (hash[31] ? 32'h04c1_1db7 : 32'h0000_0000)
                        ^ {26'd0, vid_color};
            end
        end
    end

    initial begin
        #(TIMEOUT);
        $display("nes_tb: timed out after %0d pixels of frame %0d", pixels, frames);
        $finish;
    end
endmodule
