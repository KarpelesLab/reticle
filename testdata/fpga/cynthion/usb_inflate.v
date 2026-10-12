// A decompressor on the other end of a USB serial port.
//
// `ip/usb/usb_cdc_acm` on the Cynthion's auxiliary ULPI transceiver, its
// received bytes handed to `ip/compress/inflate` and that block's output
// handed back, so a program writes a DEFLATE stream to `/dev/ttyACM*` and
// reads the plaintext out of the same file.
//
// ===================================================================
// WHY THIS EXISTS
// ===================================================================
//
// **`inflate` has never run on a part.** It is checked against RFC 1951 and
// RFC 1950, measured in simulation, and CONFIRMED against `compcol` -- which
// produced every stream in `testdata/ip/inflate_corpus.txt` and took every
// one of them back -- but no fabric has ever decoded a byte of it. A day of
// hardware testing on another board found, in one sitting, a pad whose data
// input was never connected, a clock routed through a switched-off buffer so
// that nothing on it ticked, and block RAM addressed through the wrong bits:
// three faults that every simulation passed and no amount of reading would
// have found. A block that holds 32768 bytes of history in sixteen `DP16KD`
// and walks a Huffman tree is exactly the shape of thing that class of fault
// hides in.
//
// So the observable is the plaintext. A stream goes in, bytes come out, and
// a host compares them with what `compcol` said they should be.
//
// ===================================================================
// WHERE IT HAS GOT TO ON A PART, 12 OCTOBER 2026
// ===================================================================
//
// **CHECKED: 55 of the corpus' 59 zlib cases decode on a Cynthion**,
// byte for byte, status `00`. That includes every stored and fixed-Huffman
// case, `text3_1000_l1`, and the four-thousand-byte cases that cross the
// window. Three faults were in the way and each is recorded where it was
// fixed:
//
//   * sixteen block RAM read-data nets nothing drove, so the 32768-byte
//     window read `ff` for every byte -- `docs/fpga-trellis.md` and
//     `bram_echo.v`;
//   * the `DP16KD`'s 4-bit mode, which a block-count tie put the 320 x 4
//     code-length memory into, and which is wrong on silicon --
//     `src/fpga/devices/ecp5.dev` now marks it `unverified` and
//     `choose_bram` orders a confirmed layout first;
//   * a refused stream wedging the bridge for good, because `S_FEED` read
//     neither `error` nor a timer -- `inflate_bridge.v`.
//
// **The 64 raw-deflate cases are refused, by construction.** This design
// builds `inflate` with `WRAPPER(1)`, so a stream with no zlib header is a
// header error and comes back as `81`. That is the right answer to the
// wrong question: the corpus holds both wrappings and nothing here selects
// between them. A mode byte in the framing would fix it and has not been
// written.
//
// **OPEN, and the rule is about history, not about refusals.** Four of
// the 59 zlib cases are wrong -- `text1_100`, `text1_1000`, `text5_1000`
// and `text1_4000`, all dynamic-Huffman -- and what decides it is:
//
//     a dynamic-Huffman stream decodes correctly only if this decoder has
//     not already decoded a DIFFERENT stream since configuration.
//
// Measured one row at a time, reconfiguring the part before each row,
// because one failure wedges it and every row after then reads the same:
//
//     before the dynamic stream          result
//     -------------------------------    ------------------------
//     nothing at all                     correct
//     the same dynamic stream            correct
//     a refusal only                     correct
//     a stored-block stream              4 bytes, then status C0
//     a fixed-Huffman stream             4 bytes, then status C0
//     a different dynamic stream         4 bytes, then status C0
//
// **The reading before this one was wrong and is worth leaving here.** It
// said "a dynamic stream that follows a refused one", because all four
// failures sit immediately after a raw-deflate case in corpus order and
// each one decodes correctly on its own. A refusal is harmless; what the
// corpus order hid is that every one of those cases also followed a
// successful decode. Running the rows against a part that had already run
// other rows is what made a correlation look like a cause -- the same
// contamination that made the first eight rows of the first attempt
// identical.
//
// Three things have been eliminated, two by test and one by measurement:
//
//   * `inflate.v`'s own state. `tests/ip_library.rs`'s
//     `inflate_rearms_after_a_refusal_for_a_dynamic_huffman_stream` and
//     `inflate_takes_the_corpus_in_order_with_the_deflate_cases_refused`
//     run the pairing three times over and then all 123 cases in corpus
//     order through one `WRAPPER=1` instance, refusals included. Both
//     pass, so the decoder re-arms correctly and this is a hardware-only
//     divergence.
//   * the window. `bram_echo.v` reads 32768 bytes back off the part.
//   * a block RAM read in the cycle its own address is written, which is
//     what a counting sort does and what `bram_echo.v`'s header named as
//     the nearest untested thing. `bram_rw.v` measures it: the part reads
//     **before** the write, which is what the Verilog and
//     `WRITEMODE_A/B = READBEFOREWRITE` both say. Not the fault.
//   * a distributed RAM read and written at one address in one cycle with
//     the read feeding the write, which is what this block's count file
//     literally does -- `lcnt[cnt_addr]` is combinational and
//     `lcnt[cnt_addr] <= cnt_rd + 1` writes the same address in the same
//     cycle. It matters because **`lsym` is never cleared between
//     streams**: the decoder only indexes slots the counts say were filled
//     this time, so a count wrong by anything reads a symbol the previous
//     stream left -- invisible while `lsym` is all zeros, wrong once a
//     different stream has written it, which is this rule exactly.
//     `lutram_rw.v` measures it on the same primitive at the same depth:
//     read, add, write, as modelled. Not the fault either.
//
// So what remains is something the first decode leaves behind that differs
// between the part and the simulator while its own output is correct. The
// next step is instrumentation rather than more reasoning: four rounds have
// each cost a build and a load to eliminate one candidate, and what is
// wanted now is the decoder's own state at the moment the second stream
// stalls -- the counts it computed and the index it took into `lsym`.
// `testdata/fpga/cynthion/` already holds the serial console and the trace
// buffer that found this backend's earlier hardware-only faults, none of
// which was found by reading.
//
// ===================================================================
// THE FRAMING, AND WHY THERE IS ANY
// ===================================================================
//
// A serial port is a byte pipe with no record boundaries, and `inflate`
// needs to be told where a stream ends: `in_last` is how a stream that needs
// more bits reports `E_TRUNC` rather than waiting for ever. `out_last` from
// the CDC block marks the end of a USB transfer and not of whatever a
// program chose to write, so it cannot carry that meaning.
//
// Hence a length prefix, which is two bytes and no ambiguity:
//
//     host -> device   len_hi len_lo, then exactly that many bytes
//     device -> host   the plaintext, then one status byte
//
// The status byte is `00` when the stream decoded and `80 | error_code`
// otherwise, with the code as `inflate.v` defines it: 1 header, 2 btype,
// 3 nlen, 4 code, 5 dist, 6 trunc, 7 adler. A host that knows the expected
// plaintext -- which is the only kind of host this is for -- reads that many
// bytes and then one more, so the status is never confused with data.
//
// A length of zero is legal and means an empty stream: `in_last` with no
// byte, which `inflate.v`'s header says is the only way to say so.
//
// ===================================================================
// WHAT THIS WOULD CATCH, AND WHAT IT WOULD NOT
// ===================================================================
//
// It would catch a window that reads back wrong, a Huffman table built in
// LUTs that the fabric builds differently from the simulator, a `DP16KD`
// addressed through the wrong bits, and any of the three faults named above
// that touch this path.
//
// It would not catch anything about streams the corpus does not contain, and
// it cannot distinguish a fault in `inflate` from a fault in the USB path
// carrying its bytes -- `usb_cdc_uart.v` is the control for that, being the
// same transceiver and the same class with a loopback instead of a
// decompressor, and it is CHECKED on this board.
//
// Pins: testdata/fpga/cynthion/usb_inflate.rcf.
// Sources: testdata/fpga/cynthion/inflate_bridge.v,
//          ip/usb/usb_cdc_acm/rtl/*.v,
//          ip/usb/usb_device_ulpi/rtl/usb_ulpi_link.v,
//          ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v and
//          ip/compress/inflate/rtl/*.v.

