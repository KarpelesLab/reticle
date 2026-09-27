// A USB device on a Cynthion's AUX port, with a ULPI bus trace on the console.
//
// `testdata/fpga/cynthion/usb_ulpi_device.v` plus a logic analyser: EVENTS
// entries of what the ULPI bus did, starting at the first packet byte whose
// PID matches TRIG_PID, printed as hex over the UART on T14, which Apollo
// bridges to /dev/ttyACM0 on the machine the CONTROL port is plugged into.
//
// Why a trace rather than counters: a counter is arithmetic and a saturating
// counter behind a condition is a clock enable, and the first version of this
// probe reported a transmit count of zero beside a byte count that could only
// have come from transmits. A shift register has one enable for all of it and
// the reader does the arithmetic.
//
// WHY T14 IS SAFE
// ---------------
// T14 is the FPGA's `uart.tx` in Great Scott Gadgets' platform file and it is
// on the same net as the debug microcontroller's JTAG `TMS` output — their
// file says so, and the PCB has no series resistor. Three things keep that
// from being two drivers on one wire:
//
//  1. the pad drives only inside a window, about 0.28 s to 17.9 s after
//     configuration, and is high impedance before and after. The window is a
//     counter and two latches and nothing else can hold it open;
//  2. `PULLMODE=UP`, as the platform file asks, so the released line idles;
//  3. Apollo hands the pin to its SERCOM for the console and takes it back
//     for JTAG (`jtag_init` calls `uart_release_pinmux`).
//
// So do not start a JTAG transaction until the window has shut.
//
// HOW TO READ IT FROM THE HOST
// ---------------------------
// Load it, then open `/dev/ttyACM0` at 115200 8N1 raw — **after asking for some
// other rate first**. Apollo only re-initialises its SERCOM when the host
// *changes* the CDC line coding (`tud_cdc_line_coding_cb`);
// `tud_cdc_line_state_cb` skips it when the firmware already thinks the UART is
// active, which it does after every JTAG transaction, because `jtag_deinit`
// calls `uart_configure_pinmux`. So 9600 and then 115200 gets a console where
// 115200 alone gets silence. That cost an hour, and so did this: the window's
// latches have to be written `q <= q | e`, because `if (e) q <= 1'b1` infers a
// clock enable and the pad never drove at all. `usb_ulpi_device.v`'s header had
// already said so about its LED latches.
//
// The loop each run wants is `quiesce`, then this, then the console, then
// `quiesce` again — `docs/fpga-trellis.md` has why, and has a decoded trace to
// compare against.
//
// THE FORMAT
// ----------
// One line per dump: `X`, two hex digits of flags, two of the transmitted-PID
// bitmap, then EVENTS entries of four hex digits each, then CRLF. The bitmap is
// `{ack, nak, stall, data0, data1, other, 0, 0}`. Each entry is
//
//     gggg r d n s bbbbbbbb
//
// as sixteen bits, most significant first: a four-bit gap, `usb_reset`, then
// `dir`, `nxt`, `stp`, then the eight-bit bus. The gap is how many clocks
// passed since the previous entry **less one**, saturating at 15 — so add one
// to it and `F` means "at least 16".
// An entry is recorded whenever any of the eleven bits changes, so the trace
// is the bus and nothing but the bus.
//
// The flags byte is `{phy_ready, saw_reset, triggered, full, bus_reset_now,
// configured, addressed, bus_reset_after_the_device_answered}`.
//
// `bbbbbbbb` is the **pad**, so it is what the link drives while `dir` is low
// and what the transceiver drives while `dir` is high.

