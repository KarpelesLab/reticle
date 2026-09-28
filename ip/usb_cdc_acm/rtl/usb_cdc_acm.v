// usb_cdc_acm — a USB serial port: the descriptors, the class requests and
// the endpoints a host's own CDC ACM driver binds to, above the line and
// below whatever wants the bytes.
//
// What it does
//   A device this block is the class layer of appears on a Linux host as
//   `/dev/ttyACM*`, on macOS as `/dev/cu.usbmodem*` and on Windows as a COM
//   port, and **no driver ships with it**: CDC ACM is a class the operating
//   system already knows, which is the whole reason it is the first class
//   in this library. `out_*` is what a program wrote to that port and
//   `in_*` is what it will read, a byte at a time each way.
//
//   It is `usb_dev_core` from `ip/usb_device_fs` with three things added to
//   it, and it is worth being clear that those three are all a class *is*:
//
//     1. the descriptors, as `IFACE_DESC` — two interfaces, five
//        functional descriptors and three endpoints, stated below with the
//        table each field comes from;
//     2. `usb_cdc_req` on endpoint 0's class hook, answering the three
//        requests a serial driver sends;
//     3. an interrupt IN endpoint, which the descriptors promise and which
//        the core builds because `NOTIF_ENDP` is not zero.
//
//   Everything else — the packet decoding, the standard requests, the data
//   toggles, the transmitter and the arbitration between endpoints — is the
//   core's and is not restated here. What this module brings out on its
//   link side is exactly what `usb_dev_core` brings out, so
//   `usb_cdc_acm_fs` and `usb_cdc_acm_ulpi` beside it are the same two
//   link layers `usb_device_fs` and `usb_device_ulpi` are, with this in
//   place of the core.
//
// THE DESCRIPTORS, AND WHERE EVERY FIELD COMES FROM
//   CDC is the class that cannot be done with one interface. A serial port
//   is a **communications** interface that carries the control requests and
//   a **data** interface that carries the bytes, and the thing that says
//   the two belong to one function is the **union functional descriptor**.
//   Linux's `cdc_acm` reads it to find which interface is which; a device
//   that omits it reaches `cdc_acm`'s quirk path and needs to be in a
//   table of known-broken devices to bind at all. **That last sentence is a
//   reading of a driver and not a measurement** — nothing here built the
//   device that would test it — and README.md §2 marks it as one. So it is
//   here, and so is the rest of what the specifications list as required:
//
//   | Descriptor | Bytes | Where the fields are |
//   |------------|-------|----------------------|
//   | INTERFACE 0, class 02h subclass 02h | 9 | CDC 1.1 Table 15 (class), Table 16 (subclass) |
//   | Header functional | 5 | CDC 1.1 Table 26 |
//   | Call Management functional | 5 | PSTN 1.2 Table 3 |
//   | Abstract Control Management functional | 4 | PSTN 1.2 Table 4 |
//   | Union functional | 5 | CDC 1.1 Table 33 |
//   | ENDPOINT 82h, interrupt IN | 7 | USB 2.0 Table 9-13 |
//   | INTERFACE 1, class 0Ah | 9 | CDC 1.1 Table 18 |
//   | ENDPOINT 01h, bulk OUT | 7 | USB 2.0 Table 9-13 |
//   | ENDPOINT 81h, bulk IN | 7 | USB 2.0 Table 9-13 |
//
//   Fifty-eight bytes, which with the nine of the configuration descriptor
//   `usb_ctrl_ep` writes is a `wTotalLength` of sixty-seven. `DESC_MAX` in
//   that module is sixty-four, so this fits with six bytes to spare and is
//   the descriptor set that number was chosen for.
//
//   `bNumInterfaces`, both `bNumEndpoints` and `wTotalLength` are **not**
//   in the blob below: `usb_ctrl_ep` counts them out of it at elaboration
//   and writes them over what is there, so a descriptor added here cannot
//   disagree with the arithmetic that describes it. The `8'd0` sitting in
//   byte 4 of each interface descriptor is a placeholder and is documented
//   as one at both ends.
//
//   README.md in this package says which of these facts were **checked**
//   against a host and which are **quoted** from a table, which is the
//   distinction this project's protocol documents keep.
//
// THE NOTIFICATION ENDPOINT, AND WHAT IT SENDS
//   The communications interface has an interrupt IN endpoint because CDC 1.1
//   §3.2 gives one to the notification element and because `cdc_acm` is
//   understood to take `endpoint[0]` of that interface and refuse the device
//   if it is not an interrupt IN — a reading of a driver, marked as one in
//   README.md §4, not something measured here. So the descriptor declares
//   one: endpoint 2 IN, sixteen bytes, polled every 16 frames.
//
//   What it sends is **SERIAL_STATE**, PSTN 1.2 §6.5.4: ten bytes, an
//   eight-byte notification header and a two-byte `wSerialState` bitmap.
//   Every field of it is written out at `notif_byte` below with the table it
//   comes from. It used to send nothing at all, and the reason was arithmetic
//   rather than choice: ten bytes did not fit an endpoint that held eight,
//   because the length both transmitters took was four bits. That field is
//   seven bits now — `usb_fs_tx`'s header says why seven — so the endpoint
//   holds sixteen and the notification fits.
//
//   **WHEN it is sent, which is a reading of §6.5.4 rather than a quotation of
//   it.** SERIAL_STATE carries the *current state* of the UART's lines and its
//   error conditions, and Table 4's `bmCapabilities` D1 groups it with the
//   line-coding requests as one feature; what §6.5.4 does not say is when a
//   device must produce one, and nothing in PSTN 1.2 asks for one on a
//   schedule. Reading it as a **state-change** notification — sent when what
//   it would say changes — is what this block does, and README.md §4 marks
//   that as a reading. So it sends one
//
//     * when the host configures the device, because the host's own idea of
//       the state starts empty and nothing else would ever fill it — Linux's
//       `cdc_acm` keeps the last bitmap it was sent in `ctrlin` and answers
//       `TIOCMGET` out of it, so a device that never sends one reports no
//       carrier for ever; and
//     * whenever `serial_state` changes afterwards.
//
//   and **not** on a timer, because a state that has not changed is not news
//   and an interrupt endpoint that NAKs a poll is the ordinary state of a
//   device that is not changing.
//
//   **What a device with no modem lines should report.** `serial_state` is a
//   port and not a constant, because the answer is a property of what is
//   wired to the block and not of the block. What this library's own designs
//   tie it to is `7'b000_0011` — `bRxCarrier` and `bTxCarrier` set, every
//   error bit clear — and the argument is what §6.5.4 makes those two bits:
//   `bRxCarrier` is the state of the device's receiver carrier detection, which
//   the table equates with V.24 signal 109 and RS-232's DCD, and `bTxCarrier`
//   is the transmission carrier, V.24 signal 106 and RS-232's DSR. A device
//   whose serial port is inside the same die as the USB endpoint has its
//   carrier present and its data set ready from the moment it exists, so both
//   are one; there is nothing that could ever make them zero. That is an
//   argument and not a rule: the specification says what the bits mean, not
//   what a device with no such lines must put in them. The error bits —
//   break, ring, framing, parity, overrun — are **events a UART reports**, so
//   a design with a UART drives them and one without leaves them clear.
//
//   **What this establishes and what it assumes.** That the ten bytes are
//   these ten bytes is read off PSTN 1.2 §6.5.4 and is a **quotation**. That
//   a host acts on them is a **measurement**: §5 of README.md has `TIOCMGET`
//   on the part reporting DCD and DSR, which is `cdc_acm` having received and
//   decoded this notification, and §4 says what was believed about the driver
//   before that measurement existed.
//
// What it does not do
//   One serial port. A composite device with two of them needs two of
//   everything below and an interface association descriptor above it.
//
//   Sixty-four bytes a packet on the bulk endpoints, which is the largest
//   USB 2.0 §5.8.3 allows a full-speed bulk endpoint — the other three legal
//   sizes are 8, 16 and 32 — and `MAXPKT` is a parameter so a design may
//   choose one of the others. It was eight, and what that cost was
//   throughput: a host is limited in **transactions** a frame rather than in
//   bytes, so eight bytes a transaction was eight times as many of them.
//   README.md §6 has the two figures measured on a part.
//
//   A host reading this port still reads **one packet at a time**, and the
//   reason has not changed with the size: a bulk IN transfer ends on a short
//   packet or a full buffer, so a read of 128 bytes answered with 64 is not
//   finished and the host asks again. What has changed is that a read of
//   exactly 64 now ends on a full buffer, which is the common case.
//
//   No flow control on either side but USB's own NAK. There is no FIFO
//   here: the endpoint holds one packet each way and NAKs the host while
//   it is full, which is what bulk means. `fifo_sync` is a block in this
//   library, so a design that wants depth — and a design bridging to
//   something as slow as a UART wants depth on the receive side — puts one
//   on either side of this block.
//
//   Nothing here acts on the line coding. `baud`, `char_format`, `parity`
//   and `data_bits` are what the host asked for and are brought out for a
//   design to use; a UART divisor that followed `baud` would be a divide by
//   a run-time value. `usb_cdc_req` says the same thing at more length.
//
//   No strings, so the port has no product name in `lsusb`. Strings are
//   the one thing `usb_ctrl_ep` stalls that a class hook could now answer,
//   and this block does not, because a string descriptor is a device's
//   property and not a serial port's.
//
//   Nothing here knows about NRZI, bit stuffing, SYNC, EOP, line states or
//   ULPI. The link layer below delivers bytes and takes packets, exactly as
//   it does for `usb_dev_core`.
module usb_cdc_acm #(
    parameter [15:0] VID        = 16'h1209,
    parameter [15:0] PID        = 16'h0001,
    // bmAttributes and bMaxPower of the configuration: bus powered, 100 mA.
    parameter [7:0]  CFG_ATTR   = 8'h80,
    parameter [7:0]  CFG_POWER  = 8'd50,
    // Bytes in a bulk packet, which goes into `wMaxPacketSize` of both bulk
    // endpoint descriptors below rather than being typed there as well. USB
    // 2.0 §5.8.3 allows a full-speed bulk endpoint 8, 16, 32 or 64 and
    // **nothing else**, which no arithmetic here can check; 64 is the largest
    // and is what a serial port wants, and it is a parameter so that a design
    // short of flip-flops can take one of the smaller ones and have the
    // descriptor follow.
    parameter [6:0]  MAXPKT     = 7'd64,
    // Endpoint 0's, which is `bMaxPacketSize0`. §5.5.3 allows it the same
    // four values; `usb_ctrl_ep`'s parameter of the same name says what
    // raising it buys and what the one asymmetry of it is.
    parameter [6:0]  MAXPKT0    = 7'd64,
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

    // The serial port's bytes. `out_*` is what the host wrote to it and
    // `in_*` is what it will read; `in_commit` sends what has been given
    // however short, which on a serial port is what keeps a single
    // keystroke from waiting for seven more.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit,

    // What the host asked the line to be, and the control lines it
    // asserted. `dtr` is a program having the port open.
    output wire [31:0] baud,
    output wire [7:0]  char_format,
    output wire [7:0]  parity,
    output wire [7:0]  data_bits,
    output wire        dtr,
    output wire        rts,

    // THE STATE THIS PORT REPORTS TO THE HOST
    //
    // `wSerialState` of a SERIAL_STATE notification, PSTN 1.2 §6.5.4 Table 31,
    // one bit a line:
    //
    //   [0] bRxCarrier    DCD, V.24 signal 109
    //   [1] bTxCarrier    DSR, V.24 signal 106
    //   [2] bBreak        a break is being received
    //   [3] bRingSignal   a ring signal is being received
    //   [4] bFraming      a framing error has been detected
    //   [5] bParity       a parity error has been detected
    //   [6] bOverRun      a character was overwritten before it was read
    //
    // Bits 7 to 15 of the field are reserved and this block sends them zero,
    // which is why the port is seven bits and not sixteen: a port as wide as
    // the field would invite a design to set a bit the specification reserves.
    //
    // **A notification goes out whenever this changes**, and once when the
    // host configures the device, so a design that ties it to a constant gets
    // exactly one notification per configuration. `7'b000_0011` is what a port
    // with no modem lines reports and "THE NOTIFICATION ENDPOINT" above has
    // the argument for it.
    input  wire [6:0]  serial_state
);
    // -----------------------------------------------------------------
    // The interfaces, and the endpoints on them.
    // -----------------------------------------------------------------
    // The communications interface carries the class requests; the data
    // interface carries the bytes. `usb_cdc_req` compares `wIndex` against
    // the first of these and the union functional descriptor names both, so
    // they are localparams rather than literals in three places.
    localparam [7:0] COMM_IFACE = 8'd0;
    localparam [7:0] DATA_IFACE = 8'd1;
    // The endpoint numbers. `NOTIF_ENDP` is on the communications
    // interface and is IN only; `DATA_ENDP` is the bulk pair on the data
    // interface.
    localparam [3:0] NOTIF_ENDP = 4'd2;
    localparam [3:0] DATA_ENDP  = 4'd1;
    // Bytes in a notification packet. Ten are needed and this is sixteen: a
    // SERIAL_STATE is ten bytes, `usb_bulk_ep` masks its buffer's byte index
    // to the index's own width so the size has to be a power of two, and USB
    // 2.0 §5.7.3 allows a full-speed interrupt endpoint **anything up to 64**
    // — unlike a bulk one, which §5.8.3 restricts to four values — so sixteen
    // is legal and is the smallest legal size that holds the notification.
    localparam [6:0] NOTIF_MAXPKT = 7'd16;
    // Bytes in a SERIAL_STATE notification: the eight-byte header of
    // PSTN 1.2 §6.5 and the two bytes of `wSerialState`.
    localparam integer NOTIF_BYTES = 10;
    // The index of the last of them, sized so that nothing takes a
    // part-select of an `integer`.
    localparam [3:0]   NOTIF_LAST  = NOTIF_BYTES - 1;
    // How often a host polls the notification endpoint, in milliseconds:
    // `bInterval` of a full-speed interrupt endpoint is a count of frames
    // (USB 2.0 Table 9-13). Sixteen milliseconds is how long the host may be
    // behind this device's idea of its own line state, which for a port whose
    // carrier never drops is sixteen milliseconds once, at configuration.
    localparam [7:0] NOTIF_INTERVAL = 8'd16;

    // Class codes. CDC 1.1 Table 14 for the device, Table 15 and Table 16
    // for the communications interface, Table 18 for the data interface.
    localparam [7:0] CLASS_COMM = 8'h02;
    localparam [7:0] CLASS_DATA = 8'h0A;
    localparam [7:0] SUBCLASS_ACM = 8'h02;
    // bInterfaceProtocol. CDC 1.1 Table 17 lists `00h` as "no class
    // specific protocol required" and `01h` as AT commands, V.250. This
    // device answers no AT commands, so it says so: `00h`. Linux binds
    // `cdc_acm` on the class and subclass and does not read this field to
    // decide, which §5 of README.md establishes on the part.
    localparam [7:0] PROTOCOL_NONE = 8'h00;

    // The functional descriptors' own type and subtypes: CDC 1.1 Table 24
    // for `24h` (CS_INTERFACE) and Table 25 for the subtypes.
    localparam [7:0] CS_INTERFACE  = 8'h24;
    localparam [7:0] FD_HEADER     = 8'h00;
    localparam [7:0] FD_CALL_MGMT  = 8'h01;
    localparam [7:0] FD_ACM        = 8'h02;
    localparam [7:0] FD_UNION      = 8'h06;

    // bmCapabilities of the Abstract Control Management functional
    // descriptor, PSTN 1.2 Table 4:
    //
    //   D0  Set/Clear/Get_Comm_Feature        — not implemented, clear
    //   D1  Set_Line_Coding, Set_Control_Line_State, Get_Line_Coding
    //       and the Serial_State notification — the three requests are
    //       implemented, so this is set
    //   D2  Send_Break                        — not implemented, clear
    //   D3  the Network_Connection notification — clear
    //
    // D1 is one bit over four things and this block does three of them:
    // the notification is the fourth and it is never sent, for the reason
    // "THE NOTIFICATION ENDPOINT" above gives. Setting D1 is what makes
    // Linux send SET_LINE_CODING at all — `cdc_acm` gates it on this bit —
    // so a device that cleared D1 to be pedantic about the notification
    // would stop being asked the questions it *can* answer.
    localparam [7:0] ACM_CAPS = 8'h02;

    // bmCapabilities of the Call Management functional descriptor, PSTN
    // 1.2 Table 3: `00h` is a device that handles no call management
    // itself and carries no call management over the data interface, which
    // is what a serial port is.
    localparam [7:0] CALL_CAPS = 8'h00;

    // bcdCDC of the header functional descriptor: 1.10, as two bytes
    // little endian (CDC 1.1 Table 26).
    localparam [7:0] CDC_VER_LO = 8'h10;
    localparam [7:0] CDC_VER_HI = 8'h01;

    // bmAttributes of an endpoint descriptor, USB 2.0 Table 9-13 bits 1:0.
    localparam [7:0] EP_BULK      = 8'h02;
    localparam [7:0] EP_INTERRUPT = 8'h03;

    // Descriptor types, USB 2.0 Table 9-5.
    localparam [7:0] DESC_INTERFACE = 8'd4;
    localparam [7:0] DESC_ENDPOINT  = 8'd5;

    // The whole of it, in descriptor order. A Verilog concatenation lists
    // its most significant part first, which is the order `lsusb -v` prints
    // and the order a host walks. `usb_ctrl_ep` turns it round once at
    // elaboration.
    //
    // Byte 4 of each interface descriptor is `bNumEndpoints` and is written
    // as `8'd0`: it is **counted** from the endpoint descriptors that
    // follow, at elaboration, and written over whatever is here.
    localparam integer IFACE_BYTES = 58;
    localparam [IFACE_BYTES*8-1:0] IFACE_DESC = {
        // INTERFACE 0 — the communications interface.
        8'd9, DESC_INTERFACE, COMM_IFACE, 8'd0, 8'd0,
              CLASS_COMM, SUBCLASS_ACM, PROTOCOL_NONE, 8'd0,
        // Header functional descriptor: CDC 1.1 Table 26.
        8'd5, CS_INTERFACE, FD_HEADER, CDC_VER_LO, CDC_VER_HI,
        // Call Management functional descriptor: PSTN 1.2 Table 3.
        // bDataInterface names the interface that would carry call
        // management if bmCapabilities asked for it.
        8'd5, CS_INTERFACE, FD_CALL_MGMT, CALL_CAPS, DATA_IFACE,
        // Abstract Control Management functional descriptor: PSTN 1.2
        // Table 4.
        8'd4, CS_INTERFACE, FD_ACM, ACM_CAPS,
        // Union functional descriptor: CDC 1.1 Table 33. This is the one
        // that says interface 1 is subordinate to interface 0 and that the
        // two are one function, and the one a host is understood to read to
        // tell them apart; README.md §2 says how sure of that this is.
        8'd5, CS_INTERFACE, FD_UNION, COMM_IFACE, DATA_IFACE,
        // ENDPOINT 82h — the notification endpoint. Interrupt IN, sixteen
        // bytes, every 16 frames. wMaxPacketSize is two bytes, low first, and
        // is the localparam above rather than a number typed twice.
        8'd7, DESC_ENDPOINT, {4'h8, NOTIF_ENDP}, EP_INTERRUPT,
              {1'b0, NOTIF_MAXPKT}, 8'd0, NOTIF_INTERVAL,
        // INTERFACE 1 — the data interface.
        8'd9, DESC_INTERFACE, DATA_IFACE, 8'd0, 8'd0,
              CLASS_DATA, 8'h00, PROTOCOL_NONE, 8'd0,
        // ENDPOINT 01h — bulk OUT, the bytes the host writes.
        8'd7, DESC_ENDPOINT, {4'h0, DATA_ENDP}, EP_BULK,
              {1'b0, MAXPKT}, 8'd0, 8'd0,
        // ENDPOINT 81h — bulk IN, the bytes the host reads. bInterval is
        // ignored for a full-speed bulk endpoint (USB 2.0 Table 9-13).
        8'd7, DESC_ENDPOINT, {4'h8, DATA_ENDP}, EP_BULK,
              {1'b0, MAXPKT}, 8'd0, 8'd0
    };

    // -----------------------------------------------------------------
    // The class requests.
    // -----------------------------------------------------------------
    wire [63:0] class_setup;
    wire        class_req;
    wire        class_claim;
    wire [6:0]  class_len;
    wire [6:0]  class_index;
    wire [7:0]  class_byte;
    wire [63:0] class_out;
    wire [6:0]  class_out_len;
    wire        class_out_valid;

    usb_cdc_req #(
        .COMM_IFACE (COMM_IFACE)
    ) u_req (
        .clk         (clk),
        .rst_n       (rst_n),
        .setup       (class_setup),
        .req         (class_req),
        .claim       (class_claim),
        .len         (class_len),
        .index       (class_index),
        .resp        (class_byte),
        .out_data    (class_out),
        .out_len     (class_out_len),
        .out_valid   (class_out_valid),
        .baud        (baud),
        .char_format (char_format),
        .parity      (parity),
        .data_bits   (data_bits),
        .dtr         (dtr),
        .rts         (rts)
    );

    // -----------------------------------------------------------------
    // SERIAL_STATE, into the notification endpoint.
    // -----------------------------------------------------------------
    // Ten bytes handed to `usb_bulk_ep`'s IN side one a cycle, with
    // `notif_commit` on the last of them so that a ten-byte packet goes out of
    // a sixteen-byte endpoint instead of waiting for six bytes that are not
    // coming. "THE NOTIFICATION ENDPOINT" above says what the ten bytes are
    // for and when they go.
    //
    // `nidx` counts to nine, so four bits, and **every one of those four bits
    // is reachable**: bit 3 is set for 8 and 9. That is the test this library
    // applies to a register's width — not whether the top value is a power of
    // two, but whether an expression can set each bit — and it is why a
    // ten-state counter in four bits is not the hazard a three-bit `stage` for
    // four states was.
    reg [3:0] nidx;      // the byte being handed over
    reg       sending;   // a notification is on its way into the endpoint
    reg [6:0] reported;  // the state the last one carried
    reg       ever;      // one has been sent since the host configured this

    // The byte at `nidx`. PSTN 1.2 §6.5 for the eight-byte
    // notification header and §6.5.4 Table 31 for the two bytes of
    // `wSerialState`; USB 2.0 Table 9-2 for the shape of the header itself,
    // which is a SETUP packet's.
    function [7:0] notif_byte;
        input [3:0] i;
        begin
            case (i)
                // bmRequestType: device to host, class, to an interface.
                4'd0:    notif_byte = 8'hA1;
                // bNotification: SERIAL_STATE, PSTN 1.2 §6.5.4.
                4'd1:    notif_byte = 8'h20;
                // wValue: zero for this notification.
                4'd2:    notif_byte = 8'h00;
                4'd3:    notif_byte = 8'h00;
                // wIndex: the interface the notification is about, which is
                // the communications one — the same interface the class
                // requests are addressed to.
                4'd4:    notif_byte = COMM_IFACE;
                4'd5:    notif_byte = 8'h00;
                // wLength: the two bytes of data that follow.
                4'd6:    notif_byte = 8'd2;
                4'd7:    notif_byte = 8'h00;
                // wSerialState, low byte first. The seven bits the port
                // carries, then the nine the specification reserves.
                4'd8:    notif_byte = {1'b0, serial_state};
                default: notif_byte = 8'h00;
            endcase
        end
    endfunction

    wire [7:0] notif_data   = notif_byte(nidx);
    wire       notif_valid  = sending;
    wire       notif_commit = sending & (nidx == NOTIF_LAST);
    wire       notif_ready;

    // A notification is owed when the host has configured the device and
    // either none has gone yet or what one would say has changed. The device
    // is not configured until SET_CONFIGURATION, and sending before that would
    // be a packet on an endpoint the host has not enabled.
    wire       notif_owed = configured & (~ever | (serial_state != reported));

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            nidx     <= 4'd0;
            sending  <= 1'b0;
            reported <= 7'd0;
            ever     <= 1'b0;
        end else begin
            if (sending) begin
                if (notif_ready) begin
                    if (notif_commit) begin
                        // The last byte has been taken and the packet is
                        // armed. What it said is what the host will be told.
                        sending  <= 1'b0;
                        nidx     <= 4'd0;
                        ever     <= 1'b1;
                        reported <= serial_state;
                    end else begin
                        nidx <= nidx + 4'd1;
                    end
                end
            end else if (notif_owed) begin
                sending <= 1'b1;
                nidx    <= 4'd0;
            end

            // A host that has configured the device again, or reset the bus,
            // has forgotten what it was told: `usb_bulk_ep`'s buffer went with
            // it and so must the memory of having filled it.
            //
            // `sending` goes with them, and that is not belt and braces: a bus
            // reset in the ten cycles a notification takes to hand over would
            // otherwise leave this counting through the rest of a packet whose
            // buffer the endpoint had already cleared, and the host would get
            // the tail of a notification with no head. Clearing it here means
            // the next `notif_owed` starts one from byte zero.
            if (!configured) begin
                ever    <= 1'b0;
                sending <= 1'b0;
                nidx    <= 4'd0;
            end
        end
    end

    // -----------------------------------------------------------------
    // The device.
    // -----------------------------------------------------------------
    // `DEV_CLASS` is `02h` because a CDC device says its class in the
    // **device** descriptor as well as in the interface one (CDC 1.1 Table
    // 14), which is what tells a host the two interfaces belong together
    // before it has read the union descriptor.
    //
    // `CLASS_MAX` is seven, the length of the line coding structure, and it
    // is what sets the width of endpoint 0's data-stage counters.
    usb_dev_core #(
        .VID          (VID),
        .PID          (PID),
        .DEV_CLASS    (CLASS_COMM),
        .DEV_SUBCLASS (8'h00),
        .DEV_PROTOCOL (8'h00),
        .CFG_ATTR     (CFG_ATTR),
        .CFG_POWER    (CFG_POWER),
        .IFACE_BYTES  (IFACE_BYTES),
        .IFACE_DESC   (IFACE_DESC),
        .DATA_ENDP    (DATA_ENDP),
        .MAXPKT       (MAXPKT),
        .MAXPKT0      (MAXPKT0),
        .NOTIF_ENDP   (NOTIF_ENDP),
        .NOTIF_MAXPKT (NOTIF_MAXPKT),
        .CLASS_MAX    (7),
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
        .out_data     (out_data),
        .out_valid    (out_valid),
        .out_last     (out_last),
        .out_ready    (out_ready),
        .in_data      (in_data),
        .in_valid     (in_valid),
        .in_ready     (in_ready),
        .in_commit    (in_commit),
        .notif_data   (notif_data),
        .notif_valid  (notif_valid),
        .notif_ready  (notif_ready),
        .notif_commit (notif_commit)
    );
endmodule
