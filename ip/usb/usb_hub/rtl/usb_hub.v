// usb_hub — a full-speed USB 2.0 hub: the descriptors, the hub and port
// class requests and the status-change endpoint a host's own hub driver
// binds to.
//
// What it does
//   A device this block is the class layer of appears on a Linux host as a
//   **hub** — `hub 7-5:1.0: USB hub found`, `1 port detected` — and, as with
//   `ip/usb/usb_cdc_acm`, **no driver ships with it**: a hub is the one class
//   every operating system has to know, since it is how it finds anything
//   else at all.
//
//   It is `usb_dev_core` from `ip/usb/usb_device_fs` with three things added to
//   it, which are the same three a class always is:
//
//     1. the descriptors, as `IFACE_DESC` — one interface and one endpoint,
//        stated below with the section each field comes from;
//     2. `usb_hub_req` on endpoint 0's class hook, answering the hub and port
//        requests of USB 2.0 §11.24.2 and holding the port state they act on;
//     3. an interrupt IN endpoint, which the descriptors promise and which
//        the core builds because `STATUS_ENDP` is not zero, with the sender
//        that puts the status-change bitmap into it.
//
//   And one thing taken away: **there is no bulk endpoint**. `DATA_ENDP` into
//   the core is `4'd0`, so neither direction of it is built; that parameter's
//   own comment says what it costs to leave one in. A hub's endpoints are the
//   control one and the status-change one and USB 2.0 §11.23.1 allows it no
//   others.
//
// WHAT THIS IS HALF OF, AND WHICH HALF
//   **This is a hub's control endpoint and nothing else.** A real hub also
//   repeats every downstream packet to its enabled ports, within about four
//   bit times, and that is the part this block does not have and cannot be
//   given: through a ULPI transceiver the floor is roughly 24 bit times one
//   way — detect SYNC, take a byte, start a fresh SYNC outbound — and USB's
//   turnaround does not allow it. README.md §2 is that argument in full.
//
//   What is behind the port instead is a **second** USB controller:
//   `ip/usb/usb_host_ulpi`, on its own bus. Joining the two conversations is
//   `ip/usb/usb_proxy`, which takes the PC's transaction, runs it again on the
//   other bus and serves the answer back — a transaction proxy, with NAK as the
//   escape hatch that makes the slow path legal. This block on its own still
//   reports a port and never speaks through it, which is what §8 of README.md's
//   kernel log is: a PC that finds a hub, finds something on its port, resets
//   it, and gets no answer. `port_reset` and `port_reset_done` are the one
//   thing the proxy needs from this half — a reset that reaches the real
//   device — and tying the second high is the old behaviour exactly.
//
// THE DESCRIPTORS, AND WHERE EVERY FIELD COMES FROM
//   USB 2.0 §11.23.1 is one page and this is all of it: a hub is a device
//   whose `bDeviceClass` is `09h`, with **one** interface of class `09h` and
//   **one** endpoint on it, an interrupt IN for the status change bitmap.
//   There are no class-specific descriptors in the configuration — the hub
//   descriptor is fetched by a request of its own, §11.23.2.1, and
//   `usb_hub_req` is where its nine bytes are.
//
//   That is a smaller descriptor set than a serial port's and the small parts
//   are the ones a driver checks. Linux's `hub_probe` refuses an interface
//   whose `bInterfaceSubClass` is neither 0 nor 1, refuses one with any
//   number of endpoints but exactly one, and refuses one whose single
//   endpoint is not an interrupt IN — three checks, each of which this
//   descriptor set is written to pass and
//   `usb_hub_descriptors_carry_what_a_host_hub_driver_binds_on` asserts
//   separately.
//
//   **`bDeviceProtocol` is `00h` and that is the whole statement that this is
//   a full-speed hub.** §11.23.1 makes it 0 for a hub with no transaction
//   translator, 1 for a high-speed hub with a single TT and 2 for one with a
//   TT per port, and a high-speed hub is also required to have a second
//   interface alternate setting. One setting and a zero here say full speed
//   only, which is what the link layer below is.
//
// THE STATUS-CHANGE ENDPOINT
//   USB 2.0 §11.12.4: the hub reports changes as a **bitmap**, bit 0 for the
//   hub itself and bit n for port n, which for one port is one byte. A host
//   polls the endpoint, is given the bitmap when something has changed, and
//   is NAKed when nothing has.
//
//   **There is no state in the sender at all**, and that is the point of it:
//
//     wire owed = configured & (change_map != 8'h00);
//
//   The bitmap is a function of `usb_hub_req`'s change bits, those bits are
//   **sticky** — set when something changes and cleared only by the host's own
//   ClearPortFeature(C_PORT_*) — and this arms a packet whenever any of them
//   is set and the endpoint has room. Nothing here records having reported
//   anything, because a record of having reported is exactly what went wrong
//   next door: `ip/usb/usb_cdc_acm` had a latch meaning "the host has been told",
//   it was set once per configuration, and a host that was not listening at
//   that moment — or a driver bound a second time without a bus reset — never
//   heard again. §4 of that block's README.md has the measurement that
//   condemned it. A design with no such latch cannot have that defect.
//
//   `usb_bulk_ep`'s IN side is what holds the packet, and it is driven with
//   `in_valid`, `in_commit` and `in_ready` all in one cycle: one byte handed
//   over and committed together, so a **one-byte** packet goes out of a
//   two-byte endpoint rather than waiting for a second byte that is not
//   coming. That is the same `in_commit` a short serial packet uses.
//
//   **What it costs to have no latch.** A host that has read the bitmap and
//   not yet sent the ClearPortFeature is sent the same bitmap again, because
//   the change bit is still set — one extra one-byte packet per poll until
//   the clear lands, which on Linux is a few milliseconds and one or two
//   polls. The driver ORs the bitmaps together and acts once. The alternative
//   is a latch that says "this bitmap has already gone", and that is the
//   defect above with a different name: nothing in a hub can know whether the
//   host that received a bitmap is the host that will act on it. So the extra
//   packet is paid and written down.
//
//   A packet already armed when the host clears the change goes out with the
//   old bitmap, since `usb_bulk_ep` has no way to unsay a packet it has been
//   given. A host then does one GetPortStatus, finds nothing changed, and
//   carries on.
//
// What it does not do
//   **One port**, and `usb_hub_req`'s header says what a second would cost.
//
//   **No packet repeating and no frame forwarding**, which is `ip/usb/usb_proxy`'s
//   and is a transaction proxy rather than a repeater — README.md §2 is why it
//   cannot be one. No downstream SE0 of this block's own either: the port reset
//   is the `port_reset` / `port_reset_done` handshake and whatever answers it
//   does the driving. No suspend or resume signalling on the port, no
//   transaction translator and no high speed.
//
//   No strings, so the hub has no product name in `lsusb`, for the same
//   reason `ip/usb/usb_cdc_acm` has none: a string descriptor is a device's
//   property and not a class's.
//
//   No over-current detection and no local power supply, which
//   `wHubCharacteristics` says and `wHubStatus` repeats.
//
//   Nothing here knows about NRZI, bit stuffing, SYNC, EOP, line states or
//   ULPI. The link layer below delivers bytes and takes packets, exactly as
//   it does for `usb_dev_core`.
module usb_hub #(
    parameter [15:0] VID        = 16'h1209,
    parameter [15:0] PID        = 16'h0001,
    // bmAttributes and bMaxPower of the configuration: bus powered, 100 mA.
    //
    // **This is the hub controller's own draw and not its port's.** A
    // bus-powered hub that supplied its downstream ports out of its upstream
    // cable would have to declare their current here as well; the socket
    // behind this one is powered by the board's own VBUS switch, which is a
    // parameter of the design at the top and not something a host may ask
    // for. README.md §4 says it again where it matters.
    parameter [7:0]  CFG_ATTR   = 8'h80,
    parameter [7:0]  CFG_POWER  = 8'd50,
    // Endpoint 0's packet size, which is also `bMaxPacketSize0`. USB 2.0
    // §5.5.3 allows 8, 16, 32 or 64; `usb_ctrl_ep`'s parameter of the same
    // name says what raising it buys and what the one asymmetry of it is.
    parameter [6:0]  MAXPKT0    = 7'd64,
    // What shape the status endpoint's packet buffer takes: 1 an array, 0 a
    // shift register. `usb_bulk_ep`'s parameter of the same name has the
    // measurements — and at two bytes it is very nearly nothing either way,
    // so this is here for consistency with the rest of the library rather
    // than because a design should have an opinion.
    parameter        BUF_RAM    = 1,
    // Cycles of `line_idle` before an answer starts; `usb_ctrl_ep`'s
    // parameter of the same name says what it has to be and why.
    parameter [6:0]  TURNAROUND = 7'd8
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

    // WHAT IS ON THE DOWNSTREAM PORT
    //
    // Straight into `usb_hub_req`, whose port comment says what each means.
    // On the design this block was written for they are `ip/usb/usb_host_ulpi`'s
    // `attached` and `low_speed`, which is a **second USB controller's**
    // debounced sight of its own bus.
    input  wire       port_attached,
    input  wire       port_low_speed,

    // WHAT THE HOST HAS MADE OF THAT PORT
    //
    // Brought out for whatever drives the downstream half: a design's LEDs and
    // console, or `ip/usb/usb_proxy`, which is what makes the port real. All four
    // are levels.
    output wire       port_power,
    output wire       port_enabled,
    output wire       port_suspended,
    // PORT_RESET: raised by SetPortFeature(PORT_RESET) and held until
    // `port_reset_done`. `usb_hub_req`'s "THE RESET, WHICH NOW TAKES TIME" is
    // the handshake; **tie `port_reset_done` high for a design with nothing
    // downstream** and the reset is over in the cycle it is asked for, which is
    // what this block did before there was a proxy.
    output wire       port_reset,
    input  wire       port_reset_done
);
    // -----------------------------------------------------------------
    // The interface, and the one endpoint on it.
    // -----------------------------------------------------------------
    // USB 2.0 §11.23.1: `09h` in the device descriptor's `bDeviceClass` and
    // again in the interface descriptor's `bInterfaceClass`.
    localparam [7:0] CLASS_HUB = 8'h09;
    // bDeviceSubClass, bInterfaceSubClass and both protocol bytes. §11.23.1
    // makes `bDeviceProtocol` the field that says what kind of hub this is
    // and 0 the full-speed one; the header above has the three values.
    localparam [7:0] SUBCLASS_NONE = 8'h00;
    localparam [7:0] PROTOCOL_FS   = 8'h00;

    localparam [7:0] HUB_IFACE = 8'd0;
    // The status-change endpoint's number. Endpoint 1, IN, so `81h`.
    localparam [3:0] STATUS_ENDP = 4'd1;
    // Bytes in one of its packets, which goes into `wMaxPacketSize`.
    //
    // **Two, for a bitmap that is one.** §11.12.4 makes the bitmap one bit a
    // port plus bit 0 for the hub, rounded up to a byte, so one port is one
    // byte, and USB 2.0 §5.7.3 allows a full-speed interrupt endpoint
    // anything up to 64 — so one would be legal. It is two because
    // `usb_bulk_ep` masks its buffer's byte index to the index's own width
    // rather than comparing it against a bound, which needs the size to be a
    // power of two, and at a size of one that index is **zero bits wide**: a
    // part-select of nothing, which is not a register a fabric can build. Two
    // is the smallest size that is both a power of two and a width.
    //
    // A host is unaffected either way: the device sends one byte, which is a
    // short packet, and a short packet ends an interrupt transfer whatever
    // the maximum was.
    localparam [6:0] STATUS_MAXPKT = 7'd2;
    // How often a host polls it, in milliseconds: `bInterval` of a full-speed
    // interrupt endpoint is a count of frames (USB 2.0 Table 9-13).
    //
    // **Twelve is this block's choice and not a quotation.** It is how long a
    // host may be behind this hub's idea of its port — a device plugged in is
    // noticed up to 12 ms late — against one one-byte transaction every
    // 12 ms of bus time for as long as the hub is configured. Hubs in the
    // field use anything from 12 to 255.
    localparam [7:0] STATUS_INTERVAL = 8'd12;

    // bmAttributes of an endpoint descriptor, USB 2.0 Table 9-13 bits 1:0.
    localparam [7:0] EP_INTERRUPT = 8'h03;
    // Descriptor types, USB 2.0 Table 9-5.
    localparam [7:0] DESC_INTERFACE = 8'd4;
    localparam [7:0] DESC_ENDPOINT  = 8'd5;

    // The whole of it, in descriptor order. A Verilog concatenation lists its
    // most significant part first, which is the order `lsusb -v` prints and
    // the order a host walks; `usb_ctrl_ep` turns it round once at
    // elaboration and counts `bNumInterfaces`, `bNumEndpoints` and
    // `wTotalLength` out of it rather than believing them.
    //
    // Byte 4 of the interface descriptor is `bNumEndpoints` and is written as
    // `8'd0`: it is **counted** from the endpoint descriptor that follows.
    localparam integer IFACE_BYTES = 16;
    localparam [IFACE_BYTES*8-1:0] IFACE_DESC = {
        // INTERFACE 0 — the hub's only interface, USB 2.0 §11.23.1.
        8'd9, DESC_INTERFACE, HUB_IFACE, 8'd0, 8'd0,
              CLASS_HUB, SUBCLASS_NONE, PROTOCOL_FS, 8'd0,
        // ENDPOINT 81h — the status change endpoint. Interrupt IN, two bytes,
        // every twelve frames. `wMaxPacketSize` is two bytes, low first, and
        // is the localparam above rather than a number typed twice.
        8'd7, DESC_ENDPOINT, {4'h8, STATUS_ENDP}, EP_INTERRUPT,
              {1'b0, STATUS_MAXPKT}, 8'd0, STATUS_INTERVAL
    };

    // -----------------------------------------------------------------
    // The class requests, and the port state they act on.
    // -----------------------------------------------------------------
    wire [63:0] class_setup;
    wire        class_req;
    wire        class_claim;
    wire [6:0]  class_len;
    wire [6:0]  class_index;
    wire [7:0]  class_byte;
    // The hook's host-to-device half, which no request this block claims has:
    // `usb_hub_req`'s header says why, and these three go nowhere.
    wire [63:0] class_out;
    wire [6:0]  class_out_len;
    wire        class_out_valid;

    wire [7:0]  change_map;

    usb_hub_req u_req (
        .clk            (clk),
        .rst_n          (rst_n),
        .setup          (class_setup),
        .req            (class_req),
        .claim          (class_claim),
        .len            (class_len),
        .index          (class_index),
        .resp           (class_byte),
        .configured     (configured),
        .port_attached  (port_attached),
        .port_low_speed (port_low_speed),
        .port_power     (port_power),
        .port_enabled   (port_enabled),
        .port_suspended (port_suspended),
        .port_reset      (port_reset),
        .port_reset_done (port_reset_done),
        .change_map     (change_map)
    );

    // -----------------------------------------------------------------
    // The bitmap, into the status-change endpoint.
    // -----------------------------------------------------------------
    // One byte, handed over and committed in the same cycle, whenever a
    // change bit is set and the endpoint has room. "THE STATUS-CHANGE
    // ENDPOINT" above is why there is no register here and what that costs.
    //
    // The device is not configured until SET_CONFIGURATION, and sending
    // before that would be a packet on an endpoint the host has not enabled.
    wire       status_ready;
    wire       owed = configured & (change_map != 8'h00);
    wire [7:0] status_data   = change_map;
    wire       status_valid  = owed & status_ready;
    wire       status_commit = owed & status_ready;

    // -----------------------------------------------------------------
    // The device.
    // -----------------------------------------------------------------
    // The data endpoint's byte interface, which is not built: `DATA_ENDP` is
    // `4'd0`, so both directions of that endpoint are off and these four
    // outputs are constants synthesis removes along with everything behind
    // them.
    wire [7:0] no_out_data;
    wire       no_out_valid, no_out_last, no_in_ready;

    // `CLASS_MAX` is nine, the length of the hub descriptor, which is the
    // longest device-to-host class data stage this block has. It is what sets
    // the width of endpoint 0's data-stage counters, so it is the hub
    // descriptor's length and not a round number.
    usb_dev_core #(
        .VID          (VID),
        .PID          (PID),
        .DEV_CLASS    (CLASS_HUB),
        .DEV_SUBCLASS (SUBCLASS_NONE),
        .DEV_PROTOCOL (PROTOCOL_FS),
        .CFG_ATTR     (CFG_ATTR),
        .CFG_POWER    (CFG_POWER),
        .IFACE_BYTES  (IFACE_BYTES),
        .IFACE_DESC   (IFACE_DESC),
        .DATA_ENDP    (4'd0),
        .MAXPKT       (7'd8),
        .MAXPKT0      (MAXPKT0),
        .NOTIF_ENDP   (STATUS_ENDP),
        .NOTIF_MAXPKT (STATUS_MAXPKT),
        .CLASS_MAX    (9),
        .BUF_RAM      (BUF_RAM),
        .TURNAROUND   (TURNAROUND)
    ) u_dev (
        .clk          (clk),
        .rst_n        (rst_n),
        .rx_data      (rx_data),
        .rx_valid     (rx_valid),
        .rx_eop       (rx_eop),
        .rx_active    (rx_active),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .tx_start     (tx_start),
        .tx_pid       (tx_pid),
        .tx_with_data (tx_with_data),
        .tx_len       (tx_len),
        .tx_index     (tx_index),
        .tx_byte      (tx_byte),
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
        .out_data     (no_out_data),
        .out_valid    (no_out_valid),
        .out_last     (no_out_last),
        .out_ready    (1'b0),
        .in_data      (8'h00),
        .in_valid     (1'b0),
        .in_ready     (no_in_ready),
        .in_commit    (1'b0),
        .notif_data   (status_data),
        .notif_valid  (status_valid),
        .notif_ready  (status_ready),
        .notif_commit (status_commit)
    );
endmodule
