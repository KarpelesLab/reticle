// A USB **hub** on a Cynthion's AUX port, with a USB **host** on its TARGET
// port and the hub's one downstream port reporting what that host sees.
//
// This is `usb_host_target.v` next door with the AUX side changed from a
// serial port to a hub: `ip/usb_hub` on the auxiliary transceiver and
// `ip/usb_host_ulpi` on the target one, with `port_attached` wired from the
// second to the first. The computer on the AUX cable finds a hub with one
// port; what is in the TARGET-A socket is on that port.
//
// ===================================================================
// WHAT THIS IS AND, JUST AS IMPORTANTLY, WHAT IT IS NOT
// ===================================================================
//
// **It is not a USB proxy yet and must not be read as one.** Nothing joins
// the two conversations: the hub answers the PC's control requests about its
// port, the host enumerates whatever is on TARGET-A for itself, and no packet
// crosses from one bus to the other. So the expected outcome on the part is:
//
//   * the PC's own hub driver binds — `hub 7-5:1.0: USB hub found`,
//     `1 port detected`;
//   * it powers the port, is told something is attached, and resets it;
//   * and then it tries to read a device descriptor through the port and
//     **fails**, because there is nothing on the other side of a hub that does
//     not forward anything.
//
// That is correct for this design and `ip/usb_hub/README.md` §6 quotes the
// kernel log of it. A transaction proxy is what closes it, and §2 of that file
// is why it cannot be done by repeating bits: through a ULPI transceiver the
// floor for a byte in and a byte out is roughly twenty-four bit times and USB
// allows about four.
//
// ===================================================================
// THE SERIAL CONSOLE IS ON T14 HERE, AND WHY IT HAS TO BE
// ===================================================================
//
// `usb_host_target.v` reports itself over a CDC ACM serial port on **AUX**.
// This design cannot: AUX is the hub. Every other design in this directory
// that needed a console put it on the port that is now taken.
//
// So the console is the **other** one, the UART on ball **T14** that
// `usb_ulpi_trace.v` established, which Apollo bridges to `/dev/ttyACM0` and
// which is independent of AUX entirely. That file's header is the whole
// argument for it being safe and the rule it imposes, and both are repeated
// here because a design that drives T14 without them is two drivers on one
// wire:
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
// The window is seventeen seconds and the PC's hub enumeration is over in
// one, so the console is a check on the gateware's own view and the kernel
// log is the better observable. That is deliberate: a hub is the one class
// whose host side writes its own trace.
//
// THE FORMAT
// ----------
// One line a second: `H`, eight hex digits, CRLF. The eight digits are four
// bytes, most significant nibble first:
//
//   byte 0  the hub, on AUX
//           [7] phy_ready          the AUX transceiver answered
//           [6] configured         SET_CONFIGURATION accepted
//           [5] addressed          SET_ADDRESS accepted
//           [4] saw_bus_reset      the PC has reset the AUX bus at least once
//           [3] port_power         the PC sent SetPortFeature(PORT_POWER)
//           [2] port_enabled       ... and PORT_RESET, which enables it
//           [1] port_suspended     ... and PORT_SUSPEND
//           [0] saw_port_reset     a port reset has been asked for, ever
//
//   byte 1  the host, on TARGET
//           [7] phy_ready          the TARGET transceiver answered
//           [6] attached           a device is on TARGET-A, debounced
//           [5] low_speed          ... and it pulled D- up rather than D+
//           [4] up                 our host enumerated it
//           [3] failed             our host's enumeration gave up
//           [2:1] vbus_state       the transceiver's own comparators
//           [0] 0
//
//   byte 2  `5'b0` then `stage[4:0]`, our host's enumeration stage
//   byte 3  `1'b0`, `line_state[1:0]`, `fail_stage[4:0]`
//
// **Byte 0 bit 3 is the one to look at.** `port_power` is the PC's hub driver
// having sent a class request this gateware answered, which is a stronger
// statement than anything about descriptors: it means the kernel read the hub
// descriptor, believed it, and started operating the port. It is `led4_n` as
// well, so it can be read with no console at all.
//
// ===================================================================
// THE VBUS SWITCHES: EXACTLY ONE, AND IT HAS A PARAMETER
// ===================================================================
//
// Unchanged from `usb_host_target.v`, whose header has the long form. The
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
// L2, which is a deliberate refusal rather than an omission: the board's
// power topology is a property of the bitstream that was loaded, not of what
// a host asks for, and `ip/usb_hub/README.md` §4 says it again there.
//
// **A device on TARGET-A needs `VBUS_AUX = 1`.** With the default of 0 the
// socket has no power, nothing attaches, and the hub correctly reports an
// empty port — which is a useful thing to look at and is not the measurement
// this design was built for.
//
// ===================================================================
// WHAT TO LOOK AT
// ===================================================================
//
// The LEDs, which need no console:
//
//   LED 0  the AUX transceiver answered — `phy_ready` of the hub
//   LED 1  the hub was configured by the PC
//   LED 2  something is on the TARGET pair
//   LED 3  heartbeat, so a dark board is a dead board
//   LED 4  **the PC powered the hub's port**, which is its hub driver working
//   LED 5  the PC reset the port, which is it trying to enumerate through it
//
// LED 4 and LED 5 lit with LED 2 lit is this round's whole milestone: the
// kernel found a hub, found something on its port, and tried.
module usb_hub_target #(
    // How many clocks the cores are held in reset after configuration.
    parameter integer POR = 16,
    // Whether our host's enumeration of the TARGET port runs at all. 0 leaves
    // the target transceiver configured as a host — two 15 kOhm pull-downs and
    // nothing else on the pair — and drives nothing, and then `attached` never
    // rises and the hub reports an empty port.
    parameter integer HOST_EN = 1,
    // Whether `aux_vbus_en` (L2) is driven high, passing the AUX Type-C
    // port's VBUS through to TARGET A. **Read the section above before
    // changing this.** 0 is the default and closes no switch at all.
    parameter integer VBUS_AUX = 0,
    // Which `LineState` is a full-speed device's idle J, from the target
    // transceiver's point of view; `ip/usb_host_ulpi`'s `FS_LINE` says why
    // this is a parameter at all.
    parameter [1:0] FS_LINE = 2'b01,
    // Cycles of an idle bus at J before the hub's answer goes out, which ULPI
    // 1.1 Table 10 allows a full-speed Link between 7 and 18 of.
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

    // The auxiliary transceiver: the hub the computer sees.
    inout  wire [7:0] aux_data,   // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire aux_dir,          // E16
    input  wire aux_nxt,          // F15
    output wire aux_stp,          // E15
    output wire aux_rst_n,        // J13, active low at the ball
    output wire aux_clk,          // D16, the clock the board says we owe it

    // The target transceiver: our own host.
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
    output wire led4_n,           // the PC powered the hub's port
    output wire led5_n            // the PC reset the hub's port
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
    // THE HOST, on TARGET
    // =================================================================
    // Declared before the hub because the hub's downstream port reads two of
    // its outputs.
    wire [7:0] tgt_o;
    wire       tgt_oe;
    assign tgt_data = tgt_oe ? tgt_o : 8'bz;

    wire       h_phy_ready;
    wire [1:0] h_line_state;
    wire [1:0] h_vbus_state;
    wire [4:0] h_stage;
    wire       h_attached, h_low_speed, h_up, h_failed;
    wire [4:0] h_fail_stage;

    // `VENDOR_ADDR` / `VENDOR_DATA` are this board's one register and not
    // ULPI's: a Cynthion crosses DP and DM between each transceiver and its
    // connector, and register 39h bit 1 of the Microchip USB3343 undoes it.
    // `usb_ulpi_device.v`'s header has the three sources that agree on it.
    //
    // `enum_en` is a constant here, where `usb_host_target.v` held it low
    // until a nine-register probe had finished. There is no probe in this
    // design — `usb_host_target.v` is the instrument and has already read
    // those registers off this board — and `usb_host_enum` waits for
    // `phy_ready` itself, so there is nothing for a sequencer to sequence.
    usb_host_ulpi #(
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06),
        .FS_LINE     (FS_LINE),
        .DESC_MAX    (7'd64)
    ) u_host (
        .clk60        (clk),
        .rst_n        (reset_done),
        .ulpi_data_i  (tgt_data),
        .ulpi_data_o  (tgt_o),
        .ulpi_data_oe (tgt_oe),
        .ulpi_dir     (tgt_dir),
        .ulpi_nxt     (tgt_nxt),
        .ulpi_stp     (tgt_stp),
        .ulpi_rst_n   (tgt_rst_n),
        .phy_ready    (h_phy_ready),
        .rx_cmd       (),
        .rx_cmd_seen  (),
        .line_state   (h_line_state),
        .vbus_state   (h_vbus_state),
        .id_pin       (),
        .enum_en      (HOST_EN != 0),
        .reg_start    (1'b0),
        .reg_write    (1'b0),
        .reg_addr     (6'h00),
        .reg_wdata    (8'h00),
        .reg_rdata    (),
        .reg_done     (),
        .reg_ok       (),
        .reg_busy     (),
        .frame        (),
        .sof_sent     (),
        .stage        (h_stage),
        .attached     (h_attached),
        .low_speed    (h_low_speed),
        .up           (h_up),
        .failed       (h_failed),
        .fail_stage   (h_fail_stage),
        .fail_status  (),
        .dev_addr     (),
        .maxpkt0      (),
        .cfg_total    (),
        .cfg_value    (),
        .desc_data    (),
        .desc_valid   (),
        .desc_index   (),
        .desc_tag     (),
        .desc_done    (),
        .desc_len     ()
    );

    // =================================================================
    // THE HUB, on AUX
    // =================================================================
    wire [7:0] aux_o;
    wire       aux_oe;
    assign aux_data = aux_oe ? aux_o : 8'bz;

    wire [6:0] hub_address;
    wire       hub_configured;
    wire       hub_bus_reset;
    wire       hub_phy_ready;
    wire       port_power, port_enabled, port_suspended, port_reset;

    // **`port_attached` is `attached` and not `up`.** USB 2.0 §11.24.2.7.1
    // makes PORT_CONNECTION "a device is present on this port", which is what
    // our host's debounced attach is; whether our host has finished
    // enumerating it is this design's business and not the PC's. `low_speed`
    // is which line the device pulled up, which only means anything while
    // something is attached, and `ip/usb_hub` gates it on that.
    usb_hub_ulpi #(
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06)
    ) u_hub (
        .clk60          (clk),
        .rst_n          (reset_done),
        .ulpi_data_i    (aux_data),
        .ulpi_data_o    (aux_o),
        .ulpi_data_oe   (aux_oe),
        .ulpi_dir       (aux_dir),
        .ulpi_nxt       (aux_nxt),
        .ulpi_stp       (aux_stp),
        .ulpi_rst_n     (aux_rst_n),
        .address        (hub_address),
        .configured     (hub_configured),
        .usb_reset      (hub_bus_reset),
        .phy_ready      (hub_phy_ready),
        .port_attached  (h_attached),
        .port_low_speed (h_low_speed),
        .port_power     (port_power),
        .port_enabled   (port_enabled),
        .port_suspended (port_suspended),
        .port_reset     (port_reset)
    );

    // =================================================================
    // THE LATCHES
    // =================================================================
    // Written `q <= q | event` and not `if (event) q <= 1'b1` because the
    // second infers a clock enable, a slice's two flip-flops share one `CE`
    // wire, and this design's console stayed silent on a part with it.
    reg saw_bus_reset  = 1'b0;
    reg saw_port_reset = 1'b0;
    reg saw_attached   = 1'b0;
    reg saw_configured = 1'b0;
    reg saw_power      = 1'b0;
    always @(posedge clk) begin
        saw_bus_reset  <= saw_bus_reset  | hub_bus_reset;
        saw_port_reset <= saw_port_reset | port_reset;
        saw_attached   <= saw_attached   | h_attached;
        saw_configured <= saw_configured | hub_configured;
        saw_power      <= saw_power      | port_power;
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
                     saw_port_reset};
    wire [7:0] b1 = {h_phy_ready, h_attached, h_low_speed, h_up, h_failed,
                     h_vbus_state, 1'b0};
    wire [7:0] b2 = {3'b000, h_stage};
    wire [7:0] b3 = {1'b0, h_line_state, h_fail_stage};

    // `H`, eight nibbles, CR, LF: eleven characters, so `pos` runs 0 to 11
    // and four bits are not enough for the one past the end. Five are, and
    // every one of the five is reachable.
    localparam [4:0] P_LAST = 5'd9;   // the CR
    localparam [4:0] P_END  = 5'd11;  // one past the LF: the gap

    reg [4:0]  pos   = 5'd0;
    reg [9:0]  baud  = 10'd0;
    reg [3:0]  bitno = 4'd0;
    reg [7:0]  shreg = 8'h00;
    reg        line_q = 1'b1;
    reg [GAP_BITS-1:0] idle = {GAP_BITS{1'b0}};

    // Which nibble. `pos` 1 and 2 are byte 0, 3 and 4 byte 1, and so on, so
    // `pos - 1` splits into a byte index and which half of it.
    wire [4:0] q = pos - 5'd1;
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
        if (pos == 5'd0)                  chr = 8'h48;  // 'H'
        else if (pos == P_LAST)           chr = 8'h0D;
        else if (pos == P_LAST + 5'd1)    chr = 8'h0A;
        else                              chr = hex;
    end

    wire sending = (bitno != 4'd0);

    always @(posedge clk) begin
        if (!window) begin
            bitno  <= 4'd0;
            baud   <= 10'd0;
            pos    <= 5'd0;
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
                pos  <= 5'd0;
            end else begin
                idle <= idle + {{(GAP_BITS-1){1'b0}}, 1'b1};
            end
        end else begin
            shreg  <= chr;
            bitno  <= 4'd1;
            baud   <= 10'd0;
            line_q <= 1'b0;
            pos    <= pos + 5'd1;
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
    assign led4_n = ~saw_power;
    assign led5_n = ~saw_port_reset;
endmodule
