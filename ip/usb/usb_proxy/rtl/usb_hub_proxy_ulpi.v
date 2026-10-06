// usb_hub_proxy_ulpi — a one-port USB hub that **forwards**, on two ULPI
// transceivers: the hub a PC binds to on one, a host on the other, and the
// PC's own transactions run again at the device behind the port.
//
// What it does
//   Five blocks, and only one of them is new to this library:
//
//     usb_ulpi_link       `ip/usb/usb_device_ulpi`: the upstream ULPI bus, as a
//                         peripheral — which is the bus the PC is on
//     usb_hub             `ip/usb/usb_hub`: the hub's own descriptors, class
//                         requests, port state and status-change endpoint
//     usb_proxy_relay     the forwarding, on the same upstream bus as the hub
//                         and on the downstream transaction engine
//     usb_ulpi_host_link  `ip/usb/usb_host_ulpi`: the downstream ULPI bus, as a
//                         host, with the transceiver's register port brought
//                         out
//     usb_host_sie        `ip/usb/usb_host_ulpi`: frames, tokens, CRC5, one
//                         transaction at a time with a timeout and retries
//     usb_proxy_dn        the downstream port: attach, speed, and the bus
//                         reset the PC asked for
//
//   **`usb_host_enum` is not here**, and that is the architectural decision
//   this design is built on. `usb_proxy_relay`'s "PASS-THROUGH ADDRESSING" is
//   the argument in full; the short of it is that in a proxy the **PC** must be
//   the thing that enumerates the device, or there are two authorities
//   assigning addresses and a translation layer between them. So our host stops
//   being an autonomous enumerator and becomes a transaction engine the proxy
//   drives, the PC's SET_ADDRESS is relayed like any other request, and there
//   is no translation table at all. The enumerator stays in its own package,
//   where `testdata/fpga/cynthion/usb_host_target.v` uses it as the instrument
//   that established this downstream bus works.
//
// TWO DEVICES ON ONE WIRE, AND THE ONE REGISTER THAT SORTS THEM OUT
//   The upstream bus carries tokens for the hub **and** for whatever is behind
//   its port, because that is what a hub's upstream port is: everything
//   reachable through it. The hub's endpoints answer tokens addressed to the
//   hub's own address — `usb_ctrl_ep` and `usb_bulk_ep` each compare `tok_addr`
//   against it — and the relay claims every other address while the port is
//   enabled, which is exactly the rest.
//
//   So the transmitter is multiplexed on one registered bit, `owns`, the same
//   way `usb_dev_core` multiplexes its three endpoints on which one the last
//   token named. Nothing can collide: the hub never raises `tx_start` for a
//   token that was not for its address, and the relay never raises one for a
//   token that was.
//
// WHAT IT DOES NOT DO, AND WHAT THAT MEANS ON A BOARD
//   Everything the five blocks do not, and each of their headers says so. The
//   three that matter at the top level:
//
//   **It does not switch VBUS.** The socket behind the downstream port is
//   powered by a switch in the design's own top level, and on the board this
//   was written for those switches are *bidirectional* between connectors, so
//   closing two of them ties two hosts' supplies together. The PC's
//   SetPortFeature(PORT_POWER) moves a bit in `usb_hub` and reaches no pin,
//   which `ip/usb/usb_hub/README.md` §4 states as a deliberate refusal.
//
//   **It is one clock.** Both ULPI buses run at the 60 MHz interface clock,
//   which on a Cynthion is the board's own oscillator, and both transceivers
//   are in ULPI's input clock mode with the FPGA driving the clock out. There
//   is no crossing anywhere in this design and `usb_proxy_is_one_clock_domain`
//   asserts it.
//
//   **The two frame counters are not the same counter.** `usb_host_sie` sends
//   its own SOF every millisecond with its own number; the PC's frame number is
//   not forwarded. Bulk, control and interrupt transfers do not depend on it
//   and isochronous ones do, which is the first entry in the list of what this
//   does not carry.
module usb_hub_proxy_ulpi #(
    // The hub's own identity and configuration. `1209:0001` is pid.codes'
    // test pair, which every device in this library uses by default.
    parameter [15:0] VID          = 16'h1209,
    parameter [15:0] PID          = 16'h0001,
    parameter [7:0]  CFG_ATTR     = 8'h80,
    parameter [7:0]  CFG_POWER    = 8'd50,
    parameter [6:0]  MAXPKT0      = 7'd64,
    parameter        BUF_RAM      = 1,

    // Both Links. `RESET_CYCLES` is the transceiver's own reset, held and then
    // waited out; `SE0_CYCLES` and `LINE_TRIES` are the upstream peripheral
    // Link's and `usb_ulpi_link`'s own parameters say what each measured.
    parameter integer RESET_CYCLES = 300,
    parameter integer SE0_CYCLES   = 150,
    parameter integer LINE_TRIES   = 40000,
    parameter integer REG_TRIES    = 16,
    // The vendor register each transceiver is given before anything a bus can
    // see. A Cynthion exchanges DP and DM between **every** transceiver and its
    // connector, and `39h` bit 1 of the Microchip USB334x undoes it, so both
    // ports want the same value — but they are two parameters because that is a
    // property of a board and a board may not be symmetrical.
    parameter [5:0]  UP_VENDOR_ADDR = 6'h00,
    parameter [7:0]  UP_VENDOR_DATA = 8'h00,
    parameter [5:0]  DN_VENDOR_ADDR = 6'h00,
    parameter [7:0]  DN_VENDOR_DATA = 8'h00,

    // Cycles of an idle bus before an answer goes out, on each side. ULPI 1.1
    // Table 10 allows a full-speed Link 7 to 18 and nine is the middle of it.
    parameter [6:0]  TURNAROUND    = 7'd9,

    // `usb_host_sie`'s, whose own header says what each is and why.
    parameter integer FRAME_CYCLES   = 60000,
    parameter integer TIMEOUT_CYCLES = 4096,
    parameter integer RETRIES        = 3,
    parameter integer TX_PER_BYTE    = 48,
    parameter integer TX_OVERHEAD    = 128,
    parameter integer GAP_CYCLES     = 12,

    // `usb_proxy_dn`'s.
    parameter integer DEBOUNCE_CYCLES = 6_000_000,
    parameter integer RESET_HOLD      = 900_000,
    parameter integer RESET_RECOVERY  = 1_200_000,
    parameter [1:0]   FS_LINE         = 2'b01,

    // The largest packet the relay forwards; `usb_proxy_relay`'s parameter of
    // the same name says why a power of two and why 64 relays anything.
    parameter [6:0]  MAXPKT = 7'd64
) (
    input  wire       clk60,
    input  wire       rst_n,

    // THE UPSTREAM TRANSCEIVER — the PC's. The three-state buffers belong to
    // the top of the design, as they do for every bidirectional pin in this
    // library, and so does forwarding this clock to the transceiver's clock
    // pin.
    input  wire [7:0] up_data_i,
    output wire [7:0] up_data_o,
    output wire       up_data_oe,
    input  wire       up_dir,
    input  wire       up_nxt,
    output wire       up_stp,
    output wire       up_rst_n,

    // THE DOWNSTREAM TRANSCEIVER — the device's.
    input  wire [7:0] dn_data_i,
    output wire [7:0] dn_data_o,
    output wire       dn_data_oe,
    input  wire       dn_dir,
    input  wire       dn_nxt,
    output wire       dn_stp,
    output wire       dn_rst_n,

    // The hub, as the PC has made it.
    output wire [6:0] address,
    output wire       configured,
    output wire       usb_reset,
    output wire       phy_ready,
    output wire       port_power,
    output wire       port_enabled,
    output wire       port_suspended,
    output wire       port_reset,

    // The downstream port, as it is.
    output wire       dn_phy_ready,
    output wire       dn_attached,
    output wire       dn_low_speed,
    output wire [1:0] dn_line_state,
    output wire [1:0] dn_vbus_state,
    output wire [3:0] dn_stage,
    output wire       dn_reg_failed,
    output wire [10:0] dn_frame,

    // The forwarding, as it is going. `proxied` is the one bit that says the
    // path was reached at all: the PC has addressed something behind the port.
    output wire       proxied,
    output wire       ctrl_active,
    output wire [1:0] job,
    // One cycle each: a SETUP taken and forwarded, and a transaction forwarded
    // that is **not** part of a control transfer. `usb_proxy_relay`'s own port
    // comment says what question each answers that a console cannot otherwise
    // ask.
    output wire       setup_seen,
    output wire       data_fwd
);
    // =================================================================
    // The upstream bus: the hub and the relay share one Link.
    // =================================================================
    wire [7:0] up_rx_data;
    wire       up_rx_valid, up_rx_eop, up_rx_active, up_line_idle, up_bus_reset;
    wire [6:0] up_tx_index;
    wire       up_tx_busy;

    wire       h_tx_start, h_tx_with_data;
    wire [3:0] h_tx_pid;
    wire [6:0] h_tx_len;
    wire [7:0] h_tx_byte;

    wire       p_tx_start, p_tx_with_data;
    wire [3:0] p_tx_pid;
    wire [6:0] p_tx_len;
    wire [7:0] p_tx_byte;

    wire       owns;

    // The transmitter, to whichever of the two the last token named. This is
    // `usb_dev_core`'s own multiplexer one level up, and the header says why it
    // cannot collide.
    wire       up_tx_start     = owns ? p_tx_start     : h_tx_start;
    wire [3:0] up_tx_pid       = owns ? p_tx_pid       : h_tx_pid;
    wire       up_tx_with_data = owns ? p_tx_with_data : h_tx_with_data;
    wire [6:0] up_tx_len       = owns ? p_tx_len       : h_tx_len;
    wire [7:0] up_tx_byte      = owns ? p_tx_byte      : h_tx_byte;

    usb_ulpi_link #(
        .RESET_CYCLES (RESET_CYCLES),
        .SE0_CYCLES   (SE0_CYCLES),
        .LINE_TRIES   (LINE_TRIES),
        .VENDOR_ADDR  (UP_VENDOR_ADDR),
        .VENDOR_DATA  (UP_VENDOR_DATA)
    ) u_up_link (
        .clk          (clk60),
        .rst_n        (rst_n),
        .ulpi_dir     (up_dir),
        .ulpi_nxt     (up_nxt),
        .ulpi_data_i  (up_data_i),
        .ulpi_data_o  (up_data_o),
        .ulpi_data_oe (up_data_oe),
        .ulpi_stp     (up_stp),
        .ulpi_rst_n   (up_rst_n),
        .rx_data      (up_rx_data),
        .rx_valid     (up_rx_valid),
        .rx_eop       (up_rx_eop),
        .rx_active    (up_rx_active),
        .line_idle    (up_line_idle),
        .bus_reset    (up_bus_reset),
        .tx_start     (up_tx_start),
        .tx_pid       (up_tx_pid),
        .tx_with_data (up_tx_with_data),
        .tx_len       (up_tx_len),
        .tx_index     (up_tx_index),
        .tx_byte      (up_tx_byte),
        .tx_busy      (up_tx_busy),
        .phy_ready    (phy_ready)
    );

    // =================================================================
    // The hub.
    // =================================================================
    wire reset_done;

    usb_hub #(
        .VID        (VID),
        .PID        (PID),
        .CFG_ATTR   (CFG_ATTR),
        .CFG_POWER  (CFG_POWER),
        .MAXPKT0    (MAXPKT0),
        .BUF_RAM    (BUF_RAM),
        .TURNAROUND (TURNAROUND)
    ) u_hub (
        .clk          (clk60),
        .rst_n        (rst_n),
        .rx_data      (up_rx_data),
        .rx_valid     (up_rx_valid),
        .rx_eop       (up_rx_eop),
        .rx_active    (up_rx_active),
        .line_idle    (up_line_idle),
        .bus_reset    (up_bus_reset),
        .tx_start     (h_tx_start),
        .tx_pid       (h_tx_pid),
        .tx_with_data (h_tx_with_data),
        .tx_len       (h_tx_len),
        .tx_index     (up_tx_index),
        .tx_byte      (h_tx_byte),
        .tx_busy      (up_tx_busy),
        .address      (address),
        .configured   (configured),
        // **`port_attached` and not `up`**, which `ip/usb/usb_hub/README.md` §5
        // argues: USB 2.0 §11.24.2.7.1 makes PORT_CONNECTION a device being
        // present, and whether anybody has enumerated it is not the PC's
        // business. In a proxy there is nothing else it could be — nothing here
        // enumerates.
        .port_attached   (dn_attached),
        .port_low_speed  (dn_low_speed),
        .port_power      (port_power),
        .port_enabled    (port_enabled),
        .port_suspended  (port_suspended),
        .port_reset      (port_reset),
        .port_reset_done (reset_done)
    );

    assign usb_reset = up_bus_reset;

    // =================================================================
    // The downstream bus: a host Link and the transaction engine.
    // =================================================================
    wire [7:0] dn_rx_data;
    wire       dn_rx_valid, dn_rx_eop, dn_rx_active;
    wire       dn_tx_start, dn_tx_busy, dn_tx_abort;
    wire [3:0] dn_tx_pid;
    wire [1:0] dn_tx_mode;
    wire [6:0] dn_tx_len, dn_tx_index;
    wire [7:0] dn_tx_byte;

    wire       reg_start, reg_write;
    wire [5:0] reg_addr;
    wire [7:0] reg_wdata;
    wire       reg_done, reg_ok, reg_busy;

    wire       sof_en, dn_ready;
    wire       trn_start, trn_busy, trn_done, trn_toggle;
    wire [1:0] trn_kind;
    wire [6:0] trn_addr, trn_len, trn_index;
    wire [3:0] trn_endp;
    wire [7:0] trn_byte;
    wire [2:0] trn_status;
    wire [3:0] trn_rx_pid;
    wire [6:0] trn_rx_len;
    wire [7:0] in_byte;
    wire       in_push;
    wire [6:0] in_index;

    usb_ulpi_host_link #(
        .RESET_CYCLES (RESET_CYCLES),
        .VENDOR_ADDR  (DN_VENDOR_ADDR),
        .VENDOR_DATA  (DN_VENDOR_DATA),
        .REG_TRIES    (REG_TRIES)
    ) u_dn_link (
        .clk          (clk60),
        .rst_n        (rst_n),
        .ulpi_dir     (dn_dir),
        .ulpi_nxt     (dn_nxt),
        .ulpi_data_i  (dn_data_i),
        .ulpi_data_o  (dn_data_o),
        .ulpi_data_oe (dn_data_oe),
        .ulpi_stp     (dn_stp),
        .ulpi_rst_n   (dn_rst_n),
        .rx_data      (dn_rx_data),
        .rx_valid     (dn_rx_valid),
        .rx_eop       (dn_rx_eop),
        .rx_active    (dn_rx_active),
        .rx_error     (),
        .rx_cmd       (),
        .rx_cmd_seen  (),
        .line_state   (dn_line_state),
        .vbus_state   (dn_vbus_state),
        .id_pin       (),
        .tx_start     (dn_tx_start),
        .tx_pid       (dn_tx_pid),
        .tx_mode      (dn_tx_mode),
        .tx_len       (dn_tx_len),
        .tx_index     (dn_tx_index),
        .tx_byte      (dn_tx_byte),
        .tx_busy      (dn_tx_busy),
        .tx_done      (),
        .tx_abort     (dn_tx_abort),
        .reg_start    (reg_start),
        .reg_write    (reg_write),
        .reg_addr     (reg_addr),
        .reg_wdata    (reg_wdata),
        // Nothing reads a register here. The only register transaction in a
        // proxy is the pair of writes that drive the port's reset, and a
        // write's answer is `reg_ok`; a design that wants to read the
        // transceiver uses `ip/usb/usb_host_ulpi` with `enum_en` low, which
        // is what that block's read-only instrument mode is for.
        .reg_rdata    (),
        .reg_done     (reg_done),
        .reg_ok       (reg_ok),
        .reg_busy     (reg_busy),
        .phy_ready    (dn_phy_ready)
    );

    usb_host_sie #(
        .FRAME_CYCLES   (FRAME_CYCLES),
        .TIMEOUT_CYCLES (TIMEOUT_CYCLES),
        .RETRIES        (RETRIES),
        .TX_PER_BYTE    (TX_PER_BYTE),
        .TX_OVERHEAD    (TX_OVERHEAD),
        .GAP_CYCLES     (GAP_CYCLES),
        .TURNAROUND     (TURNAROUND)
    ) u_sie (
        .clk        (clk60),
        .rst_n      (rst_n),
        .rx_data    (dn_rx_data),
        .rx_valid   (dn_rx_valid),
        .rx_eop     (dn_rx_eop),
        .rx_active  (dn_rx_active),
        .tx_start   (dn_tx_start),
        .tx_pid     (dn_tx_pid),
        .tx_mode    (dn_tx_mode),
        .tx_len     (dn_tx_len),
        .tx_index   (dn_tx_index),
        .tx_byte    (dn_tx_byte),
        .tx_busy    (dn_tx_busy),
        .tx_abort   (dn_tx_abort),
        .sof_en     (sof_en),
        .frame      (dn_frame),
        .sof_sent   (),
        .trn_start  (trn_start),
        .trn_kind   (trn_kind),
        .trn_addr   (trn_addr),
        .trn_endp   (trn_endp),
        .trn_toggle (trn_toggle),
        .trn_len    (trn_len),
        .trn_byte   (trn_byte),
        .trn_index  (trn_index),
        .trn_busy   (trn_busy),
        .trn_done   (trn_done),
        .trn_status (trn_status),
        .trn_rx_pid (trn_rx_pid),
        .trn_rx_len (trn_rx_len),
        .in_byte    (in_byte),
        .in_push    (in_push),
        .in_index   (in_index)
    );

    usb_proxy_dn #(
        .DEBOUNCE_CYCLES (DEBOUNCE_CYCLES),
        .RESET_HOLD      (RESET_HOLD),
        .RESET_RECOVERY  (RESET_RECOVERY),
        .FS_LINE         (FS_LINE)
    ) u_dn (
        .clk        (clk60),
        .rst_n      (rst_n),
        .phy_ready  (dn_phy_ready),
        .line_state (dn_line_state),
        .reg_start  (reg_start),
        .reg_write  (reg_write),
        .reg_addr   (reg_addr),
        .reg_wdata  (reg_wdata),
        .reg_done   (reg_done),
        .reg_ok     (reg_ok),
        .reg_busy   (reg_busy),
        .reset_req  (port_reset),
        .reset_done (reset_done),
        .attached   (dn_attached),
        .low_speed  (dn_low_speed),
        .dn_ready   (dn_ready),
        .sof_en     (sof_en),
        .stage      (dn_stage),
        .reg_failed (dn_reg_failed)
    );

    // =================================================================
    // The forwarding.
    // =================================================================
    usb_proxy_relay #(
        .MAXPKT     (MAXPKT),
        .TURNAROUND (TURNAROUND)
    ) u_relay (
        .clk          (clk60),
        .rst_n        (rst_n),
        .rx_data      (up_rx_data),
        .rx_valid     (up_rx_valid),
        .rx_eop       (up_rx_eop),
        .rx_active    (up_rx_active),
        .line_idle    (up_line_idle),
        .bus_reset    (up_bus_reset),
        .tx_start     (p_tx_start),
        .tx_pid       (p_tx_pid),
        .tx_with_data (p_tx_with_data),
        .tx_len       (p_tx_len),
        .tx_index     (up_tx_index),
        .tx_byte      (p_tx_byte),
        .tx_busy      (up_tx_busy),
        .hub_addr     (address),
        // The port is the relay's only while the hub is configured and the PC
        // has enabled the port, which it does by resetting it — so a token is
        // never forwarded to a device the PC has not reset through this port.
        .enabled      (configured & port_enabled),
        .port_reset   (port_reset),
        .owns         (owns),
        .dn_ready     (dn_ready),
        .trn_start    (trn_start),
        .trn_kind     (trn_kind),
        .trn_addr     (trn_addr),
        .trn_endp     (trn_endp),
        .trn_toggle   (trn_toggle),
        .trn_len      (trn_len),
        .trn_byte     (trn_byte),
        .trn_index    (trn_index),
        .trn_busy     (trn_busy),
        .trn_done     (trn_done),
        .trn_status   (trn_status),
        .trn_rx_pid   (trn_rx_pid),
        .trn_rx_len   (trn_rx_len),
        .in_byte      (in_byte),
        .in_push      (in_push),
        .in_index     (in_index),
        .proxied      (proxied),
        .ctrl_active  (ctrl_active),
        .job          (job),
        .setup_seen   (setup_seen),
        .data_fwd     (data_fwd)
    );
endmodule
