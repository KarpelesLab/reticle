// A Ledger Nano S secure element on a Cynthion's AUX-side card contact, brought up by
// `ip/bus/seph_mcu` over `ip/bus/iso7816_uart`, with every byte the SE sends streamed to a
// host USB serial port by `ip/usb/usb_cdc_acm`.
//
// This is the Nano S counterpart of the Nano X card bench in `examples/basys3/iso7816_terminal.v`.
// The Nano S differs from the Nano X at the contact and in the session blob, and all of it is a
// parameter here (confirmed on a real ST31, 2026-11-12/13):
//   * 3-wire ISO 7816 T=0 on the board: the MCU supplies CLK and RST, I/O is the open-drain
//     contact. We generate CLK (CARD_DIV from the 60 MHz oscillator) and a cold-reset on RST.
//   * The ST31 echoes PPS but stays at the ATR rate, so there is NO fast-etu switch: one rate,
//     etu = Fi/Di = 512/12 = 372 card clocks, throughout. No PPS is sent (not needed).
//   * SESSION_START is the small Nano S blob `01 00 0A 00 00000221 04 "1.1\0"` (USB | 2 buttons |
//     small screen), not the Nano X 48-byte identity.
// `seph_mcu`'s turn engine is protocol-generic (MCU events 0x01-0x1F, SE commands 0x30-0x5F, SE
// status 0x60-0x6F ends the turn), so it keeps the ST31 alive; we capture the whole byte stream.
//
// The host side needs no driver and no program: the AUX port enumerates as /dev/ttyACM* (1209:0001).
// RAISE DTR when you open it, then read:  stty -F /dev/ttyACMx raw; <open with DTR> cat.
//
// Build (reticle):
//   reticle fpga --device ecp5-12f-CABGA256 --constraints nanos_seph_usb.rcf \
//     nanos_seph_usb.v \
//     ../../../ip/bus/iso7816_uart/rtl/iso7816_uart.v \
//     ../../../ip/bus/seph_mcu/rtl/seph_mcu.v \
//     ../../../ip/memory/fifo_sync/rtl/fifo_sync.v \
//     ../../../ip/usb/usb_device_fs/rtl/*.v ../../../ip/usb/usb_device_ulpi/rtl/*.v \
//     ../../../ip/usb/usb_cdc_acm/rtl/*.v
//   (then nextpnr-ecp5 + ecppack, or reticle's bitstream step; program with `reticle program`)

