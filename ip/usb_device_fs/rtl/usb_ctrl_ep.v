// The device above the line: four modules, in one file.
//
// `usb_pkt_rx` decodes a packet, `usb_ctrl_ep` is endpoint 0, `usb_bulk_ep`
// is the endpoint that moves bytes, and `usb_dev_core` is the three of them
// and the transmitter they share. Each has its own header below and each
// would rather be its own file, the way every other module in this library
// is.
//
// **They are one file because two tests outside this package name the files
// of it one by one.** `tests/fpga_trellis.rs` elaborates
// `testdata/fpga/cynthion/usb_ulpi_device.v` and `usb_ulpi_trace.v` from a
// list of paths written out in Rust, and a module those paths do not reach
// does not fail to elaborate — it becomes a **black box whose outputs are
// undefined**, and the first of those two tests then reports `FAIL: LED 1 is
// lit and no host has configured anything` from a design that is perfectly
// well. That is a sharp edge in the elaborator rather than in the tests, and
// it is not this round's to file down.
//
// Splitting this file back into four is a `git mv` and three lines in each of
// those two lists. Whoever is free to edit that file should do it.

// usb_ctrl_ep — USB endpoint 0: the control endpoint of a USB device, with
// no knowledge of how its bytes reach the bus and none of what the device is
// for.
//
// What it does
//   Everything a host needs to enumerate a device, above the line: the
//   data toggle, the control transfer's three stages, and the standard
//   requests. `usb_pkt_rx` beside it does the packet decoding — the PID
//   check nibble, the CRC5 of tokens and the CRC16 of data packets — and
//   this reads its one pulse per whole packet, so a second endpoint gets
//   the same decoding without a second copy of it.
//
//   It is shared. `usb_device_fs` puts it behind its own full-speed
//   encoder and serialiser; `usb_device_ulpi` puts it behind a ULPI
//   transceiver, which does that work in silicon. There is one statement
//   of the control endpoint for both, the way `eth_mac_tx` and
//   `eth_mac_rx` are one statement of Ethernet framing for RMII and
//   RGMII.
//
//   Endpoint 0, maximum packet size 8:
//
//     GET_DESCRIPTOR, device        the 18-byte device descriptor, VID,
//                                   PID and the class triple from the
//                                   parameters
//     GET_DESCRIPTOR, configuration the 9-byte configuration descriptor
//                                   this module writes, followed by the
//                                   interface and endpoint descriptors
//                                   `IFACE_DESC` holds
//     SET_ADDRESS                   taken after the status stage, as the
//                                   specification says
//     SET_CONFIGURATION 0 or 1      accepted; `configured` follows it,
//                                   and every data toggle goes back to
//                                   DATA0 as USB 2.0 §9.4.5 asks
//     CLEAR_FEATURE ENDPOINT_HALT   accepted; that one endpoint's data
//                                   toggle goes back to DATA0, which is
//                                   how a host and a device agree on a
//                                   toggle again without a bus reset
//
//   A descriptor goes out in as many DATA1 / DATA0 packets as it takes,
//   never more than the host's wLength, and a packet the host does not
//   acknowledge is sent again with the same toggle. The status stage is
//   an OUT of zero length after a read and an IN of zero length after
//   the others. Anything else — string descriptors, GET_STATUS,
//   requests to an interface or an endpoint, class and vendor requests —
//   is answered with STALL until the next SETUP.
//
//   A token addressed elsewhere, or to another endpoint, is ignored.
//   `bus_reset` sets the address back to 0 and the configuration to none.
//
// THE CONFIGURATION DESCRIPTOR IS THE CLASS'S, NOT THIS MODULE'S
//   This used to be eighteen bytes of `case` in here: a configuration, one
//   interface, no endpoints, and `wTotalLength` typed out as `18` four
//   lines above the `9` and the `9` it is the sum of. A device that moves
//   bytes has endpoint descriptors, a device that is a HID has an
//   interface that says so, and none of that belongs to endpoint 0.
//
//   So a design states its interface and endpoint descriptors, and this
//   module states the nine bytes of the configuration descriptor that
//   wrap them — because those nine bytes are **arithmetic over the rest**
//   and arithmetic is what gets restated wrongly:
//
//     wTotalLength    9 + IFACE_BYTES, computed here
//     bNumInterfaces  the INTERFACE descriptors in `IFACE_DESC`, counted
//                     here at elaboration
//     bNumEndpoints   the ENDPOINT descriptors after each INTERFACE
//                     descriptor, counted here at elaboration and
//                     **written over** whatever byte 4 of that interface
//                     descriptor held
//
//   None of the three can disagree with the descriptors, because none of
//   the three is read from them. `IFACE_DESC` is a concatenation, so it is
//   written in descriptor order — most significant part first is what
//   Verilog concatenates, and that is the order `lsusb -v` prints — and
//   `in_index_order` below turns it round once, at elaboration, into the
//   byte-indexed form the rest of this file uses. `IFACE_BYTES` is that
//   concatenation's length in bytes and is the one number a design states
//   twice: get it wrong and the value is truncated at its top, so byte 0
//   is no longer a bLength and `bNumInterfaces` comes out wrong, which
//   `tests/ip_library.rs` compares against a descriptor written forwards
//   in Rust.
//
//   A wide parameter was chosen over a descriptor module with a byte port
//   for one reason: a descriptor is not logic. As a parameter it is a
//   constant this module indexes, `bNumEndpoints` can be counted by a
//   constant function at elaboration, and a design that wants a different
//   class changes one instantiation rather than adding a module and four
//   wires to its top level. A descriptor module would have put the
//   arithmetic back in two places — the module's bytes and the core's
//   length — which is the thing being fixed.
//
//   The receive side is `usb_pkt_rx`'s one pulse per packet. The transmit
//   side is a packet at a time: `tx_start` with `tx_pid`, `tx_with_data`
//   and `tx_len`, the payload fetched through `tx_index` and `tx_byte` in
//   the same cycle, and `tx_busy` until the packet is gone. `sel` is what
//   says this endpoint owns the transmitter, since a device with more than
//   one endpoint has more than one thing that could answer.
//
//   `TURNAROUND` is how many cycles of `line_idle` must pass after a
//   host packet before the answer starts, and what it has to be depends
//   on the layer below. Eight cycles is two bit times on
//   `usb_device_fs`'s 48 MHz clock. ULPI states the same delay as a
//   count of 60 MHz clocks — 7 to 18 for a full-speed link, ULPI 1.1
//   Table 10 — from the receive command that reports the bus back at J.
//
// What it does not do
//   Endpoint 0 only. `usb_bulk_ep` is the endpoint that moves bytes and
//   it sits beside this one, reading the same `usb_pkt_rx`; `usb_dev_core`
//   is the two of them and the transmitter they share.
//
//   No strings, no remote wake-up, no suspend, no SOF tracking and no
//   low speed. Eight bytes of payload at most in either direction, which
//   is what a maximum packet size of eight needs. A configuration
//   descriptor of at most 9 + 64 bytes, which is `DESC_MAX` below and is
//   room for a HID interface or a CDC ACM pair.
//
//   Nothing here knows about NRZI, bit stuffing, SYNC, EOP or line
//   states, and nothing here checks a CRC — `usb_pkt_rx` does that, and
//   whatever transmits for this is expected to append one, because that
//   is the device's job in both arrangements: a ULPI transceiver prepends
//   SYNC and appends the EOP but never touches a CRC (ULPI 1.1 §3.8.2.2).
module usb_ctrl_ep #(
    parameter [15:0] VID          = 16'h1209,
    parameter [15:0] PID          = 16'h0001,
    // bDeviceClass, bDeviceSubClass and bDeviceProtocol. `FFh` is vendor
    // specific, which is what a device whose class lives in its interface
    // descriptor says; a CDC device says `02h` here instead.
    parameter [7:0]  DEV_CLASS    = 8'hFF,
    parameter [7:0]  DEV_SUBCLASS = 8'h00,
    parameter [7:0]  DEV_PROTOCOL = 8'h00,
    // bmAttributes and bMaxPower of the configuration: bus powered, 100 mA.
    parameter [7:0]  CFG_ATTR     = 8'h80,
    parameter [7:0]  CFG_POWER    = 8'd50,
    // The interface and endpoint descriptors, in descriptor order, and
    // their length in bytes. The default is one vendor-specific interface
    // with a bulk OUT and a bulk IN on endpoint 1, which is what
    // `usb_bulk_ep` implements.
    parameter integer IFACE_BYTES = 23,
    parameter [IFACE_BYTES*8-1:0] IFACE_DESC = {
        // INTERFACE: one interface, vendor specific. Byte 4 is
        // bNumEndpoints and is counted below rather than believed.
        8'd9, 8'd4, 8'd0, 8'd0, 8'd2, 8'hFF, 8'h00, 8'h00, 8'd0,
        // ENDPOINT 1 OUT: bulk, 8 bytes, no interval.
        8'd7, 8'd5, 8'h01, 8'd2, 8'd8, 8'd0, 8'd0,
        // ENDPOINT 1 IN: bulk, 8 bytes, no interval.
        8'd7, 8'd5, 8'h81, 8'd2, 8'd8, 8'd0, 8'd0
    },
    // Cycles of `line_idle` before an answer starts. Seven bits wide, and
    // not four, because what a **host** tolerates is wider than what ULPI
    // asks a Link for: USB 2.0 §7.1.19.1 has a host wait 16 bit times for a
    // device's response before calling it a timeout, which is 80 clocks of a
    // 60 MHz ULPI bus, and a four-bit field cannot reach a third of that. A
    // device that is not being heard is worth sweeping across the whole of
    // it rather than across ULPI's 7 to 18.
    parameter [6:0]  TURNAROUND = 7'd8
) (
    input  wire       clk,
    input  wire       rst_n,

    // One whole packet, from `usb_pkt_rx`.
    input  wire        pkt,
    input  wire [3:0]  pkt_pid,
    input  wire        pkt_is_token,
    input  wire        pkt_is_data,
    input  wire        tok_ok,
    input  wire [6:0]  tok_addr,
    input  wire [3:0]  tok_endp,
    input  wire        dat_ok,
    input  wire [3:0]  dat_len,
    input  wire [63:0] dat,

    // The bus is idle, so an answer may be timed from now.
    input  wire       line_idle,
    // The host has reset the bus.
    input  wire       bus_reset,
    // This endpoint owns the transmitter.
    input  wire       sel,

    // One packet out.
    output reg        tx_start,
    output reg  [3:0] tx_pid,
    output reg        tx_with_data,
    output reg  [3:0] tx_len,
    input  wire [3:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured,

    // Data toggles, for the endpoints beside this one. `ep_reset` is one
    // cycle when SET_CONFIGURATION has been accepted and every endpoint's
    // toggle goes back to DATA0; `ep_clear` is one cycle when
    // CLEAR_FEATURE(ENDPOINT_HALT) has been accepted for the endpoint
    // address `ep_clear_ep` names, direction bit and all.
    output reg        ep_reset,
    output reg        ep_clear,
    output reg  [7:0] ep_clear_ep
);
    // PIDs, the low nibble as it appears on the wire.
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_SETUP = 4'b1101;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;
    localparam [3:0] PID_ACK   = 4'b0010;
    localparam [3:0] PID_NAK   = 4'b1010;
    localparam [3:0] PID_STALL = 4'b1110;

    // Control transfer stages.
    // **Two bits, because there are four of them.**
    //
    // This was `[2:0]`, and the third bit was a bit no expression in this file
    // ever assigns anything but zero. Read back off a real ECP5 through a debug
    // port, `stage` was **5**: the third bit had come up set, `case (stage)`
    // matched none of its four labels, and every IN token the host sent was
    // answered from the `default` arm with a NAK. The host retried for five
    // seconds and gave up — `device descriptor read/64, error -110` — with the
    // device's SETUP acknowledgement, its receive path and its transmit path
    // all working.
    //
    // A state register wide enough for states that do not exist is a hostage to
    // whatever a backend does with a flip-flop whose data input is a constant,
    // and on this one an untied slice input reads as a one. So the width is the
    // number of states from now on, and `docs/fpga-trellis.md` carries the
    // backend half of it.
    localparam [1:0] C_IDLE       = 2'd0;
    localparam [1:0] C_DATA_IN    = 2'd1;
    localparam [1:0] C_STATUS_IN  = 2'd2;
    localparam [1:0] C_STALL      = 2'd3;

    // What the last token asked for.
    localparam [1:0] X_NONE  = 2'd0;
    localparam [1:0] X_SETUP = 2'd1;
    localparam [1:0] X_OUT   = 2'd2;

    // -----------------------------------------------------------------
    // The descriptors, worked out once at elaboration.
    // -----------------------------------------------------------------
    // Bytes of interface and endpoint descriptors this module can index.
    // A power of two, because the index is masked to its width rather than
    // trusted to be in range.
    localparam integer DESC_MAX = 64;

    // `IFACE_DESC` with byte 0 in the low eight bits.
    //
    // A concatenation's first element is its most significant, so taking
    // the low byte of the value repeatedly walks the descriptors
    // **backwards**, and shifting each one into the top of an accumulator
    // puts them back in order. Written with shifts and no part-selects on
    // purpose: an index that a width checker cannot bound is a warning at
    // best and a wrong byte at worst.
    function [DESC_MAX*8-1:0] in_index_order;
        input [IFACE_BYTES*8-1:0] blob;
        integer                   k;
        reg [IFACE_BYTES*8-1:0]   rest;
        reg [DESC_MAX*8-1:0]      out;
        begin
            rest = blob;
            out  = {(DESC_MAX*8){1'b0}};
            for (k = 0; k < IFACE_BYTES; k = k + 1) begin
                out  = (out << 8) | (rest & {{(IFACE_BYTES*8-8){1'b0}}, 8'hFF});
                rest = rest >> 8;
            end
            in_index_order = out;
        end
    endfunction

    // One byte of a byte-indexed blob.
    function [7:0] byte_at;
        input [DESC_MAX*8-1:0] blob;
        input integer          off;
        begin
            byte_at = (blob >> (off * 8)) & {{(DESC_MAX*8-8){1'b0}}, 8'hFF};
        end
    endfunction

    // A byte written over whatever was there, by XOR so that no mask of
    // the blob's width has to be spelled out.
    function [DESC_MAX*8-1:0] byte_over;
        input [DESC_MAX*8-1:0] blob;
        input integer          off;
        input integer          val;
        reg [7:0]              was, now;
        begin
            was       = byte_at(blob, off);
            now       = val[7:0];
            byte_over = blob ^ ({{(DESC_MAX*8-8){1'b0}}, was ^ now} << (off * 8));
        end
    endfunction

    // The INTERFACE descriptors in the blob, counted along the chain of
    // bLength fields.
    function [7:0] iface_count;
        input [DESC_MAX*8-1:0] blob;
        integer                k, len, n;
        begin
            n = 0;
            k = 0;
            while (k + 1 < IFACE_BYTES) begin
                len = byte_at(blob, k);
                if (len == 0) begin
                    k = IFACE_BYTES;
                end else begin
                    if (byte_at(blob, k + 1) == 8'd4) n = n + 1;
                    k = k + len;
                end
            end
            iface_count = n[7:0];
        end
    endfunction

    // bNumEndpoints of every interface descriptor, replaced by the number
    // of ENDPOINT descriptors that follow it before the next interface.
    function [DESC_MAX*8-1:0] with_endpoint_counts;
        input [DESC_MAX*8-1:0] blob;
        integer                k, len, kind, at, n;
        reg [DESC_MAX*8-1:0]   out;
        begin
            out = blob;
            at  = -1;
            n   = 0;
            k   = 0;
            while (k + 1 < IFACE_BYTES) begin
                len  = byte_at(blob, k);
                kind = byte_at(blob, k + 1);
                if (len == 0) begin
                    k = IFACE_BYTES;
                end else begin
                    if (kind == 4) begin
                        if (at >= 0) out = byte_over(out, at + 4, n);
                        at = k;
                        n  = 0;
                    end else if (kind == 5) begin
                        n = n + 1;
                    end
                    k = k + len;
                end
            end
            if (at >= 0) out = byte_over(out, at + 4, n);
            with_endpoint_counts = out;
        end
    endfunction

    localparam [DESC_MAX*8-1:0] IFACE = with_endpoint_counts(in_index_order(IFACE_DESC));
    localparam [7:0]            NUM_IFACE = iface_count(IFACE);
    // wTotalLength: the nine bytes below plus the class's own.
    localparam [15:0]           CFG_TOTAL = 16'd9 + IFACE_BYTES;

    // -----------------------------------------------------------------
    // Endpoint 0.
    // -----------------------------------------------------------------
    reg [6:0]  addr;
    reg [6:0]  pending_addr;
    reg        set_addr;
    reg        config_q;
    reg        pending_config;
    reg        set_config;
    reg [1:0]  stage;
    reg [1:0]  expect;
    reg        toggle;
    reg        desc_sel;    // 0 device, 1 configuration
    reg [6:0]  in_total;    // bytes the data stage sends
    reg [6:0]  in_offset;   // bytes the host has acknowledged
    reg [3:0]  in_len;      // bytes in the packet awaiting its ACK
    reg        await_ack;

    // A response waits for the turnaround after the host's EOP.
    reg        pending;
    reg [3:0]  pend_pid;
    reg        pend_data;
    reg [3:0]  pend_len;
    reg [6:0]  turn;

    assign address    = addr;
    assign configured = config_q;

    // The descriptors, one byte at a time.
    function [7:0] desc;
        input       sel_in;
        input [6:0] i;
        reg   [6:0] j;
        begin
            if (!sel_in) begin
                case (i)
                    7'd0:    desc = 8'd18;        // bLength
                    7'd1:    desc = 8'd1;         // DEVICE
                    7'd2:    desc = 8'h00;        // bcdUSB 2.00
                    7'd3:    desc = 8'h02;
                    7'd4:    desc = DEV_CLASS;
                    7'd5:    desc = DEV_SUBCLASS;
                    7'd6:    desc = DEV_PROTOCOL;
                    7'd7:    desc = 8'd8;         // bMaxPacketSize0
                    7'd8:    desc = VID[7:0];
                    7'd9:    desc = VID[15:8];
                    7'd10:   desc = PID[7:0];
                    7'd11:   desc = PID[15:8];
                    7'd12:   desc = 8'h00;        // bcdDevice 1.00
                    7'd13:   desc = 8'h01;
                    7'd14:   desc = 8'd0;         // no strings
                    7'd15:   desc = 8'd0;
                    7'd16:   desc = 8'd0;
                    7'd17:   desc = 8'd1;         // one configuration
                    default: desc = 8'd0;
                endcase
            end else if (i < 7'd9) begin
                case (i[3:0])
                    4'd0:    desc = 8'd9;         // bLength
                    4'd1:    desc = 8'd2;         // CONFIGURATION
                    4'd2:    desc = CFG_TOTAL[7:0];
                    4'd3:    desc = CFG_TOTAL[15:8];
                    4'd4:    desc = NUM_IFACE;
                    4'd5:    desc = 8'd1;         // bConfigurationValue
                    4'd6:    desc = 8'd0;         // iConfiguration
                    4'd7:    desc = CFG_ATTR;
                    default: desc = CFG_POWER;
                endcase
            end else begin
                // The class's own descriptors, the index masked to the
                // blob's width so that no expression here can reach
                // outside it.
                // A byte index scaled to a bit index by concatenation and
                // not by `* 8`: this compiler's synthesis leaves a multiply
                // by a constant as a `mul` cell, which on the ECP5 is a
                // hard multiplier.
                j    = i - 7'd9;
                desc = IFACE[{j[5:0], 3'b000} +: 8];
            end
        end
    endfunction

    assign tx_byte = desc(desc_sel, in_offset + {3'b000, tx_index});

    // The next packet of the data stage.
    wire [6:0] in_left  = in_total - in_offset;
    wire [3:0] in_chunk = (in_left > 7'd8) ? 4'd8 : in_left[3:0];

    // The SETUP request, from the data packet in the cycle it arrives.
    wire [7:0]  s0 = dat[7:0];
    wire [7:0]  s1 = dat[15:8];
    wire [7:0]  s2 = dat[23:16];
    wire [7:0]  s3 = dat[31:24];
    wire [7:0]  s4 = dat[39:32];
    wire [15:0] w_length = dat[63:48];
    wire        get_desc = (s0 == 8'h80) & (s1 == 8'h06) & (s2 == 8'h00)
                         & ((s3 == 8'h01) | (s3 == 8'h02));
    wire        set_adr  = (s0 == 8'h00) & (s1 == 8'h05) & ~s2[7] & (s3 == 8'h00);
    wire        set_cfg  = (s0 == 8'h00) & (s1 == 8'h09) & (s2[7:1] == 7'd0) & (s3 == 8'h00);
    // CLEAR_FEATURE(ENDPOINT_HALT) on an endpoint: bmRequestType 02h is
    // host to device, standard, to an endpoint; bRequest 01h is
    // CLEAR_FEATURE; wValue 0000h is ENDPOINT_HALT; wIndex is the
    // endpoint's address. Nothing here ever halts an endpoint, so this is
    // accepted for its other documented effect, which is the toggle.
    wire        clr_halt = (s0 == 8'h02) & (s1 == 8'h01) & (s2 == 8'h00) & (s3 == 8'h00);
    wire [6:0]  desc_len = (s3 == 8'h02) ? CFG_TOTAL[6:0] : 7'd18;
    wire [6:0]  send_len = (w_length < {9'd0, desc_len}) ? w_length[6:0] : desc_len;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            addr           <= 7'd0;
            pending_addr   <= 7'd0;
            set_addr       <= 1'b0;
            config_q       <= 1'b0;
            pending_config <= 1'b0;
            set_config     <= 1'b0;
            stage          <= C_IDLE;
            expect         <= X_NONE;
            toggle         <= 1'b0;
            desc_sel       <= 1'b0;
            in_total       <= 7'd0;
            in_offset      <= 7'd0;
            in_len         <= 4'd0;
            await_ack      <= 1'b0;
            pending        <= 1'b0;
            pend_pid       <= 4'd0;
            pend_data      <= 1'b0;
            pend_len       <= 4'd0;
            turn           <= 7'd0;
            tx_start       <= 1'b0;
            tx_pid         <= 4'd0;
            tx_with_data   <= 1'b0;
            tx_len         <= 4'd0;
            ep_reset       <= 1'b0;
            ep_clear       <= 1'b0;
            ep_clear_ep    <= 8'd0;
        end else begin
            tx_start <= 1'b0;
            ep_reset <= 1'b0;
            ep_clear <= 1'b0;

            // A whole packet.
            if (pkt) begin
                if (pkt_is_token) begin
                    // A token ends any wait for a handshake.
                    await_ack <= 1'b0;
                    expect    <= X_NONE;
                    if (tok_ok && tok_addr == addr && tok_endp == 4'd0) begin
                        if (pkt_pid == PID_SETUP) begin
                            expect <= X_SETUP;
                        end else if (pkt_pid == PID_OUT) begin
                            expect <= X_OUT;
                        end else begin
                            // IN: answer from the stage we are in.
                            pending <= 1'b1;
                            turn    <= 7'd0;
                            case (stage)
                                C_DATA_IN: begin
                                    pend_pid  <= toggle ? PID_DATA1 : PID_DATA0;
                                    pend_data <= 1'b1;
                                    pend_len  <= in_chunk;
                                    in_len    <= in_chunk;
                                    await_ack <= 1'b1;
                                end
                                C_STATUS_IN: begin
                                    pend_pid  <= PID_DATA1;
                                    pend_data <= 1'b1;
                                    pend_len  <= 4'd0;
                                    await_ack <= 1'b1;
                                end
                                C_STALL: begin
                                    pend_pid  <= PID_STALL;
                                    pend_data <= 1'b0;
                                end
                                default: begin
                                    pend_pid  <= PID_NAK;
                                    pend_data <= 1'b0;
                                end
                            endcase
                        end
                    end
                end else if (pkt_is_data) begin
                    expect <= X_NONE;
                    if (dat_ok && expect == X_SETUP) begin
                        // A SETUP is always acknowledged, and starts a
                        // new control transfer whatever the last one was
                        // doing.
                        pending   <= 1'b1;
                        turn      <= 7'd0;
                        pend_pid  <= PID_ACK;
                        pend_data <= 1'b0;
                        toggle    <= 1'b1;
                        in_offset <= 7'd0;
                        set_addr  <= 1'b0;
                        set_config <= 1'b0;
                        if (pkt_pid != PID_DATA0 || dat_len != 4'd8) begin
                            stage <= C_STALL;
                        end else if (get_desc) begin
                            stage    <= C_DATA_IN;
                            desc_sel <= (s3 == 8'h02);
                            in_total <= send_len;
                        end else if (set_adr) begin
                            stage        <= C_STATUS_IN;
                            pending_addr <= s2[6:0];
                            set_addr     <= 1'b1;
                        end else if (set_cfg) begin
                            stage          <= C_STATUS_IN;
                            pending_config <= s2[0];
                            set_config     <= 1'b1;
                        end else if (clr_halt) begin
                            stage       <= C_STATUS_IN;
                            ep_clear    <= 1'b1;
                            ep_clear_ep <= s4;
                        end else begin
                            stage <= C_STALL;
                        end
                    end else if (dat_ok && expect == X_OUT) begin
                        pending   <= 1'b1;
                        turn      <= 7'd0;
                        pend_data <= 1'b0;
                        if (stage == C_DATA_IN && dat_len == 4'd0) begin
                            // The status stage of a read.
                            pend_pid <= PID_ACK;
                            stage    <= C_IDLE;
                        end else begin
                            pend_pid <= PID_STALL;
                        end
                    end
                end else if (pkt_pid == PID_ACK && await_ack) begin
                    await_ack <= 1'b0;
                    if (stage == C_DATA_IN) begin
                        in_offset <= in_offset + {3'b000, in_len};
                        toggle    <= ~toggle;
                    end else if (stage == C_STATUS_IN) begin
                        stage <= C_IDLE;
                        if (set_addr)   addr     <= pending_addr;
                        if (set_config) begin
                            config_q <= pending_config;
                            ep_reset <= 1'b1;
                        end
                        set_addr   <= 1'b0;
                        set_config <= 1'b0;
                    end
                end else begin
                    await_ack <= 1'b0;
                end
            end

            // The answer, once the host's EOP is over and the bus has
            // been J for the turnaround. `sel` gates it because the
            // transmitter is shared: a token for another endpoint is
            // that endpoint's to answer, and this one keeps its state
            // until the host comes back to it.
            if (pending && !tx_busy && sel) begin
                if (!line_idle) begin
                    turn <= 7'd0;
                end else if (turn == TURNAROUND) begin
                    pending      <= 1'b0;
                    tx_start     <= 1'b1;
                    tx_pid       <= pend_pid;
                    tx_with_data <= pend_data;
                    tx_len       <= pend_len;
                end else begin
                    turn <= turn + 7'd1;
                end
            end

            if (bus_reset) begin
                addr       <= 7'd0;
                config_q   <= 1'b0;
                stage      <= C_IDLE;
                expect     <= X_NONE;
                await_ack  <= 1'b0;
                pending    <= 1'b0;
                set_addr   <= 1'b0;
                set_config <= 1'b0;
            end
        end
    end
endmodule

// usb_pkt_rx — a USB packet, decoded once for every endpoint above it.
//
// What it does
//   Turns the byte stream a link layer delivers into one pulse per whole
//   packet, with everything an endpoint needs to decide what to do about
//   it: the PID and its check nibble, a token's address and endpoint
//   number with its CRC5 checked, a data packet's payload with its CRC16
//   checked, and how long that payload was.
//
//   This is the part of a USB device that is **the same for every
//   endpoint**, and that is why it is a module of its own. It used to be
//   the first third of `usb_ctrl_ep`, which was the whole device when
//   endpoint 0 was the only endpoint there was. A second endpoint beside
//   it would have needed its own copy of a CRC16 generator, a byte
//   counter and a PID check — three chances for two statements of one
//   thing to drift apart, and about a hundred LUTs of duplication. So the
//   decoder came out and both endpoints read the same pulse.
//
//   `pkt` is high for one cycle when a packet has ended and its PID check
//   nibble was right. Everything else is valid in that cycle:
//
//     pkt_is_token   the PID is OUT, IN or SETUP
//     tok_ok         it was three bytes and its CRC5 checks
//     tok_addr       the device address it names
//     tok_endp       the endpoint number it names
//     pkt_is_data    the PID is DATA0 or DATA1
//     dat_ok         its CRC16 checks and it was not too long
//     dat_len        the payload's length in bytes, 0 to 8
//     dat            the payload, byte 0 in the low eight bits
//
//   A handshake is neither, so an endpoint reads `pkt` and `pkt_pid` for
//   those. `pkt` fires for a packet with a bad CRC too — `tok_ok` and
//   `dat_ok` are what say the packet is usable — because a packet that
//   arrived at all, right or wrong, is a reason to stop waiting for a
//   handshake, and an endpoint needs to know that.
//
// What it does not do
//   Eight bytes of payload at most, which is what a maximum packet size
//   of eight needs, and `too_long` withdraws `dat_ok` from anything
//   longer rather than truncating it silently. Nothing here knows about
//   addresses, endpoints, toggles or requests: the decoder does not care
//   who a packet is for.
//
//   Nothing here knows about NRZI, bit stuffing, SYNC, EOP or line
//   states either. A packet whose PID check fails, whose bit stuffing is
//   broken or which ends off a byte boundary never reaches `rx_eop` at
//   all, so the link layer below has already dropped it.
module usb_pkt_rx (
    input  wire       clk,
    input  wire       rst_n,

    // The bytes of a received packet, from the link layer.
    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    input  wire       rx_eop,
    input  wire       rx_active,

    // One whole packet, for one cycle.
    output wire        pkt,
    output wire [3:0]  pkt_pid,
    output wire        pkt_is_token,
    output wire        pkt_is_data,
    output wire        tok_ok,
    output wire [6:0]  tok_addr,
    output wire [3:0]  tok_endp,
    output wire        dat_ok,
    output wire [3:0]  dat_len,
    output wire [63:0] dat
);
    // PIDs, the low nibble as it appears on the wire.
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_SETUP = 4'b1101;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;

    // The residues a correct CRC leaves in these reflected registers.
    localparam [4:0]  CRC5_RESIDUE  = 5'h06;
    localparam [15:0] CRC16_RESIDUE = 16'hB001;

    // -----------------------------------------------------------------
    // CRCs, a byte at a time, reflected, as the bytes arrive.
    // -----------------------------------------------------------------
    function [4:0] crc5_byte;
        input [4:0] c;
        input [7:0] d;
        integer     i;
        reg   [4:0] r;
        begin
            r = c;
            for (i = 0; i < 8; i = i + 1)
                r = (r[0] ^ d[i]) ? ((r >> 1) ^ 5'h14) : (r >> 1);
            crc5_byte = r;
        end
    endfunction

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

    // -----------------------------------------------------------------
    // Receiving a packet.
    // -----------------------------------------------------------------
    reg [3:0]  n;          // bytes received, PID included
    reg [7:0]  pid_byte;
    reg [7:0]  tok0, tok1;
    reg [7:0]  d0, d1, d2, d3, d4, d5, d6, d7;
    reg [4:0]  crc5;
    reg [15:0] crc16;
    reg        too_long;

    wire [3:0] pid    = pid_byte[3:0];
    wire       pid_ok = (pid_byte[7:4] == ~pid_byte[3:0]);

    assign pkt          = rx_eop & pid_ok;
    assign pkt_pid      = pid;
    assign pkt_is_token = (pid == PID_OUT) | (pid == PID_IN) | (pid == PID_SETUP);
    assign pkt_is_data  = (pid == PID_DATA0) | (pid == PID_DATA1);
    assign tok_addr     = tok0[6:0];
    assign tok_endp     = {tok1[2:0], tok0[7]};
    assign tok_ok       = (n == 4'd3) & (crc5 == CRC5_RESIDUE);
    assign dat_ok       = (n >= 4'd3) & ~too_long & (crc16 == CRC16_RESIDUE);
    assign dat_len      = n - 4'd3;
    assign dat          = {d7, d6, d5, d4, d3, d2, d1, d0};

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            n        <= 4'd0;
            pid_byte <= 8'd0;
            tok0     <= 8'd0;
            tok1     <= 8'd0;
            d0 <= 8'd0; d1 <= 8'd0; d2 <= 8'd0; d3 <= 8'd0;
            d4 <= 8'd0; d5 <= 8'd0; d6 <= 8'd0; d7 <= 8'd0;
            crc5     <= 5'h1F;
            crc16    <= 16'hFFFF;
            too_long <= 1'b0;
        end else begin
            // Bytes as they arrive.
            if (!rx_active) begin
                n        <= 4'd0;
                crc5     <= 5'h1F;
                crc16    <= 16'hFFFF;
                too_long <= 1'b0;
            end
            if (rx_valid) begin
                if (n != 4'd15) n <= n + 4'd1;
                if (n == 4'd0) begin
                    pid_byte <= rx_data;
                end else begin
                    crc5  <= crc5_byte(crc5, rx_data);
                    crc16 <= crc16_byte(crc16, rx_data);
                    case (n)
                        4'd1:  begin tok0 <= rx_data; d0 <= rx_data; end
                        4'd2:  begin tok1 <= rx_data; d1 <= rx_data; end
                        4'd3:  d2 <= rx_data;
                        4'd4:  d3 <= rx_data;
                        4'd5:  d4 <= rx_data;
                        4'd6:  d5 <= rx_data;
                        4'd7:  d6 <= rx_data;
                        4'd8:  d7 <= rx_data;
                        4'd9:  begin end
                        4'd10: begin end
                        default: too_long <= 1'b1;
                    endcase
                end
            end
        end
    end
endmodule

// usb_bulk_ep — the endpoint that moves bytes: one bulk OUT and one bulk IN
// on the same endpoint number, with a byte interface for whatever is above
// them.
//
// What it does
//   Once a host has enumerated a device it stops asking questions and
//   starts moving data, and this is the part that answers that. It reads
//   the same `usb_pkt_rx` pulse `usb_ctrl_ep` reads, answers tokens for
//   endpoint `ENDP` only, and presents each direction as a byte stream:
//
//     OUT, host to device        `out_data` with `out_valid`, taken when
//                                `out_ready`, and `out_last` on the last
//                                byte of the packet the host sent
//     IN, device to host         `in_data` with `in_valid`, taken when
//                                `in_ready`; `in_commit` sends what has
//                                been given, however short
//
//   One packet of each direction is in flight at a time, which is what
//   makes the endpoint small: a buffer of MAXPKT bytes each way and no
//   FIFO. `fifo_sync` is a block in this library already, so a design that
//   wants depth puts one on either side rather than paying for it here.
//
//   **Flow control is the host's problem, which is what bulk means.** An
//   OUT packet that arrives while the last one has not been drained is
//   answered with NAK and the host sends it again; an IN token that
//   arrives with nothing ready is answered with NAK and the host asks
//   again. Neither loses a byte and neither needs the logic above to be
//   fast.
//
//   The data toggle is kept per direction, as USB 2.0 §8.6 asks. An OUT
//   packet whose PID is not the toggle expected is the host not having
//   heard the last ACK, so it is acknowledged again and **discarded**
//   rather than delivered twice. An IN packet the host does not
//   acknowledge is sent again with the same toggle, because nothing is
//   released until the ACK arrives. `ep_reset` puts both toggles back to
//   DATA0, which is what SET_CONFIGURATION means, and `ep_clear` does it
//   for one direction, which is what CLEAR_FEATURE(ENDPOINT_HALT) means.
//
//   The turnaround — `TURNAROUND` cycles of `line_idle` after the host's
//   packet before the answer goes out — is counted here and also in
//   `usb_ctrl_ep`. That is two statements of one rule, and deliberately:
//   the alternative is a shared counter driven by whichever endpoint is
//   answering, and then an endpoint has to be told that its packet went
//   out a cycle after it did. Ten lines of counter in each endpoint is
//   cheaper to read and to be sure of than a handshake between three
//   modules.
//
// What it does not do
//   One endpoint number, one packet deep, `MAXPKT` of at most 8 bytes —
//   which is what the four-bit length the transmitters take allows, and
//   which USB 2.0 §5.8.3 lists as a legal full-speed bulk size beside 16,
//   32 and 64. No isochronous and no interrupt endpoint, though an
//   interrupt endpoint is this module with a different bmAttributes in the
//   descriptor and nothing else: the packets are identical and only the
//   host's scheduling differs.
//
//   No STALL of its own: nothing here halts, so there is nothing to clear
//   except the toggle. A SETUP addressed to a bulk endpoint is ignored,
//   since a bulk endpoint has no control pipe and a host that sends one
//   has made a mistake no answer would tell it about.
//
//   Nothing here decodes a packet or checks a CRC — `usb_pkt_rx` does —
//   and nothing here knows the device's address: `address` comes from
//   `usb_ctrl_ep`, which is what SET_ADDRESS moved.
module usb_bulk_ep #(
    // The endpoint number both directions use. Endpoint 0 is the control
    // endpoint's and must not be given here.
    parameter [3:0]  ENDP       = 4'd1,
    // Bytes in a packet, 1 to 8.
    parameter [3:0]  MAXPKT     = 4'd8,
    // Cycles of `line_idle` before an answer starts; `usb_ctrl_ep`'s
    // parameter of the same name says what it has to be and why.
    parameter [6:0]  TURNAROUND = 7'd8
) (
    input  wire       clk,
    input  wire       rst_n,

    // One whole packet, from `usb_pkt_rx`.
    input  wire        pkt,
    input  wire [3:0]  pkt_pid,
    input  wire        pkt_is_token,
    input  wire        pkt_is_data,
    input  wire        tok_ok,
    input  wire [6:0]  tok_addr,
    input  wire [3:0]  tok_endp,
    input  wire        dat_ok,
    input  wire [3:0]  dat_len,
    input  wire [63:0] dat,

    // The device's address, from the control endpoint.
    input  wire [6:0] address,
    // The bus is idle, so an answer may be timed from now.
    input  wire       line_idle,
    // The host has reset the bus.
    input  wire       bus_reset,
    // Both toggles back to DATA0, and one direction's.
    input  wire       ep_reset,
    input  wire       ep_clear,
    input  wire [7:0] ep_clear_ep,
    // This endpoint owns the transmitter.
    input  wire       sel,

    // One packet out.
    output reg        tx_start,
    output reg  [3:0] tx_pid,
    output reg        tx_with_data,
    output reg  [3:0] tx_len,
    input  wire [3:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    // The bytes the host sent.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,

    // The bytes to send it.
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit
);
    // PIDs, the low nibble as it appears on the wire.
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;
    localparam [3:0] PID_ACK   = 4'b0010;
    localparam [3:0] PID_NAK   = 4'b1010;

    // The endpoint addresses this endpoint answers CLEAR_FEATURE for: the
    // direction bit and the number.
    localparam [7:0] ADDR_OUT = {4'h0, ENDP};
    localparam [7:0] ADDR_IN  = {4'h8, ENDP};

    // -----------------------------------------------------------------
    // Host to device.
    // -----------------------------------------------------------------
    // The byte index is scaled to a bit index by concatenation and not by
    // `* 8`. A multiply by a power of two is a shift, but this compiler's
    // synthesis does not strength-reduce one: `ordx * 8` became a `mul`
    // cell, and on the ECP5 a `MULT18X18D`. Three of them, across the two
    // buffers and the descriptor, cost 640 LUT4 and two hard multipliers.
    reg [63:0] obuf;        // the packet, byte 0 in the low eight bits
    reg [3:0]  olen;        // bytes in it, 0 when it has been drained
    reg [2:0]  ordx;        // the byte being handed over
    reg        out_toggle;  // the PID the next packet should carry
    reg        expect_out;  // an OUT token has been seen and its data is next

    assign out_valid = (olen != 4'd0);
    assign out_data  = obuf[{ordx, 3'b000} +: 8];
    assign out_last  = (({1'b0, ordx} + 4'd1) == olen);

    // -----------------------------------------------------------------
    // Device to host.
    // -----------------------------------------------------------------
    // Eight byte registers and a `case`, rather than one wide register and
    // a part-select: an assignment to a part-select whose bound is not
    // constant is not something every tool takes, and a decoder is what
    // this becomes either way.
    reg [7:0] i0, i1, i2, i3, i4, i5, i6, i7;
    reg [3:0] ilen;         // bytes given so far
    reg       armed;        // the packet is ready to go out
    reg       in_toggle;    // the PID it will carry
    reg       in_await;     // it has gone out and its ACK has not come back

    wire [63:0] ibuf = {i7, i6, i5, i4, i3, i2, i1, i0};

    // Room while no packet is waiting to be sent. `armed` covers the full
    // buffer too, since the eighth byte arms it.
    assign in_ready = ~armed;
    assign tx_byte  = ibuf[{tx_index[2:0], 3'b000} +: 8];

    // -----------------------------------------------------------------
    // The answer, and when it may go out.
    // -----------------------------------------------------------------
    reg       pending;
    reg [3:0] pend_pid;
    reg       pend_data;
    reg [3:0] pend_len;
    reg [6:0] turn;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            obuf       <= 64'd0;
            olen       <= 4'd0;
            ordx       <= 3'd0;
            out_toggle <= 1'b0;
            expect_out <= 1'b0;
            i0 <= 8'd0; i1 <= 8'd0; i2 <= 8'd0; i3 <= 8'd0;
            i4 <= 8'd0; i5 <= 8'd0; i6 <= 8'd0; i7 <= 8'd0;
            ilen       <= 4'd0;
            armed      <= 1'b0;
            in_toggle  <= 1'b0;
            in_await   <= 1'b0;
            pending    <= 1'b0;
            pend_pid   <= 4'd0;
            pend_data  <= 1'b0;
            pend_len   <= 4'd0;
            turn       <= 7'd0;
            tx_start   <= 1'b0;
            tx_pid     <= 4'd0;
            tx_with_data <= 1'b0;
            tx_len     <= 4'd0;
        end else begin
            tx_start <= 1'b0;

            // ---------------------------------------------------------
            // A whole packet from the host.
            // ---------------------------------------------------------
            if (pkt) begin
                if (pkt_is_token) begin
                    // A token ends any wait for a handshake, whoever it
                    // was for.
                    in_await   <= 1'b0;
                    expect_out <= 1'b0;
                    if (tok_ok && tok_addr == address && tok_endp == ENDP) begin
                        if (pkt_pid == PID_OUT) begin
                            expect_out <= 1'b1;
                        end else if (pkt_pid == PID_IN) begin
                            pending <= 1'b1;
                            turn    <= 7'd0;
                            if (armed) begin
                                pend_pid  <= in_toggle ? PID_DATA1 : PID_DATA0;
                                pend_data <= 1'b1;
                                pend_len  <= ilen;
                                in_await  <= 1'b1;
                            end else begin
                                pend_pid  <= PID_NAK;
                                pend_data <= 1'b0;
                            end
                        end
                        // A SETUP is not answered at all. A bulk endpoint
                        // has no control pipe, and a token is not a thing
                        // a device answers: what it would answer is the
                        // data packet behind the token, and a STALL sent
                        // in the token's turnaround would land on top of
                        // that packet — which is how this was wrong once,
                        // and what `the host saw the protocol broken: the
                        // device drives the pair while the host does`
                        // said about it.
                    end
                end else if (pkt_is_data) begin
                    expect_out <= 1'b0;
                    if (dat_ok && expect_out) begin
                        pending   <= 1'b1;
                        turn      <= 7'd0;
                        pend_data <= 1'b0;
                        if (pkt_pid != (out_toggle ? PID_DATA1 : PID_DATA0)) begin
                            // The host did not hear the last ACK. Say it
                            // again and drop the copy.
                            pend_pid <= PID_ACK;
                        end else if (olen == 4'd0) begin
                            obuf       <= dat;
                            olen       <= dat_len;
                            ordx       <= 3'd0;
                            out_toggle <= ~out_toggle;
                            pend_pid   <= PID_ACK;
                        end else begin
                            // Nothing has taken the last packet yet.
                            pend_pid <= PID_NAK;
                        end
                    end
                end else if (pkt_pid == PID_ACK && in_await) begin
                    // The packet arrived. The buffer is free and the
                    // toggle moves on.
                    in_await  <= 1'b0;
                    ilen      <= 4'd0;
                    armed     <= 1'b0;
                    in_toggle <= ~in_toggle;
                end else begin
                    in_await <= 1'b0;
                end
            end

            // ---------------------------------------------------------
            // The bytes handed over, and the bytes given.
            // ---------------------------------------------------------
            if (out_valid && out_ready) begin
                if (out_last) begin
                    olen <= 4'd0;
                    ordx <= 3'd0;
                end else begin
                    ordx <= ordx + 3'd1;
                end
            end

            if (in_valid && in_ready) begin
                case (ilen)
                    4'd0:    i0 <= in_data;
                    4'd1:    i1 <= in_data;
                    4'd2:    i2 <= in_data;
                    4'd3:    i3 <= in_data;
                    4'd4:    i4 <= in_data;
                    4'd5:    i5 <= in_data;
                    4'd6:    i6 <= in_data;
                    default: i7 <= in_data;
                endcase
                ilen <= ilen + 4'd1;
                if ((ilen + 4'd1) == MAXPKT || in_commit) armed <= 1'b1;
            end else if (in_commit && !armed) begin
                // A short packet, or a zero-length one, on request.
                armed <= 1'b1;
            end

            // ---------------------------------------------------------
            // The answer, once the bus has been idle for the turnaround.
            // ---------------------------------------------------------
            if (pending && !tx_busy && sel) begin
                if (!line_idle) begin
                    turn <= 7'd0;
                end else if (turn == TURNAROUND) begin
                    pending      <= 1'b0;
                    tx_start     <= 1'b1;
                    tx_pid       <= pend_pid;
                    tx_with_data <= pend_data;
                    tx_len       <= pend_len;
                end else begin
                    turn <= turn + 7'd1;
                end
            end

            // ---------------------------------------------------------
            // The toggles, and a bus reset.
            // ---------------------------------------------------------
            if (ep_reset) begin
                out_toggle <= 1'b0;
                in_toggle  <= 1'b0;
            end
            if (ep_clear && ep_clear_ep == ADDR_OUT) out_toggle <= 1'b0;
            if (ep_clear && ep_clear_ep == ADDR_IN)  in_toggle  <= 1'b0;

            if (bus_reset) begin
                olen       <= 4'd0;
                ordx       <= 3'd0;
                out_toggle <= 1'b0;
                expect_out <= 1'b0;
                ilen       <= 4'd0;
                armed      <= 1'b0;
                in_toggle  <= 1'b0;
                in_await   <= 1'b0;
                pending    <= 1'b0;
            end
        end
    end
endmodule

// usb_dev_core — a USB device above the line: the packet decoder, the
// control endpoint, a bulk endpoint pair, and the one transmitter they
// share.
//
// What it does
//   This is **everything both cores of this library have in common**, and
//   it is a module for that reason alone. `usb_device_fs` puts it behind
//   its own full-speed encoder and serialiser; `usb_device_ulpi` puts it
//   behind a ULPI transceiver, which does that work in silicon. There is
//   one statement of the device for both, the way `eth_mac_tx` and
//   `eth_mac_rx` are one statement of Ethernet framing for RMII and
//   RGMII, and it is the same argument: a control endpoint is the hardest
//   part of a USB device to get right, and two copies of one drifting
//   apart is a cost that arrives later and is paid by whoever is unlucky.
//
//   It used to be `usb_ctrl_ep` alone, instantiated twice. It is a module
//   of its own now because there are three things to share and not one:
//
//     usb_pkt_rx     the PID check nibble, the CRC5 of tokens, the CRC16
//                    of data packets, and the payload — decoded once for
//                    every endpoint rather than once per endpoint
//     usb_ctrl_ep    endpoint 0: the standard requests, the descriptors
//     usb_bulk_ep    endpoint `DATA_ENDP`, IN and OUT, with a byte
//                    interface for whatever is above it
//
//   and a transmitter that only one of them may have at a time.
//
// THE TRANSMITTER, AND WHO OWNS IT
//   A USB device only ever speaks when it has been asked to, and the
//   asking is a token. So the endpoint that may answer is the endpoint the
//   **last token named**, which is one register:
//
//     owner <= (tok_endp != 0)   on any token addressed to this device
//
//   Everything after that token belongs to the same endpoint — the data
//   packet of an OUT, the handshake of an IN — because a host does not
//   interleave transactions on one device. `sel` into each endpoint is
//   that register, it gates the endpoint's turnaround counter, and the
//   whole arbitration is a multiplexer. There is no request-and-grant and
//   no round robin, because there is never a second answer waiting: the
//   endpoint that has not been asked has nothing to say.
//
//   A token for an endpoint number neither of them has hands ownership to
//   the data endpoint, which then ignores it because the number is not
//   its own. Nothing answers, and the host retries and gives up, which is
//   what a device with no such endpoint is supposed to do.
//
// What it does not do
//   One bulk endpoint pair. A second pair is a second `usb_bulk_ep` with
//   another `ENDP`, another pair of byte interfaces on this module's port
//   list, and `owner` widened from a bit to a number — at which point the
//   width of that register is the number of endpoints and not one more,
//   for the reason `usb_ctrl_ep`'s `stage` gives at length.
//
//   Nothing here knows about NRZI, bit stuffing, SYNC, EOP, line states
//   or ULPI. The link layer below delivers bytes and takes packets.
module usb_dev_core #(
    parameter [15:0]  VID          = 16'h1209,
    parameter [15:0]  PID          = 16'h0001,
    parameter [7:0]   DEV_CLASS    = 8'hFF,
    parameter [7:0]   DEV_SUBCLASS = 8'h00,
    parameter [7:0]   DEV_PROTOCOL = 8'h00,
    parameter [7:0]   CFG_ATTR     = 8'h80,
    parameter [7:0]   CFG_POWER    = 8'd50,
    // The class's interface and endpoint descriptors; `usb_ctrl_ep` says
    // how they are written and what is derived from them.
    parameter integer IFACE_BYTES  = 23,
    parameter [IFACE_BYTES*8-1:0] IFACE_DESC = {
        8'd9, 8'd4, 8'd0, 8'd0, 8'd2, 8'hFF, 8'h00, 8'h00, 8'd0,
        8'd7, 8'd5, 8'h01, 8'd2, 8'd8, 8'd0, 8'd0,
        8'd7, 8'd5, 8'h81, 8'd2, 8'd8, 8'd0, 8'd0
    },
    // The endpoint number the byte interfaces belong to, and the bytes in
    // one of its packets. They have to agree with the endpoint descriptors
    // above, which no arithmetic can check: a descriptor says what the
    // host will do and these say what the device will do.
    parameter [3:0]   DATA_ENDP    = 4'd1,
    parameter [3:0]   MAXPKT       = 4'd8,
    parameter [6:0]   TURNAROUND   = 7'd8
) (
    input  wire       clk,
    input  wire       rst_n,

    // The bytes of a received packet, from the link layer.
    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    input  wire       rx_eop,
    input  wire       rx_active,
    // The bus is idle, so an answer may be timed from now.
    input  wire       line_idle,
    // The host has reset the bus.
    input  wire       bus_reset,

    // One packet out.
    output wire       tx_start,
    output wire [3:0] tx_pid,
    output wire       tx_with_data,
    output wire [3:0] tx_len,
    input  wire [3:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured,

    // The data endpoint's bytes.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit
);
    // -----------------------------------------------------------------
    // One packet, decoded once.
    // -----------------------------------------------------------------
    wire        pkt;
    wire [3:0]  pkt_pid;
    wire        pkt_is_token, pkt_is_data;
    wire        tok_ok;
    wire [6:0]  tok_addr;
    wire [3:0]  tok_endp;
    wire        dat_ok;
    wire [3:0]  dat_len;
    wire [63:0] dat;

    usb_pkt_rx u_pkt (
        .clk          (clk),
        .rst_n        (rst_n),
        .rx_data      (rx_data),
        .rx_valid     (rx_valid),
        .rx_eop       (rx_eop),
        .rx_active    (rx_active),
        .pkt          (pkt),
        .pkt_pid      (pkt_pid),
        .pkt_is_token (pkt_is_token),
        .pkt_is_data  (pkt_is_data),
        .tok_ok       (tok_ok),
        .tok_addr     (tok_addr),
        .tok_endp     (tok_endp),
        .dat_ok       (dat_ok),
        .dat_len      (dat_len),
        .dat          (dat)
    );

    // -----------------------------------------------------------------
    // Who the last token asked.
    // -----------------------------------------------------------------
    reg owner;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) owner <= 1'b0;
        else if (pkt && pkt_is_token && tok_ok && tok_addr == address)
            owner <= (tok_endp != 4'd0);
    end

    // -----------------------------------------------------------------
    // Endpoint 0.
    // -----------------------------------------------------------------
    wire       c_tx_start, c_tx_with_data;
    wire [3:0] c_tx_pid, c_tx_len;
    wire [7:0] c_tx_byte;
    wire       ep_reset, ep_clear;
    wire [7:0] ep_clear_ep;

    usb_ctrl_ep #(
        .VID          (VID),
        .PID          (PID),
        .DEV_CLASS    (DEV_CLASS),
        .DEV_SUBCLASS (DEV_SUBCLASS),
        .DEV_PROTOCOL (DEV_PROTOCOL),
        .CFG_ATTR     (CFG_ATTR),
        .CFG_POWER    (CFG_POWER),
        .IFACE_BYTES  (IFACE_BYTES),
        .IFACE_DESC   (IFACE_DESC),
        .TURNAROUND   (TURNAROUND)
    ) u_ep0 (
        .clk          (clk),
        .rst_n        (rst_n),
        .pkt          (pkt),
        .pkt_pid      (pkt_pid),
        .pkt_is_token (pkt_is_token),
        .pkt_is_data  (pkt_is_data),
        .tok_ok       (tok_ok),
        .tok_addr     (tok_addr),
        .tok_endp     (tok_endp),
        .dat_ok       (dat_ok),
        .dat_len      (dat_len),
        .dat          (dat),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .sel          (~owner),
        .tx_start     (c_tx_start),
        .tx_pid       (c_tx_pid),
        .tx_with_data (c_tx_with_data),
        .tx_len       (c_tx_len),
        .tx_index     (tx_index),
        .tx_byte      (c_tx_byte),
        .tx_busy      (tx_busy),
        .address      (address),
        .configured   (configured),
        .ep_reset     (ep_reset),
        .ep_clear     (ep_clear),
        .ep_clear_ep  (ep_clear_ep)
    );

    // -----------------------------------------------------------------
    // The data endpoint.
    // -----------------------------------------------------------------
    wire       b_tx_start, b_tx_with_data;
    wire [3:0] b_tx_pid, b_tx_len;
    wire [7:0] b_tx_byte;

    usb_bulk_ep #(
        .ENDP       (DATA_ENDP),
        .MAXPKT     (MAXPKT),
        .TURNAROUND (TURNAROUND)
    ) u_ep1 (
        .clk          (clk),
        .rst_n        (rst_n),
        .pkt          (pkt),
        .pkt_pid      (pkt_pid),
        .pkt_is_token (pkt_is_token),
        .pkt_is_data  (pkt_is_data),
        .tok_ok       (tok_ok),
        .tok_addr     (tok_addr),
        .tok_endp     (tok_endp),
        .dat_ok       (dat_ok),
        .dat_len      (dat_len),
        .dat          (dat),
        .address      (address),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .ep_reset     (ep_reset),
        .ep_clear     (ep_clear),
        .ep_clear_ep  (ep_clear_ep),
        .sel          (owner),
        .tx_start     (b_tx_start),
        .tx_pid       (b_tx_pid),
        .tx_with_data (b_tx_with_data),
        .tx_len       (b_tx_len),
        .tx_index     (tx_index),
        .tx_byte      (b_tx_byte),
        .tx_busy      (tx_busy),
        .out_data     (out_data),
        .out_valid    (out_valid),
        .out_last     (out_last),
        .out_ready    (out_ready),
        .in_data      (in_data),
        .in_valid     (in_valid),
        .in_ready     (in_ready),
        .in_commit    (in_commit)
    );

    // -----------------------------------------------------------------
    // The transmitter, to whichever endpoint the token named.
    // -----------------------------------------------------------------
    assign tx_start     = owner ? b_tx_start     : c_tx_start;
    assign tx_pid       = owner ? b_tx_pid       : c_tx_pid;
    assign tx_with_data = owner ? b_tx_with_data : c_tx_with_data;
    assign tx_len       = owner ? b_tx_len       : c_tx_len;
    assign tx_byte      = owner ? b_tx_byte      : c_tx_byte;
endmodule
