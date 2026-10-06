// A USB **proxy** on a Cynthion: the hub the PC binds to on the AUX port, the
// device it reaches *through* that hub in the TARGET-A socket, and the PC's own
// transactions carried from one bus to the other.
//
// This is `usb_hub_target.v` with the thing that was missing from it put in.
// There, `ip/usb_hub` answered the PC's class requests about its port and
// `ip/usb_host_ulpi` enumerated whatever was on TARGET-A **for itself**, and no
// packet crossed between the two buses — so the PC found a hub, found a device
// on its port, reset it, and got `device descriptor read/64, error -71`, which
// was the correct outcome of a hub that forwards nothing.
// `ip/usb_hub/README.md` §8 quotes that log.
//
// `ip/usb_proxy` is what closes it, and the whole of this design is that block
// with a board around it.
//
// ===================================================================
// WHAT IS SUPPOSED TO HAPPEN, AND HOW TO TELL
// ===================================================================
//
//   * the PC's own hub driver binds — `hub 7-5:1.0: USB hub found`,
//     `1 port detected`;
//   * it powers the port, is told something is attached, and resets it;
//   * **this design resets the device too**, which is `ip/usb_proxy`'s
//     `usb_proxy_dn` writing `50h` and then `45h` into the TARGET
//     transceiver's Function Control register — 15 ms of SE0 and 20 ms of
//     recovery — and only then does the hub report the reset complete and the
//     port enabled;
//   * and then the PC enumerates the device **itself**, through the port, and
//     the device that appears in `lsusb` is the one in the socket with its own
//     VID and PID.
//
// Nothing of this design's own appears in that device's descriptors, because
// nothing of this design's own is in them: with pass-through addressing the
// bytes the PC reads are the bytes the device sent. `ip/usb_proxy/README.md`
// §2 is why that was the architecture chosen.
//
// ===================================================================
// THE SERIAL CONSOLE IS ON T14 HERE, AND WHY IT HAS TO BE
// ===================================================================
//
// `usb_host_target.v` reports itself over a CDC ACM serial port on **AUX**.
// This design cannot: AUX is the hub. So the console is the UART on ball
// **T14** that `usb_ulpi_trace.v` established, which Apollo bridges to
// `/dev/ttyACM0` and which is independent of AUX entirely. That file's header
// is the whole argument for it being safe and the rule it imposes, and both are
// repeated here because a design that drives T14 without them is two drivers on
// one wire:
//
//   * T14 is the FPGA's `uart.tx` in Great Scott Gadgets' platform file and
//     it is on the **same net** as the debug microcontroller's JTAG `TMS`
//     output, with no series resistor on the PCB;
//   * so the pad drives only inside a window — about 0.28 s to 17.9 s after
//     configuration — and is high impedance before and after. The window is a
//     counter and two latches and nothing else can hold it open;
//   * `PULLMODE=UP` in the constraints, as the platform file asks, so the
//     released line idles high;
//   * and Apollo hands the pin to its SERCOM for the console and takes it
//     back for JTAG.
//
// **So do not start a JTAG transaction until the window has shut**, which is
// about eighteen seconds after configuration.
//
// Two host-side things that each cost an hour once and are not obvious:
//
//   * open `/dev/ttyACM0` at **9600 first and then 115200**. Apollo only
//     re-initialises its SERCOM when the host *changes* the CDC line coding,
//     and it believes the UART is already active after every JTAG
//     transaction, so 115200 alone gets silence;
//   * the window's latches are written `q <= q | e`. `if (e) q <= 1'b1`
//     infers a clock enable, a slice's two flip-flops share one `CE` wire,
//     and the pad never drove at all.
//
// THE FORMAT
// ----------
// One line a second: `P`, eight hex digits, CRLF. The eight digits are four
// bytes, most significant nibble first:
//
//   byte 0  the hub, on AUX
//           [7] phy_ready          the AUX transceiver answered
//           [6] configured         SET_CONFIGURATION accepted
//           [5] addressed          SET_ADDRESS accepted
//           [4] saw_bus_reset      the PC has reset the AUX bus at least once
//           [3] port_power         the PC sent SetPortFeature(PORT_POWER)
//           [2] port_enabled       ... and the port reset finished, so it is on
//           [1] port_suspended     ... and PORT_SUSPEND
//           [0] port_reset         a port reset is in progress **now**
//
//   byte 1  the downstream port, on TARGET
//           [7] dn_phy_ready       the TARGET transceiver answered
//           [6] dn_attached        a device is on TARGET-A, debounced
//           [5] dn_low_speed       ... and it pulled D- up rather than D+
//           [4] saw_proxied        **the PC has addressed it through the hub**
//           [3] dn_reg_failed      a Function Control write the part refused
//           [2:1] dn_vbus_state    the transceiver's own comparators
//           [0] saw_data_fwd       **a bulk or interrupt transaction has been
//                                  forwarded**, which is the one thing that
//                                  tells an endpoint the proxy never reached
//                                  from one the device NAKed
//
//   byte 2  `1'b0`, `ctrl_active`, `job[1:0]`, `dn_stage[3:0]` — a control
//           transfer the relay holds a SETUP for, the relay's one job, and the
//           downstream port's state machine. `job` is 0 idle, 1 wanted, 2
//           running, 3 an answer the PC has not taken; `dn_stage` is 1 an empty
//           port, 4 a device the PC has not reset, 5 to 8 the reset, 10 a
//           device that has been reset and may be relayed to. `ctrl_active` is
//           high from the first SETUP until the port is reset and is **not** a
//           transfer in progress — nothing in the relay needs to know when a
//           transfer is over, so nothing tracks it.
//
//   byte 3  `1'b0`, `dn_line_state[1:0]`, `setups[4:0]` — the TARGET pair as
//           the transceiver last reported it, and a count of SETUP packets the
//           PC has sent to something behind the port, saturating at 31.
//
// **Byte 1 bits 4 and 0, and byte 3's low five bits, are the ones to look at.**
// `saw_proxied` says the PC addressed the device at all, which no previous
// design on this board could produce; `setups` climbing says it is enumerating
// it; `saw_data_fwd` says a transaction outside a control transfer went across.
// `saw_proxied` is `led5_n` as well, so it can be read with no console.
//
// **Why `saw_data_fwd` is a bit of its own.** A host turns a NAK for ever into a
// timeout, so a bulk endpoint the proxy never reached and one the device NAKed
// look identical from the host's side — `operation timed out` either way. This
// bit is the difference, and it is the only place that difference is visible.
//
// What the console **cannot** say is whether the enumeration succeeded. That is
// the PC's own kernel log and `lsusb`, and `ip/usb_proxy/README.md` §8 is where
// it is quoted. A count of SETUPs that climbs and stops is a PC that enumerated
// the device and went quiet, and one that sits at 2 is a PC retrying the same
// request.
//
// ===================================================================
// THE VBUS SWITCHES: EXACTLY ONE, AND IT HAS A PARAMETER
// ===================================================================
//
// Unchanged from `usb_hub_target.v`, whose header has the long form. The
// platform file's own comment is the hazard:
//
//     # VBUS on each of the Type-C ports can be connected to TARGET A
//     # through a bidirectional switch. If any of these switches is
//     # enabled, TARGET A is considered an output.
//
// Three of them reach that node and **this design drives all three**, because
// a pin a design does not mention comes up however the bitstream's defaults
// leave it and "exactly one switch" should be a property of the bitstream.
// `control_vbus_en` (L1) and `target_c_vbus_en` (K5) are driven low with no
// parameter to raise them; `aux_vbus_en` (L2) is `VBUS_AUX` and nothing else.
//
// AUX rather than CONTROL because CONTROL is the port the board's own supply
// and its Apollo debug microcontroller come in on, and a device that browns
// that out takes away the thing that loads bitstreams into this FPGA.
//
// **Nothing here switches that pin from a class request.** The PC's
// SetPortFeature(PORT_POWER) moves a bit inside `usb_hub` and does not reach
// L2, which is a deliberate refusal rather than an omission: the board's power
// topology is a property of the bitstream that was loaded, not of what a host
// asks for.
//
// **A device on TARGET-A needs `VBUS_AUX = 1`.** With the default of 0 the
// socket has no power, nothing attaches, and the hub correctly reports an
// empty port.
//
// **And a device that draws more than 100 mA needs `SELF_POWERED = 1`**, which
// is the default and whose parameter comment is the whole argument: a
// bus-powered hub may offer each port only 100 mA, a GreatFET's configuration
// declares 500, and Linux's answer is to enumerate the device perfectly and then
// leave it unconfigured — `rejected 1 configuration due to insufficient
// available bus power`. That is one of the two ways this design can look like it
// works and not work, and it was measured here rather than reasoned about.
//
// ===================================================================
// WHAT TO LOOK AT
// ===================================================================
//
// The LEDs, which need no console:
//
//   LED 0  the AUX transceiver answered — the hub's `phy_ready`
//   LED 1  the hub was configured by the PC
//   LED 2  something is on the TARGET pair
//   LED 3  heartbeat, so a dark board is a dead board
//   LED 4  the port is enabled — the PC reset it and the reset reached the
//          device
//   LED 5  **the PC addressed the device behind the port**
//
// LED 5 lit is this round's whole milestone, and the kernel log is the proof.
//
// ===================================================================
// ELEVEN SIGNALS THIS DESIGN DOES NOT ROUTE, AND WHICH ELEVEN
// ===================================================================
//
// `reticle fpga` reports `routed 4637 of 4648 signal(s)` for this design, and
// the difference is **`dn_frame[10:0]`**: the downstream bus's frame number,
// which `usb_hub_proxy_ulpi` brings out and this design leaves unconnected.
// `Netlist::is_routable` requires a signal to have both a driver and a sink, and
// a bus with eleven drivers and no sink has eleven signals with nothing to route
// to.
//
// It is written down because the arithmetic looks alarming and is not, which is
// the same note `ip/usb_host_ulpi/README.md` §9 makes about its own "4423 of
// 4425" — there the two were pads driven by constants. The line that carries the
// weight in either report is the last one: every set bit decodes back through
// the database and the arcs they select are exactly the router's.
module usb_proxy_target #(
    // How many clocks the cores are held in reset after configuration.
    parameter integer POR = 16,
    // Whether `aux_vbus_en` (L2) is driven high, passing the AUX Type-C port's
    // VBUS through to TARGET A. **Read the section above before changing
    // this.** 0 is the default and closes no switch at all.
    parameter integer VBUS_AUX = 0,
    // WHETHER THE HUB TELLS THE PC IT IS SELF POWERED, AND WHY IT SAYS YES
    //
    // Bit 6 of the configuration descriptor's `bmAttributes` (USB 2.0 §9.6.3).
    // It is **not** decoration and it is not the IP block's business: it decides
    // how much current Linux will let a device behind the port draw, and with
    // the wrong value the device enumerates perfectly and is then left
    // unconfigured.
    //
    // Measured on this board, with the GreatFET in the TARGET-A socket and this
    // bit **clear**:
    //
    //   usb 7-5.1: New USB device found, idVendor=1d50, idProduct=60e6
    //   usb 7-5.1: Product: GreatFET
    //   usb 7-5.1: rejected 1 configuration due to insufficient available bus power
    //   usb 7-5.1: no configuration chosen from 1 choice
    //
    // The GreatFET's one configuration declares `MaxPower 500mA`, and Linux
    // gives each port of a **bus-powered** hub 100 mA — which is all a
    // bus-powered hub has to give, since every milliamp it hands downstream
    // comes out of the allowance its own upstream cable granted it.
    //
    // **A Cynthion is not powered through AUX.** The board's own supply and its
    // Apollo debug microcontroller come in on the CONTROL port, so the hub
    // controller in this FPGA draws **nothing at all** from the AUX cable, and
    // "self powered" is the true statement about it. `CFG_POWER` is left at
    // 100 mA anyway, which is a self-powered device declaring that it may still
    // draw some from the cable: the conservative of the two readings.
    //
    // **And here is the inaccuracy, recorded rather than papered over.** The
    // current the *port* draws does come out of the AUX cable, through the
    // bidirectional switch `VBUS_AUX` closes — so a host told 500 mA a port is
    // being told something this board does not guarantee, and a strictly
    // compliant self-powered hub would supply its ports from its own rail
    // instead. What makes it safe in practice is that the switch is closed by
    // the bitstream and not by the host: whatever the socket draws, it drew
    // before the PC ever asked, and no class request can change it.
    // `ip/usb_hub/README.md` §4 is the long form of that refusal.
    parameter integer SELF_POWERED = 1,
    // Which `LineState` is a full-speed device's idle J, from the target
    // transceiver's point of view; `ip/usb_host_ulpi`'s `FS_LINE` says why this
    // is a parameter at all.
    parameter [1:0] FS_LINE = 2'b01,
    // Cycles of an idle bus at J before an answer goes out, which ULPI 1.1
    // Table 10 allows a full-speed Link between 7 and 18 of.
    parameter [6:0] TURNAROUND = 7'd9,
    // Clocks a bit on the T14 console: 60 MHz over 115200 is 520.8, and the
    // rounding is one part in five hundred against a receiver that samples in
    // the middle of a bit.
    parameter integer BAUD_DIV = 521,
    // Clocks between one line of the console and the next, as a count of ones
    // in a counter this wide: 2^26 at 60 MHz is about 1.1 seconds.
    parameter integer GAP_BITS = 26,
    // Which bit of the age counter opens the drive window on T14 and which
    // shuts it: 2^24 and 2^30 clocks at 60 MHz, so about 0.28 s to 17.9 s
    // after configuration. `usb_ulpi_trace.v`'s header is why there is a
    // window at all.
    parameter integer OPEN_BIT = 24,
    parameter integer SHUT_BIT = 30
) (
    input  wire clk,              // A8, the 60.000 MHz oscillator

    // The auxiliary transceiver: the hub the computer sees, and the proxy's
    // upstream bus.
    inout  wire [7:0] aux_data,   // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire aux_dir,          // E16
    input  wire aux_nxt,          // F15
    output wire aux_stp,          // E15
    output wire aux_rst_n,        // J13, active low at the ball
    output wire aux_clk,          // D16, the clock the board says we owe it

    // The target transceiver: the proxy's downstream bus.
    inout  wire [7:0] tgt_data,   // R2 R1 P2 P1 N3 N1 M2 M1
    input  wire tgt_dir,          // R3
    input  wire tgt_nxt,          // T2
    output wire tgt_stp,          // T3
    output wire tgt_rst_n,        // R4, active low at the ball
    output wire tgt_clk,          // T4

    // The three bidirectional VBUS switches onto the TARGET A node. Two of
    // them are driven low and have no parameter; the header says why.
    output wire aux_vbus_en,      // L2
    output wire control_vbus_en,  // L1
    output wire target_c_vbus_en, // K5

    // The console, on the net JTAG `TMS` shares. Read the header.
    inout  wire uart_tx,          // T14

    output wire led0_n,           // the AUX transceiver answered
    output wire led1_n,           // the hub was configured
    output wire led2_n,           // something is on the target pair
    output wire led3_n,           // heartbeat
    output wire led4_n,           // the hub's port is enabled
    output wire led5_n            // the PC addressed the device behind it
);
    // -----------------------------------------------------------------
    // The power-on reset: a one walked along a shift register, so the cores
    // are held in reset for `POR` clocks. `usb_ulpi_device.v`'s header says
    // why this is a shift register and not `rst_n` tied high — an ECP5
    // releases every flip-flop into its `REGSET` state and tying it high
    // would work on the part, but a simulator has no `REGSET` and every
    // register would stay unknown for ever.
    // -----------------------------------------------------------------
    reg [POR-1:0] por = {POR{1'b0}};
    always @(posedge clk) begin
        por <= {por[POR-2:0], 1'b1};
    end
    wire reset_done = por[POR-1];

    // The interface clock both transceivers ask the FPGA to provide.
    // `clk_dir='o'` and 60 MHz both ways, so there is nothing to make.
    assign aux_clk = clk;
    assign tgt_clk = clk;

    // -----------------------------------------------------------------
    // THE VBUS SWITCHES. Exactly one of them has a parameter.
    // -----------------------------------------------------------------
    assign aux_vbus_en      = (VBUS_AUX != 0);
    assign control_vbus_en  = 1'b0;
    assign target_c_vbus_en = 1'b0;

    // =================================================================
    // THE PROXY
    // =================================================================
    wire [7:0] aux_o;
    wire       aux_oe;
    assign aux_data = aux_oe ? aux_o : 8'bz;

    wire [7:0] tgt_o;
    wire       tgt_oe;
    assign tgt_data = tgt_oe ? tgt_o : 8'bz;

    wire [6:0] hub_address;
    wire       hub_configured, hub_bus_reset, hub_phy_ready;
    wire       port_power, port_enabled, port_suspended, port_reset;
    wire       dn_phy_ready, dn_attached, dn_low_speed, dn_reg_failed;
    wire [1:0] dn_line_state, dn_vbus_state;
    wire [3:0] dn_stage;
    wire       proxied, ctrl_active, setup_seen, data_fwd;
    wire [1:0] job;

    // `*_VENDOR_ADDR` / `*_VENDOR_DATA` are this board's one register and not
    // ULPI's: a Cynthion crosses DP and DM between **each** transceiver and its
    // connector, and register 39h bit 1 of the Microchip USB3343 undoes it.
    // Great Scott Gadgets' own platform file applies `{0x39: 0b000110}` to
    // whichever ULPI interface is built, not per port, so both ports get the
    // same value here — and `usb_ulpi_device.v`'s header has the three sources
    // that agree on it for AUX. For TARGET it is still only quoted:
    // `ip/usb_host_ulpi/README.md` §5 says so, and `FS_LINE` is the parameter
    // that settles it if the quotation is wrong.
    // `CFG_ATTR` is `bmAttributes` of the configuration descriptor: bit 7 is
    // reserved and set, and bit 6 is Self Powered — `SELF_POWERED` above is the
    // whole argument for it and the measurement that made it a parameter. It is
    // also what the **standard** GET_STATUS of USB 2.0 §9.4.5 reports in bit 0
    // of its two bytes, which `usb_ctrl_ep` derives from this same byte so the
    // two can never disagree.
    usb_hub_proxy_ulpi #(
        .CFG_ATTR       (SELF_POWERED != 0 ? 8'hC0 : 8'h80),
        .UP_VENDOR_ADDR (6'h39),
        .UP_VENDOR_DATA (8'h06),
        .DN_VENDOR_ADDR (6'h39),
        .DN_VENDOR_DATA (8'h06),
        .FS_LINE        (FS_LINE),
        .TURNAROUND     (TURNAROUND)
    ) u_proxy (
        .clk60          (clk),
        .rst_n          (reset_done),
        .up_data_i      (aux_data),
        .up_data_o      (aux_o),
        .up_data_oe     (aux_oe),
        .up_dir         (aux_dir),
        .up_nxt         (aux_nxt),
        .up_stp         (aux_stp),
        .up_rst_n       (aux_rst_n),
        .dn_data_i      (tgt_data),
        .dn_data_o      (tgt_o),
        .dn_data_oe     (tgt_oe),
        .dn_dir         (tgt_dir),
        .dn_nxt         (tgt_nxt),
        .dn_stp         (tgt_stp),
        .dn_rst_n       (tgt_rst_n),
        .address        (hub_address),
        .configured     (hub_configured),
        .usb_reset      (hub_bus_reset),
        .phy_ready      (hub_phy_ready),
        .port_power     (port_power),
        .port_enabled   (port_enabled),
        .port_suspended (port_suspended),
        .port_reset     (port_reset),
        .dn_phy_ready   (dn_phy_ready),
        .dn_attached    (dn_attached),
        .dn_low_speed   (dn_low_speed),
        .dn_line_state  (dn_line_state),
        .dn_vbus_state  (dn_vbus_state),
        .dn_stage       (dn_stage),
        .dn_reg_failed  (dn_reg_failed),
        .dn_frame       (),
        .proxied        (proxied),
        .ctrl_active    (ctrl_active),
        .job            (job),
        .setup_seen     (setup_seen),
        .data_fwd       (data_fwd)
    );

    // =================================================================
    // THE LATCHES AND THE ONE COUNTER
    // =================================================================
    // Written `q <= q | event` and not `if (event) q <= 1'b1` because the
    // second infers a clock enable, a slice's two flip-flops share one `CE`
    // wire, and this design's console stayed silent on a part with it.
    reg saw_bus_reset  = 1'b0;
    reg saw_attached   = 1'b0;
    reg saw_configured = 1'b0;
    reg saw_proxied    = 1'b0;
    reg saw_data_fwd   = 1'b0;
    always @(posedge clk) begin
        saw_bus_reset  <= saw_bus_reset  | hub_bus_reset;
        saw_attached   <= saw_attached   | dn_attached;
        saw_configured <= saw_configured | hub_configured;
        saw_proxied    <= saw_proxied    | proxied;
        saw_data_fwd   <= saw_data_fwd   | data_fwd;
    end

    // A count of SETUP packets the PC has sent to something behind the port,
    // which is the one number that says whether the forwarding is **working**
    // rather than merely reached: a PC enumerating a device sends eight or so and
    // stops.
    //
    // `setup_seen` is one cycle per SETUP the relay took and forwarded, so this
    // counts it directly. **It used to count edges of `ctrl_active` and that was
    // wrong**: `ctrl_active` is a control transfer the relay holds a SETUP for and
    // it stays high from the first SETUP until the port is reset, so the count
    // read 2 on a board where the PC had sent dozens. The pulse exists because of
    // that reading.
    //
    // Five bits and it **saturates** rather than wrapping, because a counter that
    // wraps reads the same as one that never ran.
    reg [4:0] setups = 5'd0;
    always @(posedge clk) begin
        if (setup_seen && setups != 5'd31) setups <= setups + 5'd1;
    end

    // =================================================================
    // THE CONSOLE, on T14
    // =================================================================
    // THE WINDOW IN WHICH THE PAD MAY DRIVE
    //
    // `clock_blink.v`'s spelling of `+ 1`, and the two latches written
    // `q <= q | e` for the reason above.
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

    // The four bytes, which the header's table is the field list of.
    wire [7:0] b0 = {hub_phy_ready, hub_configured, hub_address != 7'd0,
                     saw_bus_reset, port_power, port_enabled, port_suspended,
                     port_reset};
    wire [7:0] b1 = {dn_phy_ready, dn_attached, dn_low_speed, saw_proxied,
                     dn_reg_failed, dn_vbus_state, saw_data_fwd};
    wire [7:0] b2 = {1'b0, ctrl_active, job, dn_stage};
    wire [7:0] b3 = {1'b0, dn_line_state, setups};

    // `P`, eight nibbles, CR, LF: eleven characters, and `pos` runs 0 to 11 —
    // one past the last one, which is the state the gap between lines is
    // counted in.
    //
    // **Four bits, because eleven fits in four.** `usb_hub_target.v`'s own
    // comment is why that matters: a fifth bit would be a bit no expression in
    // this file can set, which is a flip-flop whose data input is the constant
    // zero, which on an ECP5 is the shape that cost this project eight rounds
    // of investigation over a `reg [2:0]` for four states.
    localparam [3:0] P_LAST = 4'd9;   // the CR
    localparam [3:0] P_END  = 4'd11;  // one past the LF: the gap

    reg [3:0]  pos   = 4'd0;
    reg [9:0]  baud  = 10'd0;
    reg [3:0]  bitno = 4'd0;
    reg [7:0]  shreg = 8'h00;
    reg        line_q = 1'b1;
    reg [GAP_BITS-1:0] idle = {GAP_BITS{1'b0}};

    // Which nibble. `pos` 1 and 2 are byte 0, 3 and 4 byte 1, and so on, so
    // `pos - 1` splits into a byte index and which half of it. At `pos` 0, 9
    // and 10 it is not a nibble at all and `chr` below does not read one.
    wire [3:0] q = pos - 4'd1;
    reg  [7:0] sel;
    always @(*) begin
        case (q[2:1])
            2'd0:    sel = b0;
            2'd1:    sel = b1;
            2'd2:    sel = b2;
            default: sel = b3;
        endcase
    end
    wire [3:0] nib = q[0] ? sel[3:0] : sel[7:4];
    wire [7:0] hex = (nib < 4'd10) ? (8'h30 + {4'd0, nib})
                                   : (8'h37 + {4'd0, nib});
    reg [7:0] chr;
    always @(*) begin
        if (pos == 4'd0)                  chr = 8'h50;  // 'P'
        else if (pos == P_LAST)           chr = 8'h0D;
        else if (pos == P_LAST + 4'd1)    chr = 8'h0A;
        else                              chr = hex;
    end

    wire sending = (bitno != 4'd0);

    always @(posedge clk) begin
        if (!window) begin
            bitno  <= 4'd0;
            baud   <= 10'd0;
            pos    <= 4'd0;
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
        end else if (pos == P_END) begin
            if (idle == {GAP_BITS{1'b1}}) begin
                idle <= {GAP_BITS{1'b0}};
                pos  <= 4'd0;
            end else begin
                idle <= idle + {{(GAP_BITS-1){1'b0}}, 1'b1};
            end
        end else begin
            shreg  <= chr;
            bitno  <= 4'd1;
            baud   <= 10'd0;
            line_q <= 1'b0;
            pos    <= pos + 4'd1;
        end
    end

    // High impedance outside the window, which is the whole of the safety
    // argument in the header.
    assign uart_tx = window ? line_q : 1'bz;

    // =================================================================
    // THE HEARTBEAT
    // =================================================================
    // The reduction spelling of `+ 1` that `clock_blink.v` explains.
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

    // Active low: a pin driven low lights one.
    assign led0_n = ~hub_phy_ready;
    assign led1_n = ~saw_configured;
    assign led2_n = ~saw_attached;
    assign led3_n = ~count[25];
    assign led4_n = ~port_enabled;
    assign led5_n = ~saw_proxied;
endmodule
