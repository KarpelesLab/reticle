// A block RAM behind a USB serial port, to answer one question.
//
// `usb_inflate.v` decodes a stream's first literal byte correctly on this
// board and returns `ff` for every byte that comes out of its window, and an
// unrouted input here reads as a one. This is that memory with nothing else
// attached: write bytes, read them back. `bram_echo.v`'s header says what
// it found -- a block RAM data pin the flow had never routed -- and why the
// mode this comment first blamed was not even the mode it was in.
//
// Pins: testdata/fpga/cynthion/usb_bram.rcf.
// Sources: testdata/fpga/cynthion/bram_echo.v, ip/usb/usb_cdc_acm/rtl/*.v,
//          ip/usb/usb_device_ulpi/rtl/usb_ulpi_link.v and
//          ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v.

module usb_bram #(
    // How many clocks the core is held in reset after configuration.
    parameter integer POR = 16,
    // Cycles of an idle bus at J before the device's answer goes out.
    parameter [6:0] TURNAROUND = 7'd9,
    // The same depth and width as `inflate`'s window, which is what this
    // exists to test: sixteen `DP16KD`, in 9-bit mode now that
    // `choose_bram` breaks a block-count tie by preferring one block per
    // word.
    parameter integer ADDR_BITS = 15
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

    bram_echo #(
        .ADDR_BITS (ADDR_BITS)
    ) u_echo (
        .clk        (clk),
        .rst_n      (reset_done),
        .host_data  (out_data),
        .host_valid (out_valid),
        .host_ready (out_ready),
        .dev_data   (in_data),
        .dev_valid  (in_valid),
        .dev_ready  (in_ready),
        .dev_commit (in_commit)
    );

    reg [23:0] beat = 24'd0;
    always @(posedge clk) beat <= beat + 24'd1;

    assign led0_n = ~out_valid;
    assign led1_n = ~configured;
    assign led2_n = ~(in_valid & in_ready);
    assign led3_n = ~beat[23];
    assign led4_n = 1'b1;
    assign led5_n = 1'b1;
endmodule
