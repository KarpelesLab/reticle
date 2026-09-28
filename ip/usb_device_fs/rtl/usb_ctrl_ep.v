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
//   Endpoint 0, maximum packet size `MAXPKT0` — 64 by default, and that
//   parameter's own comment says what the specification allows and what
//   raising it buys:
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
//   requests to an interface or an endpoint — is offered to the class
//   hook below, and answered with STALL until the next SETUP if the class
//   does not claim it.
//
//   A token addressed elsewhere, or to another endpoint, is ignored.
//   `bus_reset` sets the address back to 0 and the configuration to none.
//
// THE CLASS HOOK, AND WHY IT IS COMBINATIONAL
//   A control endpoint that stalls everything it does not itself
//   understand is a control endpoint no class can be built on. CDC ACM
//   needs SET_LINE_CODING, GET_LINE_CODING and SET_CONTROL_LINE_STATE; a
//   human interface device needs GET_REPORT and SET_IDLE; the next class
//   needs something else again. What they share is the *shape* of the
//   thing, and that shape is what is on these ports:
//
//     class_setup      the eight bytes of a SETUP, byte 0 in the low bits
//     class_req        one cycle: that SETUP is well formed and **no
//                      standard request this module implements matched
//                      it**
//     class_claim      the class's answer, read in that same cycle
//     class_len        bytes it will send, if the request reads
//     class_index      which of those bytes is wanted
//     class_byte       that byte
//     class_out        a host-to-device data stage's payload, up to 8
//     class_out_len    its length
//     class_out_valid  one cycle: it arrived, its CRC checked, and it is
//                      about to be acknowledged
//
//   **`class_claim`, `class_len` and `class_byte` have to be
//   combinational**, and that is a decision rather than an oversight.
//   This module chooses the transfer's stage in the very cycle the SETUP's
//   data packet ends — `stage <= C_DATA_IN` or `C_STATUS_IN` or `C_STALL`
//   is one arm of one `if`, and the ACK it schedules goes out
//   `TURNAROUND` cycles later whatever it decided. A class that answered a
//   cycle later would need a fifth stage here, a handshake back, and a
//   rule about what happens if the host's next token arrives first. A
//   request decoder is pure combinational logic over eight bytes —
//   `bmRequestType`, `bRequest`, `wIndex` compared against constants — so
//   asking for it combinationally asks for nothing a class cannot give.
//
//   `class_req` is deliberately **not** raised for the requests this
//   module implements, so a class cannot shadow SET_ADDRESS or
//   GET_DESCRIPTOR by claiming them. Everything else is offered,
//   string descriptors and GET_STATUS included, so a class that wants
//   those can have them without this file changing again.
//
//   The direction is `bmRequestType` bit 7 and is read here rather than
//   asked for: a read becomes a data stage of `min(wLength, class_len)`
//   bytes fetched through `class_index`, and a write becomes the status
//   stage with one data packet expected first when `wLength` is not zero.
//   That is **one** OUT packet, so a host-to-device class data stage is at
//   most 8 bytes — which is every CDC ACM request and is stated in "What
//   it does not do" below.
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
//   No strings of its own, no remote wake-up, no suspend, no SOF tracking
//   and no low speed. `MAXPKT0` bytes of payload **out** and eight bytes
//   **in**, which is the one asymmetry of this module and is what
//   `MAXPKT0`'s comment is about: the data stage this endpoint sends is cut
//   into packets of whatever the parameter says, and the one it receives is
//   `usb_pkt_rx`'s eight-byte word. A configuration descriptor of at most
//   9 + 64 bytes, which is `DESC_MAX` below and is room for a HID interface
//   or a CDC ACM pair.
//
//   A class request that **writes** carries at most one data packet, so
//   eight bytes: `class_out` is `usb_pkt_rx`'s word and there is no
//   accumulator behind it. Every CDC ACM request fits — SET_LINE_CODING
//   is seven bytes — and a class needing more would need a counter here
//   and the byte stream `usb_pkt_rx` brings out for the bulk endpoints. A
//   longer one is **stalled** rather than half-taken. A class request that
//   **reads** may be as long as `CLASS_MAX`, since those bytes are fetched
//   one at a time and the packets are this module's to cut.
//
//   Nothing here gives a class a say in the standard requests, and
//   nothing here lets a class stall one it has claimed: `class_claim`
//   promises an answer. A class that must refuse a request it recognises
//   leaves `class_claim` low and takes the STALL this module was going to
//   send anyway.
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
    // The longest device-to-host data stage a class request may ask for,
    // in bytes. Zero is a device with no class layer, and then the whole
    // hook below is constant and synthesis removes it. Seven bytes is what
    // CDC ACM's GET_LINE_CODING needs. It is a parameter and not a
    // constant because it sets the width of `in_total` and `in_offset`,
    // and a register is as wide as the values it holds.
    parameter integer CLASS_MAX = 0,
    // ENDPOINT 0'S MAXIMUM PACKET SIZE, WHICH IS ALSO `bMaxPacketSize0`
    //
    // USB 2.0 §5.5.3 allows a full-speed control endpoint 8, 16, 32 or 64
    // bytes and nothing else, and byte 7 of the device descriptor has to
    // say which — so this parameter is written into that byte below rather
    // than typed there, because two statements of one number is how they
    // come to disagree.
    //
    // **64, and the subtlety is on the receive side.** What this buys is
    // transactions: a CDC ACM configuration descriptor is 67 bytes, which
    // is nine IN transactions at eight bytes a packet and **two** at 64.
    // What a host does first is read the device descriptor before it knows
    // this field — Linux asks for 64 bytes of it, which is what
    // `device descriptor read/64` in a kernel log is — and that is safe
    // whichever size this is: the data stage is capped at the host's own
    // `wLength`, so the first packet is `min(wLength, 18)` bytes, and a
    // packet shorter than the maximum ends the transfer whatever the
    // maximum was. A host that asks for only the first eight bytes gets a
    // short packet too. Neither case needs the host to know this number
    // first, which is the thing that would otherwise make raising it
    // dangerous.
    //
    // **What this endpoint will not receive is 64 bytes.** A control OUT
    // data stage arrives as `dat`, which is eight bytes, so a host-to-device
    // data stage longer than one eight-byte packet is **stalled** below
    // rather than half-taken. Nothing this module or any class on it
    // implements has one — SET_ADDRESS, SET_CONFIGURATION,
    // CLEAR_FEATURE and GET_DESCRIPTOR have no OUT data stage at all, and
    // CDC ACM's longest is SET_LINE_CODING's seven bytes — so the limit is
    // on requests that do not exist. It is enforced instead of assumed
    // because a field in a descriptor should not be the only thing standing
    // between a device and a packet it cannot hold.
    parameter [6:0]   MAXPKT0   = 7'd64,
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
    input  wire [6:0]  dat_len,
    input  wire [63:0] dat,

    // The bus is idle, so an answer may be timed from now.
    input  wire       line_idle,
    // The host has reset the bus.
    input  wire       bus_reset,
    // This endpoint owns the transmitter.
    input  wire       sel,

    // One packet out. `tx_len` and `tx_index` are seven bits because a
    // full-speed payload is at most 64 bytes; `usb_fs_tx`'s header says why
    // that is seven and not six.
    output reg        tx_start,
    output reg  [3:0] tx_pid,
    output reg        tx_with_data,
    output wire [6:0] tx_len,
    input  wire [6:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured,

    // The class hook. "THE CLASS HOOK, AND WHY IT IS COMBINATIONAL" above
    // states the contract; the three inputs are read in the same cycle
    // `class_req` is high and must be combinational in `class_setup`.
    output wire [63:0] class_setup,
    output wire        class_req,
    input  wire        class_claim,
    input  wire [6:0]  class_len,
    output wire [6:0]  class_index,
    input  wire [7:0]  class_byte,
    output wire [63:0] class_out,
    output wire [6:0]  class_out_len,
    output wire        class_out_valid,

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

    // Bits in a descriptor offset, **from the descriptors and not by hand**.
    //
    // The longest descriptor this endpoint sends is the configuration one, so
    // an offset runs from 0 to `CFG_TOTAL`, and `tx_byte` is fetched up to
    // seven bytes past the offset the host has acknowledged, so the widest
    // number here is `CFG_TOTAL + 7`.
    //
    // This was `[6:0]` for both, and seven bits it is not: with the default
    // descriptors `CFG_TOTAL` is 32, so bit 6 of `in_total` is a bit no
    // expression can set, and **the ECP5 backend refused the bitstream**:
    //
    //   flip-flop `u_dev.u_dev.u_ep0.in_total$ff$ff6` has nothing driving its
    //   data input and is not tied high, and an unrouted slice input on this
    //   family reads as a one
    //
    // which is the same fault as the three-bit `stage` for four states, found
    // this time by a tool instead of by a person with an oscilloscope. A
    // register is as wide as the values it holds, and when the values come
    // from a parameter so does the width.
    //
    // A class request's data stage is counted by the same two registers, so
    // the widest offset is over whichever of the two is longer.
    //
    // There are **two** widths and not one, and they are different numbers:
    //
    //   OFF_BITS   `in_total` and `in_offset`, which are bytes of the data
    //              stage and never exceed `LONGEST`
    //   IDX_BITS   `in_offset + tx_index`, the byte being fetched, which
    //              runs up to `MAXPKT0` past the offset the host
    //              acknowledged because the transmitter fetches the whole
    //              packet before the ACK for it arrives
    //
    // This used to be one width for both, `$clog2(LONGEST + 8)`, and with a
    // packet size of eight the two happened to coincide. At 64 they do not:
    // a 67-byte configuration descriptor needs seven bits of offset and
    // **eight** of fetch index, and the two registers are the ones that must
    // stay at seven — a register is as wide as the values it holds, and the
    // ECP5 backend refused a bitstream over exactly that once, which the
    // note above records.
    localparam integer LONGEST  = (CLASS_MAX > CFG_TOTAL) ? CLASS_MAX : CFG_TOTAL;
    localparam integer OFF_BITS = $clog2(LONGEST + 1);
    localparam integer IDX_BITS = $clog2(LONGEST + MAXPKT0 + 1);
    // Bits in a packet this endpoint sends. A packet is at most `MAXPKT0`
    // bytes and also at most the whole data stage, so it is the **smaller**
    // of the two that sets the width: a device whose longest descriptor is 32
    // bytes never sends a 64-byte packet however large `MAXPKT0` is, and a
    // register that could hold one would have a bit nothing sets.
    localparam integer PKT_BITS   = $clog2(MAXPKT0 + 1);
    localparam integer CHUNK_BITS = (OFF_BITS < PKT_BITS) ? OFF_BITS : PKT_BITS;

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
    reg [OFF_BITS-1:0] in_total;   // bytes the data stage sends
    reg [OFF_BITS-1:0] in_offset;  // bytes the host has acknowledged
    reg [CHUNK_BITS-1:0] in_len;   // bytes in the packet awaiting its ACK
    reg        await_ack;
    // The transfer in progress belongs to the class, so its data stage
    // comes from `class_byte` and not from `desc`.
    reg        class_active;
    // ... and, when it writes, one data packet is still expected.
    reg        class_out_wait;

    // A response waits for the turnaround after the host's EOP.
    reg        pending;
    reg [3:0]  pend_pid;
    reg        pend_data;
    reg [CHUNK_BITS-1:0] pend_len;
    reg [6:0]  turn;

    // The length the transmitter is given. `tx_len_q` is as wide as a packet
    // of this endpoint and the port is seven bits, which is the width the
    // transmitter takes for every endpoint; the continuous assignment is
    // what zero-extends it.
    reg [CHUNK_BITS-1:0] tx_len_q;

    assign tx_len     = tx_len_q;
    assign address    = addr;
    assign configured = config_q;

    // The descriptors, one byte at a time.
    function [7:0] desc;
        input                  sel_in;
        input [IDX_BITS-1:0]   i;
        // The offset past the nine bytes this module writes, and that offset
        // **narrowed to the blob's width**.
        //
        // `j` is six bits because `DESC_MAX` is 64 bytes: an offset into the
        // class's descriptors cannot be wider than that whatever `IDX_BITS`
        // is, and giving it exactly those bits is what keeps the part-select
        // below inside `IFACE` without a mask a width checker has to trust.
        //
        // The narrowing is written as a **part-select** and not as an
        // assignment that happens to truncate, because dropping the bits
        // above the blob is the intent: `i` can reach `CFG_TOTAL + MAXPKT0`,
        // since `tx_byte` is fetched for the whole packet before the ACK for
        // it arrives, and the bytes past `tx_len` are fetched and never sent.
        // The top of that range is not a byte anybody reads, and a
        // part-select says so where an assignment would only warn.
        reg   [IDX_BITS-1:0]   off;
        reg   [5:0]            j;
        begin
            if (!sel_in) begin
                case (i)
                    0:       desc = 8'd18;        // bLength
                    1:       desc = 8'd1;         // DEVICE
                    2:       desc = 8'h00;        // bcdUSB 2.00
                    3:       desc = 8'h02;
                    4:       desc = DEV_CLASS;
                    5:       desc = DEV_SUBCLASS;
                    6:       desc = DEV_PROTOCOL;
                    // bMaxPacketSize0, from the parameter that sets what this
                    // endpoint actually does rather than typed again here.
                    7:       desc = {1'b0, MAXPKT0};
                    8:       desc = VID[7:0];
                    9:       desc = VID[15:8];
                    10:      desc = PID[7:0];
                    11:      desc = PID[15:8];
                    12:      desc = 8'h00;        // bcdDevice 1.00
                    13:      desc = 8'h01;
                    14:      desc = 8'd0;         // no strings
                    15:      desc = 8'd0;
                    16:      desc = 8'd0;
                    17:      desc = 8'd1;         // one configuration
                    default: desc = 8'd0;
                endcase
            end else if (i < 9) begin
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
                off  = i - 9;
                j    = off[5:0];
                // One part-select of the whole blob, which is a 64-entry ROM:
                // eight independent six-input Boolean functions.
                //
                // THIS USED TO BE READ A PAGE AT A TIME, AND WHY IT IS NOT
                //
                // Mapped onto LUT4, three of the sixty-seven bytes of a CDC
                // ACM descriptor set came out wrong, and a host refused the
                // device over one of them:
                //
                //   config 1 has 1 interface, different from the
                //   descriptor's value: 2
                //
                // because `bInterfaceNumber` of the data interface read 0
                // where these sources say 1. In simulation the same design
                // was byte-perfect. The gap was in the technology mapper and
                // not here: a cut reduced to its support is no longer a cut,
                // and the function of a parent that merged one was computed
                // by simulating a cone over leaves that did not separate it,
                // reading the input they missed as constant zero.
                // `src/synth/techmap/cuts.rs` has it in full.
                //
                // Reading the blob as four 128-bit pages was the same
                // function with a different cover, and it mapped correctly.
                // It was a workaround and it is gone: the mapper composes a
                // merged cut's function from its fanin cuts' functions now,
                // `synth::techmap::verify` proves every mapping of every
                // block in the library equivalent to what it was mapped
                // from, and `tests/ip_library.rs`'s
                // `usb_descriptors_survive_lookup_table_mapping` still reads
                // these bytes off the mapped netlist at two widths. Reverting
                // the mapper fix fails all three.
                desc = IFACE[{j, 3'b000} +: 8];
            end
        end
    endfunction

    // The byte of the data stage the transmitter is asking for: the
    // class's when the class owns this transfer, a descriptor's otherwise.
    // `class_index` is the same offset, brought out so that the class
    // indexes its own bytes without restating the arithmetic.
    wire [IDX_BITS-1:0] fetch = in_offset + tx_index;
    assign class_index = fetch[6:0];
    assign tx_byte     = class_active ? class_byte : desc(desc_sel, fetch);

    // The next packet of the data stage: a whole one until the last, which
    // is whatever is left and is short — and a short packet is what ends a
    // control read, which is why nothing has to send a zero-length one
    // unless the data stage is an exact multiple of `MAXPKT0`. Then
    // `in_left` reaches zero with the host still asking, `in_chunk` is zero,
    // and the zero-length DATA packet that ends the transfer goes out of the
    // same arm as any other.
    //
    // The comparison is done at `IDX_BITS`, which is wider than either
    // operand, because the two are not the same width: a data stage may be
    // shorter than a packet — the plain device's configuration descriptor is
    // 32 bytes and a packet is 64 — and then `in_left` has fewer bits than
    // `MAXPKT0` does, and the other way round for a long descriptor set.
    wire [IDX_BITS-1:0]   in_left  = in_total - in_offset;
    wire [IDX_BITS-1:0]   in_max   = MAXPKT0;
    wire [CHUNK_BITS-1:0] in_chunk = (in_left > in_max) ? in_max[CHUNK_BITS-1:0]
                                                       : in_left[CHUNK_BITS-1:0];

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

    // A SETUP this module will act on at all: the right stage, eight bytes,
    // DATA0, and a CRC that checked. The arms of the `if` below repeat
    // these conditions; this wire is what the class hook is gated by, so
    // that a class is never offered a packet endpoint 0 is going to ignore.
    wire        setup_now = pkt & pkt_is_data & dat_ok & (expect == X_SETUP)
                          & (pkt_pid == PID_DATA0) & (dat_len == 7'd8);
    // A request this module implements itself. A class is offered
    // everything else and nothing of this.
    wire        std_req   = get_desc | set_adr | set_cfg | clr_halt;

    // A host-to-device data packet this endpoint can hold: `dat` is the
    // first eight payload bytes, so a longer one is refused rather than
    // taken in part. `MAXPKT0`'s own comment says why the field in the
    // descriptor is 64 and this is eight.
    wire        out_fits  = (dat_len <= 7'd8);

    assign class_setup     = dat;
    assign class_req       = setup_now & ~std_req;
    assign class_out       = dat;
    assign class_out_len   = dat_len;
    // The data stage of a class request that writes: one packet, handed
    // over in the cycle it is decoded and about to be acknowledged. A
    // second copy of it — the host not having heard the first ACK — is
    // acknowledged again and **not** handed over twice, which is what
    // `class_out_wait` is for and is what a data toggle means.
    assign class_out_valid = pkt & pkt_is_data & dat_ok & (expect == X_OUT)
                           & (stage == C_STATUS_IN) & class_active
                           & class_out_wait & out_fits;

    // How long the data stage is: the class's offer for a class request,
    // the descriptor's length otherwise, and never more than wLength.
    wire [OFF_BITS-1:0] desc_len = class_req ? class_len[OFF_BITS-1:0]
                                 : ((s3 == 8'h02) ? CFG_TOTAL[OFF_BITS-1:0] : 18);
    wire [OFF_BITS-1:0] send_len = (w_length < desc_len) ? w_length[OFF_BITS-1:0] : desc_len;

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
            in_total       <= 0;
            in_offset      <= 0;
            in_len         <= 0;
            await_ack      <= 1'b0;
            class_active   <= 1'b0;
            class_out_wait <= 1'b0;
            pending        <= 1'b0;
            pend_pid       <= 4'd0;
            pend_data      <= 1'b0;
            pend_len       <= 0;
            turn           <= 7'd0;
            tx_start       <= 1'b0;
            tx_pid         <= 4'd0;
            tx_with_data   <= 1'b0;
            tx_len_q       <= 0;
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
                                    pend_len  <= 0;
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
                        in_offset <= 0;
                        set_addr  <= 1'b0;
                        set_config <= 1'b0;
                        // A new transfer is nobody's until an arm claims
                        // it, so whatever the last one left is cleared
                        // here and set again below.
                        class_active   <= 1'b0;
                        class_out_wait <= 1'b0;
                        if (pkt_pid != PID_DATA0 || dat_len != 7'd8) begin
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
                        end else if (class_claim) begin
                            // The class above this endpoint answers it.
                            // `class_req` is high in this very cycle and
                            // `class_claim` is the reply to it.
                            class_active <= 1'b1;
                            if (s0[7]) begin
                                // Device to host: a data stage of the bytes
                                // the class offers, capped by wLength the
                                // way a descriptor's is, fetched through
                                // `class_index`.
                                stage    <= C_DATA_IN;
                                in_total <= send_len;
                            end else begin
                                // Host to device: the status stage, with one
                                // data packet expected first if wLength says
                                // there is one. There is no separate stage
                                // for that packet, because the answer to an
                                // IN token is the same either way — a
                                // zero-length DATA1 — and the packet is
                                // recognised by `class_out_wait` instead.
                                stage          <= C_STATUS_IN;
                                class_out_wait <= (w_length != 16'd0);
                            end
                        end else begin
                            stage <= C_STALL;
                        end
                    end else if (dat_ok && expect == X_OUT) begin
                        pending   <= 1'b1;
                        turn      <= 7'd0;
                        pend_data <= 1'b0;
                        if (stage == C_DATA_IN && dat_len == 7'd0) begin
                            // The status stage of a read.
                            pend_pid <= PID_ACK;
                            stage    <= C_IDLE;
                        end else if (stage == C_STATUS_IN && class_active
                                     && !out_fits) begin
                            // A host-to-device data stage longer than one
                            // eight-byte packet, which no request this device
                            // implements has. Stalled, because the alternative
                            // is acknowledging a packet of which only the
                            // first eight bytes were kept.
                            pend_pid <= PID_STALL;
                            stage    <= C_STALL;
                        end else if (stage == C_STATUS_IN && class_active) begin
                            // The data stage of a class request that
                            // writes. Acknowledged whether or not it is the
                            // first copy — a host repeats a packet whose ACK
                            // it did not hear — and `class_out_valid` above
                            // hands over the first copy only.
                            pend_pid       <= PID_ACK;
                            class_out_wait <= 1'b0;
                        end else begin
                            pend_pid <= PID_STALL;
                        end
                    end
                end else if (pkt_pid == PID_ACK && await_ack) begin
                    await_ack <= 1'b0;
                    if (stage == C_DATA_IN) begin
                        in_offset <= in_offset + in_len;
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
                    tx_len_q     <= pend_len;
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
                class_active   <= 1'b0;
                class_out_wait <= 1'b0;
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
//     dat_len        the payload's length in bytes, 0 to 64
//     dat            the **first eight** payload bytes, byte 0 in the low
//                    eight bits
//
//   A handshake is neither, so an endpoint reads `pkt` and `pkt_pid` for
//   those. `pkt` fires for a packet with a bad CRC too — `tok_ok` and
//   `dat_ok` are what say the packet is usable — because a packet that
//   arrived at all, right or wrong, is a reason to stop waiting for a
//   handshake, and an endpoint needs to know that.
//
// WHY THERE IS A WORD AND ALSO A STREAM
//   `dat` is eight bytes and a data endpoint's packets are up to 64, so
//   the payload leaves here **twice**, in two shapes, and the two are for
//   two different readers:
//
//     dat / dat_len          the first eight bytes as one word, held until
//                            the next packet. `usb_ctrl_ep` reads this and
//                            nothing else: a SETUP is exactly eight bytes
//                            (USB 2.0 §9.3) and so is the longest data
//                            stage a class request on this device has.
//     pay_byte / pay_push    one payload byte a cycle as it arrives, with
//                            the PID and the two CRC bytes **already taken
//                            out**. `usb_bulk_ep` fills its own buffer from
//                            this.
//
//   The alternative was one 64-byte word here, and it costs a second
//   64-byte register: the endpoint cannot read this one while it is being
//   overwritten by the next packet, so it would copy all of it. A byte a
//   cycle costs an 8-bit bus and the endpoint's own buffer, which it needs
//   anyway. It is also the only shape in which the endpoint never has to
//   know where the CRC stopped: a `pay_push` byte is a payload byte.
//
//   Taking the CRC out needs two bytes of delay, because **nothing knows a
//   byte is not the CRC until two more arrive**: the packet's length is
//   where it ends and not anything in it. So `hold0` and `hold1` below hold
//   the two most recent bytes and `pay_push` hands over the one before
//   them, which is how the last two bytes of a packet are the two that are
//   never handed over.
//
// What it does not do
//   Sixty-four bytes of payload at most — the largest a full-speed
//   endpoint may declare, USB 2.0 §5.5.3 and §5.8.3 — and `too_long`
//   withdraws `dat_ok` from anything longer rather than truncating it
//   silently. Nothing here knows about addresses, endpoints, toggles or
//   requests: the decoder does not care who a packet is for, and it does
//   not know any endpoint's own `wMaxPacketSize` either — an endpoint
//   narrower than 64 refuses an over-long packet itself, since the size it
//   promised is in its own descriptor.
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
    output wire [6:0]  dat_len,
    output wire [63:0] dat,

    // The payload, a byte a cycle, with the PID and the CRC16 removed.
    output wire [7:0]  pay_byte,
    output wire        pay_push
);
    // PIDs, the low nibble as it appears on the wire.
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_SETUP = 4'b1101;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;

    // The longest payload this decoder will accept, in bytes. USB 2.0
    // §5.5.3 and §5.8.3 allow a full-speed control or bulk endpoint 8, 16,
    // 32 or 64 and §5.7.3 allows an interrupt endpoint up to 64, so 64 is
    // the most any endpoint of this device can promise and the most that is
    // worth decoding. An isochronous endpoint may carry 1023 (§5.6.3) and
    // this device has none.
    localparam integer PAYMAX = 64;

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
    // Bytes received, the PID included, and it **saturates**: a packet of
    // `PAYMAX` bytes ends with `n` at `PAYMAX + 3`, so that is where the
    // count stops rather than wrapping round and making an over-long packet
    // look like a short one. Seven bits, and bit 6 is set for every value
    // from 64 up, so none of them is a bit no expression can reach.
    localparam [6:0] NMAX = PAYMAX + 3;

    reg [6:0]  n;
    reg [7:0]  pid_byte;
    reg [7:0]  tok0, tok1;
    reg [7:0]  d0, d1, d2, d3, d4, d5, d6, d7;
    // The two most recent bytes, neither of which is known to be payload
    // yet: at the end of a packet they are the CRC16 and are the two that
    // are never handed over. "WHY THERE IS A WORD AND ALSO A STREAM" above
    // says why the delay is two and not zero.
    reg [7:0]  hold0, hold1;
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
    assign tok_ok       = (n == 7'd3) & (crc5 == CRC5_RESIDUE);
    assign dat_ok       = (n >= 7'd3) & ~too_long & (crc16 == CRC16_RESIDUE);
    assign dat_len      = n - 7'd3;
    assign dat          = {d7, d6, d5, d4, d3, d2, d1, d0};

    // The payload byte two behind the one arriving, which is a payload byte
    // exactly when a fourth byte of the packet is arriving: the PID, one
    // byte and two more mean the first of them cannot be part of the CRC.
    assign pay_byte     = hold1;
    assign pay_push     = rx_valid & (n >= 7'd3);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            n        <= 7'd0;
            pid_byte <= 8'd0;
            tok0     <= 8'd0;
            tok1     <= 8'd0;
            d0 <= 8'd0; d1 <= 8'd0; d2 <= 8'd0; d3 <= 8'd0;
            d4 <= 8'd0; d5 <= 8'd0; d6 <= 8'd0; d7 <= 8'd0;
            hold0    <= 8'd0;
            hold1    <= 8'd0;
            crc5     <= 5'h1F;
            crc16    <= 16'hFFFF;
            too_long <= 1'b0;
        end else begin
            // Bytes as they arrive.
            if (!rx_active) begin
                n        <= 7'd0;
                crc5     <= 5'h1F;
                crc16    <= 16'hFFFF;
                too_long <= 1'b0;
            end
            if (rx_valid) begin
                if (n != NMAX) n <= n + 7'd1;
                if (n == 7'd0) begin
                    pid_byte <= rx_data;
                end else begin
                    crc5  <= crc5_byte(crc5, rx_data);
                    crc16 <= crc16_byte(crc16, rx_data);
                    // The two-byte delay the stream is taken from. Every
                    // byte after the PID goes through it, CRC included.
                    hold1 <= hold0;
                    hold0 <= rx_data;
                    // A byte arriving at `NMAX` would be payload byte
                    // `PAYMAX`, one more than any endpoint of this device
                    // promised, so the packet is refused whole rather than
                    // cut short.
                    if (n >= NMAX) too_long <= 1'b1;
                    // The first eight payload bytes, as one word, for
                    // `usb_ctrl_ep`. A token's two bytes share the first
                    // two, which is what they were before there was a
                    // stream and costs nothing.
                    case (n)
                        7'd1:    begin tok0 <= rx_data; d0 <= rx_data; end
                        7'd2:    begin tok1 <= rx_data; d1 <= rx_data; end
                        7'd3:    d2 <= rx_data;
                        7'd4:    d3 <= rx_data;
                        7'd5:    d4 <= rx_data;
                        7'd6:    d5 <= rx_data;
                        7'd7:    d6 <= rx_data;
                        7'd8:    d7 <= rx_data;
                        default: begin end
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
// WHERE THE TWO PACKETS ARE KEPT, AND WHY NEITHER IS AN ARRAY
//   A packet is up to 64 bytes each way, so the two buffers are 1024
//   flip-flops and the shape they are written in decides what they cost in
//   lookup tables as well. Three shapes were available and two of them are
//   not:
//
//     an array indexed by a register — `reg [7:0] buf [0:63]` — is a
//     **distributed RAM**, which the ECP5 backend could not place when this
//     was written: `src/fpga/devices/ecp5.dev` declared `TRELLIS_DPR16X4`
//     with no site count, so `fpga::place` counted zero of them and refused.
//     `testdata/fpga/cynthion/usb_cdc_uart.v` says the same thing about
//     `ip/fifo_sync`, and that is why neither block uses an array. **If that
//     has since been fixed, this is the shape worth reconsidering**, because
//     a 64-by-8 distributed RAM is the only one of the three that needs
//     neither a write decoder nor a read multiplexer.
//
//     one wide register written at a computed offset —
//     `buf[{idx, 3'b000} +: 8] <= byte` — is
//     `an assignment to a part-select with a non-constant bound`, which
//     this compiler refuses, and a `case` with sixty-four arms writing
//     sixty-four named byte registers is the same thing spelled out at
//     length: it also costs a six-to-sixty-four decoder to make the write
//     enables.
//
//   So both buffers are **shift registers**, which is what the traffic
//   already is: bytes arrive in order and are given in order, so a byte
//   always goes in at the same end. The write costs no decoder and no
//   lookup table at all, and what is left is the read — one 64-way byte
//   multiplexer per direction, which any of the three shapes needs.
//
//   A shift register puts the packet at the **top** of the buffer and the
//   first byte of a short one therefore sits lower than the first byte of a
//   long one. That is what `obase` and `ibase` are: each counts down from
//   the buffer's size as bytes go in, so it ends up being the position of
//   byte zero, and a read is `base + index`. They are counters and not
//   subtractions, so there is no arithmetic on a length anywhere.
//
//   Both buffers are exactly `MAXPKT` bytes, because what `usb_pkt_rx` hands
//   over are **payload** bytes with the PID and the CRC16 already taken out —
//   its "WHY THERE IS A WORD AND ALSO A STREAM" says how — so a legal packet
//   shifts in exactly its own length. An over-long one would walk the start
//   of itself out of the bottom, so the shift stops at `obase == 0`; such a
//   packet is refused anyway and nothing reads what it left behind.
//
// What it does not do
//   One endpoint number, one packet deep, `MAXPKT` of at most 64 bytes —
//   which USB 2.0 §5.8.3 gives as the largest of the four legal full-speed
//   bulk sizes, the others being 8, 16 and 32, and §5.7.3 as the largest an
//   interrupt endpoint may have. **`MAXPKT` must be a power of two**, which
//   all four of those are: the buffers' byte index is masked to its own
//   width rather than compared against a bound, which is only the same thing
//   when the bound is a power of two. No isochronous endpoint, which may be
//   1023 bytes and would need a ten-bit length.
//
//   An **interrupt** endpoint is this module with a different bmAttributes
//   in the descriptor and nothing else: the packets are identical and only
//   the host's scheduling differs. `WITH_OUT = 0` makes it IN only, which
//   is the shape an interrupt endpoint usually has. A CDC ACM
//   **SERIAL_STATE** notification is ten bytes and this is where it goes;
//   `ip/usb_cdc_acm` gives that endpoint `MAXPKT = 16`, which is the next
//   power of two above ten.
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
    // Bytes in a packet: a power of two, 1 to 64. "WHERE THE TWO PACKETS
    // ARE KEPT" above says why a power of two and USB 2.0 §5.8.3 which four
    // of them a bulk endpoint may declare.
    parameter [6:0]  MAXPKT     = 7'd64,
    // WHICH DIRECTIONS THIS ENDPOINT NUMBER HAS
    //
    // Both, by default, which is a bulk pair. `WITH_OUT = 0` is an **IN-only**
    // endpoint — a CDC ACM notification endpoint, a human interface device's
    // report pipe — and `WITH_IN = 0` is the other way round. A direction
    // that is zero is not answered **at all**: a token for it is ignored
    // rather than NAKed, which is what a host must see from an endpoint that
    // is not in the descriptors, and is the same silence a token for an
    // endpoint number nobody has gets.
    //
    // Nothing is generated away by hand. The registers of a direction that
    // cannot be asked for have no reader — `expect_out` can never be set, so
    // `obuf` is never written and `out_valid` is constantly low — so
    // synthesis removes them, which is why both directions of this module
    // read as one piece of logic below rather than as two halves behind a
    // `generate`. With 64-byte packets that is 528 flip-flops a direction,
    // so it matters more than it did: the notification endpoint of
    // `ip/usb_cdc_acm` is `WITH_OUT = 0` and pays for none of them.
    parameter        WITH_OUT   = 1,
    parameter        WITH_IN    = 1,
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
    input  wire [6:0]  dat_len,
    // The payload a byte a cycle, the PID and the CRC16 already out of it.
    // This is where the OUT buffer is filled from; `dat` is the eight-byte
    // word endpoint 0 reads and is not used here.
    input  wire [7:0]  pay_byte,
    input  wire        pay_push,

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
    output wire [3:0] tx_pid,
    output wire       tx_with_data,
    output wire [6:0] tx_len,
    input  wire [6:0] tx_index,
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

    // WHAT THIS ENDPOINT CAN ANSWER, AND WHY IT IS NOT A PID
    //
    // Four answers: a NAK, an ACK, a DATA0 packet and a DATA1 packet. Held as
    // the PID itself — four bits — bit 2 of those four values is a zero no
    // expression here can set, and **the ECP5 backend refused the
    // bitstream**:
    //
    //   flip-flop `u_dev.u_dev.u_ep1.pend_pid$ff$ff2` has nothing driving its
    //   data input and is not tied high, and an unrouted slice input on this
    //   family reads as a one
    //
    // `usb_ctrl_ep` can also answer a STALL, which is `1110`, so there bit 2
    // is real and four bits are four bits. Here they are not, and the answer
    // to that is not to pad the register: a PID nibble is a **decoding of the
    // answer**, so two bits of answer are registered and the nibble is wires.
    // `PID_DATA0`, `PID_ACK` and the rest stay spelled out above because the
    // decoding has to be readable as the specification's table.
    localparam [1:0] A_NAK   = 2'd0;
    localparam [1:0] A_ACK   = 2'd1;
    localparam [1:0] A_DATA0 = 2'd2;
    localparam [1:0] A_DATA1 = 2'd3;

    // The endpoint addresses this endpoint answers CLEAR_FEATURE for: the
    // direction bit and the number.
    localparam [7:0] ADDR_OUT = {4'h0, ENDP};
    localparam [7:0] ADDR_IN  = {4'h8, ENDP};

    // The widths, from `MAXPKT` and not by hand. A register is as wide as the
    // values it holds, and this module is instantiated three times in
    // `usb_dev_core` with two different packet sizes, so "wide enough for 64"
    // would give the notification endpoint two bits nothing can set.
    //
    //   LEN_BITS   a length, a count of bytes, or a base: 0 to MAXPKT
    //   IDX_BITS   a byte's position in a buffer: 0 to MAXPKT - 1
    localparam integer LEN_BITS = $clog2(MAXPKT + 1);
    localparam integer IDX_BITS = $clog2(MAXPKT);
    // Where each buffer's base counter starts, sized so that nothing has to
    // take a part-select of an `integer` to get it.
    localparam [LEN_BITS-1:0] BASE_TOP = MAXPKT;

    // -----------------------------------------------------------------
    // Host to device.
    // -----------------------------------------------------------------
    // The byte index is scaled to a bit index by concatenation and not by
    // `* 8`. A multiply by a power of two is a shift, but this compiler's
    // synthesis does not strength-reduce one: `ordx * 8` became a `mul`
    // cell, and on the ECP5 a `MULT18X18D`. Three of them, across the two
    // buffers and the descriptor, cost 640 LUT4 and two hard multipliers.
    reg [MAXPKT*8-1:0] obuf;   // the packet, shifted in from the top
    reg [LEN_BITS-1:0] obase;  // where byte 0 of it ended up
    reg [LEN_BITS-1:0] olen;   // bytes in it, 0 when it has been drained
    reg [IDX_BITS-1:0] ordx;   // the byte being handed over
    reg        out_toggle;  // the PID the next packet should carry
    reg        expect_out;  // an OUT token has been seen and its data is next
    reg        out_take;    // ... and there was room, so it is being stored

    // The position of the byte being handed over. `obase` has counted down to
    // byte 0's position, so this is in range for every byte of an accepted
    // packet — `MAXPKT - olen` to `MAXPKT - 1` — and the mask to `IDX_BITS`
    // therefore drops nothing. It is a mask and not a bound because the two
    // are the same thing when `MAXPKT` is a power of two, which the header
    // above requires it to be.
    wire [LEN_BITS-1:0] ordx_at = obase + ordx;
    // One past the byte being handed over, at a length's width.
    wire [LEN_BITS-1:0]   ordx_next = ordx + 1'b1;

    assign out_valid = (olen != 0);
    assign out_data  = obuf[{ordx_at[IDX_BITS-1:0], 3'b000} +: 8];
    assign out_last  = (ordx_next == olen);

    // -----------------------------------------------------------------
    // Device to host.
    // -----------------------------------------------------------------
    // The same shift register the other way round: bytes are given in order,
    // so each one goes in at the top and `ibase` counts down to where byte 0
    // ended up. The packet is **not** consumed by being sent, because a
    // packet the host does not acknowledge goes out again with the same
    // toggle, so nothing shifts on the way out and the read is a multiplexer.
    reg [MAXPKT*8-1:0] ibuf;
    reg [LEN_BITS-1:0] ibase;   // where byte 0 of it is
    reg [LEN_BITS-1:0] ilen;    // bytes given so far
    reg       armed;        // the packet is ready to go out
    reg       in_toggle;    // the PID it will carry
    reg       in_await;     // it has gone out and its ACK has not come back

    // Room while no packet is waiting to be sent. `armed` covers the full
    // buffer too, since the last byte arms it.
    assign in_ready = ~armed;
    // The byte the transmitter is asking for. `tx_index` reaches `ilen` for
    // one fetch the transmitter throws away — `usb_fs_tx` reads `byte_in` in
    // the cycle it decides the payload is over — so the sum reaches `MAXPKT`,
    // which the mask to `IDX_BITS` turns into position 0. That byte is never
    // sent.
    wire [LEN_BITS-1:0] tx_at = ibase + tx_index;
    assign tx_byte  = ibuf[{tx_at[IDX_BITS-1:0], 3'b000} +: 8];

    // -----------------------------------------------------------------
    // The answer, and when it may go out.
    // -----------------------------------------------------------------
    reg       pending;
    reg [1:0] pend_ans;
    reg [LEN_BITS-1:0] pend_len;
    reg [6:0] turn;
    reg [1:0] tx_ans;
    reg [LEN_BITS-1:0] tx_len_q;

    // The PID nibble, and whether the packet carries a payload: both are the
    // answer decoded, and neither is state.
    assign tx_pid = tx_ans[1] ? (tx_ans[0] ? PID_DATA1 : PID_DATA0)
                              : (tx_ans[0] ? PID_ACK   : PID_NAK);
    assign tx_with_data = tx_ans[1];
    // As wide as a packet of this endpoint, zero-extended to the width the
    // transmitter takes for all of them.
    assign tx_len       = tx_len_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            obuf       <= {(MAXPKT*8){1'b0}};
            obase      <= BASE_TOP;
            olen       <= 0;
            ordx       <= 0;
            out_toggle <= 1'b0;
            expect_out <= 1'b0;
            out_take   <= 1'b0;
            ibuf       <= {(MAXPKT*8){1'b0}};
            ibase      <= BASE_TOP;
            ilen       <= 0;
            armed      <= 1'b0;
            in_toggle  <= 1'b0;
            in_await   <= 1'b0;
            pending    <= 1'b0;
            pend_ans   <= A_NAK;
            pend_len   <= 0;
            turn       <= 7'd0;
            tx_ans     <= A_NAK;
            tx_start   <= 1'b0;
            tx_len_q   <= 0;
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
                    out_take   <= 1'b0;
                    if (tok_ok && tok_addr == address && tok_endp == ENDP) begin
                        if (WITH_OUT && pkt_pid == PID_OUT) begin
                            expect_out <= 1'b1;
                            // WHETHER THE PACKET BEHIND THIS TOKEN IS STORED
                            //
                            // Decided **here** and not at the end of it,
                            // because the buffer is filled as the bytes
                            // arrive: a packet whose bytes were shifted in
                            // over a packet that had not been drained would
                            // destroy it, and whether it has been drained can
                            // change halfway through a packet. So the answer
                            // is latched from the one moment at which the
                            // question can still be asked, and a packet that
                            // arrives with no room is NAKed and sent again by
                            // the host, which is what bulk means.
                            //
                            // `obase` goes back to the top **only** when the
                            // packet is going to be stored. It used to be
                            // reset here unconditionally, and that destroyed
                            // a packet which had not been drained yet: the
                            // host's next OUT is NAKed, nothing is shifted in,
                            // but the base of the packet still sitting in
                            // `obuf` had already been thrown away, so every
                            // byte of it read back as zero.
                            out_take <= (olen == 0);
                            if (olen == 0) obase <= BASE_TOP;
                        end else if (WITH_IN && pkt_pid == PID_IN) begin
                            pending <= 1'b1;
                            turn    <= 7'd0;
                            if (armed) begin
                                pend_ans <= in_toggle ? A_DATA1 : A_DATA0;
                                pend_len <= ilen;
                                in_await <= 1'b1;
                            end else begin
                                pend_ans <= A_NAK;
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
                    out_take   <= 1'b0;
                    if (dat_ok && expect_out) begin
                        pending <= 1'b1;
                        turn    <= 7'd0;
                        if (pkt_pid != (out_toggle ? PID_DATA1 : PID_DATA0)) begin
                            // The host did not hear the last ACK. Say it
                            // again and drop the copy.
                            pend_ans <= A_ACK;
                        end else if (dat_len > MAXPKT) begin
                            // Longer than the `wMaxPacketSize` this
                            // endpoint's descriptor promised, which a host
                            // must not send: NAKed, because storing it would
                            // need a buffer the descriptor did not ask for.
                            // `usb_pkt_rx` already refuses anything over 64,
                            // so this is the gap between 64 and an endpoint
                            // narrower than that.
                            pend_ans <= A_NAK;
                        end else if (out_take) begin
                            // The bytes are already in `obuf`: they were
                            // shifted in as they arrived, and `obase` counted
                            // down to where byte 0 of them ended up. All that
                            // is left is to say how many there are.
                            olen       <= dat_len[LEN_BITS-1:0];
                            ordx       <= 0;
                            out_toggle <= ~out_toggle;
                            pend_ans   <= A_ACK;
                        end else begin
                            // Nothing has taken the last packet yet.
                            pend_ans <= A_NAK;
                        end
                    end
                end else if (pkt_pid == PID_ACK && in_await) begin
                    // The packet arrived. The buffer is free and the
                    // toggle moves on.
                    in_await  <= 1'b0;
                    ilen      <= 0;
                    ibase     <= BASE_TOP;
                    armed     <= 1'b0;
                    in_toggle <= ~in_toggle;
                end else begin
                    in_await <= 1'b0;
                end
            end

            // ---------------------------------------------------------
            // The OUT packet's bytes, as they arrive.
            // ---------------------------------------------------------
            // One byte a cycle into the top of `obuf`, which is why there is
            // no decoder and no second copy of the packet. `obase` stops at
            // zero so that a packet longer than the buffer cannot shift the
            // start of itself out of the bottom; such a packet is refused
            // above and nothing reads what it left.
            if (WITH_OUT && expect_out && out_take && pay_push && obase != 0) begin
                obuf  <= {pay_byte, obuf[MAXPKT*8-1:8]};
                obase <= obase - 1'b1;
            end

            // ---------------------------------------------------------
            // The bytes handed over, and the bytes given.
            // ---------------------------------------------------------
            if (out_valid && out_ready) begin
                if (out_last) begin
                    olen <= 0;
                    ordx <= 0;
                end else begin
                    ordx <= ordx + 1'b1;
                end
            end

            if (in_valid && in_ready) begin
                ibuf  <= {in_data, ibuf[MAXPKT*8-1:8]};
                ibase <= ibase - 1'b1;
                ilen  <= ilen + 1'b1;
                if ((ilen + 1'b1) == MAXPKT || in_commit) armed <= 1'b1;
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
                    pending  <= 1'b0;
                    tx_start <= 1'b1;
                    tx_ans   <= pend_ans;
                    tx_len_q <= pend_len;
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
                olen       <= 0;
                ordx       <= 0;
                obase      <= BASE_TOP;
                out_toggle <= 1'b0;
                expect_out <= 1'b0;
                out_take   <= 1'b0;
                ilen       <= 0;
                ibase      <= BASE_TOP;
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
//     usb_bulk_ep    a second endpoint, IN only, for a class that needs
//                    one: `NOTIF_ENDP`, which is `4'd0` for a device that
//                    does not and then nothing of it is built
//
//   and a transmitter that only one of them may have at a time.
//
// THE TRANSMITTER, AND WHO OWNS IT
//   A USB device only ever speaks when it has been asked to, and the
//   asking is a token. So the endpoint that may answer is the endpoint the
//   **last token named**, which is two registers — one per endpoint that
//   is not endpoint 0:
//
//     own_notif <= the token named NOTIF_ENDP and there is one
//     own_data  <= the token named some other non-zero endpoint
//
//   and endpoint 0 owns the transmitter when neither does. Everything
//   after that token belongs to the same endpoint — the data packet of an
//   OUT, the handshake of an IN — because a host does not interleave
//   transactions on one device. `sel` into each endpoint is its own bit, it
//   gates that endpoint's turnaround counter, and the whole arbitration is
//   a multiplexer. There is no request-and-grant and no round robin,
//   because there is never a second answer waiting: the endpoint that has
//   not been asked has nothing to say.
//
//   **Two bits and not an encoded number**, which was the first shape this
//   took. Three owners need two bits either way, and an index costs a
//   decoder at each `sel` and at the multiplexer; two bits that are
//   already the `sel` signals cost neither. It is also what makes the
//   notification endpoint free when there is not one: with
//   `NOTIF_ENDP = 0`, `own_notif` is a flip-flop whose data input is the
//   constant zero, `synth::opt::FfOpt` replaces it with that constant, and
//   the whole second endpoint and its half of the multiplexer go with it.
//   An encoded owner would have left the high bit of a register in the same
//   place, which is a shape this family has cost this project eight rounds
//   of investigation over.
//
//   A token for an endpoint number nobody has hands ownership to the data
//   endpoint, which then ignores it because the number is not its own.
//   Nothing answers, and the host retries and gives up, which is what a
//   device with no such endpoint is supposed to do.
//
// What it does not do
//   Two data endpoints: one pair and one IN. A third is a third
//   `usb_bulk_ep`, a third `own_*` bit and a third arm of the
//   multiplexer, all of it the same shape as the second.
//
//   Nothing here is a class. `usb_ctrl_ep`'s class hook comes straight out
//   of this module, and what answers it is a block above — `ip/usb_cdc_acm`
//   is the first one — because what a class request means is not something
//   a device core can know.
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
    parameter [6:0]   MAXPKT       = 7'd64,
    // A second data endpoint, **IN only**: a CDC ACM notification
    // endpoint, a human interface device's report pipe. `4'd0` is none, and
    // then nothing of it is built. It must not be `DATA_ENDP`, which no
    // arithmetic can check — a descriptor says what the host will do and
    // these say what the device will do.
    parameter [3:0]   NOTIF_ENDP   = 4'd0,
    parameter [6:0]   NOTIF_MAXPKT = 7'd16,
    // Endpoint 0's own packet size, which is also `bMaxPacketSize0`;
    // `usb_ctrl_ep`'s parameter of the same name says what it costs and buys.
    parameter [6:0]   MAXPKT0      = 7'd64,
    // The longest device-to-host class data stage; `usb_ctrl_ep`'s
    // parameter of the same name says what it sets.
    parameter integer CLASS_MAX    = 0,
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
    output wire [6:0] tx_len,
    input  wire [6:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured,

    // The class hook, straight out of `usb_ctrl_ep`, which states the
    // contract.
    output wire [63:0] class_setup,
    output wire        class_req,
    input  wire        class_claim,
    input  wire [6:0]  class_len,
    output wire [6:0]  class_index,
    input  wire [7:0]  class_byte,
    output wire [63:0] class_out,
    output wire [6:0]  class_out_len,
    output wire        class_out_valid,

    // The data endpoint's bytes.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit,

    // The second endpoint's, device to host only. Leave `notif_valid` low
    // for a device that has no second endpoint, which is also what
    // `NOTIF_ENDP = 0` means.
    input  wire [7:0] notif_data,
    input  wire       notif_valid,
    output wire       notif_ready,
    input  wire       notif_commit
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
    wire [6:0]  dat_len;
    wire [63:0] dat;
    wire [7:0]  pay_byte;
    wire        pay_push;

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
        .dat          (dat),
        .pay_byte     (pay_byte),
        .pay_push     (pay_push)
    );

    // -----------------------------------------------------------------
    // Who the last token asked.
    // -----------------------------------------------------------------
    // The second endpoint exists only when it has a number of its own, and
    // `NOTIF_ENDP = 0` is endpoint 0's number, so the test is both.
    wire tok_notif = (NOTIF_ENDP != 4'd0) && (tok_endp == NOTIF_ENDP);
    wire tok_data  = (tok_endp != 4'd0) && !tok_notif;

    reg own_data;
    reg own_notif;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            own_data  <= 1'b0;
            own_notif <= 1'b0;
        end else if (pkt && pkt_is_token && tok_ok && tok_addr == address) begin
            own_data  <= tok_data;
            own_notif <= tok_notif;
        end
    end

    // -----------------------------------------------------------------
    // Endpoint 0.
    // -----------------------------------------------------------------
    wire       c_tx_start, c_tx_with_data;
    wire [3:0] c_tx_pid;
    wire [6:0] c_tx_len;
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
        .CLASS_MAX    (CLASS_MAX),
        .MAXPKT0      (MAXPKT0),
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
        .sel          (~(own_data | own_notif)),
        .tx_start     (c_tx_start),
        .tx_pid       (c_tx_pid),
        .tx_with_data (c_tx_with_data),
        .tx_len       (c_tx_len),
        .tx_index     (tx_index),
        .tx_byte      (c_tx_byte),
        .tx_busy      (tx_busy),
        .address      (address),
        .configured   (configured),
        .class_setup     (class_setup),
        .class_req       (class_req),
        .class_claim     (class_claim),
        .class_len       (class_len),
        .class_index     (class_index),
        .class_byte      (class_byte),
        .class_out       (class_out),
        .class_out_len   (class_out_len),
        .class_out_valid (class_out_valid),
        .ep_reset     (ep_reset),
        .ep_clear     (ep_clear),
        .ep_clear_ep  (ep_clear_ep)
    );

    // -----------------------------------------------------------------
    // The data endpoint.
    // -----------------------------------------------------------------
    wire       b_tx_start, b_tx_with_data;
    wire [3:0] b_tx_pid;
    wire [6:0] b_tx_len;
    wire [7:0] b_tx_byte;

    usb_bulk_ep #(
        .ENDP       (DATA_ENDP),
        .MAXPKT     (MAXPKT),
        .WITH_OUT   (1),
        .WITH_IN    (1),
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
        .pay_byte     (pay_byte),
        .pay_push     (pay_push),
        .address      (address),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .ep_reset     (ep_reset),
        .ep_clear     (ep_clear),
        .ep_clear_ep  (ep_clear_ep),
        .sel          (own_data),
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
    // The second endpoint: IN only, for a class that needs one.
    // -----------------------------------------------------------------
    // With `NOTIF_ENDP = 0` both directions are off, so this instance
    // answers nothing, `pending` inside it can never be set, and synthesis
    // removes all of it along with `own_notif` and this arm of the
    // multiplexer below.
    wire       n_tx_start, n_tx_with_data;
    wire [3:0] n_tx_pid;
    wire [6:0] n_tx_len;
    wire [7:0] n_tx_byte;
    wire [7:0] n_out_data;
    wire       n_out_valid, n_out_last;

    usb_bulk_ep #(
        .ENDP       (NOTIF_ENDP),
        .MAXPKT     (NOTIF_MAXPKT),
        .WITH_OUT   (0),
        .WITH_IN    (NOTIF_ENDP != 4'd0),
        .TURNAROUND (TURNAROUND)
    ) u_ep2 (
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
        .pay_byte     (pay_byte),
        .pay_push     (pay_push),
        .address      (address),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .ep_reset     (ep_reset),
        .ep_clear     (ep_clear),
        .ep_clear_ep  (ep_clear_ep),
        .sel          (own_notif),
        .tx_start     (n_tx_start),
        .tx_pid       (n_tx_pid),
        .tx_with_data (n_tx_with_data),
        .tx_len       (n_tx_len),
        .tx_index     (tx_index),
        .tx_byte      (n_tx_byte),
        .tx_busy      (tx_busy),
        // There is no OUT direction, so these go nowhere and the buffer
        // behind them has no reader.
        .out_data     (n_out_data),
        .out_valid    (n_out_valid),
        .out_last     (n_out_last),
        .out_ready    (1'b0),
        .in_data      (notif_data),
        .in_valid     (notif_valid),
        .in_ready     (notif_ready),
        .in_commit    (notif_commit)
    );

    // -----------------------------------------------------------------
    // The transmitter, to whichever endpoint the token named.
    // -----------------------------------------------------------------
    assign tx_start     = own_notif ? n_tx_start
                        : own_data  ? b_tx_start     : c_tx_start;
    assign tx_pid       = own_notif ? n_tx_pid
                        : own_data  ? b_tx_pid       : c_tx_pid;
    assign tx_with_data = own_notif ? n_tx_with_data
                        : own_data  ? b_tx_with_data : c_tx_with_data;
    assign tx_len       = own_notif ? n_tx_len
                        : own_data  ? b_tx_len       : c_tx_len;
    assign tx_byte      = own_notif ? n_tx_byte
                        : own_data  ? b_tx_byte      : c_tx_byte;
endmodule
