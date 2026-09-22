// eth_mac_rmii — a 100BASE-TX Ethernet MAC over the RMII interface.
//
// What it does
//   RMII is single data rate: one 50 MHz reference clock shared with the
//   PHY, two bits of data per edge, which is 100 Mbit/s and one octet
//   every four cycles. Nothing here needs a DDR register, a PLL or an IO
//   delay, so it is ordinary logic that any device can build.
//
//   The frame logic is `eth_mac_tx` and `eth_mac_rx`, in this package,
//   two bits a cycle; `eth_mac_rgmii` uses the same two modules eight
//   bits a cycle behind a double-data-rate front end.
//
//   Everything runs in the `ref_clk` domain, the user side included.
//   There is no clock crossing inside the block on purpose: a MAC that
//   also crossed clocks would hide two problems in one box. Put
//   `fifo_async` on each side if the rest of the design runs elsewhere.
//
//   Transmit. The user offers octets on `tx_data` with `tx_valid`, marks
//   the last one of a frame with `tx_last`, and the block takes one every
//   four cycles by pulsing `tx_ready`. A frame starts when `tx_valid`
//   first goes high: the MAC sends seven octets of 0x55 and the 0xD5
//   start-of-frame delimiter, takes the first octet as the SFD ends, then
//   the payload, then four octets of frame check sequence, then holds
//   `tx_en` low for IFG_CYCLES cycles — 48, which is the 96 bit times the
//   standard asks for. Bits go out least significant first, which is what
//   Ethernet means by transmission order.
//
//   The frame check sequence is CRC-32 with the polynomial 0x04C11DB7 in
//   its reflected form, seeded with all ones and complemented at the end,
//   computed two bits at a time as they are transmitted. That is the same
//   arithmetic as a byte-parallel table, at a quarter of the logic,
//   because the wire is two bits wide anyway.
//
//   Receive. `crs_dv` high starts the search for the end of the preamble;
//   the dibit 2'b11 is the top of the 0xD5 delimiter and everything after
//   it is the frame. Octets come out on `rx_data` with `rx_valid`, and
//   the last one carries `rx_last` together with `rx_crc_ok`. The last
//   five octets are held back in a pipeline, so the four bytes of frame
//   check sequence are never presented as data and the final data octet
//   can be marked as such: the receiver cannot know which octet was last
//   until `crs_dv` falls, four octet times later.
//
//   `rx_crc_ok` is the CRC residue test: the check sequence is run over
//   the data *and* the four FCS octets, and a frame that arrived intact
//   leaves exactly 0xDEBB20E3 in the register whatever it contained. A
//   corrupted frame leaves something else, and the octets are still
//   delivered — with `rx_crc_ok` low — because dropping them is the
//   user's decision, not the MAC's.
//
//   `rx_error` pulses for a frame that ended in the middle of an octet,
//   one too short to hold a check sequence, or one the PHY flagged on
//   `rx_er`. Those are malformed frames rather than corrupted ones, and
//   nothing is delivered for them.
//
// What it does not do
//   No padding. A frame shorter than 60 octets goes out short, which is
//   not a legal Ethernet frame; padding it is one loop, and it belongs in
//   whatever assembles the frame, which knows where the header ends. No
//   length check on receive either, and no maximum: a giant is delivered
//   like anything else.
//
//   No addressing. There is no MAC address, no unicast filter, no
//   multicast hash and no promiscuous switch: every frame the PHY hands
//   over is delivered. No VLAN handling, no jumbo frames, no flow control
//   and no pause frames. No statistics counters.
//
//   No back-pressure on receive: a wire cannot be stalled, so `rx_valid`
//   is a strobe and there is no `rx_ready`. Put `fifo_sync` behind it,
//   which is what a store-and-forward MAC is, and use `rx_crc_ok` to
//   decide whether to keep what the FIFO collected.
//
//   On transmit the wire cannot be stalled either. If `tx_valid` is low
//   when the block needs the next octet it transmits 0x00 and raises
//   `tx_underrun`, which stays high until the next frame starts; the
//   frame on the wire is then wrong, and the receiver's check sequence is
//   what says so. Present a whole frame, or put a FIFO in front.
//
//   `crs_dv` is treated as a plain data-valid signal. Real RMII
//   multiplexes carrier sense onto it in the second half of each nibble
//   once the carrier drops mid-frame, and that is not decoded here: the
//   frame ends when `crs_dv` does. No half duplex, so no collision
//   detection, no deferral and no back-off — 100BASE-TX with a modern PHY
//   is full duplex. No management interface: the MDIO pair is a separate
//   block, and this one never touches the PHY's registers.
module eth_mac_rmii #(
    // Reference-clock cycles of inter-frame gap. 48 is the 96 bit times
    // the standard asks for at two bits a cycle.
    parameter IFG_CYCLES = 48
) (
    input  wire       ref_clk,
    input  wire       rst_n,

    // The RMII pins.
    output wire       tx_en,
    output wire [1:0] txd,
    input  wire       crs_dv,
    input  wire [1:0] rxd,
    input  wire       rx_er,

    // Transmit, user side: one octet every four cycles.
    input  wire [7:0] tx_data,
    input  wire       tx_valid,
    output wire       tx_ready,
    input  wire       tx_last,
    output wire       tx_busy,
    output wire       tx_underrun,

    // Receive, user side: a strobe per octet, `rx_last` on the final one.
    output wire [7:0] rx_data,
    output wire       rx_valid,
    output wire       rx_last,
    output wire       rx_crc_ok,
    output wire       rx_error
);
    eth_mac_tx #(.DW(2), .IFG_CYCLES(IFG_CYCLES)) u_tx (
        .clk         (ref_clk),
        .rst_n       (rst_n),
        .tx_en       (tx_en),
        .txd         (txd),
        .tx_data     (tx_data),
        .tx_valid    (tx_valid),
        .tx_ready    (tx_ready),
        .tx_last     (tx_last),
        .tx_busy     (tx_busy),
        .tx_underrun (tx_underrun)
    );

    eth_mac_rx #(.DW(2)) u_rx (
        .clk       (ref_clk),
        .rst_n     (rst_n),
        .crs_dv    (crs_dv),
        .rxd       (rxd),
        .rx_er     (rx_er),
        .rx_data   (rx_data),
        .rx_valid  (rx_valid),
        .rx_last   (rx_last),
        .rx_crc_ok (rx_crc_ok),
        .rx_error  (rx_error)
    );
endmodule
