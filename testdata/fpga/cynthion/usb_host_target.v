// A USB full-speed **host** on a Cynthion's TARGET port, reporting what it
// finds over the USB serial console on its AUX port.
//
// Everything else USB in this tree is a peripheral. This is the other end:
// `ip/usb/usb_host_ulpi` on the target transceiver, generating a frame every
// millisecond, driving a bus reset out of the transceiver's own
// terminations, and enumerating whatever is plugged into TARGET-A as far as
// its device descriptor, its configuration descriptor, an address and a
// configuration. `ip/usb/usb_cdc_acm` on the auxiliary transceiver is the
// console it says so on, which is the same `/dev/ttyACM*` that
// `usb_cdc_uart.v` next door brings up and is left working for exactly that
// reason.
//
// ===================================================================
// WHAT THIS HAS DONE ON A PART, AND THE LIE THE REPORT TOLD FIRST
// ===================================================================
//
// **It builds, it has been loaded, and the transceiver answers.** The
// thirteen TARGET ULPI balls and the three VBUS switch balls are all at
// column 0 of the caBGA-256 — the **left** edge of the die — which
// `src/fpga/trellis`'s `Edge::of` did not describe when this file was
// written, so the flow refused the design with
//
//     error: `tgt_data$io0` is constrained to package pin `R2`, which the
//            architecture maps to no usable site
//
// All four edges are described now, and on a Cynthion r1.4 this design's
// own console says:
//
//     VIDL=24  VIDH=04     the vendor ID pair: 0424, Microchip
//     PIDL=09  PIDH=00
//     FUNC=45  OTGC=06     Function Control, OTG Control, as set here
//     IOPM=06              39h read back as written
//     DBUG=00              LineState SE0, which is an idle host port
//     RXCM=40  SEEN=01     a receive command arrived; its ID bit is set
//     PHYR=01  RFAL=00     ready, and not one register read failed
//
// That is the measurement the left edge was missing: a USB3343 cannot
// raise `nxt` without the 60 MHz clock this design drives out of **T4**,
// cannot leave reset without **R4**, and cannot hand back a register byte
// without `dir` on **R3** and the eight data balls. A wrong ball for any
// of them reads `00` everywhere, which is what a wrong left-edge tile rule
// would have produced. `ip/usb/usb_host_ulpi/README.md` §9 and
// `docs/fpga-trellis.md` have the account from each side.
//
// **And the first reading of that console was the whole report end for
// end**, which is worth leaving here because it cost a round of work: the
// label index below reversed `LABELS`' four-byte elements *and* the
// characters inside them, so the console printed `PHYR=00 RFAL=01
// VIDL=00 VIDH=00` — a transceiver that had never answered — while the
// bytes beside those names belonged to items at the other end of the
// list. The comment on `lidx` has the arithmetic. Nothing about the bus,
// the balls or the host link was wrong; the printer was.
//
// **The enumeration still stops at `STGE=01`, and that is correct here.**
// Stage 1 is waiting for an attach, and nothing can attach: no VBUS switch
// is closed, so the TARGET-A socket has no power and `DBUG` reads SE0.
// `VBUS_AUX` is what closes one and the section below is what to read
// before changing it.
//
// ===================================================================
// THE ORDER THIS DOES THINGS IN, AND WHY IT IS NOT NEGOTIABLE
// ===================================================================
//
// **The probe comes first and it is read-only.** With `HOST_EN` low — or,
// with it high, for as long as the probe is running — nothing is driven on
// the TARGET pair at all. The transceiver is brought out of reset and put
// into ULPI's full-speed **host** mode, which is two 15 kOhm pull-downs and
// no terminations and no pull-up (USB334x DS00002646A Table 5-1, "Host Full
// Speed"), and then nine of its registers are read and printed:
//
//     VIDL VIDH PIDL PIDH   00h..03h, which say what the part is
//     FUNC                  04h, Function Control, as this host set it
//     OTGC                  0Ah, OTG Control, which holds the pull-downs
//     INTS                  13h, USB Interrupt Status
//     DBUG                  15h, whose low two bits are LineState
//     IOPM                  39h, the vendor register that swaps DP and DM
//
// and then the last **receive command**, whole and decoded:
//
//     RXCM LINE VBUS IDPN SEEN
//
// Those five are the three things nothing else can say. `VBUS` is the
// transceiver's own comparators (Table 6-3): `11` is VBUS valid, `00` is
// below SessEnd — no power on the port at all. `LINE` is the pair: `00`
// means nothing is pulling anything up, `01` means D+ is high and `10`
// means D- is. And since a full-speed device pulls **D+** up, which of
// those two it is also settles whether this board exchanges DP and DM on
// the way to the TARGET connector the way it does on AUX.
//
// **No VBUS switch is enabled unless `VBUS_AUX` says so.** See the next
// section; the short of it is that these are bidirectional switches onto
// one node and closing two of them ties two hosts' supplies together.
//
// ===================================================================
// THE VBUS SWITCHES, AND WHICH ONE THIS DESIGN MAY CLOSE
// ===================================================================
//
// The platform file's own comment is the whole of the hazard:
//
//     # VBUS on each of the Type-C ports can be connected to TARGET A
//     # through a bidirectional switch. If any of these switches is
//     # enabled, TARGET A is considered an output. An additional switch
//     # can be enabled to pass VBUS through to another port in addition
//     # to TARGET A.
//
// Three of them reach that node — `control_vbus_en` (L1), `aux_vbus_en`
// (L2) and `target_c_vbus_en` (K5) — and all three are active high. So:
//
//   * **`control_vbus_en` and `target_c_vbus_en` are driven low by this
//     design, always, with no parameter to raise them.** Driving them
//     rather than leaving them unconstrained is the point: a pin this
//     design does not mention comes up however the bitstream's defaults
//     leave it, and "exactly one switch" should be a property of the
//     bitstream and not of a default.
//   * **`aux_vbus_en` is `VBUS_AUX`**, and nothing else is. It passes the
//     **AUX** Type-C port's VBUS through to TARGET A, which is where a
//     device in the full-size A socket is.
//
// Why AUX and not CONTROL, which would do the same job. CONTROL is the
// port the board's own supply and its Apollo debug microcontroller come in
// on: a device drawing more than that port will give, or shorting it,
// browns out the thing that loads bitstreams into this FPGA. AUX is a
// separate Type-C port whose VBUS does not feed the board, so the worst a
// misbehaving device under test can do through it is make the host at the
// other end of the AUX cable current-limit that port — which costs this
// console and nothing else, and is visible immediately because the console
// is how anything here is read at all.
//
// **What state the switch is left in.** A bitstream built with
// `VBUS_AUX = 1` drives `aux_vbus_en` high for as long as it is loaded, and
// a bitstream built with `VBUS_AUX = 0` — the default — drives it low. The
// FPGA's configuration SRAM is volatile: **a power cycle reloads whatever
// is in the board's flash**, and what that gateware does with these pins is
// its business and not this file's. So a board left with this design
// loaded and `VBUS_AUX = 1` has one switch closed until it is reconfigured
// or power-cycled, and a power cycle hands the pins back to the flash.
//
// ===================================================================
// WHAT TO LOOK AT
// ===================================================================
//
// The console, which is a device file:
//
//     ls -l /dev/serial/by-id/
//     stty -F /dev/ttyACM1 115200 raw -echo
//     cat -v /dev/ttyACM1
//
// The whole report is reprinted every `REPORT_CYCLES` — about two seconds —
// so it does not matter when the console is opened. A line looks like
//
//     FUNC=45
//
// and the descriptor dump like
//
//     D00:12011002000000084209010000000001
//
// where the first character is the region (`D` for the device descriptor,
// `C` for the configuration descriptor), the next two are the offset in
// hex, and the rest are eight bytes.
//
// The six LEDs are the diagnosis for when the console says nothing:
//
//     LED 0   THE TARGET TRANSCEIVER ANSWERED      `phy_ready`, live
//     LED 1   THE AUX CONSOLE IS ENUMERATED        `configured`, latched
//     LED 2   SOMETHING IS ON THE TARGET PAIR      `attached`, latched
//     LED 3   heartbeat, 0.89 Hz                   the clock runs
//     LED 4   THE DEVICE WAS ENUMERATED            `up`, latched
//     LED 5   THE ENUMERATION STOPPED ON SOMETHING `failed`, latched
//
// LED 0 dark means the target transceiver never came up, and then nothing
// else on that port means anything. LED 1 dark means the console is not
// enumerated, which is the only way the report can be missed while
// everything else works.
//
// ===================================================================
// WHY DRIVING THESE PINS IS SAFE
// ===================================================================
//
// The AUX side is `usb_cdc_uart.v`'s, ball for ball, and that file and
// `usb_ulpi_device.v` have the long form of the argument. The TARGET side
// is the same argument about a different transceiver: every direction is
// the one Great Scott Gadgets' platform file gives it, the eight data lines
// are released whenever `ulpi_dir` is high, which is the bus's own
// arbitration, and `SLEWRATE="FAST"` is asked for on all thirteen because
// the platform file asks for it on all thirteen.
//
// The `CONTROL` port the Apollo debugger lives on is a third transceiver on
// third balls and is not mentioned here, so loading this cannot take the
// debugger away. The Type-C controllers, the power-input switches
// (`control_vbus_in_en`, `aux_vbus_in_en`) and the pseudo-supply pins are
// left alone, exactly as the designs next door leave them.
module usb_host_target #(
    // How many clocks the cores are held in reset after configuration.
    parameter integer POR = 16,
    // Whether the enumeration runs at all. 0 is the probe and nothing else:
    // the transceiver is configured and read and the TARGET pair is never
    // driven.
    parameter integer HOST_EN = 1,
    // Whether `aux_vbus_en` (L2) is driven high, passing the AUX Type-C
    // port's VBUS through to TARGET A. **Read the section above before
    // changing this.** 0 is the default and closes no switch at all.
    parameter integer VBUS_AUX = 0,
    // Clocks between one printing of the report and the next: two seconds
    // at 60 MHz, so the console can be opened at any time.
    parameter integer REPORT_CYCLES = 120_000_000,
    // Which `LineState` is a full-speed device's idle J, from the target
    // transceiver's point of view. `ip/usb/usb_host_ulpi`'s `FS_LINE` says why
    // this is a parameter: with `39h` bit 1 set it is `01`, and the probe's
    // `LINE` line is what settles it on a board nobody has measured.
    parameter [1:0] FS_LINE = 2'b01,
    // Cycles of an idle bus at J before the AUX device's answer goes out,
    // which ULPI 1.1 Table 10 allows a full-speed Link between 7 and 18 of.
    parameter [6:0] TURNAROUND = 7'd9
) (
    input  wire clk,              // A8, the 60.000 MHz oscillator

    // The auxiliary transceiver: the console.
    inout  wire [7:0] aux_data,   // F16 G15 G16 H15 J15 J16 K15 K16
    input  wire aux_dir,          // E16
    input  wire aux_nxt,          // F15
    output wire aux_stp,          // E15
    output wire aux_rst_n,        // J13, active low at the ball
    output wire aux_clk,          // D16, the clock the board says we owe it

    // The target transceiver: the host.
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

    output wire led0_n,           // the target transceiver answered
    output wire led1_n,           // the console is enumerated
    output wire led2_n,           // something is on the target pair
    output wire led3_n,           // heartbeat
    output wire led4_n,           // the device was enumerated
    output wire led5_n            // the enumeration stopped on something
);
    // -----------------------------------------------------------------
    // The power-on reset: a one walked along a shift register, so the
    // cores are held in reset for `POR` clocks. `usb_ulpi_device.v`'s
    // header says why this is a shift register and not `rst_n` tied high —
    // an ECP5 releases every flip-flop into its `REGSET` state and tying
    // it high would work on the part, but a simulator has no `REGSET` and
    // every register would stay unknown for ever.
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
    // THE CONSOLE, on AUX
    // =================================================================
    wire [7:0] aux_o;
    wire       aux_oe;
    wire       con_configured;

    wire [7:0] con_out_data;
    wire       con_out_valid;
    wire       con_out_last;
    wire [7:0] con_in_data;
    wire       con_in_valid;
    wire       con_in_ready;
    wire       con_in_commit;

    assign aux_data = aux_oe ? aux_o : 8'bz;

    // Both carriers, no errors: `usb_cdc_uart.v`'s header says at length
    // what `wSerialState` is and why a port with nothing on the far end
    // reports a carrier anyway.
    wire [6:0] serial_state = 7'b000_0011;

    // `VENDOR_ADDR` / `VENDOR_DATA` are this board's one register and not
    // ULPI's: a Cynthion crosses DP and DM between each transceiver and its
    // connector, and register 39h bit 1 of the Microchip USB3343 undoes it.
    // `usb_ulpi_device.v`'s header has the three sources that agree on it.
    usb_cdc_acm_ulpi #(
        .TURNAROUND  (TURNAROUND),
        .VENDOR_ADDR (6'h39),
        .VENDOR_DATA (8'h06)
    ) u_console (
        .clk60        (clk),
        .rst_n        (reset_done),
        .ulpi_data_i  (aux_data),
        .ulpi_data_o  (aux_o),
        .ulpi_data_oe (aux_oe),
        .ulpi_dir     (aux_dir),
        .ulpi_nxt     (aux_nxt),
        .ulpi_stp     (aux_stp),
        .ulpi_rst_n   (aux_rst_n),
        .address      (),
        .configured   (con_configured),
        .usb_reset    (),
        .phy_ready    (),
        .out_data     (con_out_data),
        .out_valid    (con_out_valid),
        .out_last     (con_out_last),
        // Whatever is typed at the console is read and dropped: this
        // design takes no commands. Holding `out_ready` high is what keeps
        // a host that writes to the port from being NAKed for ever.
        .out_ready    (1'b1),
        .in_data      (con_in_data),
        .in_valid     (con_in_valid),
        .in_ready     (con_in_ready),
        .in_commit    (con_in_commit),
        .baud         (),
        .char_format  (),
        .parity       (),
        .data_bits    (),
        .dtr          (),
        .rts          (),
        .serial_state (serial_state)
    );

    // =================================================================
    // THE HOST, on TARGET
    // =================================================================
    wire [7:0] tgt_o;
    wire       tgt_oe;
    assign tgt_data = tgt_oe ? tgt_o : 8'bz;

    wire       h_phy_ready;
    wire [7:0] h_rx_cmd;
    wire       h_rx_cmd_seen;
    wire [1:0] h_line_state;
    wire [1:0] h_vbus_state;
    wire       h_id_pin;

    reg        enum_en;
    reg        reg_start_q;
    reg [3:0]  pidx;
    wire [7:0] h_reg_rdata;
    wire       h_reg_done;
    wire       h_reg_ok;
    wire       h_reg_busy;

    wire [10:0] h_frame;
    wire [4:0]  h_stage;
    wire        h_attached, h_low_speed, h_up, h_failed;
    wire [4:0]  h_fail_stage;
    wire [2:0]  h_fail_status;
    wire [6:0]  h_dev_addr;
    wire [6:0]  h_maxpkt0;
    wire [15:0] h_cfg_total;
    wire [7:0]  h_cfg_value;
    wire [7:0]  h_desc_data;
    wire        h_desc_valid;
    wire [6:0]  h_desc_index;
    wire [1:0]  h_desc_tag;
    wire        h_desc_done;
    wire [6:0]  h_desc_len;

    // The nine registers the probe reads, **index 0 first**. A
    // concatenation puts its leftmost element in the high bits, so the list
    // below is written back to front on purpose: `PROBE_ADDRS[7:0]` is
    // `00h` and is what `pidx = 0` selects.
    localparam [9*8-1:0] PROBE_ADDRS = {
        8'h39,   // 8: USB IO & Power Management, the DP/DM swap
        8'h15,   // 7: Debug, whose low two bits are LineState
        8'h13,   // 6: USB Interrupt Status
        8'h0A,   // 5: OTG Control
        8'h04,   // 4: Function Control
        8'h03,   // 3: Product ID High
        8'h02,   // 2: Product ID Low
        8'h01,   // 1: Vendor ID High
        8'h00    // 0: Vendor ID Low
    };
    localparam integer PROBE_N = 9;

    wire [7:0] probe_addr = PROBE_ADDRS[{pidx, 3'b000} +: 8];

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
        .rx_cmd       (h_rx_cmd),
        .rx_cmd_seen  (h_rx_cmd_seen),
        .line_state   (h_line_state),
        .vbus_state   (h_vbus_state),
        .id_pin       (h_id_pin),
        .enum_en      (enum_en),
        .reg_start    (reg_start_q),
        .reg_write    (1'b0),
        .reg_addr     (probe_addr[5:0]),
        .reg_wdata    (8'h00),
        .reg_rdata    (h_reg_rdata),
        .reg_done     (h_reg_done),
        .reg_ok       (h_reg_ok),
        .reg_busy     (h_reg_busy),
        .frame        (h_frame),
        .sof_sent     (),
        .stage        (h_stage),
        .attached     (h_attached),
        .low_speed    (h_low_speed),
        .up           (h_up),
        .failed       (h_failed),
        .fail_stage   (h_fail_stage),
        .fail_status  (h_fail_status),
        .dev_addr     (h_dev_addr),
        .maxpkt0      (h_maxpkt0),
        .cfg_total    (h_cfg_total),
        .cfg_value    (h_cfg_value),
        .desc_data    (h_desc_data),
        .desc_valid   (h_desc_valid),
        .desc_index   (h_desc_index),
        .desc_tag     (h_desc_tag),
        .desc_done    (h_desc_done),
        .desc_len     (h_desc_len)
    );

    // =================================================================
    // THE PROBE: nine register reads, before anything is driven
    // =================================================================
    localparam [1:0] PB_WAIT = 2'd0;
    localparam [1:0] PB_REQ  = 2'd1;
    localparam [1:0] PB_DONE = 2'd2;

    reg [1:0] pb;
    // Sixteen words rather than nine, so that the report's read of this
    // array can be indexed with four bits of the item number without ever
    // being out of range.
    reg [7:0] probe_val [0:15];
    reg       probe_fail;

    always @(posedge clk or negedge reset_done) begin
        if (!reset_done) begin
            pb          <= PB_WAIT;
            pidx        <= 4'd0;
            reg_start_q <= 1'b0;
            probe_fail  <= 1'b0;
            enum_en     <= 1'b0;
        end else begin
            // Held until the port takes it, for the reason every request in
            // this library is held: the cycle a pulse lands in may be one
            // the far end cannot accept.
            if (reg_start_q && h_reg_busy) reg_start_q <= 1'b0;

            case (pb)
                PB_WAIT: begin
                    // Nothing is read until the transceiver has been
                    // configured and read back, which is what `phy_ready`
                    // is.
                    if (h_phy_ready) pb <= PB_REQ;
                end
                PB_REQ: begin
                    // `h_reg_done` first: `h_reg_busy` falls in the very
                    // cycle it rises, so issuing first would see an idle
                    // port and ask again.
                    if (h_reg_done) begin
                        probe_val[pidx] <= h_reg_ok ? h_reg_rdata : 8'hFF;
                        probe_fail      <= probe_fail | ~h_reg_ok;
                        if (pidx == PROBE_N[3:0] - 4'd1) begin
                            pb <= PB_DONE;
                        end else begin
                            pidx <= pidx + 4'd1;
                        end
                    end else if (!reg_start_q && !h_reg_busy) begin
                        reg_start_q <= 1'b1;
                    end
                end
                default: begin
                    // The probe is over. **This is where the host is let
                    // loose**, and not before: until now nothing has been
                    // driven on the TARGET pair at all.
                    if (HOST_EN != 0) enum_en <= 1'b1;
                end
            endcase
        end
    end

    wire probe_done = (pb == PB_DONE);

    // =================================================================
    // THE DESCRIPTORS, as they arrive
    // =================================================================
    // Ninety-six bytes: the eighteen of a device descriptor at 0 and
    // sixty-four of a configuration descriptor at 32. `usb_host_enum`
    // streams every byte with its offset in its own descriptor and only
    // says `desc_done` when one is whole, so writing at a fixed base plus
    // that offset never needs undoing: a transfer that failed is retried
    // and the retry writes the same offsets again.
    localparam [6:0] DEV_BASE = 7'd0;
    localparam [6:0] CFG_BASE = 7'd32;

    reg [7:0] dbuf [0:95];
    reg [6:0] dev_len;
    reg [6:0] cfg_len;

    wire [6:0] desc_at = (h_desc_tag == 2'd0 ? DEV_BASE : CFG_BASE)
                       + h_desc_index;

    always @(posedge clk or negedge reset_done) begin
        if (!reset_done) begin
            dev_len <= 7'd0;
            cfg_len <= 7'd0;
        end else begin
            if (h_desc_valid) dbuf[desc_at] <= h_desc_data;
            if (h_desc_done) begin
                if (h_desc_tag == 2'd0) dev_len <= h_desc_len;
                else                    cfg_len <= h_desc_len;
            end
        end
    end

    // =================================================================
    // THE REPORT
    // =================================================================
    // Every item is nine characters — a four-character label, `=`, two hex
    // digits, CR, LF — and every dump line is twenty-two: a region letter,
    // two hex digits of offset, `:`, eight bytes and CR LF. Keeping both
    // shapes fixed is what makes this a counter and a multiplexer rather
    // than a program.
    localparam integer NI_PROBE = 17;   // items the probe alone fills in
    localparam integer NI_ALL   = 30;   // and the ones the enumeration adds
    localparam integer LBL      = 4;

    // The labels, in item order, four characters each — and like
    // `PROBE_ADDRS` the list is **back to front**, because a concatenation
    // puts its leftmost element in the high bits and the indexing below
    // counts from the low ones.
    localparam [NI_ALL*LBL*8-1:0] LABELS = {
        "FRML",   // 29: the frame number's low byte, so frames can be seen
        "LIN2",   // 28: LineState now, which is after the bus reset
        "CLEN",   // 27: configuration descriptor bytes captured
        "DLEN",   // 26: device descriptor bytes captured
        "CFGV",   // 25: bConfigurationValue, as SET_CONFIGURATION used it
        "CTHI",   // 24: wTotalLength, high byte
        "CTLO",   // 23: wTotalLength, low byte
        "MPS0",   // 22: bMaxPacketSize0
        "ADDR",   // 21: the address the device was given
        "FSTA",   // 20: why it stopped
        "FSTG",   // 19: where it stopped
        "FLAG",   // 18: attached, low_speed, up, failed
        "STGE",   // 17: which step of the enumeration it is on
        "RFAL",   // 16: a register read failed; the value reads FF
        "PHYR",   // 15: phy_ready
        "VBEN",   // 14: the VBUS switches this build drives
        "SEEN",   // 13: a receive command has arrived at all
        "IDPN",   // 12: the ID pin
        "VBUS",   // 11: VbusState; 11 is VBUS valid
        "LINE",   // 10: LineState; 01 is D+ high
        "RXCM",   //  9: the last receive command, whole
        "IOPM",   //  8: 39h
        "DBUG",   //  7: 15h
        "INTS",   //  6: 13h
        "OTGC",   //  5: 0Ah
        "FUNC",   //  4: 04h
        "PIDH",   //  3: 03h
        "PIDL",   //  2: 02h
        "VIDH",   //  1: 01h
        "VIDL"    //  0: 00h
    };

    // A carriage return is written as a byte and not as an escape: IEEE 1364
    // gives Verilog's string literals only `\n`, `\t`, `\\`, `\"` and an
    // octal `\ddd`, and `\r` is not one of them in any edition. A
    // concatenation of bytes and strings says the same thing and says it in
    // the language.
    localparam integer NB = 27;
    localparam [NB*8-1:0] BANNER = {8'h0D, 8'h0A,
                                    "== CYNTHION TARGET HOST",
                                    8'h0D, 8'h0A};

    // The phases of one printing.
    localparam [1:0] PR_IDLE   = 2'd0;
    localparam [1:0] PR_BANNER = 2'd1;
    localparam [1:0] PR_ITEM   = 2'd2;
    localparam [1:0] PR_DUMP   = 2'd3;

    localparam integer TICK_W = $clog2(REPORT_CYCLES);

    reg [1:0]        pr;
    reg [5:0]        pi;      // the banner's character, the item, or the line
    reg [4:0]        pc;      // the character within an item or a dump line
    reg              pregion; // which descriptor the dump is in
    reg [TICK_W-1:0] tick;
    reg              printed_once;

    // How many items and how many dump lines there are to print, which
    // grows as the design learns things.
    wire [5:0] n_items = (HOST_EN != 0) && enum_en ? NI_ALL[5:0]
                                                  : NI_PROBE[5:0];
    wire [4:0] dev_lines = (dev_len + 7'd7) >> 3;
    wire [4:0] cfg_lines = (cfg_len + 7'd7) >> 3;

    function [7:0] hex;
        input [3:0] n;
        begin
            hex = (n < 4'd10) ? (8'h30 + {4'd0, n}) : (8'h57 + {4'd0, n});
        end
    endfunction

    // The banner's character `pi`, counting from the front of the string.
    wire [5:0] bidx = NB[5:0] - 6'd1 - pi;
    wire [7:0] banner_ch = BANNER[{bidx, 3'b000} +: 8];

    // The label character `pc` of item `pi`.
    //
    // **Not the banner's way round, and getting that wrong printed every
    // label beside another item's value for a whole round of work.** The
    // banner is a table of *single bytes* whose element index and byte index
    // are the same number, so one subtraction reverses it. `LABELS` is a
    // table of **four-byte elements**: the element has to be counted from
    // the low end, because the concatenation above is written back to front
    // like `PROBE_ADDRS`, while the four characters inside it have to be
    // counted from the high end, because a string literal's first character
    // is its most significant byte. Two directions at once, and
    //
    //     lidx = (NI_ALL * LBL - 1) - (LBL * pi + pc)
    //
    // which is the banner's formula with a wider stride, reverses both — so
    // item 0 got element 29's characters. On a part that read as
    // `VIDL=00 ... FRML=24` where the transceiver had actually answered
    // `24h` to Vendor ID Low: the whole report shifted end for end, which
    // looked exactly like a transceiver that never answered at all.
    // `docs/fpga-trellis.md` has the console before and after.
    wire [7:0] lidx = {pi, 2'b00} + (LBL[7:0] - 8'd1) - {3'd0, pc};
    wire [7:0] label_ch = LABELS[{lidx, 3'b000} +: 8];

    // The byte item `pi` is reporting.
    wire [7:0] pv = probe_val[pi[3:0]];
    reg  [7:0] item_val;
    always @(*) begin
        case (pi)
            6'd0, 6'd1, 6'd2, 6'd3, 6'd4,
            6'd5, 6'd6, 6'd7, 6'd8:  item_val = pv;
            6'd9:  item_val = h_rx_cmd;
            6'd10: item_val = {6'd0, h_rx_cmd[1:0]};
            6'd11: item_val = {6'd0, h_rx_cmd[3:2]};
            6'd12: item_val = {7'd0, h_id_pin};
            6'd13: item_val = {7'd0, h_rx_cmd_seen};
            6'd14: item_val = {5'd0, aux_vbus_en, control_vbus_en,
                               target_c_vbus_en};
            6'd15: item_val = {7'd0, h_phy_ready};
            6'd16: item_val = {7'd0, probe_fail};
            6'd17: item_val = {3'd0, h_stage};
            6'd18: item_val = {4'd0, h_failed, h_up, h_low_speed, h_attached};
            6'd19: item_val = {3'd0, h_fail_stage};
            6'd20: item_val = {5'd0, h_fail_status};
            6'd21: item_val = {1'b0, h_dev_addr};
            6'd22: item_val = {1'b0, h_maxpkt0};
            6'd23: item_val = h_cfg_total[7:0];
            6'd24: item_val = h_cfg_total[15:8];
            6'd25: item_val = h_cfg_value;
            6'd26: item_val = {1'b0, dev_len};
            6'd27: item_val = {1'b0, cfg_len};
            6'd28: item_val = {6'd0, h_line_state};
            default: item_val = h_frame[7:0];
        endcase
    end

    // The dump line's own offset and the byte it is up to. `pc` 4 to 19 are
    // the eight bytes, two characters each, so the byte is `(pc-4) >> 1`
    // and the nibble is bit 0 of the same difference — which is why the
    // line has no spaces in it.
    wire [6:0] dline_off = {pi[3:0], 3'b000};
    wire [4:0] dpc       = pc - 5'd4;
    wire [6:0] dump_at   = (pregion ? CFG_BASE : DEV_BASE)
                         + dline_off + {4'd0, dpc[3:1]};
    wire [7:0] dump_byte = dbuf[dump_at];

    reg [7:0] ch;
    always @(*) begin
        case (pr)
            PR_BANNER: ch = banner_ch;
            PR_ITEM: begin
                case (pc)
                    5'd0, 5'd1, 5'd2, 5'd3: ch = label_ch;
                    5'd4:    ch = "=";
                    5'd5:    ch = hex(item_val[7:4]);
                    5'd6:    ch = hex(item_val[3:0]);
                    5'd7:    ch = 8'h0D;
                    default: ch = 8'h0A;
                endcase
            end
            PR_DUMP: begin
                case (pc)
                    5'd0:    ch = pregion ? "C" : "D";
                    5'd1:    ch = hex({1'b0, dline_off[6:4]});
                    5'd2:    ch = hex(dline_off[3:0]);
                    5'd3:    ch = ":";
                    5'd20:   ch = 8'h0D;
                    5'd21:   ch = 8'h0A;
                    default: ch = dpc[0] ? hex(dump_byte[3:0])
                                         : hex(dump_byte[7:4]);
                endcase
            end
            default: ch = 8'h00;
        endcase
    end

    wire printing = (pr != PR_IDLE);
    // One packet a line, which is what `in_commit` on the newline does:
    // `usb_bulk_ep` otherwise waits for `MAXPKT` bytes, and a report that
    // ends mid-packet would sit in the endpoint unseen.
    wire step = printing & con_in_ready;

    assign con_in_data   = ch;
    assign con_in_valid  = printing;
    assign con_in_commit = step & (ch == 8'h0A);

    always @(posedge clk or negedge reset_done) begin
        if (!reset_done) begin
            pr           <= PR_IDLE;
            pi           <= 6'd0;
            pc           <= 5'd0;
            pregion      <= 1'b0;
            tick         <= 0;
            printed_once <= 1'b0;
        end else begin
            tick <= tick + 1'b1;
            // The whole report again every `REPORT_CYCLES`, so it does not
            // matter when the console was opened — and once as soon as the
            // probe has its answers, so that the first one does not wait two
            // seconds. A trigger that lands while a report is still going
            // out is dropped rather than queued: the next one is two seconds
            // away, and a queue of reports of a state that has since changed
            // is worse than none.
            if (tick == REPORT_CYCLES[TICK_W-1:0] - 1'b1) tick <= 0;
            if (!printing && probe_done
                    && (!printed_once
                        || tick == REPORT_CYCLES[TICK_W-1:0] - 1'b1)) begin
                printed_once <= 1'b1;
                pr           <= PR_BANNER;
                pi           <= 6'd0;
                pc           <= 5'd0;
            end

            if (step) begin
                case (pr)
                    PR_BANNER: begin
                        if (pi == NB[5:0] - 6'd1) begin
                            pr <= PR_ITEM;
                            pi <= 6'd0;
                            pc <= 5'd0;
                        end else begin
                            pi <= pi + 6'd1;
                        end
                    end
                    PR_ITEM: begin
                        if (pc == 5'd8) begin
                            pc <= 5'd0;
                            if (pi == n_items - 6'd1) begin
                                // On to the descriptors, if there are any.
                                // The configuration descriptor's region is
                                // started straight away when the device
                                // descriptor's has no lines, which is what a
                                // `desc_len` of zero means.
                                pi      <= 6'd0;
                                pregion <= (dev_lines == 5'd0);
                                pr      <= ((dev_lines | cfg_lines) != 5'd0)
                                               ? PR_DUMP : PR_IDLE;
                            end else begin
                                pi <= pi + 6'd1;
                            end
                        end else begin
                            pc <= pc + 5'd1;
                        end
                    end
                    default: begin
                        // PR_DUMP.
                        if (pc == 5'd21) begin
                            pc <= 5'd0;
                            if (pi == (pregion ? cfg_lines : dev_lines)
                                      - 5'd1) begin
                                pi <= 6'd0;
                                if (!pregion && cfg_lines != 5'd0) begin
                                    pregion <= 1'b1;
                                end else begin
                                    pr <= PR_IDLE;
                                end
                            end else begin
                                pi <= pi + 6'd1;
                            end
                        end else begin
                            pc <= pc + 5'd1;
                        end
                    end
                endcase
            end
        end
    end

    // -----------------------------------------------------------------
    // The heartbeat and the latches.
    // -----------------------------------------------------------------
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

    // Written `q <= q | event` and not `if (event) q <= 1'b1` because the
    // second infers a clock enable, and a slice's two flip-flops share one
    // `CE` wire.
    reg saw_configured = 1'b0;
    reg saw_attached   = 1'b0;
    reg saw_up         = 1'b0;
    reg saw_failed     = 1'b0;
    always @(posedge clk) begin
        saw_configured <= saw_configured | con_configured;
        saw_attached   <= saw_attached | h_attached;
        saw_up         <= saw_up | h_up;
        saw_failed     <= saw_failed | h_failed;
    end

    // Active low: a pin driven low lights one.
    assign led0_n = ~h_phy_ready;
    assign led1_n = ~saw_configured;
    assign led2_n = ~saw_attached;
    assign led3_n = ~count[25];
    assign led4_n = ~saw_up;
    assign led5_n = ~saw_failed;
endmodule
