// A Cynthion that is a camera with no lens: `ip/video/pattern_gen` into
// `ip/video/mjpeg_enc` into `ip/usb/usb_cdc_acm`, so that what comes out of
// `/dev/ttyACM*` is a **Motion JPEG stream** — one baseline greyscale JPEG
// file after another, `SOI` to `EOI`, frame after frame, for as long as the
// port is open.
//
// ===================================================================
// WHAT THERE IS TO LOOK AT
// ===================================================================
//
// The observable is a video file, and the whole of the host side is two
// programs nobody here wrote:
//
//     stty -F /dev/ttyACM1 raw -echo
//     timeout 5 cat /dev/ttyACM1 > /tmp/stream.mjpg
//     ffprobe /tmp/stream.mjpg
//     ffplay  /tmp/stream.mjpg
//
// `mjpeg` is a format ffmpeg demuxes by looking for `FF D8`, so a
// concatenation of whole files is already a playable stream and needs no
// container and no framing of ours. `ffprobe` should say
// `mjpeg (Baseline), gray(bt470bg), 256x240`.
//
// Pattern 6 is a bar that moves one pixel a frame, so **the picture is the
// proof that frames are live**: a still would be one encode repeated and a
// moving bar cannot be. Writing an ASCII digit to the port picks the
// pattern:
//
//     printf '6' > /dev/ttyACM1
//
// `tests/usb_mjpeg.rs` is the same thing as a test, and it finds the port by
// walking sysfs for `1209:0001` rather than by guessing a number.
//
// ===================================================================
// WHY THIS FITS A 12F AND `examples/nes`'s ENCODER DOES NOT
// ===================================================================
//
// `examples/nes`'s `nes_jpeg` is the same encoder on the same clock and it
// does not fit this board, by the memory and not the logic: a 256x240 frame
// buffer of palette indices is 39 `DP16KD` and the encoder's own five
// memories are 7, against the 32 Lattice specifies for an `LFE5U-12`
// (FPGA-DS-02012-3.4 Table 1.1; `src/fpga/devices/ecp5.dev` has the reading
// and why the fabric database enumerates twice it).
//
// **`pattern_gen` has no frame buffer at all.** Its header says so in its
// first paragraph — "no memory, no state but a frame counter, and a colour
// that is a function of the pixel being asked for" — and that is exactly the
// property this design needs, because `mjpeg_enc` wants its samples
// **block at a time**, 64 of a block, blocks in raster order, and a source
// that is a pure function of `x` and `y` can be asked for them in that order
// as readily as in raster order. A frame buffer is the thing you need when
// the source cannot be re-asked, and a function can.
//
// So the memory here is the encoder's seven blocks and nothing else, and
// what would have been a frame buffer is one adder: `x` is the block
// column's pixel and `y` is the block row's.
//
// ===================================================================
// THE TABLES ARE THIS DESIGN'S, BECAUSE THE ENCODER HAS NONE
// ===================================================================
//
// `ip/video/mjpeg_enc/README.md` is deliberate about this: "Nothing in this
// package contains Annex K. The DC and AC code tables are the caller's", so
// that one source of truth both writes the file's `DHT` and derives the
// codes the coder uses, and a `DHT` that disagrees with the codes used to
// write the scan cannot happen. On a bench that caller is a testbench. On a
// part there is nobody to be the caller, so **the tables are a ROM in here**
// and `u_tables` walks them into `cfg_*` once, out of reset, before the
// first `start`.
//
// The two Huffman tables are T.81 Annex K Tables K.3 (DC luminance) and K.5
// (AC luminance), in the form a `DHT` segment carries them: sixteen counts
// and then the symbols. `tests/usb_mjpeg.rs` checks the ROM below against
// `testdata/ip/mjpeg_enc_corpus.txt`, which is where this repository already
// keeps those tables, so the bytes are checked against the one copy and not
// only read off a page. The counts and the symbol count agree with each
// other by construction: 0+1+5+1+1+1+1+1+1 is 12 and `dcvals` is 12 bytes,
// and the AC table's counts sum to 162.
//
// **The quantisation table is flat 16, and that is a choice.** Annex K Table
// K.1 would give a smaller file at the same visible quality and is what a
// camera would carry. Flat 16 is here for two reasons that both matter more
// on a first board bring-up than file size does: a flat table **cannot be
// put in the wrong order**, which is the one failure mode
// `ip/video/mjpeg_enc/README.md`'s table of what its tests would catch names
// for this port, and it is the table `examples/nes`'s golden and the whole
// `mjpeg_enc` corpus were measured at — so the 1.09 bits a pixel that
// example measured is a figure this design's output can be compared against
// directly. Swapping it for K.1 is 64 bytes of this ROM and nothing else.
//
// ===================================================================
// WHY THERE IS NO FIFO BETWEEN THE ENCODER AND THE ENDPOINT
// ===================================================================
//
// `testdata/fpga/cynthion/usb_cdc_uart.v`'s bridge needs a queue it cannot
// have and spends a paragraph on the fact; this one needs none, and the
// difference is back-pressure. `uart_rx` has no `rx_ready` — a byte nobody
// takes in the cycle it arrives is gone — so something must stand between it
// and an endpoint that makes things wait. `mjpeg_enc`'s output is a
// `valid`/`ready` pair, and the encoder simply stops when `out_ready` is
// low. So:
//
//     out_byte  -> in_data
//     out_valid -> in_valid
//     in_ready  -> out_ready
//
// three wires, no storage, and no byte can be lost by anything. The whole
// pipeline stalls back to the FDCT while the host is slow, which for a
// stream of frames is the correct behaviour and not a compromise: the frame
// rate becomes whatever the host collects, and no frame is ever half sent.
//
// **The packets are full ones, and the endpoint arms itself.**
// `usb_bulk_ep` sets `armed` when `(ilen + 1) == MAXPKT` as well as on
// `in_commit`, so handing it bytes and never committing produces 64-byte
// packets by itself. `in_commit` is pulsed once at the end of a frame
// instead, which flushes the tail and makes the short packet that ends a
// bulk transfer fall exactly on a `EOI`.
//
// ===================================================================
// HOW FAST, AND WHICH END IS THE LIMIT
// ===================================================================
//
// Two numbers bound it from opposite sides, and which one binds depends on
// the picture.
//
// **The encoder** takes 648 cycles a block — `jpeg_fdct`'s header says so
// and `tests/nes.rs` pins it — so 960 blocks is 622 080 clocks, 10.4 ms at
// 60 MHz, 96 frames a second.
//
// **The wire** carries 255 500 bytes a second each way through this
// endpoint at 64-byte packets, measured on this board and written up in
// `ip/usb/usb_cdc_acm/README.md` section 5. That is a round-trip figure
// from a loopback, so one direction alone has at least that.
//
// Which leaves the file size, and the test patterns are **very** easy
// pictures: the sweeping bar at 256x240 encodes to 1962 bytes in
// simulation, 0.26 bits a pixel, because most of its 960 blocks are flat
// black. At that size the wire would carry 130 frames a second and the
// encoder is the limit. A picture as busy as `examples/nes`'s demo frame
// is 8370 bytes at the same quantisation — 1.09 bits a pixel, measured
// there — and at that size the wire binds first, at about 30 frames a
// second.
//
// `tests/usb_mjpeg.rs` prints what the board actually does and asserts
// none of it: nothing in this repository compares a throughput against a
// clock.
//
// ===================================================================
// WHAT THIS DESIGN DOES NOT ESTABLISH
// ===================================================================
//
// **It is not a camera a video-conferencing application opens.** There is no
// USB video class in `ip/usb/` — it is CDC ACM, a full-speed device, a ULPI
// device, a host, a hub and a proxy — so a host sees a serial port with
// JPEG files coming out of it, which ffmpeg is happy with and Zoom is not.
// UVC wants a video control interface, a streaming interface with format and
// frame descriptors, and the probe-and-commit negotiation on top; none of
// that is written.
//
// **It is greyscale.** `mjpeg_enc` is, and says so in its version. The
// luminance below is BT.601 on `pattern_gen`'s colour, so the colour bars
// come out as the eight grey levels those bars have and not as colour.
//
// **The clock is the board's 60 MHz and there is no timing model worth the
// name for this family.** `docs/fpga-trellis.md` says what the ECP5 delay
// model is — unit placeholders — so the slack this flow reports is a count
// of logic levels and not a time, and the only evidence that this design
// runs at 60 MHz is that the part does it.

