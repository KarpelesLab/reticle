// usb_ulpi_host_link — the Link half of a ULPI bus, configured as a USB
// **host**: bytes to and from a transceiver that does the line work.
//
// What it does
//   The bus mechanics are `ip/usb/usb_device_ulpi`'s `usb_ulpi_link`'s, and
//   deliberately so: the turnaround in both directions, the transmit
//   command, the receive command, register reads and writes with a retry
//   on abort, and the rule that a byte from the transceiver is believed
//   only when `dir` was already high in the cycle before it. Those have
//   run on a Microchip USB3343 on a Great Scott Gadgets Cynthion and are
//   not re-derived here. `ip/usb/usb_device_ulpi/README.md` states every one
//   of them with the section of ULPI 1.1 it comes from.
//
//   What is different is everything about **which end of the wire this
//   is**, and it is three things and no more:
//
//     * the registers. A host writes OTG Control's `DpPulldown` and
//       `DmPulldown` — its two 15 kOhm pull-downs — where a peripheral
//       clears them, and that is the whole of the difference in
//       Function Control's own value: ULPI 1.1 §3.8.5.3.2 names the FS
//       host "XcvrSelect=01b, DpPulldown=1b, DmPulldown=1b,
//       TermSelect=1b", which is the same `04h` = `45h` a full-speed
//       peripheral wants. The pull-downs live in `0Ah`, and `06h` there
//       is both what a host needs and the register's reset value.
//
//     * the start-up does **not** wait for the pair to leave SE0. A
//       peripheral's start-up connects its own 1.5 kOhm pull-up and must
//       wait milliseconds for it to charge the pair, which is why
//       `usb_ulpi_link` has `LINE_TRIES`. A host's idle downstream port
//       is **SE0 on purpose** — two pull-downs and nothing else — and
//       stays there until something is plugged in. Reading SE0 once and
//       believing it is the right answer at this end, and waiting for J
//       here would hang on an empty socket for ever.
//
//     * the register port is **brought out**. A peripheral's Link has a
//       fixed sequence and no reason to let anything else near the bus; a
//       host's has two: the bus reset it must drive is a register write
//       (§3.8.5.1, below), and a design that wants to ask the
//       transceiver what it is needs to read `00h`. So `reg_*` is a
//       transaction port that whatever sits above this module owns once
//       `phy_ready` is high.
//
//   **Driving a bus reset is a register write and not a transmit**, which
//   is the one fact about hosts that is easiest to get wrong. ULPI 1.1
//   §3.8.5.1 step 2:
//
//     "If a host detects a full speed peripheral, it resets the
//      peripheral by writing to the Function Control register and setting
//      XcvrSelect = 00b (HS) and TermSelect = 0b which drives SE0 on the
//      bus (D+ and D- connected to ground via 45 Ohm). The host also sets
//      OpMode = 10b for correct chirp transmit and receive."
//
//   So SE0 comes from the 45 Ohm high-speed terminations being switched
//   on with no transmitter driving, and the Link's only part in it is one
//   byte in one register. This module does not do it — `usb_host_sie`
//   above it does, through `reg_*`, because how long the SE0 lasts is
//   USB's business and not ULPI's.
//
//   Three transmit shapes, where a peripheral needs two. `tx_mode`
//   selects between them and the difference is what follows the payload:
//
//     TX_HANDSHAKE   the transmit command and `stp`: an ACK.
//     TX_RAW         `tx_len` bytes exactly as given, then `stp`. This is
//                    what a **token** is: SOF, IN, OUT and SETUP each
//                    carry two bytes with a CRC5 already inside them, and
//                    a Link that appended a CRC16 to one would put five
//                    bytes on the wire where USB expects three.
//     TX_DATA        the payload, then the CRC16 this module computes,
//                    low byte first.
//
//   The CRC5 is **not** here. It covers eleven bits that are a frame
//   number or an address and an endpoint, so it belongs with whatever
//   knows those; `usb_host_sie` has it, next to the token it goes in.
//   The CRC16 is here for the same reason it is in the peripheral's Link:
//   it covers the bytes this module is already handing over one at a
//   time, and ULPI gives the transceiver the SYNC field and the EOP and
//   nothing else.
//
// What it does not do
//   **Full speed only, and that is a decision and not a gap.** High speed
//   needs the chirp handshake of §3.8.5.1 — the host driving SE0, the
//   peripheral answering with a chirp K, the host answering with
//   alternating K and J for 40 to 60 microseconds each — and a Link that
//   turns a received packet round in 1 to 14 interface clocks instead of 7
//   to 18 (Table 10). Low speed needs `XcvrSelect = 11b` and the
//   transceiver prepending a full-speed preamble to every packet
//   (USB334x DS00002646A §6.4.1.3). Neither is here, and a host that
//   finds a low-speed device attached reports it and stops rather than
//   pretending.
//
//   No suspend, no resume and no remote wake-up: `SuspendM` is left set
//   and the clock runs always, which is what ULPI's input clock mode
//   (§3.7.1.2) wants. No high-speed disconnect detection, which is the
//   transceiver's only in high speed anyway — "When in FS or LS modes,
//   the Link is expected to handle all disconnect detection"
//   (DS00002646A §6.3.2.1) — so a detach here is SE0 on `line_state`,
//   seen by whatever is above.
//
//   Nothing waits for the receive command that reports the end of this
//   Link's **own** packet on the wire before accepting another
//   `tx_start`, which §3.8.2.2 asks a Link to do. `tx_busy` falls with
//   the `stp` that ends a packet. `usb_host_sie` never starts a second
//   packet in that cycle: between two of its packets is either a reply it
//   is waiting for or the inter-packet gap it counts out, and both are
//   longer than the one left over. A design that drove this module
//   directly and sent back-to-back packets would have to add the wait.
//
//   No VBUS switching. The receive command's VbusState is reported on
//   `vbus_state` and nothing is done with it: whether the port has power
//   is a **board's** question, settled by a switch outside the
//   transceiver, and this module neither reads a switch nor drives one.
//   It also never writes OTG Control's `DrvVbus` or `ChrgVbus`: the only
//   byte it puts in `0Ah` is `OTG_CTRL_HOST`, whose top five bits are
//   zero.
//
//   The data bus is split into `ulpi_data_i`, `ulpi_data_o` and
//   `ulpi_data_oe`, as every bidirectional pin in this library is; the
//   three-state buffers belong to the top of the design.
module usb_ulpi_host_link #(
    // Cycles the transceiver's reset pin is held, and then the cycles of
    // an idle bus waited out before the first transaction. 5 us at
    // 60 MHz.
    parameter RESET_CYCLES = 300,
    // One transceiver register outside ULPI, written before anything on
    // the wire can be affected by it and read back before the sequence
    // goes on. `6'h00` disables it, which is the default: address 00h is
    // Vendor ID Low, read-only in every ULPI transceiver, so it can never
    // be a write this was asked for.
    //
    // ULPI 1.1 §4.1 reserves the immediate addresses 30h to 3Fh for the
    // transceiver's own registers and says nothing whatever about what is
    // in them. A board may **exchange DP and DM between the transceiver
    // and its connector** to keep the pair from crossing over in the
    // layout, and then a bit in one of those registers is what tells the
    // transceiver about it. It matters to a host for the mirror of the
    // reason it matters to a peripheral: a host reads the pair to find
    // out what is attached, and with the bit unwritten it reads a
    // full-speed device's pull-up on the line it calls D- and calls it
    // low speed.
    //
    // On a Great Scott Gadgets Cynthion the register is `39h` and the
    // byte is `06h`; `README.md` has the three sources that agree on it.
    parameter [5:0] VENDOR_ADDR = 6'h00,
    parameter [7:0] VENDOR_DATA = 8'h00,
    // How many times a register transaction from above is re-issued after
    // the transceiver aborted it, before it is given up on and reported
    // with `reg_ok` low.
    //
    // ULPI 1.1 §3.8.3.1 says a Link "must retry" an aborted register
    // access when the bus is idle and puts no number on it, and the
    // peripheral's Link retries for ever because the only register
    // transactions it has are its own start-up's — and a start-up that
    // cannot finish has nothing to go on to. A host's register port
    // belongs to something else, and that something is waiting for an
    // answer: a bounded number of tries turns a transceiver that will not
    // answer an address into a reported failure rather than a design that
    // stops.
    parameter integer REG_TRIES = 16
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

    // The bytes of a received packet. `rx_error` is the packet's and not
    // the byte's: ULPI reports a bit-stuff error or a packet that ended
    // off a byte boundary as RxEvent 11 and then lets go of the bus
    // (§3.8.2.5), so it is held until the packet is over and `rx_eop`
    // is never produced for a packet it was seen in.
    output wire [7:0] rx_data,
    output wire       rx_valid,
    output wire       rx_eop,
    output wire       rx_active,
    output wire       rx_error,

    // The last receive command, whole, and the fields of it a host uses.
    // `rx_cmd_seen` says at least one has arrived, which is what makes
    // the rest of these a reading rather than a reset value.
    output wire [7:0] rx_cmd,
    output wire       rx_cmd_seen,
    output wire [1:0] line_state,
    output wire [1:0] vbus_state,
    output wire       id_pin,

    // One packet out.
    input  wire       tx_start,
    input  wire [3:0] tx_pid,
    input  wire [1:0] tx_mode,
    input  wire [6:0] tx_len,
    output wire [6:0] tx_index,
    input  wire [7:0] tx_byte,
    output wire       tx_busy,
    // One cycle with the `stp` that ended a packet, and one cycle when a
    // packet was given up because the transceiver took the bus (§3.8.4.1).
    output wire       tx_done,
    output wire       tx_abort,

    // One register transaction, once `phy_ready` is high. `reg_start`
    // with `reg_busy` low latches the request; `reg_done` is one cycle
    // and `reg_rdata` holds a read's answer from then until the next one.
    // A transaction the transceiver aborts is retried here rather than
    // reported, which is what §3.8.3.1 asks of a Link.
    input  wire       reg_start,
    input  wire       reg_write,
    input  wire [5:0] reg_addr,
    input  wire [7:0] reg_wdata,
    output wire [7:0] reg_rdata,
    output wire       reg_done,
    // Whether the transaction `reg_done` reports went through at all.
    output wire       reg_ok,
    output wire       reg_busy,

    // The start-up sequence is done and the transceiver agreed with it.
    output wire       phy_ready
);
    // Transmit command codes: the top two bits of the byte (Table 6).
    localparam [1:0] CMD_TX   = 2'b01;
    localparam [1:0] CMD_REGW = 2'b10;
    localparam [1:0] CMD_REGR = 2'b11;

    // What `tx_mode` selects.
    localparam [1:0] TX_HANDSHAKE = 2'd0;
    localparam [1:0] TX_RAW       = 2'd1;
    localparam [1:0] TX_DATA      = 2'd2;

    // The registers this block touches, and what a full-speed **host**
    // wants in them.
    localparam [5:0] REG_FUNC_CTRL   = 6'h04;
    localparam [5:0] REG_OTG_CTRL    = 6'h0A;
    localparam [5:0] REG_DEBUG       = 6'h15;
    // XcvrSelect = 01 (full speed), TermSelect = 1, OpMode = 00,
    // SuspendM = 1, and the Reset bit §3.5 asks the Link to set once.
    localparam [7:0] FUNC_CTRL_RESET = 8'h65;
    localparam [7:0] FUNC_CTRL_FS    = 8'h45;
    // IdPullup = 0, DpPulldown = 1, DmPulldown = 1, and every VBUS
    // control bit above them zero. Also the register's reset value, so a
    // transceiver that came out of reset is already here; it is written
    // anyway, for the same reason Function Control is written twice.
    localparam [7:0] OTG_CTRL_HOST   = 8'h06;

    // Line states as LineState(1:0) of a receive command: bit 0 is D+.
    localparam [1:0] LINE_SE0 = 2'b00;

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
    localparam [3:0] S_READY    = 4'd10;  // idle, waiting for work
    localparam [3:0] S_TX_CMD   = 4'd11;  // the transmit command
    localparam [3:0] S_TX_DATA  = 4'd12;  // the payload
    localparam [3:0] S_TX_CRC0  = 4'd13;  // the CRC16, low byte
    localparam [3:0] S_TX_CRC1  = 4'd14;  // the CRC16, high byte

    // The steps of the start-up sequence. The two vendor steps are
    // skipped outright when `VENDOR_ADDR` is zero, which is the default.
    localparam [3:0] I_RESET = 4'd0;      // Function Control <- 0x65
    localparam [3:0] I_WAIT  = 4'd1;      // wait out the reset
    localparam [3:0] I_VEND  = 4'd2;      // VENDOR_ADDR <- VENDOR_DATA
    localparam [3:0] I_VCHK  = 4'd3;      // read that back
    localparam [3:0] I_OTG   = 4'd4;      // OTG Control <- 0x06
    localparam [3:0] I_FUNC  = 4'd5;      // Function Control <- 0x45
    localparam [3:0] I_CHECK = 4'd6;      // read it back
    localparam [3:0] I_LINE  = 4'd7;      // read Debug for LineState, once
    localparam [3:0] I_DONE  = 4'd8;

    // Whether there is a vendor register to write at all.
    localparam VENDOR_EN = (VENDOR_ADDR != 6'h00);

    reg [3:0]  state;
    reg [3:0]  step;
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
    // A byte of this packet has been delivered. A packet is bytes: the
    // PID is always the first of them, so a receive that produced none
    // was never a packet and must not produce the end of one. The
    // peripheral's Link has the measurement this comes from.
    reg        rx_any;
    reg [7:0]  rx_cmd_q;
    reg        rx_cmd_seen_q;
    // LineState, kept **separately** from the receive command it usually
    // comes in, because the start-up also learns it from the Debug register
    // and a register is not a receive command. Writing `rx_cmd_q[1:0]` from
    // one place and `rx_cmd_q` from another would be two assignments to one
    // register of two different widths, which is a shape worth not having
    // whatever a compiler makes of it.
    reg [1:0]  line_q;
    reg        ready_q;

    reg [6:0]  idx;
    reg [6:0]  len_q;
    reg [1:0]  mode_q;
    reg [15:0] crc;
    reg        tx_done_q;
    reg        tx_abort_q;

    // The register transaction above this module: latched when it is
    // taken, held until it has been through, and re-issued if the
    // transceiver aborts it.
    reg        req_pend;
    reg        req_write;
    reg [5:0]  req_addr;
    reg [7:0]  req_data;
    reg [7:0]  rd_q;
    reg        reg_done_q;
    reg        reg_ok_q;
    // Attempts at the transaction in `req_pend`. `REG_TRIES` is counted
    // to and not past, so this is exactly as wide as the parameter needs.
    reg [$clog2(REG_TRIES + 1)-1:0] req_tries;

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
    // The Link drives the bus whenever `dir` is low, **including the
    // cycle `dir` falls in**. ULPI 1.1 §2.3.1 wants `dir` "wired straight
    // to the output buffers" of both ends, and the transceiver's own
    // datasheet says why a cycle of floating is not conservatism but a
    // fault: "When the USB334x sends a RXCMD the Link is required to
    // drive the data bus back to idle at the end of the turn around
    // cycle. If the Link does not drive the databus to idle the USB334x
    // may take the information on the data bus as a TXCMD and transmit
    // data on DP and DM until the Link asserts stop" ... "The pull downs
    // are not strong enough to pull the data bus low after a ULPI RXCMD"
    // (DS00002646A §6.5.4.1). A receive command with its ID bit or its
    // `alt_int` bit set is a byte with bit 6 or bit 7 high, which is
    // exactly a register command or a transmit command.
    assign ulpi_data_oe = ~ulpi_dir;
    // `stp` is gated on `dir` for the same reason: while the transceiver
    // owns the bus, `stp` means "give it back" (§3.8.4.2), and this Link
    // never asks.
    assign ulpi_stp     = stp_q & ~ulpi_dir;
    assign ulpi_rst_n   = rst_q;

    assign rx_data     = rx_data_q;
    assign rx_valid    = rx_valid_q;
    assign rx_eop      = rx_eop_q;
    assign rx_active   = rx_active_q & rx_any;
    assign rx_error    = rx_error_q;
    assign rx_cmd      = rx_cmd_q;
    assign rx_cmd_seen = rx_cmd_seen_q;
    assign line_state  = line_q;
    assign vbus_state  = rx_cmd_q[3:2];
    assign id_pin      = rx_cmd_q[6];

    assign tx_index  = idx;
    assign tx_busy   = (state != S_READY) | ~bus_ours;
    assign tx_done   = tx_done_q;
    assign tx_abort  = tx_abort_q;

    assign reg_rdata = rd_q;
    assign reg_done  = reg_done_q;
    assign reg_ok    = reg_ok_q;
    assign reg_busy  = req_pend | ~ready_q;

    assign phy_ready = ready_q;

    // What the current transaction asks for: the start-up's own step
    // while it is running, and the latched request afterwards.
    reg       st_write;
    reg [5:0] st_addr;
    reg [7:0] st_data;

    always @(*) begin
        case (step)
            I_RESET: begin st_write = 1'b1; st_addr = REG_FUNC_CTRL; st_data = FUNC_CTRL_RESET; end
            I_VEND:  begin st_write = 1'b1; st_addr = VENDOR_ADDR;   st_data = VENDOR_DATA;     end
            I_VCHK:  begin st_write = 1'b0; st_addr = VENDOR_ADDR;   st_data = 8'h00;           end
            I_OTG:   begin st_write = 1'b1; st_addr = REG_OTG_CTRL;  st_data = OTG_CTRL_HOST;   end
            I_FUNC:  begin st_write = 1'b1; st_addr = REG_FUNC_CTRL; st_data = FUNC_CTRL_FS;    end
            I_CHECK: begin st_write = 1'b0; st_addr = REG_FUNC_CTRL; st_data = 8'h00;           end
            I_LINE:  begin st_write = 1'b0; st_addr = REG_DEBUG;     st_data = 8'h00;           end
            default: begin st_write = req_write; st_addr = req_addr;  st_data = req_data;       end
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
            state         <= S_RESET;
            step          <= I_RESET;
            data_out      <= 8'h00;
            stp_q         <= 1'b0;
            rst_q         <= 1'b0;
            wait_cnt      <= 16'd0;
            seen_dir      <= 1'b0;
            dir_q         <= 1'b0;
            rx_data_q     <= 8'h00;
            rx_valid_q    <= 1'b0;
            rx_eop_q      <= 1'b0;
            rx_active_q   <= 1'b0;
            rx_error_q    <= 1'b0;
            rx_any        <= 1'b0;
            rx_cmd_q      <= 8'h00;
            rx_cmd_seen_q <= 1'b0;
            line_q        <= LINE_SE0;
            ready_q       <= 1'b0;
            idx           <= 7'd0;
            len_q         <= 7'd0;
            mode_q        <= TX_HANDSHAKE;
            crc           <= 16'hFFFF;
            tx_done_q     <= 1'b0;
            tx_abort_q    <= 1'b0;
            req_pend      <= 1'b0;
            req_write     <= 1'b0;
            req_addr      <= 6'h00;
            req_data      <= 8'h00;
            rd_q          <= 8'h00;
            reg_done_q    <= 1'b0;
            reg_ok_q      <= 1'b0;
            req_tries     <= 0;
        end else begin
            dir_q      <= ulpi_dir;
            rx_valid_q <= 1'b0;
            rx_eop_q   <= 1'b0;
            stp_q      <= 1'b0;
            tx_done_q  <= 1'b0;
            tx_abort_q <= 1'b0;
            reg_done_q <= 1'b0;

            // -------------------------------------------------------------
            // What the transceiver said.
            //
            // **The bus is believed from the end of the transceiver's own
            // reset and not from the end of the start-up**, and the
            // difference is the one receive command a probe cannot do
            // without. ULPI 1.1 §3.5:
            //
            //   "When the reset completes, the PHY de-asserts `dir` and
            //    automatically clears the Reset bit. After de-asserting
            //    `dir`, the PHY must **immediately re-assert `dir` and send
            //    an RX CMD update to the Link**."
            //
            // That one is promised. Every other receive command is sent
            // **because something changed** (§3.8.1.3), so a port whose VBUS
            // and whose pair have been the same since before the Link
            // started listening sends no more of them — and a Link that
            // waits for its whole start-up to finish has already thrown away
            // the only one it was ever going to get. `VbusState` would then
            // read its reset value for ever on a board where VBUS is
            // present and static, which is every board that is working.
            //
            // The peripheral's Link waits for `ready_q` and loses nothing by
            // it: it has no use for VbusState and it learns LineState from
            // the Debug register instead. A host is the end that has to
            // decide whether the port has power before it does anything, so
            // it cannot.
            //
            // `step > I_WAIT` is exactly "the reset §3.5 asks for is over":
            // during it `step` is `I_WAIT` and §3.5's "during that reset the
            // data bus is driven by the transceiver and the data is
            // undefined" means nothing on the bus may be read, which is what
            // this excludes.
            // -------------------------------------------------------------
            if (step > I_WAIT) begin
                if (dir_rose && ulpi_nxt) begin
                    // `dir` and `nxt` together out of an idle bus: a
                    // packet is starting. This cycle is the turnaround
                    // and carries nothing.
                    rx_active_q <= 1'b1;
                    rx_error_q  <= 1'b0;
                    rx_any      <= 1'b0;
                end else if (phy_drives) begin
                    if (ulpi_nxt) begin
                        rx_data_q  <= ulpi_data_i;
                        rx_valid_q <= 1'b1;
                        rx_any     <= 1'b1;
                    end else if (state == S_RD_DATA) begin
                        // **The answer to a register read, and not a receive
                        // command.** `dir` high with `nxt` low is a receive
                        // command everywhere else on this bus, and in this one
                        // cycle it is the byte the transceiver was asked for:
                        // "It does **not** assert `nxt` for that byte, which
                        // is the one place in ULPI where data is not
                        // throttled" (ULPI 1.1 §3.8.3.1 and Figure 22), and
                        // the reason it does not is so that `nxt` is left free
                        // to mean "a USB receive is starting" and override the
                        // read — which is the `ulpi_nxt` arm above.
                        //
                        // So the two are told apart by the Link's own state
                        // and not by anything on the wire, because there is
                        // nothing on the wire to tell them apart by. The
                        // register path below reads the same byte.
                        //
                        // The peripheral's Link has no such guard and does not
                        // need one: its only register reads are its start-up's
                        // and it believes nothing on the bus until that is
                        // over, so the cycle never reaches a `ready_q` of one.
                        // A host's register port is open for as long as the
                        // design wants it, so for a host this is the
                        // difference between reading `VbusState` and reading
                        // Function Control's bits 3:2 and calling them VBUS.
                    end else begin
                        // A receive command.
                        rx_cmd_q      <= ulpi_data_i;
                        rx_cmd_seen_q <= 1'b1;
                        line_q        <= ulpi_data_i[1:0];
                        case (ulpi_data_i[5:4])
                            RX_ON: begin
                                if (!rx_active_q) begin
                                    rx_error_q <= 1'b0;
                                    rx_any     <= 1'b0;
                                end
                                rx_active_q <= 1'b1;
                            end
                            RX_ERROR: begin
                                rx_active_q <= 1'b1;
                                rx_error_q  <= 1'b1;
                            end
                            RX_IDLE: begin
                                if (rx_active_q && rx_any && !rx_error_q)
                                    rx_eop_q <= 1'b1;
                                rx_active_q <= 1'b0;
                                rx_any      <= 1'b0;
                            end
                            default: begin
                                // 2'b10 is host disconnect, which is the
                                // transceiver's high-speed detector and
                                // says nothing at full speed: "When in FS
                                // or LS modes, the Link is expected to
                                // handle all disconnect detection"
                                // (DS00002646A §6.3.2.1). A full-speed
                                // detach is SE0 on `line_state`, and what
                                // sits above this module watches for it.
                            end
                        endcase
                    end
                end
                if (dir_fell) begin
                    // The other way a packet ends, and it ends whatever
                    // the Link thought was going on: "the RX CMD byte
                    // shows RxActive is set to 0b, **or `dir` is
                    // de-asserted**, whichever occurs first" (§3.8.2.4).
                    if (rx_active_q && rx_any && !rx_error_q) rx_eop_q <= 1'b1;
                    rx_active_q <= 1'b0;
                    rx_any      <= 1'b0;
                end
            end

            // A register request from above, taken whatever the bus is
            // doing, so that a design does not have to find the one cycle
            // it would otherwise be accepted in. It is held in `req_pend`
            // until it has been through, and the consumer waits for
            // `reg_done` before asking for the next one.
            if (ready_q && reg_start && !req_pend) begin
                req_pend  <= 1'b1;
                req_write <= reg_write;
                req_addr  <= reg_addr;
                req_data  <= reg_wdata;
                req_tries <= 0;
            end

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
                    S_SETTLE, S_READY: begin
                        // Nothing was in flight.
                    end
                    S_NEXT: begin
                        // **One state has somewhere to go while `dir` is
                        // high, and it is the wait for the transceiver's own
                        // reset.**
                        //
                        // §3.5: "When this bit is set, the transceiver will
                        // assert `dir` and reset the UTMI+ core." It asserts
                        // it at once — in the model of it beside this block,
                        // in the same cycle the `stp` that commits the write
                        // is seen — and that is the cycle `S_WR_STP` hands to
                        // `S_NEXT`. A `S_NEXT` that does nothing while `dir`
                        // is high therefore sits out the whole reset, and
                        // only starts waiting for it once `dir` has gone
                        // **low**, which is the reset being over.
                        //
                        // What that costs is not the wait, which still ends:
                        // it is the receive command. §3.5 promises exactly
                        // one after the reset — "the PHY must immediately
                        // re-assert `dir` and send an RX CMD update to the
                        // Link" — and a Link that is still in `S_WAIT_RST`
                        // when it arrives reads that re-assertion as the
                        // reset it was waiting for and the byte as nothing at
                        // all. Measured in simulation: thirteen cycles of
                        // `dir` with the Link in `S_NEXT`, then the receive
                        // command's own byte arriving in `S_WAIT_RST` and
                        // being dropped. For a peripheral that is invisible,
                        // because it has no use for a receive command it
                        // cannot get another of; for a host it is the
                        // difference between knowing whether the port has
                        // VBUS and never finding out.
                        if (step == I_WAIT) state <= S_WAIT_RST;
                    end
                    S_TX_CMD, S_TX_DATA, S_TX_CRC0, S_TX_CRC1: begin
                        // A transmit is dropped. Whatever is above this
                        // module has to send it again; §3.8.4.1 allows
                        // the transceiver to do this at any time and
                        // names no cause for it.
                        tx_abort_q <= 1'b1;
                        state      <= S_READY;
                    end
                    default: begin
                        // A register transaction, retried when the bus is
                        // idle, since neither `step` nor `req_pend` has
                        // moved on (§3.8.3.1). One from above is given up
                        // on after `REG_TRIES` of them.
                        if (ready_q
                                && req_tries == REG_TRIES[$clog2(REG_TRIES + 1)-1:0]) begin
                            req_pend   <= 1'b0;
                            reg_done_q <= 1'b1;
                            reg_ok_q   <= 1'b0;
                        end else if (ready_q) begin
                            req_tries <= req_tries + 1'b1;
                        end
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
                        end else if (!VENDOR_EN && (step == I_VEND
                                                 || step == I_VCHK)) begin
                            // No board-specific register: the two steps
                            // for one are not there at all.
                            step <= step + 4'd1;
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
                        if (ready_q) begin
                            // A write from above: it is through.
                            req_pend   <= 1'b0;
                            reg_done_q <= 1'b1;
                            reg_ok_q   <= 1'b1;
                            state      <= S_READY;
                        end else begin
                            step  <= step + 4'd1;
                            state <= S_NEXT;
                        end
                    end
                    S_WAIT_RST: begin
                        // §3.5: the transceiver asserts `dir` while it
                        // resets its core and lets go when it is done.
                        if (ulpi_dir) seen_dir <= 1'b1;
                        if (seen_dir && !ulpi_dir) begin
                            step  <= step + 4'd1;
                            state <= S_NEXT;
                        end else if (wait_cnt == RESET_CYCLES[15:0]) begin
                            // It never took the bus. Carry on anyway: the
                            // readback at the end is what decides.
                            step  <= step + 4'd1;
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
                            // it may do in any cycle of a read
                            // (§3.8.3.2): retry.
                            if (ready_q) req_tries <= req_tries + 1'b1;
                            state <= ready_q ? S_READY : S_NEXT;
                        end else if (dir_rose) begin
                            state <= S_RD_DATA;
                        end else if (wait_cnt == 16'd3) begin
                            // No answer at all. This is the way a read of
                            // an address the part does not implement ends,
                            // and it is why the retries are counted.
                            if (ready_q
                                    && req_tries == REG_TRIES[$clog2(REG_TRIES + 1)-1:0]) begin
                                req_pend   <= 1'b0;
                                reg_done_q <= 1'b1;
                                reg_ok_q   <= 1'b0;
                            end else if (ready_q) begin
                                req_tries <= req_tries + 1'b1;
                            end
                            state <= ready_q ? S_READY : S_NEXT;
                        end else begin
                            wait_cnt <= wait_cnt + 16'd1;
                        end
                    end
                    S_RD_DATA: begin
                        if (phy_drives && ulpi_nxt) begin
                            // A receive overrode the read in its last
                            // cycle. The byte on the bus is packet data
                            // and not the answer.
                            if (ready_q) req_tries <= req_tries + 1'b1;
                            state <= ready_q ? S_READY : S_NEXT;
                        end else if (phy_drives) begin
                            rd_q <= ulpi_data_i;
                            if (ready_q) begin
                                req_pend   <= 1'b0;
                                reg_done_q <= 1'b1;
                                reg_ok_q   <= 1'b1;
                                state      <= S_READY;
                            end else if (step == I_VCHK) begin
                                // The vendor register, read back. This is
                                // the one step of the sequence ULPI does
                                // not describe — the address and the byte
                                // come from a particular transceiver's
                                // datasheet — so it is believed only when
                                // it answers with what was written, and a
                                // part that has no such register goes
                                // round this loop rather than driving a
                                // bus the board is not wired for.
                                step  <= (ulpi_data_i == VENDOR_DATA) ? I_OTG : I_VEND;
                                state <= S_NEXT;
                            end else if (step == I_CHECK) begin
                                // Believed only when it is what was
                                // written; otherwise the two writes go
                                // again.
                                step  <= (ulpi_data_i == FUNC_CTRL_FS) ? I_LINE : I_OTG;
                                state <= S_NEXT;
                            end else begin
                                // `I_LINE`: the Debug register, whose low
                                // two bits are LineState. **Read once.**
                                //
                                // The peripheral's Link reads this again
                                // and again while it says SE0, because
                                // the 1.5 kOhm pull-up it has just
                                // connected takes milliseconds to charge
                                // the pair and the first answer is the
                                // line on its way up. A host has no
                                // pull-up to charge and nothing to wait
                                // for: SE0 is what its own two 15 kOhm
                                // pull-downs make of an empty socket, and
                                // it is the right answer. Waiting for J
                                // here would hang a host with nothing
                                // plugged in.
                                //
                                // It is kept as a reading all the same,
                                // because it is the one LineState a host
                                // gets without waiting for a receive
                                // command, and a transceiver sends one of
                                // those only when something changes.
                                line_q        <= ulpi_data_i[1:0];
                                step          <= I_DONE;
                                state         <= S_NEXT;
                            end
                        end else if (!ulpi_dir) begin
                            // The transceiver let go without driving the
                            // byte at all.
                            if (ready_q) req_tries <= req_tries + 1'b1;
                            state <= ready_q ? S_READY : S_NEXT;
                        end
                    end
                    S_READY: begin
                        // A packet first: a transmit waiting on a
                        // register read would miss its slot, and
                        // `usb_host_sie` never asks for both at once.
                        if (tx_start) begin
                            data_out <= {CMD_TX, 2'b00, tx_pid};
                            len_q    <= tx_len;
                            mode_q   <= tx_mode;
                            idx      <= 7'd0;
                            crc      <= 16'hFFFF;
                            state    <= S_TX_CMD;
                        end else if (req_pend) begin
                            data_out <= st_cmd;
                            state    <= st_write ? S_WR_CMD : S_RD_CMD;
                        end
                    end
                    S_TX_CMD: begin
                        if (ulpi_nxt) begin
                            if (mode_q == TX_HANDSHAKE) begin
                                // A handshake is the command and nothing
                                // else.
                                data_out  <= 8'h00;
                                stp_q     <= 1'b1;
                                tx_done_q <= 1'b1;
                                state     <= S_READY;
                            end else if (len_q == 7'd0) begin
                                // A zero-length data packet still carries
                                // the CRC16 of nothing, which is 0x0000.
                                // A raw transmit of no bytes is not a
                                // packet USB has, and `usb_host_sie` never
                                // asks for one, so it lands here as a
                                // zero-length data packet.
                                data_out <= ~crc[7:0];
                                state    <= S_TX_CRC0;
                            end else begin
                                data_out <= tx_byte;
                                idx      <= 7'd1;
                                state    <= S_TX_DATA;
                            end
                        end
                    end
                    S_TX_DATA: begin
                        if (ulpi_nxt) begin
                            crc <= crc_next;
                            if (last_byte) begin
                                if (mode_q == TX_RAW) begin
                                    // A token: the two bytes it was given
                                    // and nothing after them. The CRC5 is
                                    // already inside them.
                                    data_out  <= 8'h00;
                                    stp_q     <= 1'b1;
                                    tx_done_q <= 1'b1;
                                    state     <= S_READY;
                                end else begin
                                    data_out <= ~crc_next[7:0];
                                    state    <= S_TX_CRC0;
                                end
                            end else begin
                                data_out <= tx_byte;
                                idx      <= idx + 7'd1;
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
                            data_out  <= 8'h00;
                            stp_q     <= 1'b1;
                            tx_done_q <= 1'b1;
                            state     <= S_READY;
                        end
                    end
                endcase
            end
        end
    end
endmodule
