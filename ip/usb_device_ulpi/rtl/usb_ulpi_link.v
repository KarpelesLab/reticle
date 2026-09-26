// usb_ulpi_link — the Link half of a ULPI bus: bytes to and from a USB
// transceiver that does the line work itself.
//
// What it does
//   ULPI (UTMI+ Low Pin Interface, revision 1.1) puts a USB 2.0
//   transceiver behind twelve pins: a 60 MHz clock, eight bidirectional
//   data lines and the three control signals `dir`, `nxt` and `stp`.
//   This module is the Link end of that bus. It replaces `usb_fs_rx` and
//   `usb_fs_tx` — NRZI, bit stuffing, SYNC, the EOP and the serialiser
//   are the transceiver's, in silicon — and presents `usb_ctrl_ep` the
//   same byte-level interface, so the device above it is the same
//   device. `ip/usb_device_ulpi/README.md` states the protocol this was
//   written from, fact by fact, with the confidence of each and the
//   section of the specification it came from.
//
//   Bus ownership. `dir` is the transceiver's. While it is low the Link
//   drives the data bus — 8'h00, the idle NOOP command, when it has
//   nothing to say — and while it is high the transceiver drives it.
//   Every change of `dir` costs a turnaround cycle in which neither end
//   drives and whose content is undefined and must be ignored (§2.3.1),
//   so `ulpi_data_oe` is low both while `dir` is high and in the first
//   cycle after it falls, and a byte from the transceiver is believed
//   only when `dir` was already high in the cycle before it.
//
//   Starting up. The transceiver's reset pin is held (`ulpi_rst_n` low)
//   for RESET_CYCLES and released, and then four register transactions
//   go by:
//
//     Function Control ← 0x65   the reset §3.5 asks the Link to perform,
//                               and with it XcvrSelect = 01 (full
//                               speed), TermSelect = 1 (the 1.5 kOhm
//                               pull-up on D+ that tells the host a
//                               full-speed device is attached) and
//                               SuspendM = 1 (powered)
//     OTG Control      ← 0x00   the 15 kOhm pull-downs off: they are a
//                               host's, and this is a peripheral
//     Function Control ← 0x45   the same settings with the reset bit
//                               clear, in case the reset took the
//                               register with it
//     Function Control read     which must read back 0x45
//     Debug read                LineState, so the Link knows where the
//                               bus is before anything changes on it
//
//   `phy_ready` waits for all of that; a readback that is not 0x45
//   starts again from the OTG Control write. A transceiver that asserts
//   `dir` in the middle of a transaction aborts it, and the Link retries
//   that transaction when the bus is idle, which is what §3.8.3.1 asks
//   for.
//
//   Receiving. A receive command — `dir` high, `nxt` low — carries
//   LineState, VbusState, RxActive, RxError and ID. `dir` and `nxt`
//   asserted together out of an idle bus is the start of a packet, and
//   from then on every cycle with `nxt` high is a byte of it: the PID
//   first, then the payload and the CRC16, which `usb_ctrl_ep` checks.
//   The packet is over when a receive command says RxActive is 0 or
//   `dir` falls, whichever comes first (§3.8.2.4). A packet in which
//   RxError was ever set never reaches `rx_eop`, so the device ignores
//   it and the host retries.
//
//   Transmitting. The Link drives a transmit command — 8'b0100_pppp, the
//   command code and the PID — and holds it until `nxt`. Then the
//   payload, a byte per `nxt`, then the CRC16 low byte and high byte,
//   then `stp` for one cycle with 8'h00 on the bus. A packet with no
//   payload at all — an ACK, a NAK, a STALL — is the transmit command
//   and `stp`. The transceiver prepends the SYNC field, works the PID's
//   check nibble out of the four bits it was given, and appends the EOP.
//   If `dir` rises in the middle of a transmit the packet is abandoned
//   where it stands, because the bus is no longer the Link's: the host
//   sees no answer and asks again.
//
//   Bus reset. LineState comes from receive commands, and the
//   transceiver sends one whenever it changes. SE0 held for SE0_CYCLES —
//   2.5 us at 60 MHz — is a bus reset. `line_idle` is the bus at J with
//   no packet in progress, which is what `usb_ctrl_ep` times its answer
//   from.
//
// What it does not do
//   Full speed only. High speed needs the chirp handshake (§3.8.5.1) and
//   a device that answers inside 1 to 14 interface clocks instead of 7 to
//   18 (Table 10); low speed needs the transceiver told about preambles.
//   Neither is here.
//
//   No suspend and no low power mode, so SuspendM is left set and the
//   60 MHz clock runs always. That is the arrangement a board wants when
//   the Link is the clock source: in ULPI's input clock mode (§3.7.1.2)
//   the transceiver drives its internal PLL from the Link's clock, and
//   nothing here ever stops it.
//
//   Eight bytes of payload at most, which is `usb_ctrl_ep`'s maximum
//   packet size. No VBUS or ID handling: a receive command's VbusState
//   and ID bits are read and dropped, so a bus-powered device is assumed
//   and a session is never negotiated. No extended register set, no
//   interrupt enable registers, and no transmit error injection
//   (§3.8.2.3), which a device with a payload this small cannot need.
//   Nothing asserts `stp` to abort the transceiver (§3.8.4.2); there is
//   no babbling port to shut down when the only endpoint answers in
//   eight bytes.
//
//   Nothing waits for the receive command that reports the end of the
//   Link's *own* packet on the wire before accepting another `tx_start`,
//   which §3.8.2.2 asks a Link to do. `tx_busy` falls with the `stp` that
//   ends a packet, not with the transceiver's report of its end of
//   packet, so a device with something to say the instant after one
//   answer could start a second packet while the first is still going
//   out. `usb_ctrl_ep` cannot: it answers one host packet at a time and
//   has nothing more to send until the host has heard the last answer.
//   Anything with an endpoint that streams would have to add the wait.
//
//   The data bus is split into `ulpi_data_i`, `ulpi_data_o` and
//   `ulpi_data_oe`, as every bidirectional pin in this library is; the
//   three-state buffers belong to the top of the design.
module usb_ulpi_link #(
    // Cycles the transceiver's reset pin is held, and then the cycles of
    // an idle bus waited out before the first transaction. 5 us at
    // 60 MHz.
    parameter RESET_CYCLES = 300,
    // Cycles of SE0 that make a bus reset. 2.5 us at 60 MHz.
    parameter SE0_CYCLES   = 150
) (
    input  wire       clk,
    input  wire       rst_n,

    // The ULPI bus.
    input  wire       ulpi_dir,
    input  wire       ulpi_nxt,
    input  wire [7:0] ulpi_data_i,
    output wire [7:0] ulpi_data_o,
    output wire       ulpi_data_oe,
    output wire       ulpi_stp,
    output wire       ulpi_rst_n,

    // The bytes of a received packet.
    output wire [7:0] rx_data,
    output wire       rx_valid,
    output wire       rx_eop,
    output wire       rx_active,
    output wire       line_idle,
    output wire       bus_reset,

    // One packet out.
    input  wire       tx_start,
    input  wire [3:0] tx_pid,
    input  wire       tx_with_data,
    input  wire [3:0] tx_len,
    output wire [3:0] tx_index,
    input  wire [7:0] tx_byte,
    output wire       tx_busy,

    // The register sequence is done and the transceiver agreed with it.
    output wire       phy_ready
);
    // Transmit command codes: the top two bits of the byte.
    localparam [1:0] CMD_TX   = 2'b01;
    localparam [1:0] CMD_REGW = 2'b10;
    localparam [1:0] CMD_REGR = 2'b11;

    // The registers this block touches, and what a full-speed peripheral
    // wants in them.
    localparam [5:0] REG_FUNC_CTRL   = 6'h04;
    localparam [5:0] REG_OTG_CTRL    = 6'h0A;
    localparam [5:0] REG_DEBUG       = 6'h15;
    localparam [7:0] FUNC_CTRL_RESET = 8'h65;
    localparam [7:0] FUNC_CTRL_FS    = 8'h45;
    localparam [7:0] OTG_CTRL_DEV    = 8'h00;

    // Line states as LineState(1:0) of a receive command: bit 0 is D+.
    localparam [1:0] LINE_SE0 = 2'b00;
    localparam [1:0] LINE_J   = 2'b01;

    // Receive events: bits 5:4 of a receive command.
    localparam [1:0] RX_IDLE  = 2'b00;
    localparam [1:0] RX_ON    = 2'b01;
    localparam [1:0] RX_ERROR = 2'b11;

    // The bus states of the Link.
    localparam [3:0] S_RESET    = 4'd0;   // the reset pin held low
    localparam [3:0] S_SETTLE   = 4'd1;   // out of reset, bus idle
    localparam [3:0] S_NEXT     = 4'd2;   // one idle cycle between steps
    localparam [3:0] S_WR_CMD   = 4'd3;   // a register write: the command
    localparam [3:0] S_WR_DATA  = 4'd4;   // the byte
    localparam [3:0] S_WR_STP   = 4'd5;   // the stop that ends it
    localparam [3:0] S_WAIT_RST = 4'd6;   // the reset the transceiver runs
    localparam [3:0] S_RD_CMD   = 4'd7;   // a register read: the command
    localparam [3:0] S_RD_TURN  = 4'd8;   // the turnaround cycle
    localparam [3:0] S_RD_DATA  = 4'd9;   // the byte the transceiver drives
    localparam [3:0] S_READY    = 4'd10;  // idle, waiting for a packet
    localparam [3:0] S_TX_CMD   = 4'd11;  // the transmit command
    localparam [3:0] S_TX_DATA  = 4'd12;  // the payload
    localparam [3:0] S_TX_CRC0  = 4'd13;  // the CRC16, low byte
    localparam [3:0] S_TX_CRC1  = 4'd14;  // the CRC16, high byte

    // The steps of the start-up sequence.
    localparam [2:0] I_RESET = 3'd0;      // Function Control <- 0x65
    localparam [2:0] I_WAIT  = 3'd1;      // wait out the reset
    localparam [2:0] I_OTG   = 3'd2;      // OTG Control <- 0x00
    localparam [2:0] I_FUNC  = 3'd3;      // Function Control <- 0x45
    localparam [2:0] I_CHECK = 3'd4;      // read it back
    localparam [2:0] I_LINE  = 3'd5;      // read Debug for LineState
    localparam [2:0] I_DONE  = 3'd6;

    reg [3:0]  state;
    reg [2:0]  step;
    reg [7:0]  data_out;
    reg        stp_q;
    reg        rst_q;
    reg [15:0] wait_cnt;
    reg        seen_dir;

    // The previous cycle's `dir`, which is what says whether this cycle
    // is a turnaround.
    reg        dir_q;

    reg [7:0]  rx_data_q;
    reg        rx_valid_q;
    reg        rx_eop_q;
    reg        rx_active_q;
    reg        rx_error_q;
    reg [1:0]  line_state;
    reg [7:0]  se0_cnt;
    reg        ready_q;

    reg [3:0]  idx;
    reg [3:0]  len_q;
    reg        with_data_q;
    reg [15:0] crc;

    // This cycle's bus content is the transceiver's, and not a
    // turnaround, only if `dir` was high in the cycle before it too.
    wire phy_drives = ulpi_dir & dir_q;
    wire dir_rose   = ulpi_dir & ~dir_q;
    wire dir_fell   = ~ulpi_dir & dir_q;
    wire bus_ours   = ~ulpi_dir & ~dir_q;

    // The three states in which `dir` high is expected rather than an
    // abort: the transceiver's own reset, and a register read.
    wire expect_dir = (state == S_WAIT_RST) | (state == S_RD_TURN)
                    | (state == S_RD_DATA);

    assign ulpi_data_o  = data_out;
    assign ulpi_data_oe = bus_ours;
    assign ulpi_stp     = stp_q;
    assign ulpi_rst_n   = rst_q;

    assign rx_data   = rx_data_q;
    assign rx_valid  = rx_valid_q;
    assign rx_eop    = rx_eop_q;
    assign rx_active = rx_active_q;
    assign line_idle = ready_q & ~rx_active_q & (line_state == LINE_J);
    assign bus_reset = (se0_cnt == SE0_CYCLES[7:0]);

    assign tx_index  = idx;
    assign tx_busy   = (state != S_READY) | ~bus_ours;
    assign phy_ready = ready_q;

    // What the current step of the start-up sequence asks for.
    reg       st_write;
    reg [5:0] st_addr;
    reg [7:0] st_data;

    always @(*) begin
        case (step)
            I_RESET: begin st_write = 1'b1; st_addr = REG_FUNC_CTRL; st_data = FUNC_CTRL_RESET; end
            I_OTG:   begin st_write = 1'b1; st_addr = REG_OTG_CTRL;  st_data = OTG_CTRL_DEV;   end
            I_FUNC:  begin st_write = 1'b1; st_addr = REG_FUNC_CTRL; st_data = FUNC_CTRL_FS;   end
            I_CHECK: begin st_write = 1'b0; st_addr = REG_FUNC_CTRL; st_data = 8'h00;          end
            I_LINE:  begin st_write = 1'b0; st_addr = REG_DEBUG;     st_data = 8'h00;          end
            default: begin st_write = 1'b0; st_addr = 6'h00;         st_data = 8'h00;          end
        endcase
    end

    wire [7:0] st_cmd = st_write ? {CMD_REGW, st_addr} : {CMD_REGR, st_addr};

    // The CRC16 of a data packet is the Link's work: ULPI gives the
    // transceiver the SYNC field and the EOP and nothing else.
    function [15:0] crc16_byte;
        input [15:0] c;
        input [7:0]  d;
        integer      i;
        reg   [15:0] r;
        begin
            r = c;
            for (i = 0; i < 8; i = i + 1)
                r = (r[0] ^ d[i]) ? ((r >> 1) ^ 16'hA001) : (r >> 1);
            crc16_byte = r;
        end
    endfunction

    wire [15:0] crc_next  = crc16_byte(crc, data_out);
    wire        last_byte = (idx == len_q);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state       <= S_RESET;
            step        <= I_RESET;
            data_out    <= 8'h00;
            stp_q       <= 1'b0;
            rst_q       <= 1'b0;
            wait_cnt    <= 16'd0;
            seen_dir    <= 1'b0;
            dir_q       <= 1'b0;
            rx_data_q   <= 8'h00;
            rx_valid_q  <= 1'b0;
            rx_eop_q    <= 1'b0;
            rx_active_q <= 1'b0;
            rx_error_q  <= 1'b0;
            line_state  <= LINE_SE0;
            se0_cnt     <= 8'd0;
            ready_q     <= 1'b0;
            idx         <= 4'd0;
            len_q       <= 4'd0;
            with_data_q <= 1'b0;
            crc         <= 16'hFFFF;
        end else begin
            dir_q      <= ulpi_dir;
            rx_valid_q <= 1'b0;
            rx_eop_q   <= 1'b0;
            stp_q      <= 1'b0;

            // -------------------------------------------------------------
            // What the transceiver said. Nothing on the bus is believed
            // until the start-up sequence is over, since a transceiver
            // being reset drives it with whatever it likes (§3.5).
            // -------------------------------------------------------------
            if (ready_q) begin
                if (dir_rose && ulpi_nxt) begin
                    // `dir` and `nxt` together out of an idle bus: a
                    // packet is starting. This cycle is the turnaround
                    // and carries nothing.
                    rx_active_q <= 1'b1;
                    rx_error_q  <= 1'b0;
                end else if (phy_drives) begin
                    if (ulpi_nxt) begin
                        rx_data_q  <= ulpi_data_i;
                        rx_valid_q <= 1'b1;
                    end else begin
                        // A receive command.
                        line_state <= ulpi_data_i[1:0];
                        case (ulpi_data_i[5:4])
                            RX_ON: begin
                                rx_active_q <= 1'b1;
                            end
                            RX_ERROR: begin
                                rx_active_q <= 1'b1;
                                rx_error_q  <= 1'b1;
                            end
                            RX_IDLE: begin
                                if (rx_active_q && !rx_error_q) rx_eop_q <= 1'b1;
                                rx_active_q <= 1'b0;
                                rx_error_q  <= 1'b0;
                            end
                            default: begin
                                // 2'b10 is host disconnect, which a
                                // peripheral must ignore (Table 7).
                            end
                        endcase
                    end
                end
                if (dir_fell && rx_active_q) begin
                    // The other way a packet can end: `dir` let go
                    // without a closing receive command.
                    if (!rx_error_q) rx_eop_q <= 1'b1;
                    rx_active_q <= 1'b0;
                    rx_error_q  <= 1'b0;
                end
            end

            // SE0 held long enough is the host resetting the bus.
            if (line_state != LINE_SE0 || !ready_q) se0_cnt <= 8'd0;
            else if (se0_cnt != SE0_CYCLES[7:0]) se0_cnt <= se0_cnt + 8'd1;

            // -------------------------------------------------------------
            // What the Link drives.
            // -------------------------------------------------------------
            if (state == S_RESET) begin
                // The reset pin is timed on its own; the bus does not
                // come into it.
                if (wait_cnt == RESET_CYCLES[15:0]) begin
                    rst_q    <= 1'b1;
                    wait_cnt <= 16'd0;
                    state    <= S_SETTLE;
                end else begin
                    wait_cnt <= wait_cnt + 16'd1;
                end
            end else if (ulpi_dir && !expect_dir) begin
                // The bus is the transceiver's. Whatever was in flight is
                // aborted where it stands, and `stp` stays low, since the
                // transceiver would read it as an order to give up the
                // bus (§3.8.4.2).
                data_out <= 8'h00;
                case (state)
                    S_SETTLE, S_NEXT, S_READY: begin
                        // Nothing was in flight.
                    end
                    default: begin
                        // A transmit is dropped and the host asks again;
                        // a register transaction is retried, since `step`
                        // has not moved on.
                        state <= ready_q ? S_READY : S_NEXT;
                    end
                endcase
            end else begin
                case (state)
                    S_SETTLE: begin
                        // Out of reset with the bus ours for long enough.
                        if (wait_cnt == RESET_CYCLES[15:0]) begin
                            wait_cnt <= 16'd0;
                            state    <= S_NEXT;
                        end else begin
                            wait_cnt <= wait_cnt + 16'd1;
                        end
                    end
                    S_NEXT: begin
                        // One idle cycle, then whatever the step wants.
                        wait_cnt <= 16'd0;
                        seen_dir <= 1'b0;
                        if (step == I_WAIT) begin
                            state <= S_WAIT_RST;
                        end else if (step == I_DONE) begin
                            ready_q <= 1'b1;
                            state   <= S_READY;
                        end else begin
                            data_out <= st_cmd;
                            state    <= st_write ? S_WR_CMD : S_RD_CMD;
                        end
                    end
                    S_WR_CMD: begin
                        if (ulpi_nxt) begin
                            data_out <= st_data;
                            state    <= S_WR_DATA;
                        end
                    end
                    S_WR_DATA: begin
                        if (ulpi_nxt) begin
                            data_out <= 8'h00;
                            stp_q    <= 1'b1;
                            state    <= S_WR_STP;
                        end
                    end
                    S_WR_STP: begin
                        step  <= step + 3'd1;
                        state <= S_NEXT;
                    end
                    S_WAIT_RST: begin
                        // §3.5: the transceiver asserts `dir` while it
                        // resets its core and lets go when it is done.
                        if (ulpi_dir) seen_dir <= 1'b1;
                        if (seen_dir && !ulpi_dir) begin
                            step  <= step + 3'd1;
                            state <= S_NEXT;
                        end else if (wait_cnt == RESET_CYCLES[15:0]) begin
                            // It never took the bus. Carry on anyway: the
                            // readback at the end is what decides.
                            step  <= step + 3'd1;
                            state <= S_NEXT;
                        end else begin
                            wait_cnt <= wait_cnt + 16'd1;
                        end
                    end
                    S_RD_CMD: begin
                        if (ulpi_nxt) begin
                            data_out <= 8'h00;
                            state    <= S_RD_TURN;
                        end
                    end
                    S_RD_TURN: begin
                        if (dir_rose && ulpi_nxt) begin
                            // A USB receive took the bus instead, which
                            // it may do in any cycle (§3.8.3.2): retry.
                            state <= S_NEXT;
                        end else if (dir_rose) begin
                            state <= S_RD_DATA;
                        end else if (wait_cnt == 16'd3) begin
                            // No answer at all.
                            state <= S_NEXT;
                        end else begin
                            wait_cnt <= wait_cnt + 16'd1;
                        end
                    end
                    S_RD_DATA: begin
                        if (phy_drives && ulpi_nxt) begin
                            state <= S_NEXT;
                        end else if (phy_drives) begin
                            if (step == I_LINE) begin
                                line_state <= ulpi_data_i[1:0];
                                step       <= I_DONE;
                                state      <= S_NEXT;
                            end else if (ulpi_data_i == FUNC_CTRL_FS) begin
                                step  <= I_LINE;
                                state <= S_NEXT;
                            end else begin
                                // The transceiver did not take the
                                // settings: write them again.
                                step  <= I_OTG;
                                state <= S_NEXT;
                            end
                        end else if (!ulpi_dir) begin
                            state <= S_NEXT;
                        end
                    end
                    S_READY: begin
                        if (tx_start) begin
                            data_out    <= {CMD_TX, 2'b00, tx_pid};
                            len_q       <= tx_len;
                            with_data_q <= tx_with_data;
                            idx         <= 4'd0;
                            crc         <= 16'hFFFF;
                            state       <= S_TX_CMD;
                        end
                    end
                    S_TX_CMD: begin
                        if (ulpi_nxt) begin
                            if (!with_data_q) begin
                                // A handshake is the command and nothing
                                // else.
                                data_out <= 8'h00;
                                stp_q    <= 1'b1;
                                state    <= S_READY;
                            end else if (len_q == 4'd0) begin
                                // A zero-length data packet still carries
                                // the CRC16 of nothing, which is 0x0000.
                                data_out <= ~crc[7:0];
                                state    <= S_TX_CRC0;
                            end else begin
                                data_out <= tx_byte;
                                idx      <= 4'd1;
                                state    <= S_TX_DATA;
                            end
                        end
                    end
                    S_TX_DATA: begin
                        if (ulpi_nxt) begin
                            crc <= crc_next;
                            if (last_byte) begin
                                data_out <= ~crc_next[7:0];
                                state    <= S_TX_CRC0;
                            end else begin
                                data_out <= tx_byte;
                                idx      <= idx + 4'd1;
                            end
                        end
                    end
                    S_TX_CRC0: begin
                        if (ulpi_nxt) begin
                            data_out <= ~crc[15:8];
                            state    <= S_TX_CRC1;
                        end
                    end
                    default: begin
                        // S_TX_CRC1, the last byte of the packet: `stp`
                        // for one cycle with 8'h00 on the bus ends it.
                        if (ulpi_nxt) begin
                            data_out <= 8'h00;
                            stp_q    <= 1'b1;
                            state    <= S_READY;
                        end
                    end
                endcase
            end
        end
    end
endmodule
