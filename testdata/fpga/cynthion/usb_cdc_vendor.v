// usb_cdc_vendor — a composite USB device for the MCU-flash bridge: the CDC
// ACM serial port of `ip/usb/usb_cdc_acm` plus a third, vendor-specific
// interface ("smartcard-ctl", class FFh) with a bulk pair for the control
// protocol.
//
// It is `usb_cdc_acm` widened by one interface and one bulk endpoint. The
// control endpoint (`usb_ctrl_ep`), the packet decoder (`usb_pkt_rx`), the
// bulk endpoints (`usb_bulk_ep`) and the line-coding class requests
// (`usb_cdc_req`) are the library's own, unchanged; what is new here is the
// three-interface descriptor, a third bulk endpoint on the device core, and a
// class hook that answers the one string descriptor the host finds the vendor
// interface by. `usb_ctrl_ep`'s `DESC_MAX` is raised to 128 because the
// descriptor set is 81 bytes rather than a CDC port's 58.
//
// The CDC half is byte-for-byte what `usb_cdc_acm` presents, so the serial
// port enumerates as /dev/ttyACM* (1209:0001) exactly as `smartcard_serial.v`
// does today; the vendor half is claimed by `ledger_console`'s `bridge.rs`,
// which finds it by its iInterface string with a class-FFh fallback.
//
// THE DESCRIPTORS — 81 bytes, three interfaces, five endpoints:
//   IF0 CDC communications (02/02), EP 82 interrupt IN (SERIAL_STATE)
//   IF1 CDC data (0A),        EP 01 bulk OUT, EP 81 bulk IN   (the card stream)
//   IF2 vendor (FF),          EP 03 bulk OUT, EP 83 bulk IN   (control)
//   with the CDC functional descriptors (header, call-management, ACM, union)
//   between IF0 and its endpoint, as `ip/usb/usb_cdc_acm` states them.
// bNumInterfaces, the three bNumEndpoints and wTotalLength are placeholders
// here; `usb_ctrl_ep` counts them at elaboration.
module usb_cdc_vendor #(
    parameter [15:0] VID        = 16'h1209,
    parameter [15:0] PID        = 16'h0001,
    parameter [7:0]  CFG_ATTR   = 8'h80,
    parameter [7:0]  CFG_POWER  = 8'd50,
    parameter [6:0]  MAXPKT     = 7'd64,
    parameter [6:0]  MAXPKT0    = 7'd64,
    parameter        BUF_RAM    = 1,
    parameter [6:0]  TURNAROUND = 7'd8
) (
    input  wire       clk,
    input  wire       rst_n,

    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    input  wire       rx_eop,
    input  wire       rx_active,
    input  wire       line_idle,
    input  wire       bus_reset,

    output wire       tx_start,
    output wire [3:0] tx_pid,
    output wire       tx_with_data,
    output wire [6:0] tx_len,
    input  wire [6:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured,

    // CDC serial port bytes (the card's stream).
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit,

    output wire [31:0] baud,
    output wire [7:0]  char_format,
    output wire [7:0]  parity,
    output wire [7:0]  data_bits,
    output wire        dtr,
    output wire        rts,
    input  wire [6:0]  serial_state,

    // Vendor control interface bytes.
    output wire [7:0] vout_data,
    output wire       vout_valid,
    output wire       vout_last,
    input  wire       vout_ready,
    input  wire [7:0] vin_data,
    input  wire       vin_valid,
    output wire       vin_ready,
    input  wire       vin_commit
);
    localparam [7:0] COMM_IFACE   = 8'd0;
    localparam [7:0] DATA_IFACE   = 8'd1;
    localparam [7:0] VENDOR_IFACE = 8'd2;
    localparam [3:0] NOTIF_ENDP   = 4'd2;
    localparam [3:0] DATA_ENDP    = 4'd1;
    localparam [3:0] VENDOR_ENDP  = 4'd3;
    localparam [6:0] NOTIF_MAXPKT = 7'd16;
    localparam integer NOTIF_BYTES = 10;
    localparam [3:0]   NOTIF_LAST  = NOTIF_BYTES - 1;
    localparam [7:0] NOTIF_INTERVAL = 8'd16;

    localparam [7:0] CLASS_COMM   = 8'h02;
    localparam [7:0] CLASS_DATA   = 8'h0A;
    localparam [7:0] CLASS_VENDOR = 8'hFF;
    localparam [7:0] SUBCLASS_ACM = 8'h02;
    localparam [7:0] PROTOCOL_NONE = 8'h00;
    localparam [7:0] CS_INTERFACE  = 8'h24;
    localparam [7:0] FD_HEADER     = 8'h00;
    localparam [7:0] FD_CALL_MGMT  = 8'h01;
    localparam [7:0] FD_ACM        = 8'h02;
    localparam [7:0] FD_UNION      = 8'h06;
    localparam [7:0] ACM_CAPS      = 8'h02;
    localparam [7:0] CALL_CAPS     = 8'h00;
    localparam [7:0] CDC_VER_LO    = 8'h10;
    localparam [7:0] CDC_VER_HI    = 8'h01;
    localparam [7:0] EP_BULK       = 8'h02;
    localparam [7:0] EP_INTERRUPT  = 8'h03;
    localparam [7:0] DESC_INTERFACE = 8'd4;
    localparam [7:0] DESC_ENDPOINT  = 8'd5;
    // iInterface of the vendor interface: string index 1, "smartcard-ctl".
    localparam [7:0] STR_SMARTCARD = 8'd1;

    localparam integer IFACE_BYTES = 81;
    localparam [IFACE_BYTES*8-1:0] IFACE_DESC = {
        // INTERFACE 0 — communications.
        8'd9, DESC_INTERFACE, COMM_IFACE, 8'd0, 8'd0,
              CLASS_COMM, SUBCLASS_ACM, PROTOCOL_NONE, 8'd0,
        8'd5, CS_INTERFACE, FD_HEADER, CDC_VER_LO, CDC_VER_HI,
        8'd5, CS_INTERFACE, FD_CALL_MGMT, CALL_CAPS, DATA_IFACE,
        8'd4, CS_INTERFACE, FD_ACM, ACM_CAPS,
        8'd5, CS_INTERFACE, FD_UNION, COMM_IFACE, DATA_IFACE,
        8'd7, DESC_ENDPOINT, {4'h8, NOTIF_ENDP}, EP_INTERRUPT,
              {1'b0, NOTIF_MAXPKT}, 8'd0, NOTIF_INTERVAL,
        // INTERFACE 1 — CDC data.
        8'd9, DESC_INTERFACE, DATA_IFACE, 8'd0, 8'd0,
              CLASS_DATA, 8'h00, PROTOCOL_NONE, 8'd0,
        8'd7, DESC_ENDPOINT, {4'h0, DATA_ENDP}, EP_BULK,
              {1'b0, MAXPKT}, 8'd0, 8'd0,
        8'd7, DESC_ENDPOINT, {4'h8, DATA_ENDP}, EP_BULK,
              {1'b0, MAXPKT}, 8'd0, 8'd0,
        // INTERFACE 2 — vendor control ("smartcard-ctl").
        8'd9, DESC_INTERFACE, VENDOR_IFACE, 8'd0, 8'd0,
              CLASS_VENDOR, 8'h00, PROTOCOL_NONE, STR_SMARTCARD,
        8'd7, DESC_ENDPOINT, {4'h0, VENDOR_ENDP}, EP_BULK,
              {1'b0, MAXPKT}, 8'd0, 8'd0,
        8'd7, DESC_ENDPOINT, {4'h8, VENDOR_ENDP}, EP_BULK,
              {1'b0, MAXPKT}, 8'd0, 8'd0
    };

    // -----------------------------------------------------------------
    // The class hook: CDC line coding, plus the string descriptor.
    // -----------------------------------------------------------------
    wire [63:0] class_setup;
    wire        class_req;
    wire [6:0]  class_index;
    wire [63:0] class_out;
    wire [6:0]  class_out_len;
    wire        class_out_valid;

    // CDC line-coding requests.
    wire        cdc_claim;
    wire [6:0]  cdc_len;
    wire [7:0]  cdc_byte;
    wire        class_reopened;
    usb_cdc_req #(.COMM_IFACE(COMM_IFACE)) u_req (
        .clk(clk), .rst_n(rst_n),
        .setup(class_setup), .req(class_req), .claim(cdc_claim), .len(cdc_len),
        .index(class_index), .resp(cdc_byte),
        .out_data(class_out), .out_len(class_out_len), .out_valid(class_out_valid),
        .baud(baud), .char_format(char_format), .parity(parity), .data_bits(data_bits),
        .dtr(dtr), .rts(rts), .reopened(class_reopened)
    );

    // GET_DESCRIPTOR(STRING): bmRequestType 80h, bRequest 06h, wValueHi 03h.
    // Endpoint 0 does not claim a STRING descriptor itself (it answers only
    // DEVICE and CONFIGURATION), so it is offered here. Index 0 is the LANGID
    // list; index 1 is "smartcard-ctl" in UTF-16LE.
    wire [7:0] s_bmreq = class_setup[7:0];
    wire [7:0] s_breq  = class_setup[15:8];
    wire [7:0] s_vlo   = class_setup[23:16];   // descriptor index
    wire [7:0] s_vhi   = class_setup[31:24];   // descriptor type
    wire str_claim = (s_bmreq == 8'h80) && (s_breq == 8'h06) && (s_vhi == 8'h03)
                   && ((s_vlo == 8'h00) || (s_vlo == 8'h01));
    wire [6:0] str_len = (s_vlo == 8'h00) ? 7'd4 : 7'd28;

    reg [7:0] str_byte;
    always @(*) begin
        if (s_vlo == 8'h00) begin
            case (class_index)
                7'd0:    str_byte = 8'd4;      // bLength
                7'd1:    str_byte = 8'd3;      // STRING
                7'd2:    str_byte = 8'h09;     // LANGID 0x0409, low byte
                7'd3:    str_byte = 8'h04;
                default: str_byte = 8'd0;
            endcase
        end else begin
            case (class_index)
                7'd0:    str_byte = 8'd28;     // bLength = 2 + 2*13
                7'd1:    str_byte = 8'd3;      // STRING
                7'd2:    str_byte = "s";
                7'd4:    str_byte = "m";
                7'd6:    str_byte = "a";
                7'd8:    str_byte = "r";
                7'd10:   str_byte = "t";
                7'd12:   str_byte = "c";
                7'd14:   str_byte = "a";
                7'd16:   str_byte = "r";
                7'd18:   str_byte = "d";
                7'd20:   str_byte = "-";
                7'd22:   str_byte = "c";
                7'd24:   str_byte = "t";
                7'd26:   str_byte = "l";
                default: str_byte = 8'd0;      // the UTF-16 high bytes, and past the end
            endcase
        end
    end

    wire        class_claim = cdc_claim | str_claim;
    wire [6:0]  class_len   = str_claim ? str_len  : cdc_len;
    wire [7:0]  class_byte  = str_claim ? str_byte : cdc_byte;

    // -----------------------------------------------------------------
    // SERIAL_STATE, into the notification endpoint (from usb_cdc_acm).
    // -----------------------------------------------------------------
    reg [3:0] nidx;
    reg       sending;
    reg [6:0] reported;
    reg       ever;

    function [7:0] notif_byte;
        input [3:0] i;
        begin
            case (i)
                4'd0:    notif_byte = 8'hA1;
                4'd1:    notif_byte = 8'h20;
                4'd2:    notif_byte = 8'h00;
                4'd3:    notif_byte = 8'h00;
                4'd4:    notif_byte = COMM_IFACE;
                4'd5:    notif_byte = 8'h00;
                4'd6:    notif_byte = 8'd2;
                4'd7:    notif_byte = 8'h00;
                4'd8:    notif_byte = {1'b0, serial_state};
                default: notif_byte = 8'h00;
            endcase
        end
    endfunction

    wire [7:0] notif_data   = notif_byte(nidx);
    wire       notif_valid  = sending;
    wire       notif_commit = sending & (nidx == NOTIF_LAST);
    wire       notif_ready;
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
            if (class_reopened) ever <= 1'b0;
            if (!configured) begin
                ever    <= 1'b0;
                sending <= 1'b0;
                nidx    <= 4'd0;
            end
        end
    end

    // -----------------------------------------------------------------
    // The device core, with a third (vendor) bulk endpoint.
    // -----------------------------------------------------------------
    usb_dev_core3 #(
        .VID(VID), .PID(PID),
        .DEV_CLASS(CLASS_COMM), .DEV_SUBCLASS(8'h00), .DEV_PROTOCOL(8'h00),
        .CFG_ATTR(CFG_ATTR), .CFG_POWER(CFG_POWER),
        .IFACE_BYTES(IFACE_BYTES), .IFACE_DESC(IFACE_DESC),
        .DESC_MAX(128),
        .DATA_ENDP(DATA_ENDP), .MAXPKT(MAXPKT),
        .NOTIF_ENDP(NOTIF_ENDP), .NOTIF_MAXPKT(NOTIF_MAXPKT),
        .VENDOR_ENDP(VENDOR_ENDP), .VENDOR_MAXPKT(MAXPKT),
        .MAXPKT0(MAXPKT0), .CLASS_MAX(28), .BUF_RAM(BUF_RAM), .TURNAROUND(TURNAROUND)
    ) u_dev (
        .clk(clk), .rst_n(rst_n),
        .rx_data(rx_data), .rx_valid(rx_valid), .rx_eop(rx_eop), .rx_active(rx_active),
        .line_idle(line_idle), .bus_reset(bus_reset),
        .tx_start(tx_start), .tx_pid(tx_pid), .tx_with_data(tx_with_data),
        .tx_len(tx_len), .tx_index(tx_index), .tx_byte(tx_byte), .tx_busy(tx_busy),
        .address(address), .configured(configured),
        .class_setup(class_setup), .class_req(class_req), .class_claim(class_claim),
        .class_len(class_len), .class_index(class_index), .class_byte(class_byte),
        .class_out(class_out), .class_out_len(class_out_len), .class_out_valid(class_out_valid),
        .out_data(out_data), .out_valid(out_valid), .out_last(out_last), .out_ready(out_ready),
        .in_data(in_data), .in_valid(in_valid), .in_ready(in_ready), .in_commit(in_commit),
        .notif_data(notif_data), .notif_valid(notif_valid), .notif_ready(notif_ready),
        .notif_commit(notif_commit),
        .vout_data(vout_data), .vout_valid(vout_valid), .vout_last(vout_last), .vout_ready(vout_ready),
        .vin_data(vin_data), .vin_valid(vin_valid), .vin_ready(vin_ready), .vin_commit(vin_commit)
    );
endmodule

// ===================================================================
// usb_dev_core3 — `usb_dev_core` with a third bulk endpoint pair, which is a
// third `usb_bulk_ep`, a third `own_*` bit and a third arm of the transmit
// multiplexer, exactly as that module's header says a third endpoint is. The
// control endpoint, packet decoder and bulk endpoints are the library's own.
// ===================================================================
module usb_dev_core3 #(
    parameter [15:0]  VID          = 16'h1209,
    parameter [15:0]  PID          = 16'h0001,
    parameter [7:0]   DEV_CLASS    = 8'h02,
    parameter [7:0]   DEV_SUBCLASS = 8'h00,
    parameter [7:0]   DEV_PROTOCOL = 8'h00,
    parameter [7:0]   CFG_ATTR     = 8'h80,
    parameter [7:0]   CFG_POWER    = 8'd50,
    parameter integer IFACE_BYTES  = 81,
    parameter [IFACE_BYTES*8-1:0] IFACE_DESC = 0,
    parameter integer DESC_MAX     = 128,
    parameter [3:0]   DATA_ENDP    = 4'd1,
    parameter [6:0]   MAXPKT       = 7'd64,
    parameter [3:0]   NOTIF_ENDP   = 4'd2,
    parameter [6:0]   NOTIF_MAXPKT = 7'd16,
    parameter [3:0]   VENDOR_ENDP  = 4'd3,
    parameter [6:0]   VENDOR_MAXPKT = 7'd64,
    parameter [6:0]   MAXPKT0      = 7'd64,
    parameter integer CLASS_MAX    = 28,
    parameter         BUF_RAM      = 1,
    parameter [6:0]   TURNAROUND   = 7'd8
) (
    input  wire       clk,
    input  wire       rst_n,
    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    input  wire       rx_eop,
    input  wire       rx_active,
    input  wire       line_idle,
    input  wire       bus_reset,

    output wire       tx_start,
    output wire [3:0] tx_pid,
    output wire       tx_with_data,
    output wire [6:0] tx_len,
    input  wire [6:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured,

    output wire [63:0] class_setup,
    output wire        class_req,
    input  wire        class_claim,
    input  wire [6:0]  class_len,
    output wire [6:0]  class_index,
    input  wire [7:0]  class_byte,
    output wire [63:0] class_out,
    output wire [6:0]  class_out_len,
    output wire        class_out_valid,

    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit,

    input  wire [7:0] notif_data,
    input  wire       notif_valid,
    output wire       notif_ready,
    input  wire       notif_commit,

    output wire [7:0] vout_data,
    output wire       vout_valid,
    output wire       vout_last,
    input  wire       vout_ready,
    input  wire [7:0] vin_data,
    input  wire       vin_valid,
    output wire       vin_ready,
    input  wire       vin_commit
);
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
        .clk(clk), .rst_n(rst_n),
        .rx_data(rx_data), .rx_valid(rx_valid), .rx_eop(rx_eop), .rx_active(rx_active),
        .pkt(pkt), .pkt_pid(pkt_pid), .pkt_is_token(pkt_is_token), .pkt_is_data(pkt_is_data),
        .tok_ok(tok_ok), .tok_addr(tok_addr), .tok_endp(tok_endp),
        .dat_ok(dat_ok), .dat_len(dat_len), .dat(dat),
        .pay_byte(pay_byte), .pay_push(pay_push)
    );

    wire tok_notif  = (NOTIF_ENDP != 4'd0) && (tok_endp == NOTIF_ENDP);
    wire tok_vendor = (VENDOR_ENDP != 4'd0) && (tok_endp == VENDOR_ENDP);
    wire tok_data   = (DATA_ENDP != 4'd0) && (tok_endp != 4'd0) && !tok_notif && !tok_vendor;

    reg own_data;
    reg own_notif;
    reg own_vendor;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            own_data   <= 1'b0;
            own_notif  <= 1'b0;
            own_vendor <= 1'b0;
        end else if (pkt && pkt_is_token && tok_ok && tok_addr == address) begin
            own_data   <= tok_data;
            own_notif  <= tok_notif;
            own_vendor <= tok_vendor;
        end
    end

    wire       c_tx_start, c_tx_with_data;
    wire [3:0] c_tx_pid;
    wire [6:0] c_tx_len;
    wire [7:0] c_tx_byte;
    wire       ep_reset, ep_clear;
    wire [7:0] ep_clear_ep;

    usb_ctrl_ep #(
        .VID(VID), .PID(PID),
        .DEV_CLASS(DEV_CLASS), .DEV_SUBCLASS(DEV_SUBCLASS), .DEV_PROTOCOL(DEV_PROTOCOL),
        .CFG_ATTR(CFG_ATTR), .CFG_POWER(CFG_POWER),
        .IFACE_BYTES(IFACE_BYTES), .IFACE_DESC(IFACE_DESC),
        .CLASS_MAX(CLASS_MAX), .MAXPKT0(MAXPKT0), .TURNAROUND(TURNAROUND), .DESC_MAX(DESC_MAX)
    ) u_ep0 (
        .clk(clk), .rst_n(rst_n),
        .pkt(pkt), .pkt_pid(pkt_pid), .pkt_is_token(pkt_is_token), .pkt_is_data(pkt_is_data),
        .tok_ok(tok_ok), .tok_addr(tok_addr), .tok_endp(tok_endp),
        .dat_ok(dat_ok), .dat_len(dat_len), .dat(dat),
        .line_idle(line_idle), .bus_reset(bus_reset),
        .sel(~(own_data | own_notif | own_vendor)),
        .tx_start(c_tx_start), .tx_pid(c_tx_pid), .tx_with_data(c_tx_with_data),
        .tx_len(c_tx_len), .tx_index(tx_index), .tx_byte(c_tx_byte), .tx_busy(tx_busy),
        .address(address), .configured(configured),
        .class_setup(class_setup), .class_req(class_req), .class_claim(class_claim),
        .class_len(class_len), .class_index(class_index), .class_byte(class_byte),
        .class_out(class_out), .class_out_len(class_out_len), .class_out_valid(class_out_valid),
        .ep_reset(ep_reset), .ep_clear(ep_clear), .ep_clear_ep(ep_clear_ep)
    );

    wire       b_tx_start, b_tx_with_data;
    wire [3:0] b_tx_pid;
    wire [6:0] b_tx_len;
    wire [7:0] b_tx_byte;

    usb_bulk_ep #(
        .ENDP(DATA_ENDP), .MAXPKT(MAXPKT), .WITH_OUT(DATA_ENDP != 4'd0),
        .WITH_IN(DATA_ENDP != 4'd0), .BUF_RAM(BUF_RAM), .TURNAROUND(TURNAROUND)
    ) u_ep1 (
        .clk(clk), .rst_n(rst_n),
        .pkt(pkt), .pkt_pid(pkt_pid), .pkt_is_token(pkt_is_token), .pkt_is_data(pkt_is_data),
        .tok_ok(tok_ok), .tok_addr(tok_addr), .tok_endp(tok_endp),
        .dat_ok(dat_ok), .dat_len(dat_len), .pay_byte(pay_byte), .pay_push(pay_push),
        .address(address), .line_idle(line_idle), .bus_reset(bus_reset),
        .ep_reset(ep_reset), .ep_clear(ep_clear), .ep_clear_ep(ep_clear_ep),
        .sel(own_data),
        .tx_start(b_tx_start), .tx_pid(b_tx_pid), .tx_with_data(b_tx_with_data),
        .tx_len(b_tx_len), .tx_index(tx_index), .tx_byte(b_tx_byte), .tx_busy(tx_busy),
        .out_data(out_data), .out_valid(out_valid), .out_last(out_last), .out_ready(out_ready),
        .in_data(in_data), .in_valid(in_valid), .in_ready(in_ready), .in_commit(in_commit)
    );

    wire       n_tx_start, n_tx_with_data;
    wire [3:0] n_tx_pid;
    wire [6:0] n_tx_len;
    wire [7:0] n_tx_byte;
    wire [7:0] n_out_data;
    wire       n_out_valid, n_out_last;

    usb_bulk_ep #(
        .ENDP(NOTIF_ENDP), .MAXPKT(NOTIF_MAXPKT), .WITH_OUT(0),
        .WITH_IN(NOTIF_ENDP != 4'd0), .BUF_RAM(BUF_RAM), .TURNAROUND(TURNAROUND)
    ) u_ep2 (
        .clk(clk), .rst_n(rst_n),
        .pkt(pkt), .pkt_pid(pkt_pid), .pkt_is_token(pkt_is_token), .pkt_is_data(pkt_is_data),
        .tok_ok(tok_ok), .tok_addr(tok_addr), .tok_endp(tok_endp),
        .dat_ok(dat_ok), .dat_len(dat_len), .pay_byte(pay_byte), .pay_push(pay_push),
        .address(address), .line_idle(line_idle), .bus_reset(bus_reset),
        .ep_reset(ep_reset), .ep_clear(ep_clear), .ep_clear_ep(ep_clear_ep),
        .sel(own_notif),
        .tx_start(n_tx_start), .tx_pid(n_tx_pid), .tx_with_data(n_tx_with_data),
        .tx_len(n_tx_len), .tx_index(tx_index), .tx_byte(n_tx_byte), .tx_busy(tx_busy),
        .out_data(n_out_data), .out_valid(n_out_valid), .out_last(n_out_last), .out_ready(1'b0),
        .in_data(notif_data), .in_valid(notif_valid), .in_ready(notif_ready), .in_commit(notif_commit)
    );

    wire       v_tx_start, v_tx_with_data;
    wire [3:0] v_tx_pid;
    wire [6:0] v_tx_len;
    wire [7:0] v_tx_byte;

    usb_bulk_ep #(
        .ENDP(VENDOR_ENDP), .MAXPKT(VENDOR_MAXPKT), .WITH_OUT(VENDOR_ENDP != 4'd0),
        .WITH_IN(VENDOR_ENDP != 4'd0), .BUF_RAM(BUF_RAM), .TURNAROUND(TURNAROUND)
    ) u_ep3 (
        .clk(clk), .rst_n(rst_n),
        .pkt(pkt), .pkt_pid(pkt_pid), .pkt_is_token(pkt_is_token), .pkt_is_data(pkt_is_data),
        .tok_ok(tok_ok), .tok_addr(tok_addr), .tok_endp(tok_endp),
        .dat_ok(dat_ok), .dat_len(dat_len), .pay_byte(pay_byte), .pay_push(pay_push),
        .address(address), .line_idle(line_idle), .bus_reset(bus_reset),
        .ep_reset(ep_reset), .ep_clear(ep_clear), .ep_clear_ep(ep_clear_ep),
        .sel(own_vendor),
        .tx_start(v_tx_start), .tx_pid(v_tx_pid), .tx_with_data(v_tx_with_data),
        .tx_len(v_tx_len), .tx_index(tx_index), .tx_byte(v_tx_byte), .tx_busy(tx_busy),
        .out_data(vout_data), .out_valid(vout_valid), .out_last(vout_last), .out_ready(vout_ready),
        .in_data(vin_data), .in_valid(vin_valid), .in_ready(vin_ready), .in_commit(vin_commit)
    );

    assign tx_start     = own_vendor ? v_tx_start     : own_notif ? n_tx_start
                        : own_data   ? b_tx_start     : c_tx_start;
    assign tx_pid       = own_vendor ? v_tx_pid       : own_notif ? n_tx_pid
                        : own_data   ? b_tx_pid       : c_tx_pid;
    assign tx_with_data = own_vendor ? v_tx_with_data : own_notif ? n_tx_with_data
                        : own_data   ? b_tx_with_data : c_tx_with_data;
    assign tx_len       = own_vendor ? v_tx_len       : own_notif ? n_tx_len
                        : own_data   ? b_tx_len       : c_tx_len;
    assign tx_byte      = own_vendor ? v_tx_byte      : own_notif ? n_tx_byte
                        : own_data   ? b_tx_byte      : c_tx_byte;
endmodule

// ===================================================================
// usb_cdc_vendor_ulpi — `usb_cdc_vendor` behind `ip/usb/usb_device_ulpi`'s
// link layer, as `usb_cdc_acm_ulpi` is `usb_cdc_acm` behind it.
// ===================================================================
module usb_cdc_vendor_ulpi #(
    parameter [15:0] VID          = 16'h1209,
    parameter [15:0] PID          = 16'h0001,
    parameter        RESET_CYCLES = 300,
    parameter        SE0_CYCLES   = 150,
    parameter        LINE_TRIES   = 40000,
    parameter [6:0]  TURNAROUND   = 7'd9,
    parameter [5:0]  VENDOR_ADDR  = 6'h00,
    parameter [7:0]  VENDOR_DATA  = 8'h00,
    parameter [7:0]  CFG_ATTR     = 8'h80,
    parameter [7:0]  CFG_POWER    = 8'd50,
    parameter [6:0]  MAXPKT       = 7'd64,
    parameter [6:0]  MAXPKT0      = 7'd64,
    parameter        BUF_RAM      = 1
) (
    input  wire       clk60,
    input  wire       rst_n,

    input  wire [7:0] ulpi_data_i,
    output wire [7:0] ulpi_data_o,
    output wire       ulpi_data_oe,
    input  wire       ulpi_dir,
    input  wire       ulpi_nxt,
    output wire       ulpi_stp,
    output wire       ulpi_rst_n,

    output wire [6:0] address,
    output wire       configured,
    output wire       usb_reset,
    output wire       phy_ready,

    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit,

    output wire [31:0] baud,
    output wire [7:0]  char_format,
    output wire [7:0]  parity,
    output wire [7:0]  data_bits,
    output wire        dtr,
    output wire        rts,
    input  wire [6:0]  serial_state,

    output wire [7:0] vout_data,
    output wire       vout_valid,
    output wire       vout_last,
    input  wire       vout_ready,
    input  wire [7:0] vin_data,
    input  wire       vin_valid,
    output wire       vin_ready,
    input  wire       vin_commit
);
    wire [7:0] rx_data;
    wire       rx_valid, rx_eop, rx_active, line_idle, bus_reset;
    wire       tx_start, tx_with_data, tx_busy;
    wire [3:0] tx_pid;
    wire [6:0] tx_len, tx_index;
    wire [7:0] tx_byte;

    usb_ulpi_link #(
        .RESET_CYCLES(RESET_CYCLES), .SE0_CYCLES(SE0_CYCLES), .LINE_TRIES(LINE_TRIES),
        .VENDOR_ADDR(VENDOR_ADDR), .VENDOR_DATA(VENDOR_DATA)
    ) u_link (
        .clk(clk60), .rst_n(rst_n),
        .ulpi_dir(ulpi_dir), .ulpi_nxt(ulpi_nxt),
        .ulpi_data_i(ulpi_data_i), .ulpi_data_o(ulpi_data_o), .ulpi_data_oe(ulpi_data_oe),
        .ulpi_stp(ulpi_stp), .ulpi_rst_n(ulpi_rst_n),
        .rx_data(rx_data), .rx_valid(rx_valid), .rx_eop(rx_eop), .rx_active(rx_active),
        .line_idle(line_idle), .bus_reset(bus_reset),
        .tx_start(tx_start), .tx_pid(tx_pid), .tx_with_data(tx_with_data),
        .tx_len(tx_len), .tx_index(tx_index), .tx_byte(tx_byte), .tx_busy(tx_busy),
        .phy_ready(phy_ready)
    );

    usb_cdc_vendor #(
        .VID(VID), .PID(PID), .CFG_ATTR(CFG_ATTR), .CFG_POWER(CFG_POWER),
        .MAXPKT(MAXPKT), .MAXPKT0(MAXPKT0), .BUF_RAM(BUF_RAM), .TURNAROUND(TURNAROUND)
    ) u_acm (
        .clk(clk60), .rst_n(rst_n),
        .rx_data(rx_data), .rx_valid(rx_valid), .rx_eop(rx_eop), .rx_active(rx_active),
        .line_idle(line_idle), .bus_reset(bus_reset),
        .tx_start(tx_start), .tx_pid(tx_pid), .tx_with_data(tx_with_data),
        .tx_len(tx_len), .tx_index(tx_index), .tx_byte(tx_byte), .tx_busy(tx_busy),
        .address(address), .configured(configured),
        .out_data(out_data), .out_valid(out_valid), .out_last(out_last), .out_ready(out_ready),
        .in_data(in_data), .in_valid(in_valid), .in_ready(in_ready), .in_commit(in_commit),
        .baud(baud), .char_format(char_format), .parity(parity), .data_bits(data_bits),
        .dtr(dtr), .rts(rts), .serial_state(serial_state),
        .vout_data(vout_data), .vout_valid(vout_valid), .vout_last(vout_last), .vout_ready(vout_ready),
        .vin_data(vin_data), .vin_valid(vin_valid), .vin_ready(vin_ready), .vin_commit(vin_commit)
    );

    assign usb_reset = bus_reset;
endmodule
