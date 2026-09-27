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
//   and a transmitter that only one of them may have at a time.
//
// THE TRANSMITTER, AND WHO OWNS IT
//   A USB device only ever speaks when it has been asked to, and the
//   asking is a token. So the endpoint that may answer is the endpoint the
//   **last token named**, which is one register:
//
//     owner <= (tok_endp != 0)   on any token addressed to this device
//
//   Everything after that token belongs to the same endpoint — the data
//   packet of an OUT, the handshake of an IN — because a host does not
//   interleave transactions on one device. `sel` into each endpoint is
//   that register, it gates the endpoint's turnaround counter, and the
//   whole arbitration is a multiplexer. There is no request-and-grant and
//   no round robin, because there is never a second answer waiting: the
//   endpoint that has not been asked has nothing to say.
//
//   A token for an endpoint number neither of them has hands ownership to
//   the data endpoint, which then ignores it because the number is not
//   its own. Nothing answers, and the host retries and gives up, which is
//   what a device with no such endpoint is supposed to do.
//
// What it does not do
//   One bulk endpoint pair. A second pair is a second `usb_bulk_ep` with
//   another `ENDP`, another pair of byte interfaces on this module's port
//   list, and `owner` widened from a bit to a number — at which point the
//   width of that register is the number of endpoints and not one more,
//   for the reason `usb_ctrl_ep`'s `stage` gives at length.
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
    parameter [3:0]   MAXPKT       = 4'd8,
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
    output wire [3:0] tx_len,
    input  wire [3:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured,

    // The data endpoint's bytes.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit
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
    wire [3:0]  dat_len;
    wire [63:0] dat;

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
        .dat          (dat)
    );

    // -----------------------------------------------------------------
    // Who the last token asked.
    // -----------------------------------------------------------------
    reg owner;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) owner <= 1'b0;
        else if (pkt && pkt_is_token && tok_ok && tok_addr == address)
            owner <= (tok_endp != 4'd0);
    end

    // -----------------------------------------------------------------
    // Endpoint 0.
    // -----------------------------------------------------------------
    wire       c_tx_start, c_tx_with_data;
    wire [3:0] c_tx_pid, c_tx_len;
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
        .sel          (~owner),
        .tx_start     (c_tx_start),
        .tx_pid       (c_tx_pid),
        .tx_with_data (c_tx_with_data),
        .tx_len       (c_tx_len),
        .tx_index     (tx_index),
        .tx_byte      (c_tx_byte),
        .tx_busy      (tx_busy),
        .address      (address),
        .configured   (configured),
        .ep_reset     (ep_reset),
        .ep_clear     (ep_clear),
        .ep_clear_ep  (ep_clear_ep)
    );

    // -----------------------------------------------------------------
    // The data endpoint.
    // -----------------------------------------------------------------
    wire       b_tx_start, b_tx_with_data;
    wire [3:0] b_tx_pid, b_tx_len;
    wire [7:0] b_tx_byte;

    usb_bulk_ep #(
        .ENDP       (DATA_ENDP),
        .MAXPKT     (MAXPKT),
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
        .dat          (dat),
        .address      (address),
        .line_idle    (line_idle),
        .bus_reset    (bus_reset),
        .ep_reset     (ep_reset),
        .ep_clear     (ep_clear),
        .ep_clear_ep  (ep_clear_ep),
        .sel          (owner),
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
    // The transmitter, to whichever endpoint the token named.
    // -----------------------------------------------------------------
    assign tx_start     = owner ? b_tx_start     : c_tx_start;
    assign tx_pid       = owner ? b_tx_pid       : c_tx_pid;
    assign tx_with_data = owner ? b_tx_with_data : c_tx_with_data;
    assign tx_len       = owner ? b_tx_len       : c_tx_len;
    assign tx_byte      = owner ? b_tx_byte      : c_tx_byte;
endmodule
