// eth_mac_rmii — a 100BASE-TX Ethernet MAC over the RMII interface.
//
// What it does
//   RMII is single data rate: one 50 MHz reference clock shared with the
//   PHY, two bits of data per edge, which is 100 Mbit/s and one octet
//   every four cycles. That is the whole reason this block exists and the
//   MII or RGMII ones do not — nothing here needs a DDR register, a PLL
//   or an IO delay, so it is ordinary logic that any device can build.
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
    // What the check sequence leaves behind when a frame and its own FCS
    // have both gone through it.
    localparam [31:0] CRC_RESIDUE = 32'hDEBB_20E3;
    localparam [7:0]  IFG_LAST    = IFG_CYCLES - 1;

    // One bit of the reflected CRC-32, in transmission order.
    //
    // Called from continuous assignments rather than from inside the
    // clocked blocks: a function call in a process with an asynchronous
    // reset makes this compiler report its locals as unreset registers
    // (see `function_locals_are_reported_as_unreset_registers` in
    // `tests/ip_library.rs`), and a wire sidesteps it at no cost.
    function [31:0] crc_step;
        input [31:0] crc;
        input        b;
        begin
            crc_step = (crc[0] ^ b) ? ((crc >> 1) ^ 32'hEDB8_8320) : (crc >> 1);
        end
    endfunction

    // -----------------------------------------------------------------
    // Transmit.
    // -----------------------------------------------------------------
    localparam [2:0] T_IDLE = 3'd0;
    localparam [2:0] T_PRE  = 3'd1;
    localparam [2:0] T_DATA = 3'd2;
    localparam [2:0] T_FCS  = 3'd3;
    localparam [2:0] T_IFG  = 3'd4;

    reg [2:0]  tx_state;
    reg [7:0]  tx_shift;
    reg [1:0]  tx_dibit;
    reg [2:0]  tx_pre;
    reg [31:0] tx_crc;
    reg [31:0] tx_fcs;
    reg [3:0]  tx_fcs_cnt;
    reg [7:0]  tx_ifg;
    reg        tx_last_q;
    reg        tx_under_q;

    wire tx_byte_end = (tx_dibit == 2'd3);
    wire tx_sending  = (tx_state == T_PRE) | (tx_state == T_DATA) | (tx_state == T_FCS);

    // The two bits going out this cycle, folded into the check sequence.
    wire [31:0] tx_crc_a = crc_step(tx_crc, tx_shift[0]);
    wire [31:0] tx_crc_b = crc_step(tx_crc_a, tx_shift[1]);

    assign tx_en = tx_sending;
    assign txd = ~tx_sending      ? 2'b00
               : (tx_state == T_FCS) ? tx_fcs[1:0]
               :                       tx_shift[1:0];
    assign tx_ready = ((tx_state == T_PRE) & (tx_pre == 3'd7) & tx_byte_end)
                    | ((tx_state == T_DATA) & tx_byte_end & ~tx_last_q);
    assign tx_busy = (tx_state != T_IDLE);
    assign tx_underrun = tx_under_q;

    always @(posedge ref_clk or negedge rst_n) begin
        if (!rst_n) begin
            tx_state   <= T_IDLE;
            tx_shift   <= 8'd0;
            tx_dibit   <= 2'd0;
            tx_pre     <= 3'd0;
            tx_crc     <= 32'hFFFF_FFFF;
            tx_fcs     <= 32'd0;
            tx_fcs_cnt <= 4'd0;
            tx_ifg     <= 8'd0;
            tx_last_q  <= 1'b0;
            tx_under_q <= 1'b0;
        end else begin
            case (tx_state)
                T_IDLE: begin
                    if (tx_valid) begin
                        tx_state   <= T_PRE;
                        tx_pre     <= 3'd0;
                        tx_dibit   <= 2'd0;
                        tx_shift   <= 8'h55;
                        tx_crc     <= 32'hFFFF_FFFF;
                        tx_last_q  <= 1'b0;
                        tx_under_q <= 1'b0;
                    end
                end
                T_PRE: begin
                    tx_dibit <= tx_dibit + 2'd1;
                    tx_shift <= {2'b00, tx_shift[7:2]};
                    if (tx_byte_end) begin
                        if (tx_pre == 3'd7) begin
                            // The delimiter has gone out; the first
                            // octet of the frame follows it.
                            tx_state   <= T_DATA;
                            tx_shift   <= tx_valid ? tx_data : 8'h00;
                            tx_last_q  <= tx_valid & tx_last;
                            tx_under_q <= ~tx_valid;
                        end else begin
                            tx_pre   <= tx_pre + 3'd1;
                            tx_shift <= (tx_pre == 3'd6) ? 8'hD5 : 8'h55;
                        end
                    end
                end
                T_DATA: begin
                    tx_dibit <= tx_dibit + 2'd1;
                    tx_shift <= {2'b00, tx_shift[7:2]};
                    tx_crc   <= tx_crc_b;
                    if (tx_byte_end) begin
                        if (tx_last_q) begin
                            tx_state   <= T_FCS;
                            tx_fcs     <= ~tx_crc_b;
                            tx_fcs_cnt <= 4'd0;
                        end else begin
                            tx_shift  <= tx_valid ? tx_data : 8'h00;
                            tx_last_q <= tx_valid & tx_last;
                            if (!tx_valid) tx_under_q <= 1'b1;
                        end
                    end
                end
                T_FCS: begin
                    tx_fcs     <= {2'b00, tx_fcs[31:2]};
                    tx_fcs_cnt <= tx_fcs_cnt + 4'd1;
                    if (tx_fcs_cnt == 4'd15) begin
                        tx_state <= T_IFG;
                        tx_ifg   <= 8'd0;
                    end
                end
                default: begin
                    tx_ifg <= tx_ifg + 8'd1;
                    if (tx_ifg == IFG_LAST) tx_state <= T_IDLE;
                end
            endcase
        end
    end

    // -----------------------------------------------------------------
    // Receive.
    // -----------------------------------------------------------------
    localparam [1:0] R_IDLE = 2'd0;
    localparam [1:0] R_PRE  = 2'd1;
    localparam [1:0] R_DATA = 2'd2;

    reg [1:0]  rx_state;
    reg [7:0]  rx_sr;
    reg [1:0]  rx_dibit;
    reg [31:0] rx_crc;
    reg [2:0]  rx_fill;
    reg        rx_er_q;
    // The five octets held back, newest first. Four of them turn out to
    // be the check sequence, and the fifth is the last octet of data.
    reg [7:0]  hold0;
    reg [7:0]  hold1;
    reg [7:0]  hold2;
    reg [7:0]  hold3;
    reg [7:0]  hold4;

    reg [7:0] rx_data_q;
    reg       rx_valid_q;
    reg       rx_last_q;
    reg       rx_ok_q;
    reg       rx_error_q;

    wire [31:0] rx_crc_a = crc_step(rx_crc, rxd[0]);
    wire [31:0] rx_crc_b = crc_step(rx_crc_a, rxd[1]);
    wire [7:0]  rx_byte  = {rxd, rx_sr[7:2]};
    wire        rx_byte_end = (rx_dibit == 2'd3);

    assign rx_data   = rx_data_q;
    assign rx_valid  = rx_valid_q;
    assign rx_last   = rx_last_q;
    assign rx_crc_ok = rx_ok_q;
    assign rx_error  = rx_error_q;

    always @(posedge ref_clk or negedge rst_n) begin
        if (!rst_n) begin
            rx_state   <= R_IDLE;
            rx_sr      <= 8'd0;
            rx_dibit   <= 2'd0;
            rx_crc     <= 32'hFFFF_FFFF;
            rx_fill    <= 3'd0;
            rx_er_q    <= 1'b0;
            hold0      <= 8'd0;
            hold1      <= 8'd0;
            hold2      <= 8'd0;
            hold3      <= 8'd0;
            hold4      <= 8'd0;
            rx_data_q  <= 8'd0;
            rx_valid_q <= 1'b0;
            rx_last_q  <= 1'b0;
            rx_ok_q    <= 1'b0;
            rx_error_q <= 1'b0;
        end else begin
            rx_valid_q <= 1'b0;
            rx_last_q  <= 1'b0;
            rx_error_q <= 1'b0;
            case (rx_state)
                R_IDLE: begin
                    rx_er_q <= 1'b0;
                    if (crs_dv) rx_state <= R_PRE;
                end
                R_PRE: begin
                    rx_er_q <= rx_er_q | rx_er;
                    if (!crs_dv) begin
                        // A carrier that never became a frame. Not an
                        // error, just noise on the line.
                        rx_state <= R_IDLE;
                    end else if (rxd == 2'b11) begin
                        rx_state <= R_DATA;
                        rx_crc   <= 32'hFFFF_FFFF;
                        rx_dibit <= 2'd0;
                        rx_fill  <= 3'd0;
                    end
                end
                R_DATA: begin
                    if (!crs_dv) begin
                        rx_state <= R_IDLE;
                        if ((rx_dibit == 2'd0) && (rx_fill == 3'd5) && !rx_er_q) begin
                            // The pipeline holds the last octet of data
                            // and the four of check sequence behind it.
                            rx_data_q  <= hold4;
                            rx_valid_q <= 1'b1;
                            rx_last_q  <= 1'b1;
                            rx_ok_q    <= (rx_crc == CRC_RESIDUE);
                        end else begin
                            rx_error_q <= 1'b1;
                        end
                    end else begin
                        rx_er_q  <= rx_er_q | rx_er;
                        rx_crc   <= rx_crc_b;
                        rx_dibit <= rx_dibit + 2'd1;
                        rx_sr    <= {rxd, rx_sr[7:2]};
                        if (rx_byte_end) begin
                            hold0 <= rx_byte;
                            hold1 <= hold0;
                            hold2 <= hold1;
                            hold3 <= hold2;
                            hold4 <= hold3;
                            if (rx_fill == 3'd5) begin
                                rx_data_q  <= hold4;
                                rx_valid_q <= 1'b1;
                            end else begin
                                rx_fill <= rx_fill + 3'd1;
                            end
                        end
                    end
                end
                default: rx_state <= R_IDLE;
            endcase
        end
    end
endmodule
