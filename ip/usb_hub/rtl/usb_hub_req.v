// usb_hub_req — the hub and port class requests a USB 2.0 hub answers, and
// the port state they act on, on `usb_ctrl_ep`'s class hook.
//
// What it does
//   Endpoint 0 in `ip/usb_device_fs` answers the standard requests and
//   offers everything else to whatever is above it. This is what a hub puts
//   there: the requests a host's own hub driver sends, the one **standard**
//   request endpoint 0 does not implement and a hub cannot do without, and
//   the state those requests read and write.
//
//   Nine claims, and every one of them is a subsection of USB 2.0 §11.24.2:
//
//     GetHubDescriptor      A0h 06h  wValue 2900h     §11.24.2.5
//     GetHubStatus          A0h 00h  wIndex 0         §11.24.2.6
//     GetPortStatus         A3h 00h  wIndex the port  §11.24.2.7
//     ClearHubFeature       20h 01h                   §11.24.2.1
//     SetPortFeature        23h 03h                   §11.24.2.12
//     ClearPortFeature      23h 01h                   §11.24.2.2
//
//   `20h` is host to device, class, to the **device**; `A0h` is the same the
//   other way. `23h` and `A3h` are the same two with the recipient
//   `11b` — "other", which for a hub means a **port**, and the port is
//   `wIndex`'s low byte, numbered from one. USB 2.0 Table 9-2 is where those
//   bits are and §11.24.2 is where the hub's use of the recipient field is.
//
//   A request to a port this hub does not have is **not claimed**, so
//   endpoint 0 stalls it, which is what §11.24.2 asks of a hub asked about a
//   port number greater than `bNbrPorts`.
//
// THE STANDARD REQUEST A HUB CANNOT DO WITHOUT, AND WHY IT IS HERE
//   `usb_ctrl_ep` implements five standard requests and offers the rest to
//   the class — its own header says so, and names GET_STATUS as one of the
//   ones a class may have "without this file changing again". A hub is the
//   first block in this library that takes it up:
//
//     GET_STATUS, device    80h 00h  wValue 0, wIndex 0, wLength 2
//
//   because **Linux's hub driver sends it during hub probe** and treats a
//   failure as fatal: `hub_configure` reads the device's standard status with
//   the comment "power budgeting mostly matters with bus-powered hubs" and
//   goes to its failure path with `can't get hub status` if the transfer does
//   not complete. A device that stalls it is not a hub as far as that driver
//   is concerned, however good its class requests are.
//
//   USB 2.0 §9.4.5 and Table 9-4 say what the two bytes are: bit 0 of
//   `wStatus` is **Self Powered** and bit 1 is **Remote Wakeup Enabled**,
//   and bits 2 to 15 are reserved and zero. This block reports **both bits
//   clear** — bus powered, remote wake-up not enabled — which agrees with the
//   `bmAttributes` of the configuration descriptor `usb_hub` writes and with
//   there being no SET_FEATURE(DEVICE_REMOTE_WAKEUP) anywhere in this
//   library.
//
//   **It does not belong in a class block and it is here anyway.** The right
//   home for a standard request is endpoint 0, and moving it there is a
//   change to `usb_ctrl_ep` that every device in the library would then make
//   — which is a better change than this one and is reported rather than
//   made, because it widens `std_req` on the two cores that already run on
//   silicon and this round has no measurement that would catch a regression
//   in them. README.md §7 says it again where a reader will find it.
//
// WHICH FEATURES ARE HONOURED, WHICH ARE ACCEPTED AND WHICH ARE STALLED
//   A hub's features are USB 2.0 §11.24.2's feature selectors, and a block
//   that claimed all of them would be lying about most. Three categories,
//   and README.md §4 is the same table with the argument for each row:
//
//   **Honoured** — the request changes something a later GetPortStatus
//   reports:
//
//     SetPortFeature(PORT_POWER)        the port's own power bit
//     ClearPortFeature(PORT_POWER)      ... and off again
//     SetPortFeature(PORT_RESET)        completes at once; see below
//     SetPortFeature(PORT_SUSPEND)      the suspend bit
//     ClearPortFeature(PORT_SUSPEND)    ... and the resume that follows it
//     ClearPortFeature(PORT_ENABLE)     the enable bit
//     ClearPortFeature(C_PORT_CONNECTION)
//     ClearPortFeature(C_PORT_SUSPEND)
//     ClearPortFeature(C_PORT_RESET)
//
//   **Accepted and ignored** — the request is claimed and acknowledged and
//   nothing changes, because the bit it names is a constant zero here and a
//   host clearing a zero has got what it asked for:
//
//     ClearHubFeature(C_HUB_LOCAL_POWER)     this hub has no local supply
//     ClearHubFeature(C_HUB_OVER_CURRENT)    ... and no current sensor
//     ClearPortFeature(C_PORT_ENABLE)        nothing here disables a port
//                                            on an error, so §11.24.2.7.2's
//                                            bit 1 never sets
//     ClearPortFeature(C_PORT_OVER_CURRENT)  as above
//
//   **Stalled**, by not being claimed at all:
//
//     SetHubFeature                 §11.24.2.11 — a hub need not implement it
//     SetHubDescriptor              §11.24.2.10 — likewise
//     GetBusState                   §11.24.2.4 — optional, for debugging
//     ClearTTBuffer, ResetTT,       §11.24.2.3, .9, .8, .13 — a transaction
//       GetTTState, StopTT            translator, which a full-speed hub has
//                                     none of
//     SetPortFeature(PORT_TEST)      no test modes
//     SetPortFeature(PORT_INDICATOR) D7 of wHubCharacteristics is clear, so
//                                    this hub never offered indicators
//     SetPortFeature of anything else, ClearPortFeature of anything else,
//     and any of the above aimed at a port number this hub does not have
//
//   The pattern is `ip/usb_cdc_acm`'s and is deliberate: what a block does
//   **not** implement is said in a descriptor field a host reads — there
//   `bmCapabilities`, here `wHubCharacteristics` — so a host that read the
//   descriptor never sends the request, and one that sends it anyway gets the
//   STALL that says the device never offered it.
//
// THE RESET THAT TAKES NO TIME, AND WHY THAT IS HONEST HERE
//   §11.5.1's **Resetting** state has a hub drive SE0 downstream for 10 to
//   20 ms when a host sets PORT_RESET, and report PORT_RESET set while it
//   does. **This block drives nothing downstream at all**, because what is
//   downstream of it is not a port of this hub yet: it is a second USB
//   controller — `ip/usb_host_ulpi` — with its own bus, its own reset and its
//   own enumeration, and joining the two is the transaction proxy this round
//   deliberately does not build.
//
//   So the reset completes in the cycle it is asked for: `port_reset` is one
//   cycle for whatever wants to know, C_PORT_RESET sets, the port becomes
//   enabled if something is connected, and **PORT_RESET in wPortStatus is a
//   constant zero** — a bit no expression in this file can set, which is why
//   there is no register for it. A host therefore sees the reset already
//   finished at its first GetPortStatus, enumerates the port, and gets
//   nothing back from the device it believes is there. That is the correct
//   outcome for a hub with no proxy behind it and README.md §8 is the kernel
//   log of it happening.
//
// WHY THE CHANGE BITS ARE STICKY STATE AND NOT A ONE-SHOT
//   The thing a hub reports on its status-change endpoint is **a set of bits
//   a host must clear**, and this file holds exactly that: a change bit is
//   set when something changes and cleared only by
//   ClearPortFeature(C_PORT_*). `usb_hub` sends the bitmap for as long as any
//   of them is set and NAKs when none is.
//
//   That shape is chosen against the one `ip/usb_cdc_acm` got wrong. Its
//   notification endpoint had a latch meaning "the host has been told", the
//   latch was set once per configuration, and a host that was not listening
//   at that moment — or a driver bound a second time without a bus reset —
//   never heard again; `ip/usb_cdc_acm/README.md` §4 has the measurement that
//   condemned it. **There is no such latch here.** Nothing in this file
//   records having reported anything, so there is nothing that can be wrong
//   about whether it has. The host's own ClearPortFeature is the
//   acknowledgement, which is what USB 2.0 asks for anyway.
//
//   One consequence is in the open: a host that has read the bitmap and not
//   yet cleared the change is sent the same bitmap again. `usb_hub`'s header
//   says what that costs and why the alternative is refused.
//
// WHAT A CHANGE IS A CHANGE OF
//   `C_PORT_CONNECTION` is set when **`connection` changes**, and
//   `connection` is `port_power & port_attached` — not `port_attached` alone.
//   §11.5.1's **Powered-off** state makes a connection status meaningless
//   while the port is
//   powered off, and gating it this way is also what makes the first
//   connection reportable: a host configures the hub, the hub's ports come up
//   powered off, the host sends SetPortFeature(PORT_POWER), and
//   **that** is when `connection` rises and the change is recorded — with the
//   host listening, because it has just finished configuring the hub.
//
//   A device already plugged into the downstream port before the host ever
//   looked therefore produces a connection change at exactly the right
//   moment, with no edge detector on `port_attached` and no one-shot at
//   configuration. The `!configured` arm below is the other half of it: a
//   hub that is not configured has no powered ports and no outstanding
//   changes, so a bus reset and a second enumeration go round the same path
//   as the first.
//
// What it does not do
//   **One port.** `bNbrPorts` is 1 and is a localparam and not a parameter,
//   because a parameter with one legal value is a lie about what has been
//   built: a second port is a second copy of every register below, a wider
//   `wIndex` comparison, a second bit of the change bitmap and two more bytes
//   of hub descriptor whose lengths §11.23.2.1 makes depend on the count.
//   None of that is hard and none of it is here.
//
//   No OUT data stage. Every request this block claims either reads or
//   carries nothing, so `usb_ctrl_ep`'s `class_out` ports are not taken at
//   all — `usb_cdc_acm`'s SET_LINE_CODING is the only request in this library
//   that has one. A SetPortFeature that arrived with a payload is not claimed
//   and is stalled, which is why `wLength` is compared against zero in the
//   decode rather than ignored.
//
//   Nothing downstream. No SE0, no suspend signalling, no frame forwarding,
//   no packet repeating: this is the hub's **control endpoint half**, and the
//   port's state is reported from what `port_attached` and `port_low_speed`
//   say rather than caused by anything here.
//
//   No over-current detection, no local power supply, no port indicators and
//   no transaction translator, each of which is a bit in
//   `wHubCharacteristics` or a request above that says so.
module usb_hub_req (
    input  wire        clk,
    input  wire        rst_n,

    // `usb_ctrl_ep`'s class hook. The three outputs are read in the one
    // cycle `req` is high and must be combinational in `setup`; that module's
    // header states the contract and why it is combinational.
    input  wire [63:0] setup,
    input  wire        req,
    output wire        claim,
    output wire [6:0]  len,
    input  wire [6:0]  index,
    output wire [7:0]  resp,

    // Endpoint 0's own `configured`. A hub that has not been configured has
    // no powered ports and no outstanding changes; "WHAT A CHANGE IS A
    // CHANGE OF" above says why that is the reset path and not an extra one.
    input  wire        configured,

    // WHAT IS ACTUALLY ON THE DOWNSTREAM PORT
    //
    // `port_attached` is a device present and debounced — on the design this
    // block was written for it is `ip/usb_host_ulpi`'s own `attached`, which
    // is that block's debounced sight of the pair leaving SE0 — and
    // `port_low_speed` is which line it pulled up: a full-speed device pulls
    // D+ up and a low-speed one D-, so the second is only meaningful while
    // the first is high.
    input  wire        port_attached,
    input  wire        port_low_speed,

    // WHAT THE HOST HAS ASKED FOR, FOR WHATEVER IS DOWNSTREAM
    //
    // The port's state as a host has set it, brought out so that the half of
    // a hub that is not in this file can act on it. Nothing in this block
    // uses them except to answer GetPortStatus.
    output wire        port_power,
    output wire        port_enabled,
    output wire        port_suspended,
    // One cycle: the host set PORT_RESET. "THE RESET THAT TAKES NO TIME"
    // above says what this block does about it, which is nothing but say so.
    output reg         port_reset,

    // THE HUB AND PORT STATUS CHANGE BITMAP, USB 2.0 §11.12.4
    //
    // One bit per port and bit 0 for the hub itself, which for a one-port hub
    // is one byte. `usb_hub` is what puts it on the status-change endpoint and
    // its header says when.
    //
    // **Bit 0 is a constant zero.** The only two things §11.24.2.6 puts in
    // `wHubChange` are a local power supply this bus-powered hub does not
    // have and an over-current detector it does not have either, so no
    // expression anywhere can set it; a register for it would be a flip-flop
    // with a constant data input, which is the shape this family has cost
    // this project eight rounds of investigation over.
    output wire [7:0]  change_map
);
    // -----------------------------------------------------------------
    // The SETUP packet, byte 0 in the low eight bits (USB 2.0 Table 9-2).
    // -----------------------------------------------------------------
    wire [7:0]  bm_request_type = setup[7:0];
    wire [7:0]  b_request       = setup[15:8];
    wire [15:0] w_value         = setup[31:16];
    wire [15:0] w_index         = setup[47:32];
    wire [15:0] w_length        = setup[63:48];

    // bmRequestType. Bits 6:5 are `01` — class — and bits 4:0 are the
    // recipient: `00000` the device and `00011` "other", which §11.24.2 makes
    // a port. Bit 7 is the direction.
    localparam [7:0] TYPE_HUB_OUT  = 8'h20;
    localparam [7:0] TYPE_HUB_IN   = 8'hA0;
    localparam [7:0] TYPE_PORT_OUT = 8'h23;
    localparam [7:0] TYPE_PORT_IN  = 8'hA3;
    // And the one standard request this block claims: device to host,
    // standard, to the device.
    localparam [7:0] TYPE_STD_IN   = 8'h80;

    // bRequest. A hub reuses the standard request codes of USB 2.0 Table 9-4
    // with a class recipient rather than defining its own, which §11.24.2 is
    // the table of.
    localparam [7:0] REQ_GET_STATUS     = 8'h00;
    localparam [7:0] REQ_CLEAR_FEATURE  = 8'h01;
    localparam [7:0] REQ_SET_FEATURE    = 8'h03;
    localparam [7:0] REQ_GET_DESCRIPTOR = 8'h06;

    // The hub descriptor's own type, USB 2.0 §11.23.2.1. GetHubDescriptor
    // puts it in the **high** byte of `wValue` and the descriptor index in
    // the low one, exactly as a standard GET_DESCRIPTOR does.
    localparam [7:0] DESC_HUB = 8'h29;

    // Feature selectors, USB 2.0 §11.24.2. The hub's two:
    localparam [15:0] FEAT_C_HUB_LOCAL_POWER  = 16'd0;
    localparam [15:0] FEAT_C_HUB_OVER_CURRENT = 16'd1;
    // ... and a port's. The numbering is the specification's and the gaps in
    // it are the specification's too.
    localparam [15:0] FEAT_PORT_ENABLE          = 16'd1;
    localparam [15:0] FEAT_PORT_SUSPEND         = 16'd2;
    localparam [15:0] FEAT_PORT_RESET           = 16'd4;
    localparam [15:0] FEAT_PORT_POWER           = 16'd8;
    localparam [15:0] FEAT_C_PORT_CONNECTION    = 16'd16;
    localparam [15:0] FEAT_C_PORT_ENABLE        = 16'd17;
    localparam [15:0] FEAT_C_PORT_SUSPEND       = 16'd18;
    localparam [15:0] FEAT_C_PORT_OVER_CURRENT  = 16'd19;
    localparam [15:0] FEAT_C_PORT_RESET         = 16'd20;

    // THE HUB DESCRIPTOR, USB 2.0 §11.23.2.1
    //
    // Nine bytes for a one-port hub, and the nine are **not nine by
    // coincidence**: `bDescLength` is 7 plus the two variable-length fields,
    // each of which is one bit per port plus one for the reserved bit 0,
    // rounded up to a byte. One port is one byte each, so 7 + 1 + 1 = 9.
    // README.md §3 has the field table with what each value claims.
    //
    // A host that disagrees with a hub about this length does not get a hub:
    // Linux asks for the whole of its own 15-byte structure and requires at
    // least `7 + 2` bytes back, and the short data stage `usb_ctrl_ep` sends
    // when `wLength` exceeds the nine offered is what makes nine the answer.
    localparam [7:0] NBR_PORTS = 8'd1;
    localparam [7:0] HUB_DESC_LEN = 8'd9;
    // wHubCharacteristics, §11.23.2.1, low byte first:
    //
    //   D1:D0  Logical Power Switching Mode    01 — individual port power
    //   D2     part of a compound device        0 — it is not
    //   D4:D3  Over-current Protection Mode    10 — none at all
    //   D6:D5  TT Think Time                   00 — no transaction translator
    //   D7     Port Indicators Supported        0 — none
    //
    // So `0011h`, and two of those five are claims worth being careful about.
    //
    // **Individual port power switching**, and not "no power switching",
    // because a host that reads no power switching never sends
    // SetPortFeature(PORT_POWER) and then nothing ever turns the port on.
    // Linux's `hub_power_on` sends it to every port of a hub whose mode is 00
    // or 01 and to none of a hub whose mode is 1X, so this bit pair is what
    // decides whether the port in this block is ever powered at all. What is
    // switched is **this hub's own idea of the port**, not a supply:
    // README.md §4 says what does power the socket and why a host is not
    // given a say in it.
    //
    // **No over-current protection** is the honest value and §11.23.2.1
    // allows it only for a bus-powered hub that does not implement any, which
    // is what the configuration descriptor's `bmAttributes` says this is.
    localparam [7:0] HUB_CHAR_LO = 8'h11;
    localparam [7:0] HUB_CHAR_HI = 8'h00;
    // bPwrOn2PwrGood, in 2 ms units: 100 ms. Nothing here sequences a supply,
    // so a smaller number would be true and would buy nothing — a host waits
    // this once, at enumeration — and understating it is the only way this
    // field can cause a fault, since a host that looks before a real supply
    // was good sees a port with nothing on it.
    localparam [7:0] PWR_ON_2_PWR_GOOD = 8'd50;
    // bHubContrCurrent, in mA: what the hub controller itself draws, which is
    // the same 100 mA the configuration descriptor's `bMaxPower` declares.
    localparam [7:0] HUB_CONTR_CURRENT = 8'd100;
    // DeviceRemovable, one byte for one port: bit 0 is reserved and bit n is
    // port n, `0` removable and `1` non-removable. Whatever is in the
    // downstream socket can be unplugged, so the byte is zero.
    localparam [7:0] DEVICE_REMOVABLE = 8'h00;
    // PortPwrCtrlMask, the same size. §11.23.2.1: "This field exists for
    // reasons of compatibility with software written for 1.0 compliant
    // devices. All bits in this field should be set to 1B."
    localparam [7:0] PORT_PWR_CTRL_MASK = 8'hFF;

    // Bytes in the data stage of each request that reads.
    localparam [6:0] LEN_HUB_DESC   = 7'd9;
    localparam [6:0] LEN_STATUS     = 7'd4;  // two 16-bit fields
    localparam [6:0] LEN_STD_STATUS = 7'd2;  // one

    // -----------------------------------------------------------------
    // Which request this is. Pure combinational logic over eight bytes,
    // because that is what `usb_ctrl_ep` reads in the cycle `req` is high.
    // -----------------------------------------------------------------
    // The one port this hub has. `wIndex`'s low byte is the port number and
    // its high byte is a selector that is reserved and zero for every feature
    // this block claims, so the whole word is compared: a request naming port
    // 2, or port 0, or carrying a selector, is not this hub's.
    wire to_port = (w_index == {8'h00, NBR_PORTS});
    wire to_hub  = (w_index == 16'h0000);

    wire get_hub_desc = (bm_request_type == TYPE_HUB_IN)
                      & (b_request == REQ_GET_DESCRIPTOR)
                      & (w_value == {DESC_HUB, 8'h00})
                      & to_hub;
    wire get_hub_stat = (bm_request_type == TYPE_HUB_IN)
                      & (b_request == REQ_GET_STATUS)
                      & (w_value == 16'h0000)
                      & to_hub;
    wire get_port_stat = (bm_request_type == TYPE_PORT_IN)
                       & (b_request == REQ_GET_STATUS)
                       & (w_value == 16'h0000)
                       & to_port;
    // The standard one. "THE STANDARD REQUEST A HUB CANNOT DO WITHOUT" above
    // is why a class block claims it; `wValue` is the status type, which USB
    // 2.0 §9.4.5 makes zero for the standard two bytes.
    wire get_std_stat = (bm_request_type == TYPE_STD_IN)
                      & (b_request == REQ_GET_STATUS)
                      & (w_value == 16'h0000)
                      & to_hub;

    // The hub's two change bits, which are constant zeros here, so clearing
    // one is accepted and does nothing.
    wire clr_hub_feat = (bm_request_type == TYPE_HUB_OUT)
                      & (b_request == REQ_CLEAR_FEATURE)
                      & (w_length == 16'd0)
                      & to_hub
                      & ((w_value == FEAT_C_HUB_LOCAL_POWER)
                       | (w_value == FEAT_C_HUB_OVER_CURRENT));

    // A port feature request at all: the right type, the right port, and no
    // data stage. Which feature it names is below, because the set of
    // features that may be set is not the set that may be cleared.
    wire port_set_req = (bm_request_type == TYPE_PORT_OUT)
                      & (b_request == REQ_SET_FEATURE)
                      & (w_length == 16'd0)
                      & to_port;
    wire port_clr_req = (bm_request_type == TYPE_PORT_OUT)
                      & (b_request == REQ_CLEAR_FEATURE)
                      & (w_length == 16'd0)
                      & to_port;

    // SetPortFeature: three features, and PORT_TEST and PORT_INDICATOR
    // deliberately not among them.
    wire set_power   = port_set_req & (w_value == FEAT_PORT_POWER);
    wire set_reset   = port_set_req & (w_value == FEAT_PORT_RESET);
    wire set_suspend = port_set_req & (w_value == FEAT_PORT_SUSPEND);

    // ClearPortFeature: three pieces of state and five change bits, two of
    // which are constant zeros and are accepted anyway.
    wire clr_power    = port_clr_req & (w_value == FEAT_PORT_POWER);
    wire clr_enable   = port_clr_req & (w_value == FEAT_PORT_ENABLE);
    wire clr_suspend  = port_clr_req & (w_value == FEAT_PORT_SUSPEND);
    wire clr_c_conn   = port_clr_req & (w_value == FEAT_C_PORT_CONNECTION);
    wire clr_c_susp   = port_clr_req & (w_value == FEAT_C_PORT_SUSPEND);
    wire clr_c_reset  = port_clr_req & (w_value == FEAT_C_PORT_RESET);
    // The two that name a bit nothing can set. Claimed so that a host
    // clearing them is answered rather than stalled, which §11.24.2.2 asks
    // for: the feature exists, it is simply already clear.
    wire clr_c_nil    = port_clr_req & ((w_value == FEAT_C_PORT_ENABLE)
                                      | (w_value == FEAT_C_PORT_OVER_CURRENT));

    wire set_any = set_power | set_reset | set_suspend;
    wire clr_any = clr_power | clr_enable | clr_suspend
                 | clr_c_conn | clr_c_susp | clr_c_reset | clr_c_nil;

    assign claim = get_hub_desc | get_hub_stat | get_port_stat | get_std_stat
                 | clr_hub_feat | set_any | clr_any;

    // How long the data stage is. A request that writes has none, and
    // `usb_ctrl_ep` does not read this for one, but a `?:` chain whose last
    // arm is a length nothing asks for would be a number with no reason, so
    // it is zero.
    assign len = get_hub_desc ? LEN_HUB_DESC
               : get_std_stat ? LEN_STD_STATUS
               : (get_hub_stat | get_port_stat) ? LEN_STATUS
               : 7'd0;

    // -----------------------------------------------------------------
    // The port, and what a host has made of it.
    // -----------------------------------------------------------------
    reg powered;    // PORT_POWER
    reg enabled;    // PORT_ENABLE
    reg suspended;  // PORT_SUSPEND
    reg c_conn;     // C_PORT_CONNECTION
    reg c_susp;     // C_PORT_SUSPEND
    reg c_reset;    // C_PORT_RESET
    reg conn_q;     // what `connection` was a cycle ago

    // PORT_CONNECTION. §11.5.1's Powered-off state makes a connection
    // meaningless while the port is powered off, and "WHAT A CHANGE IS A
    // CHANGE OF" above is why gating it here is what makes the first
    // connection reportable.
    wire connection = powered & port_attached;

    assign port_power     = powered;
    assign port_enabled   = enabled;
    assign port_suspended = suspended;

    // -----------------------------------------------------------------
    // The bytes each read sends.
    // -----------------------------------------------------------------
    // WHICH READ IS IN PROGRESS, AND WHY IT IS LATCHED
    //
    // `len` is combinational in `setup` because `usb_ctrl_ep` reads it in the
    // cycle the SETUP's data packet ends, which is the contract. `resp` is
    // read **later**, once per byte of the data stage, and what `setup` holds
    // by then is `usb_pkt_rx`'s last data packet — which today is still the
    // SETUP, because a control read has no host-to-device packet until its
    // status stage. Depending on that would be depending on something nothing
    // promises, so which read is in progress is latched instead: two
    // flip-flops against a decode that could go stale.
    reg sel_desc;  // the read in progress is GetHubDescriptor
    reg sel_port;  // ... is GetPortStatus; neither is a status word of zeros

    // The hub descriptor, a byte at a time. The index needs four bits for
    // nine bytes, and `usb_ctrl_ep` fetches up to one byte past the stage, so
    // the `case` is full and its default is zero.
    reg [7:0] desc_byte;
    always @(*) begin
        case (index[3:0])
            4'd0:    desc_byte = HUB_DESC_LEN;
            4'd1:    desc_byte = DESC_HUB;
            4'd2:    desc_byte = NBR_PORTS;
            4'd3:    desc_byte = HUB_CHAR_LO;
            4'd4:    desc_byte = HUB_CHAR_HI;
            4'd5:    desc_byte = PWR_ON_2_PWR_GOOD;
            4'd6:    desc_byte = HUB_CONTR_CURRENT;
            4'd7:    desc_byte = DEVICE_REMOVABLE;
            4'd8:    desc_byte = PORT_PWR_CTRL_MASK;
            default: desc_byte = 8'h00;
        endcase
    end

    // wPortStatus and wPortChange, four bytes, each field low byte first.
    //
    // USB 2.0 §11.24.2.7.1 for the status bits and §11.24.2.7.2 for the
    // change bits. Every bit this hub cannot produce is written as a literal
    // zero with the name of what it would have been, because a bit left out
    // of a concatenation is a bit in the wrong place:
    //
    //   wPortStatus   0 PORT_CONNECTION   1 PORT_ENABLE    2 PORT_SUSPEND
    //                 3 PORT_OVER_CURRENT 4 PORT_RESET     8 PORT_POWER
    //                 9 PORT_LOW_SPEED   10 PORT_HIGH_SPEED
    //                11 PORT_TEST        12 PORT_INDICATOR
    //   wPortChange   0 C_PORT_CONNECTION 1 C_PORT_ENABLE  2 C_PORT_SUSPEND
    //                 3 C_PORT_OVER_CURRENT               4 C_PORT_RESET
    //
    // PORT_RESET is zero because a reset here is over in the cycle it is
    // asked for; PORT_HIGH_SPEED because this is a full-speed hub; the rest
    // because nothing in this file can detect or do them.
    //
    // PORT_LOW_SPEED is gated by `connection` for the same reason the
    // specification calls it the speed of the **attached** device: which line
    // is pulled up says nothing while nothing is pulling.
    wire [7:0] port_stat_lo = {3'b000,                  // 7:5 reserved
                               1'b0,                    // 4   PORT_RESET
                               1'b0,                    // 3   PORT_OVER_CURRENT
                               suspended,               // 2
                               enabled,                 // 1
                               connection};             // 0
    wire [7:0] port_stat_hi = {4'b0000,               // 15:12, 12 INDICATOR
                               1'b0,                  // 11  PORT_TEST
                               1'b0,                  // 10  PORT_HIGH_SPEED
                               connection & port_low_speed,  // 9
                               powered};              // 8
    wire [7:0] port_chg_lo  = {3'b000,                // 7:5 reserved
                               c_reset,               // 4
                               1'b0,                  // 3   C_PORT_OVER_CURRENT
                               c_susp,                // 2
                               1'b0,                  // 1   C_PORT_ENABLE
                               c_conn};               // 0

    // The four bytes of the port's status, and the four zeros that are both
    // the hub's status (§11.24.2.6 — no local supply, no over-current, and
    // therefore no changes of either) and the two zeros of the standard
    // device status (§9.4.5 — bus powered, no remote wake-up). One `case`
    // serves all three because what the other two want is zero everywhere,
    // which is a statement about this hub and is written down in the two
    // sections above rather than left as an accident of the multiplexer.
    reg [7:0] status_byte;
    always @(*) begin
        case (index[1:0])
            2'd0:    status_byte = sel_port ? port_stat_lo : 8'h00;
            2'd1:    status_byte = sel_port ? port_stat_hi : 8'h00;
            2'd2:    status_byte = sel_port ? port_chg_lo  : 8'h00;
            default: status_byte = 8'h00;
        endcase
    end

    assign resp = sel_desc ? desc_byte : status_byte;

    // The bitmap, one bit a port with bit 0 for the hub. Bit 0 is a constant
    // zero and the port's comment says why.
    wire port_change = c_conn | c_susp | c_reset;
    assign change_map = {6'b000000, port_change, 1'b0};

    // -----------------------------------------------------------------
    // The state machine, which is one cycle long.
    // -----------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            powered    <= 1'b0;
            enabled    <= 1'b0;
            suspended  <= 1'b0;
            c_conn     <= 1'b0;
            c_susp     <= 1'b0;
            c_reset    <= 1'b0;
            conn_q     <= 1'b0;
            port_reset <= 1'b0;
            sel_desc   <= 1'b0;
            sel_port   <= 1'b0;
        end else begin
            port_reset <= 1'b0;

            // A port with nothing on it is not enabled, whatever it was told:
            // §11.5.1 takes a port out of **Enabled** on a disconnect, and a
            // host that read PORT_ENABLE set on an empty port would address a
            // device that is not there.
            if (!connection) enabled <= 1'b0;

            if (req && claim) begin
                // Which read is in progress, for `resp` above. Latched on
                // every claim, including the writes, so that a write cannot
                // leave a stale selector behind for the next read.
                sel_desc <= get_hub_desc;
                sel_port <= get_port_stat;

                // SetPortFeature. None of these has a data stage, so the
                // request is the whole of it and `wValue` in this cycle is
                // what it asked for.
                if (set_power)   powered   <= 1'b1;
                if (set_suspend) suspended <= 1'b1;
                if (set_reset) begin
                    // "THE RESET THAT TAKES NO TIME" above. Nothing is driven
                    // downstream, so the reset is finished: the port becomes
                    // enabled if something is connected and C_PORT_RESET says
                    // the reset completed.
                    port_reset <= 1'b1;
                    c_reset    <= 1'b1;
                    enabled    <= connection;
                end

                // ClearPortFeature.
                if (clr_power) begin
                    // §11.5.1's Powered-off: no connection and
                    // no enable, and `connection` follows `powered` on its
                    // own. The enable does not, so it is cleared here.
                    powered <= 1'b0;
                    enabled <= 1'b0;
                end
                if (clr_enable) enabled <= 1'b0;
                if (clr_suspend) begin
                    // §11.5.1's Resuming: clearing PORT_SUSPEND is a resume,
                    // and C_PORT_SUSPEND is set when the resume is complete —
                    // which, with nothing downstream to resume, is now.
                    suspended <= 1'b0;
                    c_susp    <= 1'b1;
                end
                if (clr_c_susp)  c_susp  <= 1'b0;
                if (clr_c_reset) c_reset <= 1'b0;
                // `clr_c_conn` is below, after the arm that sets `c_conn`.
                // `clr_c_nil` and `clr_hub_feat` name bits that are constant
                // zeros, so they are claimed and nothing happens.
            end

            // A HOST CLEARING A CHANGE MUST NOT LOSE ONE THAT HAPPENED
            //
            // Verilog's last assignment wins, so the clear is written first
            // and the set second: a connection that changes in the same cycle
            // as the ClearPortFeature that was meant for the previous change
            // leaves the bit **set**, and the host is told again. The other
            // way round the change is gone and nothing will ever mention it.
            if (req && claim && clr_c_conn) c_conn <= 1'b0;
            conn_q <= connection;
            if (connection != conn_q) c_conn <= 1'b1;

            // A hub that is not configured has no powered ports and nothing
            // outstanding. `configured` goes low on a bus reset and on
            // SET_CONFIGURATION 0, so this is the whole of both: the next
            // SetPortFeature(PORT_POWER) raises `connection` again and the
            // change the host needs is produced then, with the host
            // listening. `ip/usb_cdc_acm`'s notification endpoint needed an
            // extra trigger for exactly the case this covers for free.
            if (!configured) begin
                powered   <= 1'b0;
                enabled   <= 1'b0;
                suspended <= 1'b0;
                c_conn    <= 1'b0;
                c_susp    <= 1'b0;
                c_reset   <= 1'b0;
                conn_q    <= 1'b0;
            end
        end
    end
endmodule
