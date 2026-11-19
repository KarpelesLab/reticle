// smartcard_serial_mcu — the smart-card serial bridge of `smartcard_serial.v`
// with the MCU's flash reads answered inside the FPGA from a 32 KB image in
// block RAM, so a Ledger secure element's boot-time flash probes are met in a
// few clocks instead of a USB round trip.
//
// WHY THIS EXISTS
//   `smartcard_serial.v` carries every byte between a secure element (the card
//   on PMOD A) and a host program that plays the element's MCU over SEPROXYHAL
//   (`~/projects/ledger_console`). From firmware 1.4 on the element times four
//   random 32-byte reads of the MCU's signed flash at every boot: a real MCU
//   answers 37-41 us after the element's status packet, while an answer that
//   has to cross SE -> bridge -> USB (1 ms frames) -> host -> bridge takes
//   805-2290 us, and the element gives up with "MCU firmware is not genuine".
//
//   The fix is to keep the MCU's flash image in the FPGA and answer the timed
//   reads — UNSEC_CHUNK_READ_EXT (tag 0x33) kind 2, a read at an offset — from
//   it, ahead of the host's transmit path, before the element's deadline. The
//   proven `smartcard_serial.v` is left untouched; this is a second top beside
//   it with its own pin map (`smartcard_serial_mcu.rcf`).
//
//   The host still sees every element byte on the same /dev/ttyACM* serial
//   port (1209:0001), unchanged, and when the responder is **disabled** the
//   card path is byte-for-byte what `smartcard_serial.v` does today — an
//   element on 1.3.1 / MCU 1.0, which reads the flash sequentially (tag 0x32)
//   and is not timed, keeps working through the host. A second, vendor-class
//   USB interface ("smartcard-ctl") carries the image upload, the enable, and
//   the notices; `ip/bus/seph_mcu`'s turn engine and
//   `~/projects/ledger_console/src/{mcu,seph,bridge}.rs` are the protocol
//   precedent this follows.
//
// WHAT IS CHECKED AND WHAT IS QUOTED
//   `smartcard_mcu_resp_tb.v` drives the responder's SEPROXYHAL framing, the
//   trigger, the exact answer bytes, the end-of-flash clamp, the
//   disabled-pass-through and the framing resets, and
//   `smartcard_mcu_ctrl_tb.v` drives the control protocol and the CRC. Those
//   are CHECKED in simulation. The SEPROXYHAL byte formats, the 0x33 kind-2
//   layout and the 60 00 02 00 00 status are QUOTED from the ledger_console
//   sources and its capture of a real device; the 37-41 us budget is QUOTED
//   from docs/card-bench.md. Nothing here has been on a part yet.
//
//   Pin map (PMOD A, 3.3 V): A1 = C9 VCC, A2 = B9 RST, A3 = D11 CLK,
//   A4 = C12 I/O — the same contact `smartcard_serial.v` uses.

