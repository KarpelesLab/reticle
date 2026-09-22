// eth_mac_rgmii — a gigabit Ethernet MAC over RGMII.
//
// What it does
//   The frame logic of `eth_mac_rmii` — the same `eth_mac_tx` and
//   `eth_mac_rx`, eight bits a cycle instead of two — behind an RGMII
//   front end. RGMII is double data rate: at 1000 Mbit/s a 125 MHz
//   clock carries a nibble on each edge, the low nibble of the octet on
//   the rising edge and the high nibble on the falling one, and the
//   control line carries the enable on the rising edge and the enable
//   XOR the error on the falling one. Every one of those pins goes
//   through a double-data-rate IO register, asked for with a `ddr`
//   attribute on its port, so an eight-bit port here is the four pins of
//   a nibble bus and needs no reordering: the register's two halves are
//   exactly the two nibbles.
//
//   Two clock domains, one per direction, as the interface has them.
//   Transmit runs on `tx_clk`, the 125 MHz the MAC is given, and
//   forwards it to the PHY as TXC through a DDR output register driven
//   2'b01, so TXC rises as each octet's low nibble starts. Receive runs
//   on RXC, the clock the PHY sends with its data; the receive DDR
//   registers are clocked by it directly. Nothing crosses between the
//   two, so the user's transmit side is in the `tx_clk` domain and the
//   receive side in the RXC domain; put `fifo_async` on either if the
//   rest of the design runs elsewhere.
//
//   The clock relationship. RGMII launches clock and data together, edge
//   aligned, and the receiving end needs the clock about 2 ns later than
//   the data to sample in the middle of the eye. Either the PHY adds
//   that skew internally (RGMII-ID, which most current PHYs do by
//   default or by a strap) or the MAC must. TX_DELAY puts that many
//   steps of the device's IO delay after TXC, and RX_DELAY that many
//   before the receive data and control registers; about 80 steps is
//   2 ns on the ECP5's DELAYG. Both default to 0, for a PHY that adds
//   the delays itself.
//
// What it does not do
//   Gigabit only. RGMII at 10 and 100 Mbit/s is a 2.5 or 25 MHz clock
//   with a nibble per cycle rather than per edge, and this block does
//   not switch speeds. No in-band status: the link state a PHY may put
//   on RXD between frames is not decoded. The control line's error
//   encodings outside a frame (false carrier, carrier extension) are
//   ignored, since a full-duplex MAC has no use for them.
//
//   Everything `eth_mac_rmii`'s header lists: no padding, no address
//   filter, no pause frames, no statistics, no MDIO, no back-pressure on
//   receive, and an underrun on transmit sends zeros and raises
//   `tx_underrun`.
//
//   A delay of 0 still asks for the delay element, set to nothing, on
//   the ECP5, and draws a warning on the iCE40, which has no delay and
//   whose IO buffer registers both edges itself; on the iCE40 the PHY
//   must add both delays.
module eth_mac_rgmii #(
    // Clock cycles of inter-frame gap: 12 is the standard's 96 bit
    // times at eight bits a cycle.
    parameter IFG_CYCLES = 12,
    // IO delay steps after TXC and before the receive registers.
    parameter TX_DELAY   = 0,
    parameter RX_DELAY   = 0
) (
    input  wire       tx_clk,
    input  wire       rst_n,

    // The RGMII pins. Each `ddr` port is two bits per pin.
    (* ddr = "tx_clk", io_delay = TX_DELAY *)
    output wire [1:0] rgmii_txc,
    (* ddr = "tx_clk" *)
    output wire [7:0] rgmii_txd,
    (* ddr = "tx_clk" *)
    output wire [1:0] rgmii_tx_ctl,
    input  wire       rgmii_rxc,
    (* ddr = "rgmii_rxc", io_delay = RX_DELAY *)
    input  wire [7:0] rgmii_rxd,
    (* ddr = "rgmii_rxc", io_delay = RX_DELAY *)
    input  wire [1:0] rgmii_rx_ctl,

    // Transmit, user side, in the `tx_clk` domain: an octet a cycle.
    input  wire [7:0] tx_data,
    input  wire       tx_valid,
    output wire       tx_ready,
    input  wire       tx_last,
    output wire       tx_busy,
    output wire       tx_underrun,

    // Receive, user side, in the `rgmii_rxc` domain.
    output wire [7:0] rx_data,
    output wire       rx_valid,
    output wire       rx_last,
    output wire       rx_crc_ok,
    output wire       rx_error
);
    // `rst_n` is asserted asynchronously in both domains and released in
    // each by two flip-flops of that domain, so neither half leaves reset
    // on an edge of the other's clock.
    reg [1:0] tx_rst_q;
    reg [1:0] rx_rst_q;
    always @(posedge tx_clk or negedge rst_n) begin
        if (!rst_n) tx_rst_q <= 2'b00;
        else        tx_rst_q <= {tx_rst_q[0], 1'b1};
    end
    always @(posedge rgmii_rxc or negedge rst_n) begin
        if (!rst_n) rx_rst_q <= 2'b00;
        else        rx_rst_q <= {rx_rst_q[0], 1'b1};
    end
    wire tx_rst_n = tx_rst_q[1];
    wire rx_rst_n = rx_rst_q[1];

    wire       tx_en;
    wire [7:0] txd;

    eth_mac_tx #(.DW(8), .IFG_CYCLES(IFG_CYCLES)) u_tx (
        .clk         (tx_clk),
        .rst_n       (tx_rst_n),
        .tx_en       (tx_en),
        .txd         (txd),
        .tx_data     (tx_data),
        .tx_valid    (tx_valid),
        .tx_ready    (tx_ready),
        .tx_last     (tx_last),
        .tx_busy     (tx_busy),
        .tx_underrun (tx_underrun)
    );

    // The DDR output registers take the octet and the enable at each
    // rising edge: the low nibble and the enable for the first half of
    // the next cycle, the high nibble and enable XOR error (no error is
    // ever sent) for the second.
    assign rgmii_txd    = txd;
    assign rgmii_tx_ctl = {tx_en, tx_en};
    assign rgmii_txc    = 2'b01;

    // Receive: the two halves of RX_CTL are the valid and valid XOR
    // error; the two halves of RXD are the octet.
    wire rx_dv = rgmii_rx_ctl[0];
    wire rx_er = rgmii_rx_ctl[0] ^ rgmii_rx_ctl[1];

    eth_mac_rx #(.DW(8)) u_rx (
        .clk       (rgmii_rxc),
        .rst_n     (rx_rst_n),
        .crs_dv    (rx_dv),
        .rxd       (rgmii_rxd),
        .rx_er     (rx_er),
        .rx_data   (rx_data),
        .rx_valid  (rx_valid),
        .rx_last   (rx_last),
        .rx_crc_ok (rx_crc_ok),
        .rx_error  (rx_error)
    );
endmodule
