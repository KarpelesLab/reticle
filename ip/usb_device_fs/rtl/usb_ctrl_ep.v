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