module smartcard_serial_mcu #(
    parameter integer CARD_DIV   = 8,         // card clock = 60 MHz / CARD_DIV = 7.5 MHz
    parameter integer VCC_SETTLE = 600_000,   // sys cycles from VCC to CLK (10 ms)
    parameter integer RST_HOLD   = 500,       // card clocks RST stays low after CLK starts
    parameter integer DEACT_STEP = 6_000      // sys cycles between deactivation steps (100 us)
) (
    input  wire        clk,            // A8, 60 MHz
    output wire        card_vcc,       // C9  (PMOD A1)
    output wire        card_rst,       // B9  (PMOD A2), high = run
    output wire        card_clk,       // D11 (PMOD A3)
    inout  wire        card_io,        // C12 (PMOD A4), open drain, internal pull-up
    inout  wire [7:0]  ulpi_data,
    input  wire        ulpi_dir,
    input  wire        ulpi_nxt,
    output wire        ulpi_stp,
    output wire        ulpi_rst_n,
    output wire        ulpi_clk,
    output wire        led0_n,         // card powered
    output wire        led1_n,         // a byte from the card
    output wire        led2_n,         // USB configured
    output wire        led3_n,         // the responder answered a flash read
    output wire        led4_n,         // the image is loaded and enabled
    output wire        led5_n          // card running (out of reset)
);
    localparam integer DEFAULT_ETU = 372 * CARD_DIV;   // the ATR's rate

    // ---- power-on reset ----
    reg [15:0] por = 16'd0;
    always @(posedge clk) por <= {por[14:0], 1'b1};
    wire rst_n = por[15];

    // ---- USB: a composite CDC serial port + vendor control interface ----
    wire [31:0] baud;
    wire [7:0]  parity;
    wire        dtr, configured, phy_ready, usb_reset;
    wire [7:0]  out_data; wire out_valid, out_ready;
    wire [7:0]  in_data;  wire in_valid, in_ready;
    wire [7:0]  ulpi_data_o; wire ulpi_data_oe;
    wire        running, io_on;
    wire [15:0] etu_div;

    // The vendor control interface's byte streams.
    wire [7:0]  vout_data; wire vout_valid, vout_last, vout_ready;
    wire [7:0]  vin_data;  wire vin_valid, vin_ready, vin_commit;

    usb_cdc_vendor_ulpi #(.VENDOR_ADDR(6'h39), .VENDOR_DATA(8'h06)) acm (
        .clk60(clk), .rst_n(rst_n),
        .ulpi_data_i(ulpi_data), .ulpi_data_o(ulpi_data_o), .ulpi_data_oe(ulpi_data_oe),
        .ulpi_dir(ulpi_dir), .ulpi_nxt(ulpi_nxt), .ulpi_stp(ulpi_stp), .ulpi_rst_n(ulpi_rst_n),
        .address(), .configured(configured), .usb_reset(usb_reset), .phy_ready(phy_ready),
        // CDC serial port, the card's byte stream (unchanged from smartcard_serial.v).
        .out_data(out_data), .out_valid(out_valid), .out_last(), .out_ready(out_ready),
        .in_data(in_data), .in_valid(in_valid), .in_ready(in_ready), .in_commit(in_valid & in_ready),
        .baud(baud), .char_format(), .parity(parity), .data_bits(), .dtr(dtr), .rts(),
        .serial_state({5'b00000, 1'b1, running}),   // DSR always, DCD while running
        // Vendor control interface.
        .vout_data(vout_data), .vout_valid(vout_valid), .vout_last(vout_last), .vout_ready(vout_ready),
        .vin_data(vin_data), .vin_valid(vin_valid), .vin_ready(vin_ready), .vin_commit(vin_commit)
    );
    assign ulpi_data = ulpi_data_oe ? ulpi_data_o : 8'bz;
    assign ulpi_clk  = clk;

    // ---- power, clock, reset and the bit rate: `smartcard_seq` in smartcard_serial.v ----
    smartcard_seq #(
        .CARD_DIV(CARD_DIV), .VCC_SETTLE(VCC_SETTLE), .RST_HOLD(RST_HOLD), .DEACT_STEP(DEACT_STEP)
    ) seq (
        .clk(clk), .rst_n(rst_n), .dtr(dtr), .configured(configured), .baud(baud),
        .card_vcc(card_vcc), .card_rst(card_rst), .card_clk(card_clk),
        .io_on(io_on), .running(running), .etu_div(etu_div)
    );

    // ---- the character layer ----
    wire [7:0] u_rx_data; wire u_rx_valid;
    wire       u_tx_ready;
    wire       io_oe, io_o;
    wire [7:0] tx_q; wire tx_empty, tx_full;

    // The responder owns the UART's transmit port ahead of the host's FIFO.
    wire [7:0] resp_tx_data; wire resp_tx_valid, resp_tx_active, resp_tx_ready;
    wire [7:0] u_tx_data  = resp_tx_active ? resp_tx_data : tx_q;
    wire       u_tx_valid = resp_tx_active ? resp_tx_valid : (~tx_empty & running);

    iso7816_uart #(.ETU_DIV(DEFAULT_ETU), .PARITY_RETRY(1)) u (
        .clk(clk), .rst_n(rst_n), .active(io_on),
        .etu_div(etu_div), .guard_etu(8'd0), .convention(parity == 8'd1),
        .wt_etu(24'hFFFFFF),
        .tx_data(u_tx_data), .tx_valid(u_tx_valid), .tx_ready(u_tx_ready), .tx_abort(),
        .rx_data(u_rx_data), .rx_ready(1'b1), .rx_valid(u_rx_valid),
        .rx_parity_error(), .rx_overrun(), .rx_timeout(),
        .io_i(card_io), .io_oe(io_oe), .io_o(io_o),
        .tx_char_count(), .rx_char_count(), .parity_error_count(),
        .repeat_count(), .timeout_count()
    );
    assign card_io = (!io_on || (io_oe && !io_o)) ? 1'b0 : 1'bz;
    assign resp_tx_ready = resp_tx_active & u_tx_ready;

    // host -> card. Frozen while the responder owns the line, so the answer
    // goes out whole and ahead of any bytes already queued.
    wire tx_take = ~resp_tx_active & ~tx_empty & running & u_tx_ready;
    fifo_sync #(.WIDTH(8), .DEPTH(512), .FWFT(1), .ADDR_WIDTH(9), .CNT_WIDTH(10)) txfifo (
        .clk(clk), .rst_n(rst_n & running),
        .wr_en(out_valid & ~tx_full & running), .wr_data(out_data), .full(tx_full),
        .rd_en(tx_take), .rd_data(tx_q), .empty(tx_empty), .count()
    );
    assign out_ready = ~tx_full | ~running;

    // card -> host
    wire rx_empty, rx_full;
    fifo_sync #(.WIDTH(8), .DEPTH(1024), .FWFT(1), .ADDR_WIDTH(10), .CNT_WIDTH(11)) rxfifo (
        .clk(clk), .rst_n(rst_n),
        .wr_en(u_rx_valid & ~rx_full & io_on), .wr_data(u_rx_data), .full(rx_full),
        .rd_en(in_valid & in_ready), .rd_data(in_data), .empty(rx_empty), .count()
    );
    assign in_valid = ~rx_empty;

    // ---- the 32 KB flash image, one write port and one read port ----
    wire        img_wr;
    wire [14:0] img_wr_addr;
    wire [7:0]  img_wr_data;
    // The read port is shared: the responder has priority, the CRC walk takes
    // it when the responder is idle.
    wire [14:0] resp_rd_addr; wire resp_rd_en;
    wire [14:0] crc_rd_addr;
    wire [7:0]  img_rd_data;
    wire [14:0] img_rd_addr = resp_rd_en ? resp_rd_addr : crc_rd_addr;

    ram_sdp #(.WIDTH(8), .DEPTH(32768)) img (
        .wr_clk(clk), .wr_en(img_wr), .wr_addr(img_wr_addr), .wr_data(img_wr_data),
        .rd_clk(clk), .rd_en(1'b1), .rd_addr(img_rd_addr), .rd_data(img_rd_data)
    );

    // ---- the responder: watch the element's bytes, answer kind-2 reads ----
    wire        en_answer, img_loaded;
    wire        trig_stb, answered_stb, resp_busy;
    wire [79:0] notice;        // {off[31:0], len[15:0], latency[31:0]}
    wire        notice_stb;

    mcu_flash_resp resp (
        .clk(clk), .rst_n(rst_n),
        .running(running),
        .rx_data(u_rx_data), .rx_valid(u_rx_valid),
        .enabled(en_answer), .loaded(img_loaded),
        .rd_addr(resp_rd_addr), .rd_en(resp_rd_en), .rd_data(img_rd_data),
        .tx_data(resp_tx_data), .tx_valid(resp_tx_valid), .tx_active(resp_tx_active),
        .tx_ready(resp_tx_ready),
        .trig_stb(trig_stb), .answered_stb(answered_stb), .busy(resp_busy),
        .notice(notice), .notice_stb(notice_stb)
    );

    // ---- the control interface: upload, enable, status, CRC, notices ----
    mcu_ctrl ctrl (
        .clk(clk), .rst_n(rst_n), .usb_reset(usb_reset),
        .running(running),
        .vout_data(vout_data), .vout_valid(vout_valid), .vout_ready(vout_ready),
        .vin_data(vin_data), .vin_valid(vin_valid), .vin_ready(vin_ready), .vin_commit(vin_commit),
        .img_wr(img_wr), .img_wr_addr(img_wr_addr), .img_wr_data(img_wr_data),
        .crc_rd_addr(crc_rd_addr), .crc_rd_data(img_rd_data), .resp_busy(resp_busy),
        .enabled(en_answer), .loaded(img_loaded),
        .trig_stb(trig_stb), .answered_stb(answered_stb),
        .notice(notice), .notice_stb(notice_stb)
    );

    // ---- LEDs ----
    reg [25:0] hb = 26'd0; always @(posedge clk) hb <= hb + 1'b1;
    reg rx_led = 1'b0, ans_led = 1'b0;
    always @(posedge clk) begin
        if (u_rx_valid)    rx_led  <= ~rx_led;
        if (answered_stb)  ans_led <= ~ans_led;
    end
    assign led0_n = ~card_vcc;
    assign led1_n = ~rx_led;
    assign led2_n = ~configured;
    assign led3_n = ~ans_led;
    assign led4_n = ~(img_loaded & en_answer);
    assign led5_n = ~running;
endmodule
