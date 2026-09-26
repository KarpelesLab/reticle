// usb_ctrl_ep — USB endpoint 0: the device half of a USB device, with no
// knowledge of how its bytes reach the bus.
//
// What it does
//   Everything a host needs to enumerate a device, above the line:
//   packet decoding with the PID check nibble, the CRC5 of tokens and
//   the CRC16 of data packets checked, the data toggle, and the standard
//   requests. It is shared. `usb_device_fs` puts it behind its own
//   full-speed encoder and serialiser; `usb_device_ulpi` puts it behind
//   a ULPI transceiver, which does that work in silicon. There is one
//   statement of the control endpoint for both, the way `eth_mac_tx` and
//   `eth_mac_rx` are one statement of Ethernet framing for RMII and
//   RGMII.
//
//   Endpoint 0, maximum packet size 8:
//
//     GET_DESCRIPTOR, device        the 18-byte device descriptor, VID
//                                   and PID from the parameters
//     GET_DESCRIPTOR, configuration one configuration of one interface
//                                   with no endpoints, vendor class,
//                                   bus powered at 100 mA: 18 bytes
//     SET_ADDRESS                   taken after the status stage, as the
//                                   specification says
//     SET_CONFIGURATION 0 or 1      accepted; `configured` follows it
//
//   A descriptor goes out in as many DATA1 / DATA0 packets as it takes,
//   never more than the host's wLength, and a packet the host does not
//   acknowledge is sent again with the same toggle. The status stage is
//   an OUT of zero length after a read and an IN of zero length after
//   the others. Anything else — string descriptors, GET_STATUS,
//   requests to an interface or an endpoint, class and vendor requests —
//   is answered with STALL until the next SETUP.
//
//   A packet whose PID check fails, whose CRC is wrong or which ends off
//   a byte boundary never reaches `rx_eop`, so it is ignored and the
//   host retries. A token addressed elsewhere, or to another endpoint,
//   is ignored too. `bus_reset` sets the address back to 0 and the
//   configuration to none.
//
//   The receive side is a byte at a time: `rx_active` brackets a packet,
//   `rx_valid` strobes each byte into it, and `rx_eop` says the packet
//   ended whole. The transmit side is a packet at a time: `tx_start`
//   with `tx_pid`, `tx_with_data` and `tx_len`, the payload fetched
//   through `tx_index` and `tx_byte` in the same cycle, and `tx_busy`
//   until the packet is gone.
//
//   `TURNAROUND` is how many cycles of `line_idle` must pass after a
//   host packet before the answer starts, and what it has to be depends
//   on the layer below. Eight cycles is two bit times on
//   `usb_device_fs`'s 48 MHz clock. ULPI states the same delay as a
//   count of 60 MHz clocks — 7 to 18 for a full-speed link, ULPI 1.1
//   Table 10 — from the receive command that reports the bus back at J.
//
// What it does not do
//   Endpoint 0 only: no bulk, interrupt or isochronous endpoints, so
//   there is nothing to move data with once enumerated. It is the part
//   every USB function needs first and the part that is easiest to get
//   wrong, and it is what the other endpoints would be added beside.
//
//   No strings, no remote wake-up, no suspend, no SOF tracking and no
//   low speed. Eight bytes of payload at most in either direction, which
//   is what a maximum packet size of eight needs.
//
//   Nothing here knows about NRZI, bit stuffing, SYNC, EOP or line
//   states. It does check the CRC16 of what it receives and it expects
//   whatever transmits for it to append one, because that is the
//   device's job in both arrangements: a ULPI transceiver prepends SYNC
//   and appends the EOP but never touches a CRC (ULPI 1.1 §3.8.2.2).
module usb_ctrl_ep #(
    parameter [15:0] VID        = 16'h1209,
    parameter [15:0] PID        = 16'h0001,
    // Cycles of `line_idle` before an answer starts.
    parameter [3:0]  TURNAROUND = 4'd8
) (
    input  wire       clk,
    input  wire       rst_n,

    // The bytes of a received packet.
    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    input  wire       rx_eop,
    input  wire       rx_active,
    // The bus is idle, so an answer may be timed from now.
    input  wire       line_idle,
    // The host has reset the bus.
    input  wire       bus_reset,

    // One packet out.
    output reg        tx_start,
    output reg  [3:0] tx_pid,
    output reg        tx_with_data,
    output reg  [3:0] tx_len,
    input  wire [3:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    output wire [6:0] address,
    output wire       configured
);
    // PIDs, the low nibble as it appears on the wire.
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_SETUP = 4'b1101;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;
    localparam [3:0] PID_ACK   = 4'b0010;
    localparam [3:0] PID_NAK   = 4'b1010;
    localparam [3:0] PID_STALL = 4'b1110;

    // The residues a correct CRC leaves in these reflected registers.
    localparam [4:0]  CRC5_RESIDUE  = 5'h06;
    localparam [15:0] CRC16_RESIDUE = 16'hB001;

    // Control transfer stages.
    localparam [2:0] C_IDLE       = 3'd0;
    localparam [2:0] C_DATA_IN    = 3'd1;
    localparam [2:0] C_STATUS_IN  = 3'd2;
    localparam [2:0] C_STALL      = 3'd3;

    // What the last token asked for.
    localparam [1:0] X_NONE  = 2'd0;
    localparam [1:0] X_SETUP = 2'd1;
    localparam [1:0] X_OUT   = 2'd2;

    // -----------------------------------------------------------------
    // CRCs, a byte at a time, reflected, as the bytes arrive.
    // -----------------------------------------------------------------
    function [4:0] crc5_byte;
        input [4:0] c;
        input [7:0] d;
        integer     i;
        reg   [4:0] r;
        begin
            r = c;
            for (i = 0; i < 8; i = i + 1)
                r = (r[0] ^ d[i]) ? ((r >> 1) ^ 5'h14) : (r >> 1);
            crc5_byte = r;
        end
    endfunction

    function [15:0] crc16_byte;
        input [15:0] c;
        input [7:0]  d;
        integer      i;
        reg   [15:0] r;
        begin
            r = c;
            for (i = 0; i < 8; i = i + 1)
                r = (r[0] ^ d[i]) ? ((r >> 1) ^ 16'hA001) : (r >> 1);
            crc16_byte = r;
        end
    endfunction

    // -----------------------------------------------------------------
    // Receiving a packet.
    // -----------------------------------------------------------------
    reg [3:0]  n;          // bytes received, PID included
    reg [7:0]  pid_byte;
    reg [7:0]  tok0, tok1;
    reg [7:0]  d0, d1, d2, d3, d4, d5, d6, d7;
    reg [4:0]  crc5;
    reg [15:0] crc16;
    reg        too_long;

    wire [3:0] pid       = pid_byte[3:0];
    wire       pid_ok    = (pid_byte[7:4] == ~pid_byte[3:0]);
    wire       is_token  = (pid == PID_OUT) | (pid == PID_IN) | (pid == PID_SETUP);
    wire       is_data   = (pid == PID_DATA0) | (pid == PID_DATA1);
    wire [6:0] tok_addr  = tok0[6:0];
    wire [3:0] tok_endp  = {tok1[2:0], tok0[7]};
    wire       token_ok  = (n == 4'd3) & (crc5 == CRC5_RESIDUE);
    wire       data_ok   = (n >= 4'd3) & ~too_long & (crc16 == CRC16_RESIDUE);
    wire [3:0] data_len  = n - 4'd3;

    // -----------------------------------------------------------------
    // Endpoint 0.
    // -----------------------------------------------------------------
    reg [6:0]  addr;
    reg [6:0]  pending_addr;
    reg        set_addr;
    reg        config_q;
    reg        pending_config;
    reg        set_config;
    reg [2:0]  stage;
    reg [1:0]  expect;
    reg        toggle;
    reg        desc_sel;    // 0 device, 1 configuration
    reg [4:0]  in_total;    // bytes the data stage sends
    reg [4:0]  in_offset;   // bytes the host has acknowledged
    reg [3:0]  in_len;      // bytes in the packet awaiting its ACK
    reg        await_ack;

    // A response waits for the turnaround after the host's EOP.
    reg        pending;
    reg [3:0]  pend_pid;
    reg        pend_data;
    reg [3:0]  pend_len;
    reg [3:0]  turn;

    assign address    = addr;
    assign configured = config_q;

    // The descriptors, one byte at a time.
    function [7:0] desc;
        input       sel;
        input [4:0] i;
        begin
            if (!sel) begin
                case (i)
                    5'd0:    desc = 8'd18;        // bLength
                    5'd1:    desc = 8'd1;         // DEVICE
                    5'd2:    desc = 8'h00;        // bcdUSB 2.00
                    5'd3:    desc = 8'h02;
                    5'd4:    desc = 8'hFF;        // vendor specific
                    5'd5:    desc = 8'h00;
                    5'd6:    desc = 8'h00;
                    5'd7:    desc = 8'd8;         // bMaxPacketSize0
                    5'd8:    desc = VID[7:0];
                    5'd9:    desc = VID[15:8];
                    5'd10:   desc = PID[7:0];
                    5'd11:   desc = PID[15:8];
                    5'd12:   desc = 8'h00;        // bcdDevice 1.00
                    5'd13:   desc = 8'h01;
                    5'd14:   desc = 8'd0;         // no strings
                    5'd15:   desc = 8'd0;
                    5'd16:   desc = 8'd0;
                    5'd17:   desc = 8'd1;         // one configuration
                    default: desc = 8'd0;
                endcase
            end else begin
                case (i)
                    5'd0:    desc = 8'd9;         // bLength
                    5'd1:    desc = 8'd2;         // CONFIGURATION
                    5'd2:    desc = 8'd18;        // wTotalLength
                    5'd3:    desc = 8'd0;
                    5'd4:    desc = 8'd1;         // one interface
                    5'd5:    desc = 8'd1;         // bConfigurationValue
                    5'd6:    desc = 8'd0;
                    5'd7:    desc = 8'h80;        // bus powered
                    5'd8:    desc = 8'd50;        // 100 mA
                    5'd9:    desc = 8'd9;         // bLength
                    5'd10:   desc = 8'd4;         // INTERFACE
                    5'd11:   desc = 8'd0;         // bInterfaceNumber
                    5'd12:   desc = 8'd0;         // bAlternateSetting
                    5'd13:   desc = 8'd0;         // no endpoints
                    5'd14:   desc = 8'hFF;        // vendor specific
                    5'd15:   desc = 8'h00;
                    5'd16:   desc = 8'h00;
                    5'd17:   desc = 8'd0;
                    default: desc = 8'd0;
                endcase
            end
        end
    endfunction

    assign tx_byte = desc(desc_sel, in_offset + {1'b0, tx_index});

    // The next packet of the data stage.
    wire [4:0] in_left  = in_total - in_offset;
    wire [3:0] in_chunk = (in_left > 5'd8) ? 4'd8 : in_left[3:0];

    // The request, decoded from the SETUP data.
    wire [15:0] w_length = {d7, d6};
    wire        get_desc = (d0 == 8'h80) & (d1 == 8'h06) & (d2 == 8'h00)
                         & ((d3 == 8'h01) | (d3 == 8'h02));
    wire        set_adr  = (d0 == 8'h00) & (d1 == 8'h05) & ~d2[7] & (d3 == 8'h00);
    wire        set_cfg  = (d0 == 8'h00) & (d1 == 8'h09) & (d2[7:1] == 7'd0) & (d3 == 8'h00);
    wire [4:0]  desc_len = 5'd18;
    wire [4:0]  send_len = (w_length < {11'd0, desc_len}) ? w_length[4:0] : desc_len;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            n              <= 4'd0;
            pid_byte       <= 8'd0;
            tok0           <= 8'd0;
            tok1           <= 8'd0;
            d0 <= 8'd0; d1 <= 8'd0; d2 <= 8'd0; d3 <= 8'd0;
            d4 <= 8'd0; d5 <= 8'd0; d6 <= 8'd0; d7 <= 8'd0;
            crc5           <= 5'h1F;
            crc16          <= 16'hFFFF;
            too_long       <= 1'b0;
            addr           <= 7'd0;
            pending_addr   <= 7'd0;
            set_addr       <= 1'b0;
            config_q       <= 1'b0;
            pending_config <= 1'b0;
            set_config     <= 1'b0;
            stage          <= C_IDLE;
            expect         <= X_NONE;
            toggle         <= 1'b0;
            desc_sel       <= 1'b0;
            in_total       <= 5'd0;
            in_offset      <= 5'd0;
            in_len         <= 4'd0;
            await_ack      <= 1'b0;
            pending        <= 1'b0;
            pend_pid       <= 4'd0;
            pend_data      <= 1'b0;
            pend_len       <= 4'd0;
            turn           <= 4'd0;
            tx_start       <= 1'b0;
            tx_pid         <= 4'd0;
            tx_with_data   <= 1'b0;
            tx_len         <= 4'd0;
        end else begin
            tx_start <= 1'b0;

            // Bytes as they arrive.
            if (!rx_active) begin
                n        <= 4'd0;
                crc5     <= 5'h1F;
                crc16    <= 16'hFFFF;
                too_long <= 1'b0;
            end
            if (rx_valid) begin
                if (n != 4'd15) n <= n + 4'd1;
                if (n == 4'd0) begin
                    pid_byte <= rx_data;
                end else begin
                    crc5  <= crc5_byte(crc5, rx_data);
                    crc16 <= crc16_byte(crc16, rx_data);
                    case (n)
                        4'd1:  begin tok0 <= rx_data; d0 <= rx_data; end
                        4'd2:  begin tok1 <= rx_data; d1 <= rx_data; end
                        4'd3:  d2 <= rx_data;
                        4'd4:  d3 <= rx_data;
                        4'd5:  d4 <= rx_data;
                        4'd6:  d5 <= rx_data;
                        4'd7:  d6 <= rx_data;
                        4'd8:  d7 <= rx_data;
                        4'd9:  begin end
                        4'd10: begin end
                        default: too_long <= 1'b1;
                    endcase
                end
            end

            // A whole packet.
            if (rx_eop && pid_ok) begin
                if (is_token) begin
                    // A token ends any wait for a handshake.
                    await_ack <= 1'b0;
                    expect    <= X_NONE;
                    if (token_ok && tok_addr == addr && tok_endp == 4'd0) begin
                        if (pid == PID_SETUP) begin
                            expect <= X_SETUP;
                        end else if (pid == PID_OUT) begin
                            expect <= X_OUT;
                        end else begin
                            // IN: answer from the stage we are in.
                            pending <= 1'b1;
                            turn    <= 4'd0;
                            case (stage)
                                C_DATA_IN: begin
                                    pend_pid  <= toggle ? PID_DATA1 : PID_DATA0;
                                    pend_data <= 1'b1;
                                    pend_len  <= in_chunk;
                                    in_len    <= in_chunk;
                                    await_ack <= 1'b1;
                                end
                                C_STATUS_IN: begin
                                    pend_pid  <= PID_DATA1;
                                    pend_data <= 1'b1;
                                    pend_len  <= 4'd0;
                                    await_ack <= 1'b1;
                                end
                                C_STALL: begin
                                    pend_pid  <= PID_STALL;
                                    pend_data <= 1'b0;
                                end
                                default: begin
                                    pend_pid  <= PID_NAK;
                                    pend_data <= 1'b0;
                                end
                            endcase
                        end
                    end
                end else if (is_data) begin
                    expect <= X_NONE;
                    if (data_ok && expect == X_SETUP) begin
                        // A SETUP is always acknowledged, and starts a
                        // new control transfer whatever the last one was
                        // doing.
                        pending   <= 1'b1;
                        turn      <= 4'd0;
                        pend_pid  <= PID_ACK;
                        pend_data <= 1'b0;
                        toggle    <= 1'b1;
                        in_offset <= 5'd0;
                        set_addr  <= 1'b0;
                        set_config <= 1'b0;
                        if (pid != PID_DATA0 || data_len != 4'd8) begin
                            stage <= C_STALL;
                        end else if (get_desc) begin
                            stage    <= C_DATA_IN;
                            desc_sel <= (d3 == 8'h02);
                            in_total <= send_len;
                        end else if (set_adr) begin
                            stage        <= C_STATUS_IN;
                            pending_addr <= d2[6:0];
                            set_addr     <= 1'b1;
                        end else if (set_cfg) begin
                            stage          <= C_STATUS_IN;
                            pending_config <= d2[0];
                            set_config     <= 1'b1;
                        end else begin
                            stage <= C_STALL;
                        end
                    end else if (data_ok && expect == X_OUT) begin
                        pending   <= 1'b1;
                        turn      <= 4'd0;
                        pend_data <= 1'b0;
                        if (stage == C_DATA_IN && data_len == 4'd0) begin
                            // The status stage of a read.
                            pend_pid <= PID_ACK;
                            stage    <= C_IDLE;
                        end else begin
                            pend_pid <= PID_STALL;
                        end
                    end
                end else if (pid == PID_ACK && await_ack) begin
                    await_ack <= 1'b0;
                    if (stage == C_DATA_IN) begin
                        in_offset <= in_offset + {1'b0, in_len};
                        toggle    <= ~toggle;
                    end else if (stage == C_STATUS_IN) begin
                        stage <= C_IDLE;
                        if (set_addr)   addr     <= pending_addr;
                        if (set_config) config_q <= pending_config;
                        set_addr   <= 1'b0;
                        set_config <= 1'b0;
                    end
                end else begin
                    await_ack <= 1'b0;
                end
            end

            // The answer, once the host's EOP is over and the bus has
            // been J for the turnaround.
            if (pending && !tx_busy) begin
                if (!line_idle) begin
                    turn <= 4'd0;
                end else if (turn == TURNAROUND) begin
                    pending      <= 1'b0;
                    tx_start     <= 1'b1;
                    tx_pid       <= pend_pid;
                    tx_with_data <= pend_data;
                    tx_len       <= pend_len;
                end else begin
                    turn <= turn + 4'd1;
                end
            end

            if (bus_reset) begin
                addr       <= 7'd0;
                config_q   <= 1'b0;
                stage      <= C_IDLE;
                expect     <= X_NONE;
                await_ack  <= 1'b0;
                pending    <= 1'b0;
                set_addr   <= 1'b0;
                set_config <= 1'b0;
            end
        end
    end
endmodule