module nanos_seph_usb #(
    parameter integer CARD_DIV    = 16,          // card clock = 60 MHz / CARD_DIV = 3.75 MHz
    parameter integer ETU_CYCLES  = 372,         // Fi/Di card clocks per bit
    parameter integer RST_HOLD    = 400,         // card clocks to hold RST low (cold reset)
    parameter integer ATR_WAIT    = 2_400_000,   // sys cycles after RST release before SESSION_START (~40 ms)
    parameter integer SEPH_MS     = 60_000,      // sys cycles per ms (60 MHz)
    parameter integer SEPH_TURN   = 6_000        // sys cycles a turn waits
) (
    input  wire        clk,            // A8, 60 MHz
    // Nano S card contact
    output wire        card_clk,       // B9  -> SE CLK (pin 5)
    output wire        card_rst,       // C9  -> SE RST (pin 6), high = run
    inout  wire        card_io,        // D11 -> SE I/O (pin 3), open drain, external 10k pull-up
    // AUX ULPI transceiver
    inout  wire [7:0]  ulpi_data,      // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire        ulpi_dir,       // E16
    input  wire        ulpi_nxt,       // F15
    output wire        ulpi_stp,       // E15
    output wire        ulpi_rst_n,     // J13 (active low at the ball)
    output wire        ulpi_clk,       // D16 (the FPGA owes the PHY the 60 MHz)
    // LEDs (active low)
    output wire        led0_n,         // booted (seph sent SESSION_START)
    output wire        led1_n,         // a byte arrived from the SE
    output wire        led2_n,         // USB configured
    output wire        led3_n,         // heartbeat
    output wire        led4_n,
    output wire        led5_n
);
    localparam integer ETU_DIV = ETU_CYCLES * CARD_DIV;   // sys cycles per bit

    // ---- power-on reset ----
    reg [15:0] por = 16'd0;
    always @(posedge clk) por <= {por[14:0], 1'b1};
    wire rst_n = por[15];

    // ---- card clock: 60 MHz / CARD_DIV, 50% duty ----
    reg [$clog2(CARD_DIV)-1:0] cdiv = 0;
    reg card_clk_r = 1'b0;
    always @(posedge clk) begin
        if (!rst_n) begin cdiv <= 0; card_clk_r <= 1'b0; end
        else if (cdiv == (CARD_DIV/2 - 1)) begin cdiv <= 0; card_clk_r <= ~card_clk_r; end
        else cdiv <= cdiv + 1'b1;
    end
    assign card_clk = card_clk_r;

    // ---- activation: hold RST low RST_HOLD card clocks, release, go active, then start seph ----
    reg card_rst_r = 1'b0, active = 1'b0, started = 1'b0, start_pulse = 1'b0;
    reg [27:0] actcnt = 28'd0, startcnt = 28'd0;
    always @(posedge clk) begin
        start_pulse <= 1'b0;
        if (!rst_n) begin
            card_rst_r <= 1'b0; active <= 1'b0; started <= 1'b0;
            actcnt <= 0; startcnt <= 0;
        end else begin
            if (actcnt < RST_HOLD*CARD_DIV) actcnt <= actcnt + 1'b1;
            else begin card_rst_r <= 1'b1; active <= 1'b1; end   // release reset, enable the UART
            if (card_rst_r && !started) begin
                if (startcnt < ATR_WAIT) startcnt <= startcnt + 1'b1;
                else begin started <= 1'b1; start_pulse <= 1'b1; end
            end
        end
    end
    assign card_rst = card_rst_r;

    // ---- ISO 7816 T=0 UART on the contact ----
    wire [7:0] u_rx_data; wire u_rx_valid;
    wire [7:0] u_tx_data; wire u_tx_valid, u_tx_ready;
    wire io_oe, io_o;
    iso7816_uart #(.ETU_DIV(ETU_DIV), .PARITY_RETRY(0)) u (
        .clk(clk), .rst_n(rst_n), .active(active),
        .etu_div(ETU_DIV[15:0]), .guard_etu(8'd2), .convention(1'b0), .wt_etu(24'd4000000),
        .tx_data(u_tx_data), .tx_valid(u_tx_valid), .tx_ready(u_tx_ready), .tx_abort(),
        .rx_data(u_rx_data), .rx_ready(1'b1), .rx_valid(u_rx_valid),
        .rx_parity_error(), .rx_overrun(), .rx_timeout(),
        .io_i(card_io), .io_oe(io_oe), .io_o(io_o),
        .tx_char_count(), .rx_char_count(), .parity_error_count(),
        .repeat_count(), .timeout_count()
    );
    assign card_io = (io_oe && !io_o) ? 1'b0 : 1'bz;   // open drain: drive low, else release

    // ---- the Nano S SEPROXYHAL MCU ----
    // SESSION_START = 01 00 0A | 00 | 00 00 02 21 | 04 | "1.1\0"   (13 bytes, right-aligned)
    wire [7:0] s_tx_data; wire s_tx_valid; wire s_booted;
    seph_mcu #(
        .MS_CYCLES(SEPH_MS), .TURN_CYCLES(SEPH_TURN),
        .SESSION_LEN(13),
        .SESSION({408'd0, 104'h01000a000000022104312e3100})
    ) seph (
        .clk(clk), .rst_n(rst_n), .start(start_pulse), .stop(1'b0),
        .rx_data(u_rx_data), .rx_valid(u_rx_valid & started),   // keep the raw ATR out of the parser
        .tx_data(s_tx_data), .tx_valid(s_tx_valid), .tx_ready(u_tx_ready),
        .buttons(2'b00), .usb_plugged(1'b0), .usb_on(),
        .inj_start(1'b0), .inj_data(8'd0), .inj_valid(1'b0), .inj_commit(1'b0),
        .inj_busy(), .inj_bad(),
        .active(), .started(s_booted), .rx_packets(), .tx_events(), .last_rx_tag(), .ms()
    );
    assign u_tx_data  = s_tx_data;
    assign u_tx_valid = s_tx_valid;

    // ---- readback: every SE byte -> FIFO -> USB CDC IN (device -> host) ----
    wire fifo_empty, fifo_full;
    wire [7:0] fifo_rd;
    wire in_ready, give;
    assign give = ~fifo_empty & in_ready;
    fifo_sync #(.WIDTH(8), .DEPTH(1024), .FWFT(1), .ADDR_WIDTH(10), .CNT_WIDTH(11)) rbfifo (
        .clk(clk), .rst_n(rst_n),
        .wr_en(u_rx_valid & ~fifo_full), .wr_data(u_rx_data), .full(fifo_full),
        .rd_en(give), .rd_data(fifo_rd), .empty(fifo_empty), .count()
    );

    wire configured, phy_ready;
    usb_cdc_acm_ulpi #(.VENDOR_ADDR(6'h39), .VENDOR_DATA(8'h06)) acm (
        .clk60(clk), .rst_n(rst_n),
        .ulpi_data_i(ulpi_data), .ulpi_data_o(ulpi_data_o), .ulpi_data_oe(ulpi_data_oe),
        .ulpi_dir(ulpi_dir), .ulpi_nxt(ulpi_nxt), .ulpi_stp(ulpi_stp), .ulpi_rst_n(ulpi_rst_n),
        .address(), .configured(configured), .usb_reset(), .phy_ready(phy_ready),
        .out_data(), .out_valid(), .out_last(), .out_ready(1'b1),
        .in_data(fifo_rd), .in_valid(give), .in_ready(in_ready), .in_commit(give),
        .baud(), .char_format(), .parity(), .data_bits(), .dtr(), .rts(),
        .serial_state(7'b000_0011)
    );
    // ULPI data bus tristate + clock to the PHY
    wire [7:0] ulpi_data_o; wire ulpi_data_oe;
    assign ulpi_data = ulpi_data_oe ? ulpi_data_o : 8'bz;
    assign ulpi_clk  = clk;

    // ---- LEDs ----
    reg [25:0] hb = 0; always @(posedge clk) hb <= hb + 1'b1;
    reg rx_led = 1'b0; always @(posedge clk) if (u_rx_valid) rx_led <= ~rx_led;
    assign led0_n = ~s_booted;
    assign led1_n = ~rx_led;
    assign led2_n = ~configured;
    assign led3_n = ~hb[25];
    assign led4_n = ~phy_ready;
    assign led5_n = 1'b1;
endmodule
