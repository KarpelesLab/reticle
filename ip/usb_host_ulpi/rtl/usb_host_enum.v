// usb_host_enum — what a USB host does to a device that has just been
// plugged in: see it, reset it, ask it what it is, give it an address and
// a configuration.
//
// What it does
//   The sequence below is USB 2.0 §9.1.2's, with the waits §7.1.7.3 and
//   §9.2.6 put on them, and every step of it is one or more control
//   transfers on `usb_host_sie`:
//
//     1.  **Attach.** A full-speed device pulls D+ up through 1.5 kOhm
//         against the host's two 15 kOhm pull-downs, so an idle downstream
//         port sits at SE0 and a device makes it J. `LineState` says so,
//         and K instead of J is a **low-speed** device, which this host
//         reports and does not try to talk to. §7.1.7.3 asks for the
//         change to be stable for 100 ms before anything is done about it,
//         which is `DEBOUNCE_CYCLES`: a plug being pushed in bounces.
//
//     2.  **Reset.** SE0 for at least 10 ms (§7.1.7.5), which a ULPI host
//         drives by writing Function Control rather than by transmitting
//         anything — see `usb_ulpi_host_link`'s header for the quotation
//         and `FUNC_CTRL_SE0` below for the byte. Then §7.1.7.3's 10 ms of
//         recovery before the first transaction, and this waits longer.
//
//     3.  **The device descriptor, twice.** Eight bytes first, because
//         until byte 7 of it has been read the host does not know what
//         `bMaxPacketSize0` is and has to assume the smallest a full-speed
//         device may declare, which §5.5.3 makes 8. Then the whole
//         eighteen with the real size, which is also the first thing that
//         proves a multi-packet data stage and its toggle work.
//
//     4.  **An address**, and `SET_ADDRESS` is the one request whose
//         effect must wait for its own status stage (§9.4.6): the device
//         answers at its **old** address and only then changes. The
//         transfer is therefore finished at address 0 and everything after
//         it is at `DEV_ADDR`, with §9.2.6.3's 2 ms of recovery in between
//         — `ADDR_SETTLE`, and again this waits longer.
//
//     5.  **The device descriptor again**, at the new address. Nothing in
//         USB asks for this; it is here because it is the one transaction
//         that distinguishes "the address was accepted" from "the device
//         is still answering at 0", and the two look identical until
//         something addresses it.
//
//     6.  **The configuration descriptor, twice**, for the same reason as
//         the device descriptor: the first nine bytes carry
//         `wTotalLength`, and only then is it known how much there is.
//         `DESC_MAX` caps what is read, because a configuration descriptor
//         can be hundreds of bytes and what is downstream of this has a
//         buffer.
//
//     7.  **A configuration**, `SET_CONFIGURATION` with the
//         `bConfigurationValue` byte 5 of that descriptor gave. Not a
//         constant 1: the value is the device's to choose, and a host that
//         assumes 1 is wrong on a device that numbered it anything else.
//
//   **The descriptor bytes leave as a stream**, with the offset of each
//   byte in its descriptor and which descriptor it is. A consumer that
//   writes `desc_data` at `base[desc_tag] + desc_index` never has to undo
//   anything: a transfer that failed is retried and the retry writes the
//   same offsets again, and `desc_done` is the only thing that says the
//   descriptor is whole. Nothing is buffered here — the three fields this
//   sequence needs for itself (`bMaxPacketSize0`, `wTotalLength` and
//   `bConfigurationValue`) are caught out of the stream by their offsets
//   as they go past, and a buffer for the rest would be a buffer the
//   consumer has anyway.
//
//   **The data toggle** is this module's, because it is a property of a
//   transfer and `usb_host_sie` deliberately holds none. A control
//   transfer's data stage starts at DATA1 whichever way it points (§8.5.3)
//   and alternates; the status stage is always DATA1.
//
//   **A NAK is not a failure.** A device may answer NAK for as long as it
//   likes while it gets ready, so a NAK re-sends the transaction and the
//   bound on that is a **clock**, not a count: `NAK_CYCLES` for the whole
//   control transfer. A count would be a different limit for a slow device
//   than for a fast one.
//
// What it does not do
//   **String descriptors**, which need a language ID read first and are
//   the one part of enumeration whose length nothing bounds. **No
//   interface descriptors are walked**: the configuration descriptor's
//   bytes go out whole and whatever reads them can parse it.
//
//   **Nothing after `SET_CONFIGURATION`.** A real host would hand the
//   device to a driver here. This reports `up` and stops, which is what
//   makes it useful as an instrument: it says what the device is and stops
//   touching it.
//
//   **No detach handling once it is up.** `LineState` going back to SE0
//   after a device was configured is a detach, and this does not go round
//   again — `up` stays high and the report stays true of a device that may
//   have gone. Re-arming on SE0 is the obvious next thing and it needs a
//   decision about what the report then says.
//
//   **No low speed.** A device that pulls D- up instead is reported on
//   `low_speed` and nothing is sent to it: a low-speed host needs
//   `XcvrSelect = 11b` and the transceiver prepending a preamble to every
//   packet (USB334x DS00002646A §6.4.1.3), which `usb_ulpi_host_link` does
//   not set up.
//
//   **No power.** Whether the port has VBUS is a board's question and
//   nothing here reads or drives a switch. A device with no VBUS has no
//   pull-up either, so it never leaves step 1 — which is the honest
//   behaviour and not a hang: `state` says exactly where it is.
module usb_host_enum #(
    // Clocks the line must hold a value other than SE0 before it counts as
    // an attachment: 100 ms at 60 MHz (USB 2.0 §7.1.7.3).
    parameter integer DEBOUNCE_CYCLES = 6_000_000,
    // Clocks of SE0 driven as a bus reset. §7.1.7.5 asks for at least
    // 10 ms; 15 ms leaves room for a device that measures it meanly.
    parameter integer RESET_HOLD = 900_000,
    // Clocks after the reset before the first transaction. §7.1.7.3 allows
    // a device 10 ms to recover; 20 ms here.
    parameter integer RESET_RECOVERY = 1_200_000,
    // Clocks after `SET_ADDRESS`'s status stage before the device is
    // addressed at its new address. §9.2.6.3 gives it 2 ms; 5 ms here.
    parameter integer ADDR_SETTLE = 300_000,
    // Clocks a control transfer may spend being NAKed before it is a
    // failure. 50 ms at 60 MHz.
    parameter integer NAK_CYCLES = 3_000_000,
    // The address this host gives the device. Any of 1 to 127.
    parameter [6:0] DEV_ADDR = 7'd1,
    // The `bMaxPacketSize0` assumed until the device descriptor says.
    // USB 2.0 §5.5.3 makes 8 the smallest a full-speed control endpoint
    // may declare, so it is the only safe assumption.
    parameter [6:0] MAXPKT0_MIN = 7'd8,
    // The most of a configuration descriptor that is read. 64 is one
    // full-speed packet of the largest size and enough for a device with
    // one interface and a few endpoints; a composite device has more and
    // this reads the first `DESC_MAX` of it.
    parameter [6:0] DESC_MAX = 7'd64,
    // Which `LineState` value is a full-speed device's idle J.
    //
    // ULPI 1.1 Table 7 makes bit 0 D+ and bit 1 D-, so a full-speed device
    // pulling D+ up is `01` and a low-speed one is `10`. That is **the
    // transceiver's** D+, though, and a board may exchange DP and DM
    // between the transceiver and its connector —
    // `usb_ulpi_host_link`'s `VENDOR_ADDR` is the register that undoes it.
    // With that register written this is `01`; without it, on a board that
    // crosses the pair, it is `10`, and a host that had it the wrong way
    // round would call every full-speed device low speed and refuse to
    // talk to it. So it is a parameter and not a constant, and whichever
    // it is, `line_state` reports what was actually seen.
    parameter [1:0] FS_LINE = 2'b01
) (
    input  wire       clk,
    input  wire       rst_n,
    // Low holds the whole sequence off and leaves the register port alone,
    // so that something else can use it. This is how a design probes the
    // transceiver before it drives anything.
    input  wire       en,

    // From `usb_ulpi_host_link`.
    input  wire       phy_ready,
    input  wire [1:0] line_state,

    // Its register port, used for the two writes a bus reset is and one
    // read of the Debug register afterwards.
    output wire       reg_start,
    output wire       reg_write,
    output wire [5:0] reg_addr,
    output wire [7:0] reg_wdata,
    input  wire [7:0] reg_rdata,
    input  wire       reg_done,
    input  wire       reg_ok,
    input  wire       reg_busy,

    // To `usb_host_sie`.
    output wire       sof_en,
    output wire       trn_start,
    output wire [1:0] trn_kind,
    output wire [6:0] trn_addr,
    output wire [3:0] trn_endp,
    output wire       trn_toggle,
    output wire [6:0] trn_len,
    output wire [7:0] trn_byte,
    input  wire [6:0] trn_index,
    input  wire       trn_busy,
    input  wire       trn_done,
    input  wire [2:0] trn_status,
    input  wire [3:0] trn_rx_pid,
    input  wire [6:0] trn_rx_len,
    input  wire [7:0] in_byte,
    input  wire       in_push,
    input  wire [6:0] in_index,

    // What was found.
    output wire [4:0] stage,
    output wire       attached,
    output wire       low_speed,
    output wire       up,
    output wire       failed,
    output wire [4:0] fail_stage,
    output wire [2:0] fail_status,
    output wire [6:0] dev_addr,
    output wire [6:0] maxpkt0,
    output wire [15:0] cfg_total,
    output wire [7:0] cfg_value,

    // The descriptor bytes, as they arrive.
    output wire [7:0] desc_data,
    output wire       desc_valid,
    output wire [6:0] desc_index,
    output wire [1:0] desc_tag,
    output wire       desc_done,
    output wire [6:0] desc_len
) ;
    // The kinds of transaction `usb_host_sie` takes.
    localparam [1:0] K_SETUP = 2'd0;
    localparam [1:0] K_IN    = 2'd1;
    localparam [1:0] K_OUT   = 2'd2;

    // What `trn_status` says.
    localparam [2:0] ST_ACK   = 3'd0;
    localparam [2:0] ST_NAK   = 3'd1;
    localparam [2:0] ST_DATA  = 3'd3;

    // `fail_status` is `usb_host_sie`'s `trn_status` when a transaction is
    // what failed, and these two when the failure was not a transaction at
    // all. 6 and 7 are the codes that engine does not use.
    localparam [2:0] FAIL_NOT_J    = 3'd6;  // not at J after the bus reset
    localparam [2:0] FAIL_REGISTER = 3'd7;  // the transceiver refused a write

    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;

    // Function Control, as this host writes it. `45h` is the full-speed
    // host of ULPI 1.1 §3.8.5.3.2 and the "Host Full Speed" row of
    // USB334x DS00002646A Table 5-1; `50h` is §3.8.5.1's
    // "XcvrSelect = 00b (HS) and TermSelect = 0b which drives SE0 on the
    // bus", with OpMode = 10b as that step also asks for, and is the same
    // table's "Host Chirp" row — the one with `HSTERM_EN` set, which is
    // the 45 Ohm terminations to ground that **are** the SE0.
    localparam [5:0] REG_FUNC_CTRL = 6'h04;
    localparam [5:0] REG_DEBUG     = 6'h15;
    localparam [7:0] FUNC_CTRL_FS  = 8'h45;
    localparam [7:0] FUNC_CTRL_SE0 = 8'h50;

    localparam [1:0] LINE_SE0 = 2'b00;

    // The stages.
    localparam [4:0] E_OFF      = 5'd0;   // disabled, or the PHY is not up
    localparam [4:0] E_IDLE     = 5'd1;   // an empty port: SE0
    localparam [4:0] E_DEBOUNCE = 5'd2;   // something on the line, settling
    localparam [4:0] E_LOWSPEED = 5'd3;   // a low-speed device; stop
    localparam [4:0] E_RST_ON   = 5'd4;   // Function Control <- 50h
    localparam [4:0] E_RST_HOLD = 5'd5;   // SE0 held
    localparam [4:0] E_RST_OFF  = 5'd6;   // Function Control <- 45h
    localparam [4:0] E_RECOVER  = 5'd7;   // the device coming back
    localparam [4:0] E_RECHECK  = 5'd8;   // read Debug: the pair at J again
    localparam [4:0] E_DEV8     = 5'd9;   // the device descriptor, 8 bytes
    localparam [4:0] E_DEV18    = 5'd10;  // and all eighteen
    localparam [4:0] E_SETADDR  = 5'd11;  // SET_ADDRESS
    localparam [4:0] E_SETTLE   = 5'd12;  // the 2 ms §9.2.6.3 asks for
    localparam [4:0] E_DEV18B   = 5'd13;  // the descriptor at the new address
    localparam [4:0] E_CFG9     = 5'd14;  // the configuration descriptor, 9
    localparam [4:0] E_CFGALL   = 5'd15;  // and wTotalLength of it
    localparam [4:0] E_SETCFG   = 5'd16;  // SET_CONFIGURATION
    localparam [4:0] E_UP       = 5'd17;  // configured
    localparam [4:0] E_FAIL     = 5'd18;  // stopped

    // The control-transfer engine inside a stage.
    localparam [2:0] C_IDLE   = 3'd0;
    localparam [2:0] C_SETUP  = 3'd1;
    localparam [2:0] C_DATA   = 3'd2;
    localparam [2:0] C_STATUS = 3'd3;
    localparam [2:0] C_OK     = 3'd4;
    localparam [2:0] C_BAD    = 3'd5;

    // Which descriptor the bytes belong to.
    localparam [1:0] TAG_DEVICE = 2'd0;
    localparam [1:0] TAG_CONFIG = 2'd1;

    // One counter does every wait, so it is as wide as the longest of them
    // and no wider.
    localparam integer WAIT_A   = (DEBOUNCE_CYCLES > RESET_HOLD)
                                      ? DEBOUNCE_CYCLES : RESET_HOLD;
    localparam integer WAIT_B   = (RESET_RECOVERY > ADDR_SETTLE)
                                      ? RESET_RECOVERY : ADDR_SETTLE;
    localparam integer WAIT_MAX = (WAIT_A > WAIT_B) ? WAIT_A : WAIT_B;
    localparam integer WAIT_W = $clog2(WAIT_MAX + 1);
    localparam integer NAK_W  = $clog2(NAK_CYCLES + 1);

    reg [4:0]        stg;
    reg [2:0]        ctl;
    reg [WAIT_W-1:0] wait_cnt;
    reg [NAK_W-1:0]  nak_cnt;

    // The eight bytes of the SETUP packet, byte 0 in the low bits.
    reg [63:0] setup_w;
    // What this transfer is: whether it has an IN data stage, how many
    // bytes of one, which address it is addressed to, and which descriptor
    // the bytes belong to.
    reg        ctl_read;
    reg [6:0]  ctl_len;
    reg [6:0]  ctl_addr;
    reg [1:0]  ctl_tag;
    reg        ctl_capture;   // the bytes are a descriptor worth reporting

    reg [6:0]  got;
    reg        toggle;

    reg        attached_q;
    reg        low_speed_q;
    reg        failed_q;
    reg [4:0]  fail_stage_q;
    reg [2:0]  fail_status_q;
    reg [6:0]  addr_q;
    reg [6:0]  maxpkt0_q;
    reg [15:0] cfg_total_q;
    reg [7:0]  cfg_value_q;
    reg        desc_done_q;
    reg [6:0]  desc_len_q;

    reg        reg_start_q;
    reg        reg_write_q;
    reg [5:0]  reg_addr_q;
    reg [7:0]  reg_wdata_q;

    reg        trn_start_q;
    reg [1:0]  trn_kind_q;
    reg [3:0]  trn_endp_q;
    reg        trn_toggle_q;
    reg [6:0]  trn_len_q;

    assign stage       = stg;
    assign attached    = attached_q;
    assign low_speed   = low_speed_q;
    assign up          = (stg == E_UP);
    assign failed      = failed_q;
    assign fail_stage  = fail_stage_q;
    assign fail_status = fail_status_q;
    assign dev_addr    = addr_q;
    assign maxpkt0     = maxpkt0_q;
    assign cfg_total   = cfg_total_q;
    assign cfg_value   = cfg_value_q;

    assign reg_start = reg_start_q;
    assign reg_write = reg_write_q;
    assign reg_addr  = reg_addr_q;
    assign reg_wdata = reg_wdata_q;

    assign sof_en     = (stg >= E_RECOVER) & (stg != E_FAIL);
    assign trn_start  = trn_start_q;
    assign trn_kind   = trn_kind_q;
    assign trn_addr   = ctl_addr;
    assign trn_endp   = trn_endp_q;
    assign trn_toggle = trn_toggle_q;
    assign trn_len    = trn_len_q;
    // The SETUP packet's bytes. Nothing else this host sends has a
    // payload: the status stage of a control read is a data packet of no
    // bytes at all.
    assign trn_byte   = setup_w[{trn_index[2:0], 3'b000} +: 8];

    assign desc_data  = in_byte;
    assign desc_valid = in_push & ctl_capture & (ctl == C_DATA);
    assign desc_index = got + in_index;
    assign desc_tag   = ctl_tag;
    assign desc_done  = desc_done_q;
    assign desc_len   = desc_len_q;

    // The absolute offset of the byte arriving, which is what the three
    // fields this sequence needs for itself are caught by.
    wire [6:0] at = got + in_index;

    // How much of the configuration descriptor is read: what it says it is,
    // capped at what a buffer downstream can be expected to have.
    wire [6:0] cfg_read_len = (cfg_total_q > {9'd0, DESC_MAX})
                                  ? DESC_MAX : cfg_total_q[6:0];

    // A GET_DESCRIPTOR request, built where it is used: `80h` is
    // device-to-host on the device itself, `06h` is GET_DESCRIPTOR, the
    // high byte of `wValue` is the descriptor type and the low byte its
    // index (USB 2.0 §9.4.3 and Table 9-5).
    function [63:0] get_descriptor;
        input [7:0] kind;
        input [6:0] len;
        begin
            get_descriptor = {8'h00, {1'b0, len}, 8'h00, 8'h00,
                              kind, 8'h00, 8'h06, 8'h80};
        end
    endfunction

    // A request with no data stage: `00h` is host-to-device on the device.
    function [63:0] no_data_request;
        input [7:0] request;
        input [7:0] value;
        begin
            no_data_request = {8'h00, 8'h00, 8'h00, 8'h00,
                               8'h00, value, request, 8'h00};
        end
    endfunction

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            stg           <= E_OFF;
            ctl           <= C_IDLE;
            wait_cnt      <= 0;
            nak_cnt       <= 0;
            setup_w       <= 64'd0;
            ctl_read      <= 1'b0;
            ctl_len       <= 7'd0;
            ctl_addr      <= 7'd0;
            ctl_tag       <= TAG_DEVICE;
            ctl_capture   <= 1'b0;
            got           <= 7'd0;
            toggle        <= 1'b1;
            attached_q    <= 1'b0;
            low_speed_q   <= 1'b0;
            failed_q      <= 1'b0;
            fail_stage_q  <= E_OFF;
            fail_status_q <= 3'd0;
            addr_q        <= 7'd0;
            maxpkt0_q     <= MAXPKT0_MIN;
            cfg_total_q   <= 16'd0;
            cfg_value_q   <= 8'd0;
            desc_done_q   <= 1'b0;
            desc_len_q    <= 7'd0;
            reg_start_q   <= 1'b0;
            reg_write_q   <= 1'b0;
            reg_addr_q    <= 6'd0;
            reg_wdata_q   <= 8'd0;
            trn_start_q   <= 1'b0;
            trn_kind_q    <= K_SETUP;
            trn_endp_q    <= 4'd0;
            trn_toggle_q  <= 1'b0;
            trn_len_q     <= 7'd0;
        end else begin
            desc_done_q <= 1'b0;
            // Both request lines are **held** until they are taken, for the
            // reason `usb_host_sie` holds `tx_start`: the cycle a pulse
            // lands in may be one the other end cannot accept.
            if (trn_start_q && trn_busy) trn_start_q <= 1'b0;
            if (reg_start_q && reg_busy) reg_start_q <= 1'b0;

            // ----------------------------------------------------------
            // The three fields this sequence needs out of the stream.
            // ----------------------------------------------------------
            if (in_push && ctl_capture && ctl == C_DATA) begin
                if (ctl_tag == TAG_DEVICE) begin
                    // Byte 7 of the device descriptor is bMaxPacketSize0
                    // (USB 2.0 Table 9-8). A full-speed device may declare
                    // 8, 16, 32 or 64, so the byte fits the seven bits
                    // `usb_host_sie`'s lengths are.
                    if (at == 7'd7) maxpkt0_q <= in_byte[6:0];
                end else begin
                    // Bytes 2 and 3 are wTotalLength and byte 5 is
                    // bConfigurationValue (Table 9-10).
                    if (at == 7'd2) cfg_total_q[7:0]  <= in_byte;
                    if (at == 7'd3) cfg_total_q[15:8] <= in_byte;
                    if (at == 7'd5) cfg_value_q       <= in_byte;
                end
            end

            if (!en || !phy_ready) begin
                stg <= E_OFF;
                ctl <= C_IDLE;
            end else begin
                // ------------------------------------------------------
                // The control-transfer engine. It runs whenever a stage
                // has started one, and the stage below reads `ctl`.
                // ------------------------------------------------------
                if (ctl != C_IDLE && ctl != C_OK && ctl != C_BAD) begin
                    if (nak_cnt != NAK_CYCLES[NAK_W-1:0])
                        nak_cnt <= nak_cnt + 1'b1;
                end
                if (trn_done) begin
                    case (ctl)
                        C_SETUP: begin
                            if (trn_status == ST_ACK) begin
                                // The data stage starts at DATA1 whichever
                                // way it points (§8.5.3). A transfer with
                                // no data stage goes straight to a status
                                // stage, which for one of those is an IN.
                                toggle       <= 1'b1;
                                got          <= 7'd0;
                                trn_kind_q   <= K_IN;
                                trn_toggle_q <= 1'b1;
                                trn_len_q    <= 7'd0;
                                trn_start_q  <= 1'b1;
                                ctl          <= ctl_read ? C_DATA : C_STATUS;
                            end else if (trn_status == ST_NAK
                                         && nak_cnt != NAK_CYCLES[NAK_W-1:0]) begin
                                // A device must not NAK a SETUP (§8.4.6),
                                // and one that does is given the same
                                // leeway as any other NAK rather than
                                // being called broken.
                                trn_start_q <= 1'b1;
                            end else begin
                                fail_status_q <= trn_status;
                                ctl           <= C_BAD;
                            end
                        end
                        C_DATA: begin
                            if (trn_status == ST_DATA
                                    && trn_rx_pid == (toggle ? PID_DATA1
                                                             : PID_DATA0)) begin
                                if (trn_rx_len < maxpkt0_q
                                        || (got + trn_rx_len) >= ctl_len) begin
                                    // A short packet or the whole of
                                    // `wLength` ends the data stage
                                    // (§5.5.3). The status stage of a read
                                    // is an OUT carrying a data packet of
                                    // no bytes, and it is always DATA1.
                                    got          <= got + trn_rx_len;
                                    desc_len_q   <= got + trn_rx_len;
                                    trn_kind_q   <= K_OUT;
                                    trn_toggle_q <= 1'b1;
                                    trn_len_q    <= 7'd0;
                                    trn_start_q  <= 1'b1;
                                    ctl          <= C_STATUS;
                                end else begin
                                    got          <= got + trn_rx_len;
                                    toggle       <= ~toggle;
                                    trn_kind_q   <= K_IN;
                                    trn_len_q    <= 7'd0;
                                    trn_start_q  <= 1'b1;
                                end
                            end else if ((trn_status == ST_NAK
                                          || trn_status == ST_DATA)
                                         && nak_cnt != NAK_CYCLES[NAK_W-1:0]) begin
                                // NAK: not ready yet. A data packet with
                                // the **other** toggle: the device did not
                                // hear the last acknowledgement and is
                                // sending it again, so it is the same
                                // non-event. Ask once more either way.
                                trn_start_q <= 1'b1;
                            end else begin
                                fail_status_q <= trn_status;
                                ctl           <= C_BAD;
                            end
                        end
                        C_STATUS: begin
                            // A read's status stage is the OUT above, and
                            // the device acknowledges it. A write's is an
                            // IN, and the device answers with a data packet
                            // of no bytes that `usb_host_sie` has already
                            // acknowledged.
                            if ((ctl_read && trn_status == ST_ACK)
                                    || (!ctl_read && trn_status == ST_DATA
                                        && trn_rx_len == 7'd0)) begin
                                if (ctl_capture) begin
                                    desc_done_q <= 1'b1;
                                end
                                ctl <= C_OK;
                            end else if (trn_status == ST_NAK
                                         && nak_cnt != NAK_CYCLES[NAK_W-1:0]) begin
                                trn_start_q <= 1'b1;
                            end else begin
                                fail_status_q <= trn_status;
                                ctl           <= C_BAD;
                            end
                        end
                        default: begin
                            // C_IDLE, C_OK, C_BAD: no transaction of this
                            // engine's is outstanding.
                        end
                    endcase
                end

                // ------------------------------------------------------
                // The sequence.
                // ------------------------------------------------------
                case (stg)
                    E_OFF: begin
                        // Out of reset with the transceiver up and the
                        // sequence enabled.
                        wait_cnt <= 0;
                        stg      <= E_IDLE;
                    end
                    E_IDLE: begin
                        if (line_state != LINE_SE0) begin
                            wait_cnt <= 0;
                            stg      <= E_DEBOUNCE;
                        end
                    end
                    E_DEBOUNCE: begin
                        if (line_state == LINE_SE0) begin
                            // It bounced back. Nothing is attached yet.
                            stg <= E_IDLE;
                        end else if (wait_cnt == DEBOUNCE_CYCLES[WAIT_W-1:0]) begin
                            attached_q <= 1'b1;
                            if (line_state == FS_LINE) begin
                                wait_cnt <= 0;
                                stg      <= E_RST_ON;
                            end else begin
                                low_speed_q <= 1'b1;
                                stg         <= E_LOWSPEED;
                            end
                        end else begin
                            wait_cnt <= wait_cnt + 1'b1;
                        end
                    end
                    E_LOWSPEED: begin
                        // Reported, and nothing is sent to it.
                    end
                    E_RST_ON: begin
                        // **`reg_done` is tested first**, and the order is
                        // not a style: `reg_busy` falls in the very cycle
                        // `reg_done` rises, because the Link clears the
                        // request it is reporting on the same edge. Issuing
                        // first would see an idle port in that cycle and
                        // send the write again, for ever.
                        if (reg_done) begin
                            if (reg_ok) begin
                                wait_cnt <= 0;
                                stg      <= E_RST_HOLD;
                            end else begin
                                fail_status_q <= FAIL_REGISTER;
                                fail_stage_q  <= E_RST_ON;
                                failed_q      <= 1'b1;
                                stg           <= E_FAIL;
                            end
                        end else if (!reg_start_q && !reg_busy) begin
                            reg_write_q <= 1'b1;
                            reg_addr_q  <= REG_FUNC_CTRL;
                            reg_wdata_q <= FUNC_CTRL_SE0;
                            reg_start_q <= 1'b1;
                        end
                    end
                    E_RST_HOLD: begin
                        if (wait_cnt == RESET_HOLD[WAIT_W-1:0]) begin
                            stg <= E_RST_OFF;
                        end else begin
                            wait_cnt <= wait_cnt + 1'b1;
                        end
                    end
                    E_RST_OFF: begin
                        if (reg_done) begin
                            if (reg_ok) begin
                                wait_cnt <= 0;
                                stg      <= E_RECOVER;
                            end else begin
                                fail_status_q <= FAIL_REGISTER;
                                fail_stage_q  <= E_RST_OFF;
                                failed_q      <= 1'b1;
                                stg           <= E_FAIL;
                            end
                        end else if (!reg_start_q && !reg_busy) begin
                            reg_write_q <= 1'b1;
                            reg_addr_q  <= REG_FUNC_CTRL;
                            reg_wdata_q <= FUNC_CTRL_FS;
                            reg_start_q <= 1'b1;
                        end
                    end
                    E_RECOVER: begin
                        if (wait_cnt == RESET_RECOVERY[WAIT_W-1:0]) begin
                            stg <= E_RECHECK;
                        end else begin
                            wait_cnt <= wait_cnt + 1'b1;
                        end
                    end
                    E_RECHECK: begin
                        // The pair should be back at J, and this is the one
                        // place a host can **ask** rather than wait to be
                        // told: a transceiver sends a receive command when
                        // LineState changes, and the change from the
                        // terminations being switched off may already have
                        // gone by.
                        if (reg_done) begin
                            if (reg_ok && reg_rdata[1:0] == FS_LINE) begin
                                setup_w     <= get_descriptor(8'h01, 7'd8);
                                ctl_read    <= 1'b1;
                                ctl_len     <= 7'd8;
                                ctl_addr    <= 7'd0;
                                ctl_tag     <= TAG_DEVICE;
                                ctl_capture <= 1'b1;
                                maxpkt0_q   <= MAXPKT0_MIN;
                                nak_cnt     <= 0;
                                got         <= 7'd0;
                                trn_kind_q  <= K_SETUP;
                                trn_endp_q  <= 4'd0;
                                trn_toggle_q<= 1'b0;
                                trn_len_q   <= 7'd8;
                                trn_start_q <= 1'b1;
                                ctl         <= C_SETUP;
                                stg         <= E_DEV8;
                            end else begin
                                // Not at J after the reset: the device went
                                // away, or it never had power.
                                fail_status_q <= FAIL_NOT_J;
                                fail_stage_q  <= E_RECHECK;
                                failed_q      <= 1'b1;
                                stg           <= E_FAIL;
                            end
                        end else if (!reg_start_q && !reg_busy) begin
                            reg_write_q <= 1'b0;
                            reg_addr_q  <= REG_DEBUG;
                            reg_start_q <= 1'b1;
                        end
                    end
                    E_DEV8, E_DEV18, E_SETADDR, E_DEV18B,
                    E_CFG9, E_CFGALL, E_SETCFG: begin
                        if (ctl == C_BAD) begin
                            fail_stage_q <= stg;
                            failed_q     <= 1'b1;
                            stg          <= E_FAIL;
                            ctl          <= C_IDLE;
                        end else if (ctl == C_OK) begin
                            ctl     <= C_IDLE;
                            nak_cnt <= 0;
                            got     <= 7'd0;
                            case (stg)
                                E_DEV8: begin
                                    // `maxpkt0_q` now holds what the device
                                    // declared, so the whole descriptor can
                                    // be asked for in packets of its size.
                                    setup_w      <= get_descriptor(8'h01, 7'd18);
                                    ctl_len      <= 7'd18;
                                    trn_kind_q   <= K_SETUP;
                                    trn_toggle_q <= 1'b0;
                                    trn_len_q    <= 7'd8;
                                    trn_start_q  <= 1'b1;
                                    ctl          <= C_SETUP;
                                    stg          <= E_DEV18;
                                end
                                E_DEV18: begin
                                    setup_w      <= no_data_request(8'h05,
                                                        {1'b0, DEV_ADDR});
                                    ctl_read     <= 1'b0;
                                    ctl_len      <= 7'd0;
                                    ctl_capture  <= 1'b0;
                                    trn_kind_q   <= K_SETUP;
                                    trn_toggle_q <= 1'b0;
                                    trn_len_q    <= 7'd8;
                                    trn_start_q  <= 1'b1;
                                    ctl          <= C_SETUP;
                                    stg          <= E_SETADDR;
                                end
                                E_SETADDR: begin
                                    // The address takes effect **after**
                                    // the status stage, which has just
                                    // finished (§9.4.6).
                                    addr_q   <= DEV_ADDR;
                                    wait_cnt <= 0;
                                    stg      <= E_SETTLE;
                                end
                                E_DEV18B: begin
                                    setup_w      <= get_descriptor(8'h02, 7'd9);
                                    ctl_read     <= 1'b1;
                                    ctl_len      <= 7'd9;
                                    ctl_tag      <= TAG_CONFIG;
                                    ctl_capture  <= 1'b1;
                                    trn_kind_q   <= K_SETUP;
                                    trn_toggle_q <= 1'b0;
                                    trn_len_q    <= 7'd8;
                                    trn_start_q  <= 1'b1;
                                    ctl          <= C_SETUP;
                                    stg          <= E_CFG9;
                                end
                                E_CFG9: begin
                                    setup_w      <= get_descriptor(8'h02,
                                                        cfg_read_len);
                                    ctl_len      <= cfg_read_len;
                                    trn_kind_q   <= K_SETUP;
                                    trn_toggle_q <= 1'b0;
                                    trn_len_q    <= 7'd8;
                                    trn_start_q  <= 1'b1;
                                    ctl          <= C_SETUP;
                                    stg          <= E_CFGALL;
                                end
                                E_CFGALL: begin
                                    setup_w      <= no_data_request(8'h09,
                                                        cfg_value_q);
                                    ctl_read     <= 1'b0;
                                    ctl_len      <= 7'd0;
                                    ctl_capture  <= 1'b0;
                                    trn_kind_q   <= K_SETUP;
                                    trn_toggle_q <= 1'b0;
                                    trn_len_q    <= 7'd8;
                                    trn_start_q  <= 1'b1;
                                    ctl          <= C_SETUP;
                                    stg          <= E_SETCFG;
                                end
                                default: begin
                                    // E_SETCFG: the device is configured.
                                    stg <= E_UP;
                                end
                            endcase
                        end
                    end
                    E_SETTLE: begin
                        if (wait_cnt == ADDR_SETTLE[WAIT_W-1:0]) begin
                            setup_w      <= get_descriptor(8'h01, 7'd18);
                            ctl_read     <= 1'b1;
                            ctl_len      <= 7'd18;
                            ctl_addr     <= DEV_ADDR;
                            ctl_tag      <= TAG_DEVICE;
                            ctl_capture  <= 1'b1;
                            nak_cnt      <= 0;
                            got          <= 7'd0;
                            trn_kind_q   <= K_SETUP;
                            trn_toggle_q <= 1'b0;
                            trn_len_q    <= 7'd8;
                            trn_start_q  <= 1'b1;
                            ctl          <= C_SETUP;
                            stg          <= E_DEV18B;
                        end else begin
                            wait_cnt <= wait_cnt + 1'b1;
                        end
                    end
                    default: begin
                        // E_UP and E_FAIL: nothing more is sent.
                    end
                endcase
            end
        end
    end
endmodule