module usb_ulpi_trace #(
    parameter integer POR = 16,
    parameter [6:0] TURNAROUND = 7'd9,
    parameter integer BAUD_DIV = 521,
    parameter integer GAP_BITS = 13,
    parameter integer OPEN_BIT = 24,
    parameter integer SHUT_BIT = 30,
    // How many bus entries the trace holds.
    parameter integer EVENTS = 96,
    // The trace starts at the first packet byte equal to this. 2Dh is a SETUP
    // token's PID byte, which is where a control transfer begins; C3h is a
    // DATA0, 69h an IN token, A5h a start of frame.
    parameter [7:0] TRIG_PID = 8'h2D,
    // ...whose **second** byte is this. A root hub repeats every downstream
    // packet to every enabled port, so this device sees the host's whole
    // conversation with every other device on the bus, and a token's PID alone
    // does not say whose it is. A token's second byte is `{endp[0],
    // addr[6:0]}`, so `00h` is address zero, endpoint zero — which is this
    // device and nothing else, since no other device is unaddressed.
    parameter [7:0] TRIG_B1 = 8'h00,
    // How many matches to let go by first.
    parameter integer TRIG_SKIP = 0,
    // Whether the trigger waits for `phy_ready`. A testbench with no
    // transceiver behind it sets this to zero.
    parameter NEED_READY = 1'b1
) (
    input  wire clk,

    inout  wire [7:0] ulpi_data,
    input  wire ulpi_dir,
    input  wire ulpi_nxt,
    output wire ulpi_stp,
    output wire ulpi_rst_n,
    output wire ulpi_clk,

    inout  wire uart_tx,

    output wire led0_n,
    output wire led1_n,
    output wire led2_n,
    output wire led3_n,
    output wire led4_n,
    output wire led5_n
);
    localparam integer BITS = EVENTS * 16;

    // -----------------------------------------------------------------
    // THE WINDOW IN WHICH THE PAD MAY DRIVE
    // -----------------------------------------------------------------
    // `clock_blink.v`'s spelling of `+ 1`, and latches written `q <= q | e`:
    // `if (e) q <= 1'b1` infers a clock enable and this design's console
    // stayed silent on the part with it.
    reg [SHUT_BIT:0] age = {(SHUT_BIT+1){1'b0}};
    wire [SHUT_BIT:0] atog;
    assign atog[0] = 1'b1;
    genvar a;
    generate
        for (a = 1; a <= SHUT_BIT; a = a + 1) begin : agecarry
            assign atog[a] = &age[a-1:0];
        end
    endgenerate
    reg win_open = 1'b0;
    reg win_shut = 1'b0;
    always @(posedge clk) begin
        age      <= age ^ atog;
        win_open <= win_open | age[OPEN_BIT];
        win_shut <= win_shut | age[SHUT_BIT];
    end
    wire window = win_open & ~win_shut;

    wire [7:0] data_o;
    wire       data_oe;
    wire [6:0] address;
    wire       configured;
    wire       usb_reset;
    wire       phy_ready;

    assign ulpi_data = data_oe ? data_o : 8'bz;
    assign ulpi_clk  = clk;

    usb_device_ulpi #(
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06)
    ) u_dev (
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
        .phy_ready    (phy_ready),
        // Endpoint 1 is tied off here. This design is an instrument for the
        // conversation on the ULPI bus, and what it traces is the bus and
        // not the bytes above it; `usb_ulpi_device.v` is the one that loops
        // the data endpoint back. An OUT packet is still received and
        // acknowledged, and then not taken, so a second one is NAKed —
        // which is correct and is also what the trace would show.
        .out_data     (),
        .out_valid    (),
        .out_last     (),
        .out_ready    (1'b0),
        .in_data      (8'd0),
        .in_valid     (1'b0),
        .in_ready     (),
        .in_commit    (1'b0)
    );

    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) begin
        por <= {por[POR-2:0], 1'b1};
    end
    wire reset_done = por[POR-1];

    // -----------------------------------------------------------------
    // THE TRIGGER
    // -----------------------------------------------------------------
    reg  dir_q = 1'b0;
    always @(posedge clk) dir_q <= ulpi_dir;
    wire phy_drives = ulpi_dir & dir_q;
    wire [7:0] pin  = ulpi_data;

    // The first byte of a receive packet, which is its PID.
    reg in_pkt     = 1'b0;
    reg first_byte = 1'b0;
    always @(posedge clk) begin
        if ((ulpi_dir & ~dir_q) & ulpi_nxt) begin
            in_pkt     <= 1'b1;
            first_byte <= 1'b1;
        end else if (phy_drives & ulpi_nxt) begin
            first_byte <= 1'b0;
        end else if (phy_drives & (pin[5:4] == 2'b01) & ~in_pkt) begin
            in_pkt     <= 1'b1;
            first_byte <= 1'b1;
        end else if (phy_drives & (pin[5:4] == 2'b00)) begin
            in_pkt     <= 1'b0;
            first_byte <= 1'b0;
        end else if (~ulpi_dir & dir_q) begin
            in_pkt     <= 1'b0;
            first_byte <= 1'b0;
        end
    end
    // The PID matched, and the **next byte of the packet** is the one still to
    // check. At full speed that byte is forty clocks away, not one, so this is
    // a flag that waits for a byte rather than a pipeline register — which is
    // what it was, and the testbench could not tell, because a stub
    // transceiver hands over bytes back to back.
    wire byte_here = phy_drives & ulpi_nxt;
    wire pid_match = byte_here & first_byte & (pin == TRIG_PID)
                   & (phy_ready | ~NEED_READY);
    reg  pid_seen  = 1'b0;
    always @(posedge clk) begin
        pid_seen <= pid_match ? 1'b1 : (byte_here ? 1'b0 : pid_seen);
    end
    wire pid_here = byte_here & pid_seen & (pin == TRIG_B1);

    // Skipped matches, and then the trigger, latched.
    reg [7:0] skipped = 8'd0;
    reg       armed   = 1'b0;
    always @(posedge clk) begin
        skipped <= skipped
                 + {7'd0, (pid_here & ~armed & (skipped != TRIG_SKIP[7:0]))};
        armed   <= armed | (pid_here & (skipped == TRIG_SKIP[7:0]));
    end

    // -----------------------------------------------------------------
    // THE TRACE
    // -----------------------------------------------------------------
    wire [11:0] bus = {usb_reset, ulpi_dir, ulpi_nxt, ulpi_stp, pin};
    reg  [11:0] bus_q = 12'd0;

    // **A receive command whose only news is LineState is not recorded.**
    // At full speed this transceiver sends one receive command per ULPI cycle
    // for the whole of a packet, and LineState follows the pair bit by bit, so
    // a trace that records every change spends ninety of its ninety-six
    // entries watching a J turn into a K. RxEvent, RxError, VbusState, the ID
    // bit and every byte the link drives are all above those two bits, so
    // comparing all of the bus but `data[1:0]` — and only while the byte is a
    // receive command — keeps everything that says what either end *did*.
    wire rxcmd_cycle = ulpi_dir & dir_q & ~ulpi_nxt;
    wire only_line   = rxcmd_cycle & (bus[11:2] == bus_q[11:2]);
    // **An empty turnaround is not news either.** A single receive command is
    // three bus cycles — `dir` up with nothing on the bus, the byte, `dir`
    // down with nothing on the bus — and the transceiver sends tens of them
    // after every packet. Recording only the middle one turns three entries
    // into one and is what makes a whole control transfer fit in sixty-four.
    wire empty_turn  = (ulpi_dir != dir_q) & ~ulpi_nxt & (pin == 8'h00);
    wire changed     = (bus != bus_q) & ~only_line & ~empty_turn;

    reg  [3:0]  gap = 4'd0;
    always @(posedge clk) begin
        bus_q <= bus;
        gap   <= changed ? 4'd0
               : (gap == 4'd15) ? 4'd15
               : gap + 4'd1;
    end

    reg [7:0] filled = 8'd0;
    wire full   = (filled == EVENTS[7:0]);
    wire record = armed & ~full & changed;
    always @(posedge clk) begin
        filled <= filled + {7'd0, record};
    end

    // One shift register for both jobs, and it only ever shifts sixteen bits,
    // so every flip-flop of it has one data path and one enable: the entry
    // during the capture, its own top sixteen bits during the dump, which
    // rotates it back to where it started after EVENTS of them.
    reg  [BITS-1:0] trace = {BITS{1'b0}};
    wire [15:0] entry = {gap, bus};   // four bits of gap, then twelve of bus
    reg         rotate;           // the console asks for the next entry
    wire        shift  = record | rotate;
    wire [15:0] into   = record ? entry : trace[BITS-1:BITS-16];
    always @(posedge clk) begin
        if (shift) trace <= {trace[BITS-17:0], into};
    end

    // -----------------------------------------------------------------
    // THE CONSOLE
    // -----------------------------------------------------------------
    reg saw_rst = 1'b0;
    always @(posedge clk) saw_rst <= saw_rst | usb_reset;
    reg rst_now      = 1'b0;
    reg tx_seen      = 1'b0;
    reg rst_after_tx = 1'b0;
    always @(posedge clk) begin
        rst_now      <= usb_reset;
        tx_seen      <= tx_seen | (~ulpi_dir & (data_o[7:6] == 2'b01));
        rst_after_tx <= rst_after_tx | (usb_reset & tx_seen);
    end
    wire [7:0] flags = {phy_ready, saw_rst, armed, full,
                        rst_now, configured, address != 7'd0, rst_after_tx};

    // Which PIDs the link has ever driven a transmit command for. `tx_data1`
    // is the one that matters: it is set if and only if `usb_ctrl_ep` was in
    // its data stage when an IN token arrived.
    wire tx_cmd_now = ~ulpi_dir & (data_o[7:6] == 2'b01);
    reg tx_ack = 1'b0, tx_nak = 1'b0, tx_stall = 1'b0;
    reg tx_d0 = 1'b0, tx_d1 = 1'b0, tx_other = 1'b0;
    always @(posedge clk) begin
        tx_ack   <= tx_ack   | (tx_cmd_now & (data_o[3:0] == 4'h2));
        tx_nak   <= tx_nak   | (tx_cmd_now & (data_o[3:0] == 4'hA));
        tx_stall <= tx_stall | (tx_cmd_now & (data_o[3:0] == 4'hE));
        tx_d0    <= tx_d0    | (tx_cmd_now & (data_o[3:0] == 4'h3));
        tx_d1    <= tx_d1    | (tx_cmd_now & (data_o[3:0] == 4'hB));
        tx_other <= tx_other | (tx_cmd_now & (data_o[3:0] != 4'h2)
                                           & (data_o[3:0] != 4'hA)
                                           & (data_o[3:0] != 4'hE)
                                           & (data_o[3:0] != 4'h3)
                                           & (data_o[3:0] != 4'hB));
    end
    wire [7:0] txpids = {tx_ack, tx_nak, tx_stall, tx_d0, tx_d1, tx_other,
                         2'b00};

    // pos 0 is 'X', 1 and 2 the flags, 3 .. 2+4*EVENTS the entries, then CRLF
    // and the gap.
    localparam integer LAST = 5 + 4 * EVENTS;   // the CR
    reg [15:0] pos    = 16'd0;
    reg [9:0]  baud   = 10'd0;
    reg [3:0]  bitno  = 4'd0;
    reg [7:0]  shreg  = 8'h00;
    reg        line_q = 1'b1;
    reg [GAP_BITS-1:0] idle = {GAP_BITS{1'b0}};

    // Which nibble of the head of the trace, and of the flags.
    wire [15:0] q = pos - 16'd5;
    wire [3:0]  tnib = (q[1:0] == 2'd0) ? trace[BITS-1:BITS-4]
                     : (q[1:0] == 2'd1) ? trace[BITS-5:BITS-8]
                     : (q[1:0] == 2'd2) ? trace[BITS-9:BITS-12]
                     :                    trace[BITS-13:BITS-16];
    wire [3:0]  fnib = (pos == 16'd1) ? flags[7:4]
                     : (pos == 16'd2) ? flags[3:0]
                     : (pos == 16'd3) ? txpids[7:4]
                     :                  txpids[3:0];
    wire [3:0]  nib  = (pos < 16'd5) ? fnib : tnib;
    wire [7:0]  hex  = (nib < 4'd10) ? (8'h30 + {4'd0, nib})
                                     : (8'h37 + {4'd0, nib});
    reg [7:0] chr;
    always @(*) begin
        if (pos == 16'd0)                chr = 8'h58;  // 'X'
        else if (pos == LAST[15:0])      chr = 8'h0D;
        else if (pos == LAST[15:0] + 16'd1) chr = 8'h0A;
        else                             chr = hex;
    end

    wire sending = (bitno != 4'd0);

    // **A dump only rotates the trace if it began after the trace was full.**
    // The rotation is what walks the shift register, and EVENTS of them bring
    // it back to where it started — so a dump that begins while the trace is
    // still filling and rotates partway through leaves the register
    // permanently out of step, and every later dump prints the entries in the
    // right cyclic order starting from the wrong one. That is not a
    // hypothetical: the testbench printed a trace rotated by four entries
    // until this was added.
    reg dump_ok = 1'b0;
    wire start_char = window & ~sending & (pos != LAST[15:0] + 16'd2);
    always @(posedge clk) begin
        if (!window) dump_ok <= 1'b0;
        else if (start_char & (pos == 16'd0)) dump_ok <= full;
    end

    // The trace advances by one entry after its fourth nibble has been sent.
    always @(*) begin
        rotate = 1'b0;
        // Only once the trace is full: until then `record` owns the shift
        // register, and two shifts in one cycle would mix the two jobs.
        if (window & full & dump_ok & ~sending & (pos >= 16'd5)
                   & (pos < LAST[15:0]) & (q[1:0] == 2'd3))
            rotate = 1'b1;
    end

    always @(posedge clk) begin
        if (!window) begin
            bitno  <= 4'd0;
            baud   <= 10'd0;
            pos    <= 16'd0;
            idle   <= {GAP_BITS{1'b0}};
            line_q <= 1'b1;
        end else if (sending) begin
            if (baud == BAUD_DIV[9:0]) begin
                baud <= 10'd0;
                if (bitno == 4'd10) begin
                    bitno  <= 4'd0;
                    line_q <= 1'b1;
                end else begin
                    bitno  <= bitno + 4'd1;
                    line_q <= (bitno == 4'd9) ? 1'b1 : shreg[0];
                    shreg  <= {1'b1, shreg[7:1]};
                end
            end else begin
                baud <= baud + 10'd1;
            end
        end else if (pos == LAST[15:0] + 16'd2) begin
            if (idle == {GAP_BITS{1'b1}}) begin
                idle <= {GAP_BITS{1'b0}};
                pos  <= 16'd0;
            end else begin
                idle <= idle + {{(GAP_BITS-1){1'b0}}, 1'b1};
            end
        end else begin
            shreg  <= chr;
            bitno  <= 4'd1;
            baud   <= 10'd0;
            line_q <= 1'b0;
            pos    <= pos + 16'd1;
        end
    end

    assign uart_tx = window ? line_q : 1'bz;

    reg [25:0] count = 26'd0;
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;
    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : carry
            assign toggle[i] = &count[i-1:0];
        end
    endgenerate
    always @(posedge clk) begin
        count <= count ^ toggle;
    end

    assign led0_n = ~phy_ready;
    assign led1_n = ~configured;
    assign led2_n = ~(address != 7'd0);
    assign led3_n = ~count[25];
    assign led4_n = ~saw_rst;
    assign led5_n = ~armed;

endmodule
