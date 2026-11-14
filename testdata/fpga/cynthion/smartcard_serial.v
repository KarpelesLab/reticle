// A bare ISO 7816 card or secure element on a Cynthion's PMOD A, with the card's
// contact exposed to the host as a plain USB serial port on the AUX connector.
//
// Nothing about the protocol above the character layer lives here: the host
// does the ATR decode, PPS, T=0/T=1 and every command. The serial port's own
// controls carry the rest, so a program needs no special driver:
//
//   * DTR raised  -> activation: VCC, then CLK, then RST released after
//                    RST_HOLD card clocks. The ATR then arrives as bytes.
//   * DTR dropped -> deactivation: RST, CLK and I/O low, then VCC low.
//   * baud rate   -> the elementary time unit: etu = 60 MHz / baud, rounded.
//                    The card clock is 60 MHz / CARD_DIV = 7.5 MHz, so the
//                    ATR's rate (F = 372, D = 1) is 7.5e6 / 372 = 20161 baud,
//                    and after a PPS the host sets 7.5e6 / (F/D): 1875000 baud
//                    for PPS1 = 87 (F/D = 4, what a Ledger SE takes; see
//                    docs/card-bench.md), exactly 32 cycles of 60 MHz per bit.
//                    8 MHz would need a PLL and a second clock domain; 7.5 is
//                    the nearest even divide, so the duty cycle is 50 %.
//   * parity      -> odd selects the inverse convention (TS = 3F); anything
//                    else is the direct convention (TS = 3B).
//   * bytes written go to the card; bytes the card sends come back. The line
//     is half duplex and a byte this end sends is not echoed back.
//   * serial_state: DCD is set while the card is powered and out of reset.
//
// Pin map (PMOD A, 3.3 V): A1 = C9 VCC, A2 = B9 RST, A3 = D11 CLK, A4 = C12 I/O.
//
// CHECKED on 2026-11-14 with a Ledger Nano S's secure element (an ST31) wired
// to PMOD A as above, VCC straight from C9 at the pin's default drive:
//   * ATR 3B 90 98 10 00 at 20161 baud: T=0, TA1 = 98 (Fi 512, Di 12), TA2 = 00.
//   * PPS FF 10 98 77 and FF 10 87 68 both echoed exactly; at 1875000 baud
//     (PPS1 87, F/D = 4) the SE answered a Nano S MCU's SESSION_START
//     (01 00 0A 00 10 00 02 21 04 31 2E 36 00, taken from a logic capture of
//     a real device) with a well-formed SEPROXYHAL turn: 32 00 03 00 F4 01,
//     then the status 60 00 02 00 00.
//   * The first PPS attempt echoed as 83 98 77: with an extra guard time of 2
//     etu after our last character the receiver was deaf to the card's start
//     bit, took FF's parity bit for one and read 83. guard_etu is TC1, the
//     guard *beyond* the mandatory 2 etu, and is 0 now.
//   * smartcard_atr.py is the host side used for all of the above.
// What is NOT checked: whether the pin holds VCC up while the SE is busy (a
// long crypto operation or a flash write); nothing heavier than the turn
// above has been run. smartcard_seq_tb.v holds the power order.
//
// **VCC comes from a logic output.** An ECP5 LVCMOS33 output is rated for a
// few mA, which a secure element at rest can live on and a busy one may not.
// Every pin is driven low while the card is off -- VCC included, so the I/O
// pin's internal pull-up cannot back-power the card through its protection
// diodes -- and nothing but this sequencer ever raises VCC.

