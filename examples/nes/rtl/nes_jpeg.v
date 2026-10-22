// nes_jpeg — the console's picture as a JPEG file, a byte at a time.
//
// What it does
//   A third top level for the same console. `nes_top` sends the picture
//   to a monitor over DVI and `nes_basys3` over VGA; this one encodes it
//   and hands back every byte of a `.jpg`, from its `SOI` to its `EOI`.
//   Run it once a frame and the bytes are Motion JPEG.
//
//   Four things are between the console and the file:
//
//     * a frame buffer, because the encoder wants blocks and the
//       console draws scan lines;
//     * `nes_luma`, because the encoder is greyscale and the console's
//       picture is a palette index;
//     * an address generator that walks the buffer in block order;
//     * `mjpeg_enc`, from the IP library, which does all the JPEG.
//
//   256 x 240 is exactly 32 x 30 blocks of eight by eight, so the
//   console's picture needs no padding and `mjpeg_enc`'s rule — both
//   dimensions a multiple of eight — is met without doing anything.
//   That is luck rather than design, and it is the reason this example
//   is the one the encoder got wired to first.
//
// Why there is a frame buffer and not eight line buffers
//   `mjpeg_enc` takes its samples in block order: 64 for one block,
//   row-major inside it, then the next block across. A design with a
//   frame buffer has that for the asking, which is what its header
//   means by "a design with a frame buffer has blocks for the asking".
//   This design has one already — `nes_video` needs it to get from the
//   console's 5.25 MHz raster to a monitor's — so the encoder costs no
//   buffering at all here. It would cost eight line buffers on a camera.
//
// One frame, whole: the writes stop, the console does not
//   `arm` asks for a frame. The controller then waits for the start of
//   the next one, lets the console fill the buffer for exactly that
//   frame, and at the following frame boundary **freezes the writes**
//   and starts the encoder. So the file is one frame as the console drew
//   it, with no tearing, which `nes_video` does not promise and does not
//   need to.
//
//   What is frozen is the buffer, not the console: it goes on running
//   and drawing, and its writes are dropped while the encode is in
//   flight. That is what a capture slower than the source does — it
//   drops frames rather than slows the thing it is watching — and it is
//   also why there is no second buffer. Two buffers would be 48 KiB of
//   block RAM to encode every frame instead of one in four or five, and
//   the part has not got it.
//
// The pixel pace, and why two cycles a sample costs nothing
//   The address generator holds the counters still until `mjpeg_enc`
//   takes the sample, moves them on, and spends one cycle waiting for
//   the buffer to answer. So a sample every two cycles at best, 122,880
//   cycles for a frame's 61,440 -- and **the encoder is nowhere near
//   that fast.** `jpeg_fdct` takes 648 cycles a block, measured and
//   pinned by a test of its own, which is 622,080 for this picture's
//   960 blocks: five times the feed. `in_ready` is low for most of the
//   frame and the counters simply wait.
//
//   A sample a cycle would want a skid buffer and a second address in
//   flight, and would buy nothing until the transform overlapped one
//   block's output with the next one's input, which it does not.
//
//   **The memory answers two assignments later, not one.** `fb_q` is
//   loaded from whatever address the counters held during the previous
//   cycle, so the counters must stand still for the cycle before the
//   sample is presented — which is exactly what the J_WAIT state is.
//   Holding them still also makes the sample stable for as long as the
//   encoder takes to accept it, with no holding register anywhere.
//
// What it does not do
//   No transport: the bytes come out of a port here, and getting them
//   off a board wants `usb_cdc_acm` or a UART behind this. No sound, so
//   nothing that could be an AVI. No restart markers, so a lost byte
//   loses the rest of the frame. And the quantisation and Huffman
//   tables come from outside, through `mjpeg_enc`'s own configuration
//   port, passed straight through below: this module adds no table of
//   its own, because `mjpeg_enc` writes the ones it is given into the
//   file and derives the codes from them, and a table written twice is
//   a table that can disagree with itself.
module nes_jpeg #(
    // The cartridge, as `nes_top` takes it.
    parameter PRG_FILE = "sw/demo.hex",
    parameter CHR_FILE = "sw/chr.hex",
    parameter MIRROR   = 1,
    // Cycles of `clk` per dot of the picture unit, as `nes_top` takes
    // it. **One by default, which is not a board's number**: a board
    // would divide its clock down to the 5.25 MHz `nes_top` runs the
    // console at, and set this the way that top does. One is what makes
    // the whole thing simulable — a frame is then 89,342 clocks instead
    // of 2.1 million, which is the same arithmetic that keeps the DVI
    // path out of a whole-frame simulation, and the picture is
    // identical either way because every part of the console is clocked
    // by `en` and none of it can tell how often that comes.
    parameter DOT_DIV  = 1
) (
    input  wire       clk,
    input  wire       rst_n,

    // `mjpeg_enc`'s tables, each in the form the file carries it.
    //   `cfg_sel` 0 — `DQT`'s 64 bytes, zig-zag order
    //   `cfg_sel` 1 — the DC `DHT`: 16 counts, then the symbols
    //   `cfg_sel` 2 — the AC `DHT`: the same
    input  wire [1:0] cfg_sel,
    input  wire [7:0] cfg_addr,
    input  wire [7:0] cfg_data,
    input  wire       cfg_we,

    // Ask for a frame. The next whole one the console draws is the one
    // encoded; raising this again after `done` asks for another.
    input  wire       arm,

    // The file.
    output wire [7:0] out_byte,
    output wire       out_valid,
    input  wire       out_ready,
    output wire       done
);
    // The console's picture, in pixels and in blocks.
    localparam integer SCREEN_W = 256;
    localparam integer SCREEN_H = 240;
    localparam integer COLS     = SCREEN_W / 8;   // 32
    localparam integer ROWS     = SCREEN_H / 8;   // 30
    localparam [4:0]   LAST_COL = COLS - 1;
    localparam [4:0]   LAST_ROW = ROWS - 1;

    // -----------------------------------------------------------------
    // The console, and one dot of it every DOT_DIV cycles.
    // -----------------------------------------------------------------
    reg [4:0] divider;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)                                divider <= 5'd0;
        else if (divider == (DOT_DIV[4:0] - 5'd1)) divider <= 5'd0;
        else                                       divider <= divider + 5'd1;
    end
    wire dot_en = (divider == 5'd0);

    wire       vid_de;
    wire [7:0] vid_x;
    wire [7:0] vid_y;
    wire [5:0] vid_color;
    wire       vid_frame;

    nes_console #(
        .PRG_FILE (PRG_FILE),
        .CHR_FILE (CHR_FILE),
        .MIRROR   (MIRROR)
    ) u_console (
        .clk        (clk),
        .rst_n      (rst_n),
        .en         (dot_en),
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

    // One cycle at the start of each frame: dot 0 of line 0, taken on
    // the `en` that reaches it so that it is one cycle and not DOT_DIV.
    wire frame_tick = dot_en & vid_frame;

    // -----------------------------------------------------------------
    // The controller
    // -----------------------------------------------------------------
    localparam [2:0] J_IDLE  = 3'd0;   // waiting to be asked
    localparam [2:0] J_SYNC  = 3'd1;   // waiting for a frame to begin
    localparam [2:0] J_FILL  = 3'd2;   // the console is drawing that frame
    localparam [2:0] J_WAIT  = 3'd3;   // the buffer is answering
    localparam [2:0] J_PRES  = 3'd4;   // a sample is on `in_pixel`
    localparam [2:0] J_DRAIN = 3'd5;   // every sample is in; waiting for `done`

    reg [2:0] state;
    reg       frozen;
    reg       enc_start;

    // Where the next sample comes from: the block, and the sample
    // within it. `{by, py}` is the row and `{bx, px}` the column, which
    // is why the block counters are five bits and the sample counters
    // three — 29 * 8 + 7 is 239 and 31 * 8 + 7 is 255, and both fit a
    // byte exactly.
    reg [4:0] bx;
    reg [4:0] by;
    reg [2:0] px;
    reg [2:0] py;

    wire [15:0] fb_ra  = {by, py, bx, px};
    wire        s_last = (px == 3'd7) & (py == 3'd7)
                       & (bx == LAST_COL) & (by == LAST_ROW);

    // The frame boundary that ends the capture, and the one thing about
    // it that is easy to get wrong. `frozen` is a register, so it is
    // still low on the edge that sets it -- and that edge is dot 0 of
    // line 0 of the *next* frame, where `vid_de` is already high. A
    // write there would put the next frame's top-left pixel into the
    // buffer just as it is being handed to the encoder, so the file
    // would be one frame plus one pixel of the one after it. The term
    // below drops that write.
    wire        cap_end = (state == J_FILL) & frame_tick;
    wire        fb_we   = dot_en & vid_de & ~frozen & ~cap_end;

    // -----------------------------------------------------------------
    // The frame buffer: one write port, one read port, which is the
    // shape a block RAM has. 64 Ki entries of six bits addressed by the
    // pixel's Y and X, the same as `nes_video`'s — the top 4 Ki rows are
    // not drawn and not read, and leaving the address a flat `{y, x}`
    // costs nothing a multiply would not cost more of.
    //
    // No reset on the read side: a block RAM has nowhere to put one.
    // -----------------------------------------------------------------
    reg [5:0] fb [0:65535];
    reg [5:0] fb_q;

    always @(posedge clk) begin
        if (fb_we) fb[{vid_y, vid_x}] <= vid_color;
        fb_q <= fb[fb_ra];
    end

    wire [7:0] sample;

    nes_luma u_luma (
        .index (fb_q),
        .y     (sample)
    );

    wire enc_ready;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state     <= J_IDLE;
            frozen    <= 1'b0;
            enc_start <= 1'b0;
            bx        <= 5'd0;
            by        <= 5'd0;
            px        <= 3'd0;
            py        <= 3'd0;
        end else begin
            // `start` is a pulse: `mjpeg_enc` clears its predictor and
            // its coefficient position whenever it sees one.
            enc_start <= 1'b0;

            case (state)
                J_IDLE: if (arm) state <= J_SYNC;

                // The frame in flight is part drawn, so it is not the
                // one to encode: wait for the next to begin.
                J_SYNC: if (frame_tick) state <= J_FILL;

                // That frame is now whole in the buffer.
                J_FILL: if (frame_tick) begin
                    frozen    <= 1'b1;
                    enc_start <= 1'b1;
                    bx        <= 5'd0;
                    by        <= 5'd0;
                    px        <= 3'd0;
                    py        <= 3'd0;
                    state     <= J_WAIT;
                end

                // The counters have stood still for a cycle, so `fb_q`
                // is about to be the sample they name.
                J_WAIT: state <= J_PRES;

                J_PRES: if (enc_ready) begin
                    if (s_last) begin
                        state <= J_DRAIN;
                    end else begin
                        state <= J_WAIT;
                        if (px != 3'd7) begin
                            px <= px + 3'd1;
                        end else begin
                            px <= 3'd0;
                            if (py != 3'd7) begin
                                py <= py + 3'd1;
                            end else begin
                                py <= 3'd0;
                                if (bx != LAST_COL) begin
                                    bx <= bx + 5'd1;
                                end else begin
                                    bx <= 5'd0;
                                    by <= by + 5'd1;
                                end
                            end
                        end
                    end
                end

                J_DRAIN: if (done) begin
                    frozen <= 1'b0;
                    state  <= J_IDLE;
                end

                default: state <= J_IDLE;
            endcase
        end
    end

    // -----------------------------------------------------------------
    // The encoder
    // -----------------------------------------------------------------
    mjpeg_enc #(
        .WIDTH  (SCREEN_W),
        .HEIGHT (SCREEN_H)
    ) u_enc (
        .clk       (clk),
        .rst_n     (rst_n),
        .cfg_sel   (cfg_sel),
        .cfg_addr  (cfg_addr),
        .cfg_data  (cfg_data),
        .cfg_we    (cfg_we),
        .start     (enc_start),
        .in_pixel  (sample),
        .in_valid  (state == J_PRES),
        .in_ready  (enc_ready),
        .out_byte  (out_byte),
        .out_valid (out_valid),
        .out_ready (out_ready),
        .done      (done)
    );
endmodule