module usb_inflate #(
    // How many clocks the core is held in reset after configuration.
    parameter integer POR = 16,
    // Cycles of an idle bus at J before the device's answer goes out.
    parameter [6:0] TURNAROUND = 7'd9,
    // 15 is RFC 1951's largest window and the only setting that decodes
    // every legal stream; it is sixteen `DP16KD` of the 12F's thirty-two.
    parameter integer WINDOW_BITS = 15
) (
    input  wire clk,             // A8, the 60.000 MHz oscillator

    inout  wire [7:0] ulpi_data, // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire ulpi_dir,        // E16
    input  wire ulpi_nxt,        // F15
    output wire ulpi_stp,        // E15
    output wire ulpi_rst_n,      // J13, active low at the ball
    output wire ulpi_clk,        // D16, the clock the board says we owe it

    output wire led0_n,          // a stream is being read in
    output wire led1_n,          // THE HOST CONFIGURED IT
    output wire led2_n,          // plaintext is going out
    output wire led3_n,          // heartbeat
    output wire led4_n,          // the last stream ended in an error
    output wire led5_n           // the decompressor is busy
);
    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) por <= {por[POR-2:0], 1'b1};
    wire reset_done = por[POR-1];

    wire [7:0] data_o;
    wire       data_oe;
    wire [6:0] address;
    wire       configured;
    wire       usb_reset;

    wire [7:0] out_data;
    wire       out_valid;
    wire       out_last;
    wire       out_ready;
    wire [7:0] in_data;
    wire       in_valid;
    wire       in_ready;
    wire       in_commit;

    assign ulpi_clk    = clk;
    assign ulpi_data   = data_oe ? data_o : 8'bz;

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
        .out_ready    (out_ready),
        .in_data      (in_data),
        .in_valid     (in_valid),
        .in_ready     (in_ready),
        .in_commit    (in_commit),
        // The line coding reaches nothing here: there is no serial line on
        // the far side, only a decompressor, so these outputs are left open
        // rather than tied -- a reader should not look for a baud rate that
        // has no meaning in this design.
        .baud         (),
        .char_format  (),
        .parity       (),
        .data_bits    (),
        .dtr          (),
        .rts          (),
        // **`serial_state` is an input**, not another of those: it is
        // `wSerialState` of the SERIAL_STATE notification, which the device
        // tells the host. Left open it reads as nothing driving it, and the
        // build says so. `0000011` is DSR and DCD asserted and no error,
        // which is what `usb_cdc_uart.v` reports and what a port with
        // nothing to go wrong on the far side should say.
        .serial_state (7'b000_0011)
    );

    wire bridge_feeding, bridge_error, bridge_busy;

    inflate_bridge #(
        .WINDOW_BITS (WINDOW_BITS)
    ) u_bridge (
        .clk        (clk),
        .rst_n      (reset_done),
        .host_data  (out_data),
        .host_valid (out_valid),
        .host_ready (out_ready),
        .dev_data   (in_data),
        .dev_valid  (in_valid),
        .dev_ready  (in_ready),
        .dev_commit (in_commit),
        .feeding_o  (bridge_feeding),
        .error_o    (bridge_error),
        .busy_o     (bridge_busy)
    );

    reg [23:0] beat = 24'd0;
    always @(posedge clk) beat <= beat + 24'd1;

    assign led0_n = ~bridge_feeding;
    assign led1_n = ~configured;
    assign led2_n = ~(in_valid & in_ready);
    assign led3_n = ~beat[23];
    assign led4_n = ~bridge_error;
    assign led5_n = ~bridge_busy;
endmodule