module smartcard_serial #(
    parameter integer CARD_DIV   = 8,         // card clock = 60 MHz / CARD_DIV = 7.5 MHz (even: 50 % duty)
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
    output wire        led3_n,         // heartbeat
    output wire        led4_n,         // a byte to the card
    output wire        led5_n          // card running (out of reset)
);
    localparam integer DEFAULT_ETU = 372 * CARD_DIV;   // the ATR's rate

    // ---- power-on reset ----
    reg [15:0] por = 16'd0;
    always @(posedge clk) por <= {por[14:0], 1'b1};
    wire rst_n = por[15];

    // ---- USB serial port ----
    wire [31:0] baud;
    wire [7:0]  parity;
    wire        dtr, configured, phy_ready;
    wire [7:0]  out_data; wire out_valid, out_ready;
    wire [7:0]  in_data;  wire in_valid, in_ready;
    wire [7:0]  ulpi_data_o; wire ulpi_data_oe;
    wire        running, io_on;
    wire [15:0] etu_div;

    usb_cdc_acm_ulpi #(.VENDOR_ADDR(6'h39), .VENDOR_DATA(8'h06)) acm (
        .clk60(clk), .rst_n(rst_n),
        .ulpi_data_i(ulpi_data), .ulpi_data_o(ulpi_data_o), .ulpi_data_oe(ulpi_data_oe),
        .ulpi_dir(ulpi_dir), .ulpi_nxt(ulpi_nxt), .ulpi_stp(ulpi_stp), .ulpi_rst_n(ulpi_rst_n),
        .address(), .configured(configured), .usb_reset(), .phy_ready(phy_ready),
        .out_data(out_data), .out_valid(out_valid), .out_last(), .out_ready(out_ready),
        .in_data(in_data), .in_valid(in_valid), .in_ready(in_ready), .in_commit(in_valid & in_ready),
        .baud(baud), .char_format(), .parity(parity), .data_bits(), .dtr(dtr), .rts(),
        .serial_state({5'b00000, 1'b1, running})   // DSR always, DCD while running
    );
    assign ulpi_data = ulpi_data_oe ? ulpi_data_o : 8'bz;
    assign ulpi_clk  = clk;

    // ---- power, clock, reset and the bit rate: see `smartcard_seq` below ----
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
    iso7816_uart #(.ETU_DIV(DEFAULT_ETU), .PARITY_RETRY(1)) u (
        .clk(clk), .rst_n(rst_n), .active(io_on),
        // guard_etu is TC1, the guard *beyond* the mandatory 2 etu. 0 here: a card
        // answers 2 etu after our last character, and a longer guard leaves the
        // receiver deaf to its start bit (a PPS echo FF 10 .. read as 83 ..).
        .etu_div(etu_div), .guard_etu(8'd0), .convention(parity == 8'd1),
        .wt_etu(24'hFFFFFF),
        .tx_data(tx_q), .tx_valid(~tx_empty & running), .tx_ready(u_tx_ready), .tx_abort(),
        .rx_data(u_rx_data), .rx_ready(1'b1), .rx_valid(u_rx_valid),
        .rx_parity_error(), .rx_overrun(), .rx_timeout(),
        .io_i(card_io), .io_oe(io_oe), .io_o(io_o),
        .tx_char_count(), .rx_char_count(), .parity_error_count(),
        .repeat_count(), .timeout_count()
    );
    // Open drain while the UART owns the line; driven low while the card is off.
    assign card_io = (!io_on || (io_oe && !io_o)) ? 1'b0 : 1'bz;

    // host -> card. Bytes written while the card is off are dropped, so a
    // program never blocks on a dead card.
    wire tx_take = ~tx_empty & running & u_tx_ready;
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

    // ---- LEDs ----
    reg [25:0] hb = 26'd0; always @(posedge clk) hb <= hb + 1'b1;
    reg rx_led = 1'b0, tx_led = 1'b0;
    always @(posedge clk) begin
        if (u_rx_valid) rx_led <= ~rx_led;
        if (tx_take)    tx_led <= ~tx_led;
    end
    assign led0_n = ~card_vcc;
    assign led1_n = ~rx_led;
    assign led2_n = ~configured;
    assign led3_n = ~hb[25];
    assign led4_n = ~tx_led;
    assign led5_n = ~running;
endmodule

// The card's power, clock and reset, and the bit rate the host asked for.
//
// Its own module so `smartcard_seq_tb.v` can hold it to the order that keeps
// a card safe: VCC before CLK before RST on the way up, the reverse on the way
// down, and every line low while the card is off.
module smartcard_seq #(
    parameter integer CARD_DIV   = 8,
    parameter integer VCC_SETTLE = 600_000,
    parameter integer RST_HOLD   = 500,
    parameter integer DEACT_STEP = 6_000
) (
    input  wire        clk,            // 60 MHz
    input  wire        rst_n,
    input  wire        dtr,            // the host holds the port open with DTR raised
    input  wire        configured,     // USB configured
    input  wire [31:0] baud,           // the host's line coding
    output wire        card_vcc,
    output wire        card_rst,
    output wire        card_clk,
    output reg         io_on = 1'b0,   // the UART owns the I/O line; low = drive it low
    output reg         running = 1'b0, // powered and out of reset
    output reg  [15:0] etu_div = 372 * CARD_DIV   // 60 MHz cycles per bit
);
    localparam integer DEFAULT_ETU = 372 * CARD_DIV;

    // ---- baud -> etu: a restoring divide, 60e6 / baud rounded, redone on change ----
    localparam [31:0] SYS_HZ = 32'd60_000_000;
    reg  [31:0] last_baud = 32'd0;
    reg  [5:0]  div_step = 6'd0;
    reg         dividing = 1'b0;
    reg  [32:0] rem = 33'd0;
    reg  [31:0] quo = 32'd0;
    reg  [31:0] num = 32'd0;
    always @(posedge clk) begin
        if (!rst_n) begin
            last_baud <= 32'd0; dividing <= 1'b0;
        end else if (!dividing) begin
            if (baud != last_baud) begin
                last_baud <= baud;
                if (baud >= 32'd916 && baud <= 32'd6_000_000) begin
                    // round: (2 * SYS_HZ / baud + 1) / 2, done as (SYS_HZ + baud/2) / baud
                    num <= SYS_HZ + (baud >> 1);
                    rem <= 33'd0; quo <= 32'd0; div_step <= 6'd0; dividing <= 1'b1;
                end
            end
        end else begin
            // one bit of the quotient per cycle, MSB first
            if ({rem[31:0], num[31]} >= {1'b0, last_baud}) begin
                rem <= {rem[31:0], num[31]} - {1'b0, last_baud};
                quo <= {quo[30:0], 1'b1};
            end else begin
                rem <= {rem[31:0], num[31]};
                quo <= {quo[30:0], 1'b0};
            end
            num <= {num[30:0], 1'b0};
            div_step <= div_step + 6'd1;
            if (div_step == 6'd31) dividing <= 1'b0;
        end
    end
    // The last quotient bit lands in the same cycle `dividing` drops, so read
    // the finished value one cycle later.
    reg        div_was = 1'b0;

    always @(posedge clk) begin
        div_was <= dividing;
        if (!rst_n) etu_div <= DEFAULT_ETU[15:0];
        else if (div_was && !dividing) etu_div <= (quo[31:16] != 16'd0) ? 16'hFFFF : quo[15:0];
    end

    // ---- card clock: 60 MHz / CARD_DIV, 50 % duty, gated low while stopped ----
    reg [$clog2(CARD_DIV)-1:0] cdiv = 0;
    reg clk_run = 1'b0, card_clk_r = 1'b0;
    always @(posedge clk) begin
        if (!rst_n || !clk_run) begin cdiv <= 0; card_clk_r <= 1'b0; end
        else if (cdiv == (CARD_DIV/2 - 1)) begin cdiv <= 0; card_clk_r <= ~card_clk_r; end
        else cdiv <= cdiv + 1'b1;
    end
    assign card_clk = card_clk_r;

    // ---- activation and deactivation ----
    localparam [2:0] S_OFF = 3'd0, S_VCC = 3'd1, S_CLK = 3'd2, S_RUN = 3'd3,
                     S_D_RST = 3'd4, S_D_CLK = 3'd5, S_D_IO = 3'd6;
    reg [2:0]  state = S_OFF;
    reg [23:0] wait_cnt = 24'd0;
    reg        vcc_r = 1'b0, rst_r = 1'b0;
    always @(posedge clk) begin
        if (!rst_n) begin
            state <= S_OFF; vcc_r <= 1'b0; rst_r <= 1'b0; clk_run <= 1'b0;
            io_on <= 1'b0; running <= 1'b0; wait_cnt <= 24'd0;
        end else begin
            wait_cnt <= wait_cnt + 24'd1;
            case (state)
                S_OFF: if (dtr && configured) begin
                    vcc_r <= 1'b1; io_on <= 1'b1;   // VCC up, I/O released to its pull-up
                    wait_cnt <= 24'd0; state <= S_VCC;
                end
                S_VCC: if (!dtr) state <= S_D_RST;
                    else if (wait_cnt == VCC_SETTLE) begin
                        clk_run <= 1'b1; wait_cnt <= 24'd0; state <= S_CLK;
                    end
                S_CLK: if (!dtr) state <= S_D_RST;
                    else if (wait_cnt == RST_HOLD * CARD_DIV) begin
                        rst_r <= 1'b1; running <= 1'b1; state <= S_RUN;
                    end
                S_RUN: if (!dtr || !configured) state <= S_D_RST;
                S_D_RST: begin
                    rst_r <= 1'b0; running <= 1'b0; wait_cnt <= 24'd0; state <= S_D_CLK;
                end
                S_D_CLK: if (wait_cnt == DEACT_STEP) begin
                    clk_run <= 1'b0; wait_cnt <= 24'd0; state <= S_D_IO;
                end
                S_D_IO: begin
                    io_on <= 1'b0;   // I/O driven low from here
                    if (wait_cnt == DEACT_STEP) begin
                        vcc_r <= 1'b0; state <= S_OFF;
                    end
                end
                default: state <= S_OFF;
            endcase
        end
    end
    assign card_vcc = vcc_r;
    assign card_rst = rst_r;
endmodule