module usb_mjpeg #(
    // How many clocks the core is held in reset after configuration.
    // `usb_cdc_uart.v`'s header says why this is a shift register and not
    // `rst_n` tied high.
    parameter integer POR = 16,
    // Cycles of an idle bus at J before the device's answer goes out, which
    // ULPI 1.1 Table 10 allows a full-speed Link between 7 and 18 of.
    parameter [6:0] TURNAROUND = 7'd9,
    // The picture. Both must be multiples of eight, because `mjpeg_enc` does
    // not pad an edge block, and **neither may exceed 256**, because `bx`
    // and `by` below are five bits and 32 blocks is as far as they count.
    // A wider picture would truncate `LAST_COL` silently rather than fail,
    // which is the one trap in this file; widen both counters together if
    // you need one.
    //
    // 256x240 is `examples/nes`'s frame, so the bits-a-pixel figure measured
    // there is directly comparable with what comes out of here.
    parameter integer WIDTH  = 256,
    parameter integer HEIGHT = 240,
    // Which pattern comes up before a host has asked for one. 6 is the
    // sweeping bar, which is the one that moves.
    parameter [2:0] PATTERN = 3'd6
) (
    input  wire clk,             // A8, the 60.000 MHz oscillator

    // The auxiliary transceiver, all on the die's right edge but D16.
    inout  wire [7:0] ulpi_data, // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire ulpi_dir,        // E16
    input  wire ulpi_nxt,        // F15
    output wire ulpi_stp,        // E15
    output wire ulpi_rst_n,      // J13, active low at the ball
    output wire ulpi_clk,        // D16, the clock the board says we owe it

    output wire led0_n,          // the tables are in the encoder
    output wire led1_n,          // THE HOST CONFIGURED IT
    output wire led2_n,          // a frame is being fed
    output wire led3_n,          // heartbeat
    output wire led4_n,          // BYTES ARE GOING OUT
    output wire led5_n           // the host picked a pattern
);

    // -----------------------------------------------------------------
    // The picture in blocks.
    // -----------------------------------------------------------------
    localparam integer COLS = WIDTH / 8;
    localparam integer ROWS = HEIGHT / 8;
    // Compared against, so they are vectors and not `integer`s: an
    // `integer` localparam cannot be part-selected, which cost one round of
    // this in `examples/nes/rtl/nes_jpeg.v`.
    localparam [4:0] LAST_COL = COLS - 1;
    localparam [4:0] LAST_ROW = ROWS - 1;

    // -----------------------------------------------------------------
    // Power-on reset.
    // -----------------------------------------------------------------
    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) begin
        por <= {por[POR-2:0], 1'b1};
    end
    wire reset_done = por[POR-1];

    // -----------------------------------------------------------------
    // The serial port.
    // -----------------------------------------------------------------
    wire [7:0] data_o;
    wire       data_oe;
    wire [6:0] address;
    wire       configured;
    wire       usb_reset;

    wire [7:0] out_data;
    wire       out_valid;
    wire       out_last;
    wire [7:0] in_data;
    wire       in_valid;
    wire       in_ready;
    wire       in_commit;

    // Acted on by nothing here. A stream of JPEG files has no baud rate and
    // no character format; the host's `stty` is answered and ignored, which
    // is what every CDC ACM device that is not a UART does.
    wire [31:0] baud;
    wire [7:0]  char_format;
    wire [7:0]  parity;
    wire [7:0]  data_bits;
    wire        dtr;
    wire        rts;

    // Both carriers, no errors. `usb_cdc_uart.v`'s header has the long form
    // of why a device with nothing to hang up reports a constant here, and
    // `ip/usb/usb_cdc_acm/README.md` §4 has the defect that made it send one
    // on every open rather than once.
    wire [6:0] serial_state = 7'b000_0011;

    assign ulpi_data = data_oe ? data_o : 8'bz;
    assign ulpi_clk  = clk;

    usb_cdc_acm_ulpi #(
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06)
    ) u_acm (
        .clk60        (clk),
        .rst_n        (reset_done),
        .ulpi_data_i  (ulpi_data),
        .ulpi_data_o  (data_o),
        .ulpi_data_oe (data_oe),
        .ulpi_dir     (ulpi_dir),
        .ulpi_nxt     (ulpi_nxt),
        .ulpi_stp     (ulpi_stp),
        .ulpi_rst_n   (ulpi_rst_n),
        .address      (address),
        .configured   (configured),
        .usb_reset    (usb_reset),
        .phy_ready    (),
        .out_data     (out_data),
        .out_valid    (out_valid),
        .out_last     (out_last),
        // Always taken. The only thing a host can say to this device is
        // which pattern it wants, and a byte that is not a digit is
        // discarded rather than held — so there is never a reason to NAK,
        // and the OUT endpoint never fills.
        .out_ready    (1'b1),
        .in_data      (in_data),
        .in_valid     (in_valid),
        .in_ready     (in_ready),
        .in_commit    (in_commit),
        .baud         (baud),
        .char_format  (char_format),
        .parity       (parity),
        .data_bits    (data_bits),
        .dtr          (dtr),
        .rts          (rts),
        .serial_state (serial_state)
    );

    // -----------------------------------------------------------------
    // WHICH PATTERN, AND WHY IT CHANGES ONLY BETWEEN FRAMES
    // -----------------------------------------------------------------
    // A digit arriving in the middle of a frame would change the picture
    // half way down it, and the half-and-half frame that came out would be
    // a perfectly valid JPEG file of two pictures. So the byte is latched
    // when it arrives and the encoder's copy is taken at `start`.
    reg [2:0] pattern_asked = PATTERN;
    reg [2:0] pattern       = PATTERN;
    reg       host_picked   = 1'b0;
    // One cycle a frame, which is what `pattern_gen` counts its sweeping
    // bar on: its header says `frame` is a level there and that it advances
    // on `frame & pix_en`, so with `pix_en` tied high this has to be a
    // single cycle or the bar would move once per clock.
    reg       frame_tick    = 1'b0;
    always @(posedge clk) begin
        if (!reset_done) begin
            pattern_asked <= PATTERN;
            host_picked   <= 1'b0;
        end else if (out_valid && out_data >= 8'h30 && out_data <= 8'h37) begin
            pattern_asked <= out_data[2:0];
            host_picked   <= 1'b1;
        end
    end

    // -----------------------------------------------------------------
    // THE TABLES, WALKED IN ONCE
    // -----------------------------------------------------------------
    // Three phases, in the order `mjpeg_enc`'s header lists its `cfg_sel`
    // codes: the 64 `DQT` bytes in zig-zag order, then the DC `DHT`'s 16
    // counts and 12 symbols, then the AC `DHT`'s 16 counts and 162 symbols.
    // 270 writes, one a cycle, and then `tables_in` stays high for ever.
    localparam [1:0] T_Q = 2'd0, T_DC = 2'd1, T_AC = 2'd2, T_IN = 2'd3;

    reg [1:0] tphase = T_Q;
    reg [7:0] taddr  = 8'd0;
    reg       tables_in = 1'b0;

    // The DC table, `DHT` order: 16 counts then 12 symbols. T.81 Table K.3.
    reg [7:0] dc_rom;
    always @* begin
        case (taddr)
            0: dc_rom = 8'h00; 1: dc_rom = 8'h01; 2: dc_rom = 8'h05; 3: dc_rom = 8'h01;
            4: dc_rom = 8'h01; 5: dc_rom = 8'h01; 6: dc_rom = 8'h01; 7: dc_rom = 8'h01;
            8: dc_rom = 8'h01; 9: dc_rom = 8'h00; 10: dc_rom = 8'h00; 11: dc_rom = 8'h00;
            12: dc_rom = 8'h00; 13: dc_rom = 8'h00; 14: dc_rom = 8'h00; 15: dc_rom = 8'h00;
            16: dc_rom = 8'h00; 17: dc_rom = 8'h01; 18: dc_rom = 8'h02; 19: dc_rom = 8'h03;
            20: dc_rom = 8'h04; 21: dc_rom = 8'h05; 22: dc_rom = 8'h06; 23: dc_rom = 8'h07;
            24: dc_rom = 8'h08; 25: dc_rom = 8'h09; 26: dc_rom = 8'h0a; 27: dc_rom = 8'h0b;
            default: dc_rom = 8'h00;
        endcase
    end

    // The AC table, `DHT` order: 16 counts then 162 symbols. T.81 Table K.5.
    reg [7:0] ac_rom;
    always @* begin
        case (taddr)
            0: ac_rom = 8'h00; 1: ac_rom = 8'h02; 2: ac_rom = 8'h01; 3: ac_rom = 8'h03;
            4: ac_rom = 8'h03; 5: ac_rom = 8'h02; 6: ac_rom = 8'h04; 7: ac_rom = 8'h03;
            8: ac_rom = 8'h05; 9: ac_rom = 8'h05; 10: ac_rom = 8'h04; 11: ac_rom = 8'h04;
            12: ac_rom = 8'h00; 13: ac_rom = 8'h00; 14: ac_rom = 8'h01; 15: ac_rom = 8'h7d;
            16: ac_rom = 8'h01; 17: ac_rom = 8'h02; 18: ac_rom = 8'h03; 19: ac_rom = 8'h00;
            20: ac_rom = 8'h04; 21: ac_rom = 8'h11; 22: ac_rom = 8'h05; 23: ac_rom = 8'h12;
            24: ac_rom = 8'h21; 25: ac_rom = 8'h31; 26: ac_rom = 8'h41; 27: ac_rom = 8'h06;
            28: ac_rom = 8'h13; 29: ac_rom = 8'h51; 30: ac_rom = 8'h61; 31: ac_rom = 8'h07;
            32: ac_rom = 8'h22; 33: ac_rom = 8'h71; 34: ac_rom = 8'h14; 35: ac_rom = 8'h32;
            36: ac_rom = 8'h81; 37: ac_rom = 8'h91; 38: ac_rom = 8'ha1; 39: ac_rom = 8'h08;
            40: ac_rom = 8'h23; 41: ac_rom = 8'h42; 42: ac_rom = 8'hb1; 43: ac_rom = 8'hc1;
            44: ac_rom = 8'h15; 45: ac_rom = 8'h52; 46: ac_rom = 8'hd1; 47: ac_rom = 8'hf0;
            48: ac_rom = 8'h24; 49: ac_rom = 8'h33; 50: ac_rom = 8'h62; 51: ac_rom = 8'h72;
            52: ac_rom = 8'h82; 53: ac_rom = 8'h09; 54: ac_rom = 8'h0a; 55: ac_rom = 8'h16;
            56: ac_rom = 8'h17; 57: ac_rom = 8'h18; 58: ac_rom = 8'h19; 59: ac_rom = 8'h1a;
            60: ac_rom = 8'h25; 61: ac_rom = 8'h26; 62: ac_rom = 8'h27; 63: ac_rom = 8'h28;
            64: ac_rom = 8'h29; 65: ac_rom = 8'h2a; 66: ac_rom = 8'h34; 67: ac_rom = 8'h35;
            68: ac_rom = 8'h36; 69: ac_rom = 8'h37; 70: ac_rom = 8'h38; 71: ac_rom = 8'h39;
            72: ac_rom = 8'h3a; 73: ac_rom = 8'h43; 74: ac_rom = 8'h44; 75: ac_rom = 8'h45;
            76: ac_rom = 8'h46; 77: ac_rom = 8'h47; 78: ac_rom = 8'h48; 79: ac_rom = 8'h49;
            80: ac_rom = 8'h4a; 81: ac_rom = 8'h53; 82: ac_rom = 8'h54; 83: ac_rom = 8'h55;
            84: ac_rom = 8'h56; 85: ac_rom = 8'h57; 86: ac_rom = 8'h58; 87: ac_rom = 8'h59;
            88: ac_rom = 8'h5a; 89: ac_rom = 8'h63; 90: ac_rom = 8'h64; 91: ac_rom = 8'h65;
            92: ac_rom = 8'h66; 93: ac_rom = 8'h67; 94: ac_rom = 8'h68; 95: ac_rom = 8'h69;
            96: ac_rom = 8'h6a; 97: ac_rom = 8'h73; 98: ac_rom = 8'h74; 99: ac_rom = 8'h75;
            100: ac_rom = 8'h76; 101: ac_rom = 8'h77; 102: ac_rom = 8'h78; 103: ac_rom = 8'h79;
            104: ac_rom = 8'h7a; 105: ac_rom = 8'h83; 106: ac_rom = 8'h84; 107: ac_rom = 8'h85;
            108: ac_rom = 8'h86; 109: ac_rom = 8'h87; 110: ac_rom = 8'h88; 111: ac_rom = 8'h89;
            112: ac_rom = 8'h8a; 113: ac_rom = 8'h92; 114: ac_rom = 8'h93; 115: ac_rom = 8'h94;
            116: ac_rom = 8'h95; 117: ac_rom = 8'h96; 118: ac_rom = 8'h97; 119: ac_rom = 8'h98;
            120: ac_rom = 8'h99; 121: ac_rom = 8'h9a; 122: ac_rom = 8'ha2; 123: ac_rom = 8'ha3;
            124: ac_rom = 8'ha4; 125: ac_rom = 8'ha5; 126: ac_rom = 8'ha6; 127: ac_rom = 8'ha7;
            128: ac_rom = 8'ha8; 129: ac_rom = 8'ha9; 130: ac_rom = 8'haa; 131: ac_rom = 8'hb2;
            132: ac_rom = 8'hb3; 133: ac_rom = 8'hb4; 134: ac_rom = 8'hb5; 135: ac_rom = 8'hb6;
            136: ac_rom = 8'hb7; 137: ac_rom = 8'hb8; 138: ac_rom = 8'hb9; 139: ac_rom = 8'hba;
            140: ac_rom = 8'hc2; 141: ac_rom = 8'hc3; 142: ac_rom = 8'hc4; 143: ac_rom = 8'hc5;
            144: ac_rom = 8'hc6; 145: ac_rom = 8'hc7; 146: ac_rom = 8'hc8; 147: ac_rom = 8'hc9;
            148: ac_rom = 8'hca; 149: ac_rom = 8'hd2; 150: ac_rom = 8'hd3; 151: ac_rom = 8'hd4;
            152: ac_rom = 8'hd5; 153: ac_rom = 8'hd6; 154: ac_rom = 8'hd7; 155: ac_rom = 8'hd8;
            156: ac_rom = 8'hd9; 157: ac_rom = 8'hda; 158: ac_rom = 8'he1; 159: ac_rom = 8'he2;
            160: ac_rom = 8'he3; 161: ac_rom = 8'he4; 162: ac_rom = 8'he5; 163: ac_rom = 8'he6;
            164: ac_rom = 8'he7; 165: ac_rom = 8'he8; 166: ac_rom = 8'he9; 167: ac_rom = 8'hea;
            168: ac_rom = 8'hf1; 169: ac_rom = 8'hf2; 170: ac_rom = 8'hf3; 171: ac_rom = 8'hf4;
            172: ac_rom = 8'hf5; 173: ac_rom = 8'hf6; 174: ac_rom = 8'hf7; 175: ac_rom = 8'hf8;
            176: ac_rom = 8'hf9; 177: ac_rom = 8'hfa;
            default: ac_rom = 8'h00;
        endcase
    end

    reg [1:0] cfg_sel  = 2'd0;
    reg [7:0] cfg_addr = 8'd0;
    reg [7:0] cfg_data = 8'd0;
    reg       cfg_we   = 1'b0;

    always @(posedge clk) begin
        cfg_we <= 1'b0;
        if (!reset_done) begin
            tphase    <= T_Q;
            taddr     <= 8'd0;
            tables_in <= 1'b0;
        end else begin
            case (tphase)
                // A flat table: the divisor is the constant and there is no
                // ROM to get the order of wrong.
                T_Q: begin
                    cfg_sel  <= 2'd0;
                    cfg_addr <= taddr;
                    cfg_data <= 8'd16;
                    cfg_we   <= 1'b1;
                    if (taddr == 8'd63) begin
                        tphase <= T_DC;
                        taddr  <= 8'd0;
                    end else begin
                        taddr <= taddr + 8'd1;
                    end
                end
                T_DC: begin
                    cfg_sel  <= 2'd1;
                    cfg_addr <= taddr;
                    cfg_data <= dc_rom;
                    cfg_we   <= 1'b1;
                    if (taddr == 8'd27) begin
                        tphase <= T_AC;
                        taddr  <= 8'd0;
                    end else begin
                        taddr <= taddr + 8'd1;
                    end
                end
                T_AC: begin
                    cfg_sel  <= 2'd2;
                    cfg_addr <= taddr;
                    cfg_data <= ac_rom;
                    cfg_we   <= 1'b1;
                    if (taddr == 8'd177) begin
                        tphase    <= T_IN;
                        tables_in <= 1'b1;
                    end else begin
                        taddr <= taddr + 8'd1;
                    end
                end
                T_IN: begin
                    tables_in <= 1'b1;
                end
            endcase
        end
    end

    // -----------------------------------------------------------------
    // THE SAMPLES, IN THE ORDER THE ENCODER WANTS THEM
    // -----------------------------------------------------------------
    // 64 of a block, row-major in the block, blocks in raster order. So the
    // pixel being asked for is `x = {bx, px}`, `y = {by, py}`, and the four
    // counters ripple in that order: `px` fastest, `by` slowest.
    //
    // Never wider than the values they hold. `docs/fpga-trellis.md` and this
    // repository's CLAUDE.md both carry the reason — a register wider than
    // its values once read 5 for a three-state machine on an unrouted part —
    // and `bx` and `by` are five bits because 31 and 29 need five.
    reg [2:0] px = 3'd0, py = 3'd0;
    reg [4:0] bx = 5'd0, by = 5'd0;

    wire [11:0] gx = {4'b0, bx, px};
    wire [11:0] gy = {4'b0, by, py};

    // The last sample of the last block.
    wire s_last = (px == 3'd7) && (py == 3'd7) &&
                  (bx == LAST_COL) && (by == LAST_ROW);

    // The picture. `de` is high because every sample this design asks for is
    // inside the active area by construction — the counters cannot name a
    // pixel outside it — and `pix_en` is high because this clock is the
    // pixel clock here: a sample is consumed whenever the encoder takes one,
    // not on a raster's schedule.
    wire [7:0] pr, pg, pb;
    pattern_gen #(
        .H_ACTIVE (WIDTH),
        .V_ACTIVE (HEIGHT)
    ) u_pat (
        .clk     (clk),
        .rst_n   (reset_done),
        .pix_en  (1'b1),
        .de      (1'b1),
        .frame   (frame_tick),
        .x       (gx),
        .y       (gy),
        .pattern (pattern),
        .r       (pr),
        .g       (pg),
        .b       (pb)
    );

    // -----------------------------------------------------------------
    // LUMINANCE
    // -----------------------------------------------------------------
    // BT.601 at eight bits: 77, 150 and 29 over 256. They sum to 256
    // exactly, so white is 255 and nothing overflows — the same arithmetic
    // and the same reason as `examples/nes/rtl/nes_luma.v`, which has the
    // test that checks all 64 of that palette's colours against it.
    //
    //   77 = 64 + 8 + 4 + 1      150 = 128 + 16 + 4 + 2      29 = 16 + 8 + 4 + 1
    wire [15:0] weighted =
          {2'b0, pr, 6'b0} + {5'b0, pr, 3'b0} + {6'b0, pr, 2'b0} + {8'b0, pr}
        + {1'b0, pg, 7'b0} + {4'b0, pg, 4'b0} + {6'b0, pg, 2'b0} + {7'b0, pg, 1'b0}
        + {4'b0, pb, 4'b0} + {5'b0, pb, 3'b0} + {6'b0, pb, 2'b0} + {8'b0, pb};
    wire [7:0] sample = weighted[15:8];

    // -----------------------------------------------------------------
    // THE ENCODER, AND THE ONE-FRAME-AFTER-ANOTHER LOOP
    // -----------------------------------------------------------------
    // `in_ready` is `pix_gate && fdct_ready` inside the encoder, and
    // `pix_gate` is low through the seven-hundred-cycle prepare phase, so
    // `feeding` may be raised in the same cycle as `start` and the handshake
    // takes care of the wait. `done` is a **level** — `state == S_DONE` —
    // and `start` is a **pulse** that `S_DONE` answers, so the restart is
    // the rising edge of `done` and `done_seen` is what makes it one.
    wire       enc_ready;
    wire [7:0] enc_byte;
    wire       enc_valid;
    wire       enc_done;

    reg  feeding   = 1'b0;
    reg  enc_start = 1'b0;
    reg  done_seen = 1'b0;
    reg  commit    = 1'b0;

    wire took = feeding && enc_ready;

    always @(posedge clk) begin
        enc_start  <= 1'b0;
        frame_tick <= 1'b0;
        commit     <= 1'b0;

        if (!reset_done) begin
            feeding   <= 1'b0;
            done_seen <= 1'b0;
            px <= 3'd0; py <= 3'd0; bx <= 5'd0; by <= 5'd0;
        end else if (!tables_in) begin
            // Nothing may be fed before the encoder has its tables: the
            // header it writes carries them, and it assembles that header
            // at `start`.
            feeding <= 1'b0;
        end else if (!feeding && !done_seen) begin
            // The first frame, and every frame after one that finished.
            enc_start  <= 1'b1;
            frame_tick <= 1'b1;
            pattern    <= pattern_asked;
            feeding    <= 1'b1;
            px <= 3'd0; py <= 3'd0; bx <= 5'd0; by <= 5'd0;
        end else if (feeding) begin
            if (took) begin
                if (s_last) begin
                    feeding   <= 1'b0;
                    done_seen <= 1'b1;
                end else if (px != 3'd7) begin
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
                            by <= (by == LAST_ROW) ? 5'd0 : by + 5'd1;
                        end
                    end
                end
            end
        end else if (done_seen && enc_done) begin
            // Every byte of the frame has been taken. Flush whatever is
            // short of a packet, so the file ends on a short packet, and go
            // round again.
            commit    <= 1'b1;
            done_seen <= 1'b0;
        end
    end

    mjpeg_enc #(
        .WIDTH  (WIDTH),
        .HEIGHT (HEIGHT)
    ) u_enc (
        .clk       (clk),
        .rst_n     (reset_done),
        .cfg_sel   (cfg_sel),
        .cfg_addr  (cfg_addr),
        .cfg_data  (cfg_data),
        .cfg_we    (cfg_we),
        .start     (enc_start),
        .in_pixel  (sample),
        .in_valid  (feeding),
        .in_ready  (enc_ready),
        .out_byte  (enc_byte),
        .out_valid (enc_valid),
        .out_ready (in_ready),
        .done      (enc_done)
    );

    assign in_data   = enc_byte;
    assign in_valid  = enc_valid;
    assign in_commit = commit;

    // -----------------------------------------------------------------
    // THE SIX LEDS
    // -----------------------------------------------------------------
    // The diagnosis for when no video appears, read left to right:
    // tables in, host configured, feeding a frame, alive, bytes moving,
    // host picked a pattern. `usb_cdc_uart.v`'s header says why the
    // heartbeat is the top bit of a free-running counter.
    reg [25:0] beat = 26'd0;
    always @(posedge clk) beat <= beat + 26'd1;

    // Stretched so that a byte going out is visible: the bulk endpoint
    // moves 64 bytes in 43 microseconds and then waits for the host, which
    // no eye can see.
    reg [21:0] flow = 22'd0;
    always @(posedge clk) begin
        if (!reset_done) flow <= 22'd0;
        else if (enc_valid && in_ready) flow <= {22{1'b1}};
        else if (flow != 22'd0) flow <= flow - 22'd1;
    end

    assign led0_n = ~tables_in;
    assign led1_n = ~configured;
    assign led2_n = ~feeding;
    assign led3_n = ~beat[25];
    assign led4_n = ~(flow != 22'd0);
    assign led5_n = ~host_picked;

endmodule
