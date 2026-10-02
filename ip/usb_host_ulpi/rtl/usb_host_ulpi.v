// usb_host_ulpi — a USB 2.0 full-speed **host** behind a ULPI
// transceiver: the three blocks of this package wired together, and the
// transceiver's register port brought out so a design can look before it
// drives.
//
// What it does
//   `usb_ulpi_host_link` does the ULPI bus and the transceiver's
//   registers, `usb_host_sie` does the frames, the tokens and the
//   transactions, and `usb_host_enum` does what a host does to a device
//   that has just appeared. Each of their headers states its own subject;
//   this file is the wiring and one decision, which is whose the register
//   port is.
//
//   **`enum_en` low is a read-only instrument.** The transceiver is
//   brought out of reset and configured as a full-speed host — which is
//   two 15 kOhm pull-downs and nothing else on the pair — and then nothing
//   happens: no bus reset, no SOF, no token. The `reg_*` port is the
//   design's, so it can read Vendor ID, Product ID, Function Control, OTG
//   Control, the Debug register and whatever vendor register the board
//   has, and `rx_cmd` holds the last receive command with `VbusState`,
//   `LineState` and the ID pin in it.
//
//   That order is deliberate and it is the only order in which the three
//   questions a new port raises can be answered at all:
//
//     * **is the transceiver there** — it answers a register read with the
//       byte its datasheet gives;
//     * **does the port have power** — `VbusState`, which no amount of
//       driving the bus will tell you and which decides whether a device
//       can pull anything up;
//     * **which line is the device on** — `LineState`, which says both
//       that something is attached and, since a full-speed device pulls
//       **D+** up, whether the board exchanges DP and DM on the way to the
//       connector.
//
//   Raising `enum_en` hands the register port to `usb_host_enum` and the
//   sequence starts. Raise it when the design's own register traffic is
//   finished: the port is a multiplexer and not an arbiter, and switching
//   it in the middle of a transaction would abandon one.
//
// What it does not do
//   Everything the three blocks do not, and each says so: full speed only,
//   one device, one transaction at a time, no hub, no suspend, no VBUS
//   switching and nothing after `SET_CONFIGURATION`.
//
//   **It does not switch VBUS and it does not read a switch.** Whether a
//   port has power is a board's wiring — on the board this was written for
//   it is a bidirectional switch between two connectors, and enabling the
//   wrong one ties two hosts' supplies together. So the only thing this
//   block does about power is **report** `VbusState`, and a design that
//   drives a switch does it in its own top level where the board's own
//   comment about it can be read.
//
//   The eight data lines are split into `ulpi_data_i`, `ulpi_data_o` and
//   `ulpi_data_oe`, as every bidirectional pin in this library is, and
//   `dir` turns them around. The three-state buffers belong to the top of
//   the design, as does forwarding the 60 MHz clock to the transceiver's
//   clock pin when the board asks the FPGA for it.
module usb_host_ulpi #(
    // `usb_ulpi_host_link`'s.
    parameter integer RESET_CYCLES = 300,
    parameter [5:0]   VENDOR_ADDR  = 6'h00,
    parameter [7:0]   VENDOR_DATA  = 8'h00,
    parameter integer REG_TRIES    = 16,
    // `usb_host_sie`'s.
    parameter integer FRAME_CYCLES   = 60000,
    parameter integer TIMEOUT_CYCLES = 4096,
    parameter integer RETRIES        = 3,
    parameter integer TX_PER_BYTE    = 48,
    parameter integer TX_OVERHEAD    = 128,
    parameter integer GAP_CYCLES     = 12,
    parameter integer TURNAROUND     = 9,
    // `usb_host_enum`'s.
    parameter integer DEBOUNCE_CYCLES = 6_000_000,
    parameter integer RESET_HOLD      = 900_000,
    parameter integer RESET_RECOVERY  = 1_200_000,
    parameter integer ADDR_SETTLE     = 300_000,
    parameter integer NAK_CYCLES      = 3_000_000,
    parameter [6:0]   DEV_ADDR        = 7'd1,
    parameter [6:0]   MAXPKT0_MIN     = 7'd8,
    parameter [6:0]   DESC_MAX        = 7'd64,
    parameter [1:0]   FS_LINE         = 2'b01
) (
    input  wire       clk60,
    input  wire       rst_n,

    // The ULPI bus to the transceiver.
    input  wire [7:0] ulpi_data_i,
    output wire [7:0] ulpi_data_o,
    output wire       ulpi_data_oe,
    input  wire       ulpi_dir,
    input  wire       ulpi_nxt,
    output wire       ulpi_stp,
    output wire       ulpi_rst_n,

    // The transceiver, as it was found.
    output wire       phy_ready,
    output wire [7:0] rx_cmd,
    output wire       rx_cmd_seen,
    output wire [1:0] line_state,
    output wire [1:0] vbus_state,
    output wire       id_pin,

    // The register port, which is this design's while `enum_en` is low.
    input  wire       enum_en,
    input  wire       reg_start,
    input  wire       reg_write,
    input  wire [5:0] reg_addr,
    input  wire [7:0] reg_wdata,
    output wire [7:0] reg_rdata,
    output wire       reg_done,
    output wire       reg_ok,
    output wire       reg_busy,

    // The bus, while it is running.
    output wire [10:0] frame,
    output wire        sof_sent,

    // What was found on the other end.
    output wire [4:0]  stage,
    output wire        attached,
    output wire        low_speed,
    output wire        up,
    output wire        failed,
    output wire [4:0]  fail_stage,
    output wire [2:0]  fail_status,
    output wire [6:0]  dev_addr,
    output wire [6:0]  maxpkt0,
    output wire [15:0] cfg_total,
    output wire [7:0]  cfg_value,

    // The descriptor bytes, as they arrive: `desc_data` belongs at
    // `desc_index` of descriptor `desc_tag`, and `desc_done` says that one
    // is whole and `desc_len` long.
    output wire [7:0]  desc_data,
    output wire        desc_valid,
    output wire [6:0]  desc_index,
    output wire [1:0]  desc_tag,
    output wire        desc_done,
    output wire [6:0]  desc_len
);
    wire [7:0] rx_data;
    wire       rx_valid, rx_eop, rx_active, rx_error;
    wire       tx_start, tx_busy, tx_done, tx_abort;
    wire [3:0] tx_pid;
    wire [1:0] tx_mode;
    wire [6:0] tx_len, tx_index;
    wire [7:0] tx_byte;

    // The register port of the Link, and the two things that ask for it.
    wire       l_reg_done, l_reg_ok, l_reg_busy;
    wire [7:0] l_reg_rdata;
    wire       e_reg_start, e_reg_write;
    wire [5:0] e_reg_addr;
    wire [7:0] e_reg_wdata;

    wire       sof_en;
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
        .VENDOR_ADDR  (VENDOR_ADDR),
        .VENDOR_DATA  (VENDOR_DATA),
        .REG_TRIES    (REG_TRIES)
    ) u_link (
        .clk          (clk60),
        .rst_n        (rst_n),
        .ulpi_dir     (ulpi_dir),
        .ulpi_nxt     (ulpi_nxt),
        .ulpi_data_i  (ulpi_data_i),
        .ulpi_data_o  (ulpi_data_o),
        .ulpi_data_oe (ulpi_data_oe),
        .ulpi_stp     (ulpi_stp),
        .ulpi_rst_n   (ulpi_rst_n),
        .rx_data      (rx_data),
        .rx_valid     (rx_valid),
        .rx_eop       (rx_eop),
        .rx_active    (rx_active),
        .rx_error     (rx_error),
        .rx_cmd       (rx_cmd),
        .rx_cmd_seen  (rx_cmd_seen),
        .line_state   (line_state),
        .vbus_state   (vbus_state),
        .id_pin       (id_pin),
        .tx_start     (tx_start),
        .tx_pid       (tx_pid),
        .tx_mode      (tx_mode),
        .tx_len       (tx_len),
        .tx_index     (tx_index),
        .tx_byte      (tx_byte),
        .tx_busy      (tx_busy),
        .tx_done      (tx_done),
        .tx_abort     (tx_abort),
        // Whichever of the two owns the port. A multiplexer and not an
        // arbiter: `enum_en` is meant to be raised once, when the design's
        // own register traffic is over.
        .reg_start    (enum_en ? e_reg_start : reg_start),
        .reg_write    (enum_en ? e_reg_write : reg_write),
        .reg_addr     (enum_en ? e_reg_addr  : reg_addr),
        .reg_wdata    (enum_en ? e_reg_wdata : reg_wdata),
        .reg_rdata    (l_reg_rdata),
        .reg_done     (l_reg_done),
        .reg_ok       (l_reg_ok),
        .reg_busy     (l_reg_busy),
        .phy_ready    (phy_ready)
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
        .rx_data    (rx_data),
        .rx_valid   (rx_valid),
        .rx_eop     (rx_eop),
        .rx_active  (rx_active),
        .tx_start   (tx_start),
        .tx_pid     (tx_pid),
        .tx_mode    (tx_mode),
        .tx_len     (tx_len),
        .tx_index   (tx_index),
        .tx_byte    (tx_byte),
        .tx_busy    (tx_busy),
        .tx_abort   (tx_abort),
        .sof_en     (sof_en),
        .frame      (frame),
        .sof_sent   (sof_sent),
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

    usb_host_enum #(
        .DEBOUNCE_CYCLES (DEBOUNCE_CYCLES),
        .RESET_HOLD      (RESET_HOLD),
        .RESET_RECOVERY  (RESET_RECOVERY),
        .ADDR_SETTLE     (ADDR_SETTLE),
        .NAK_CYCLES      (NAK_CYCLES),
        .DEV_ADDR        (DEV_ADDR),
        .MAXPKT0_MIN     (MAXPKT0_MIN),
        .DESC_MAX        (DESC_MAX),
        .FS_LINE         (FS_LINE)
    ) u_enum (
        .clk         (clk60),
        .rst_n       (rst_n),
        .en          (enum_en),
        .phy_ready   (phy_ready),
        .line_state  (line_state),
        .reg_start   (e_reg_start),
        .reg_write   (e_reg_write),
        .reg_addr    (e_reg_addr),
        .reg_wdata   (e_reg_wdata),
        .reg_rdata   (l_reg_rdata),
        .reg_done    (l_reg_done & enum_en),
        .reg_ok      (l_reg_ok),
        .reg_busy    (l_reg_busy),
        .sof_en      (sof_en),
        .trn_start   (trn_start),
        .trn_kind    (trn_kind),
        .trn_addr    (trn_addr),
        .trn_endp    (trn_endp),
        .trn_toggle  (trn_toggle),
        .trn_len     (trn_len),
        .trn_byte    (trn_byte),
        .trn_index   (trn_index),
        .trn_busy    (trn_busy),
        .trn_done    (trn_done),
        .trn_status  (trn_status),
        .trn_rx_pid  (trn_rx_pid),
        .trn_rx_len  (trn_rx_len),
        .in_byte     (in_byte),
        .in_push     (in_push),
        .in_index    (in_index),
        .stage       (stage),
        .attached    (attached),
        .low_speed   (low_speed),
        .up          (up),
        .failed      (failed),
        .fail_stage  (fail_stage),
        .fail_status (fail_status),
        .dev_addr    (dev_addr),
        .maxpkt0     (maxpkt0),
        .cfg_total   (cfg_total),
        .cfg_value   (cfg_value),
        .desc_data   (desc_data),
        .desc_valid  (desc_valid),
        .desc_index  (desc_index),
        .desc_tag    (desc_tag),
        .desc_done   (desc_done),
        .desc_len    (desc_len)
    );

    // What the design sees of the register port. `reg_done` is gated the
    // other way from the enumerator's so that neither is told about a
    // transaction the other asked for.
    assign reg_rdata = l_reg_rdata;
    assign reg_done  = l_reg_done & ~enum_en;
    assign reg_ok    = l_reg_ok;
    assign reg_busy  = l_reg_busy;
endmodule
