// usb_pkt_rx — a USB packet, decoded once for every endpoint above it.
//
// What it does
//   Turns the byte stream a link layer delivers into one pulse per whole
//   packet, with everything an endpoint needs to decide what to do about
//   it: the PID and its check nibble, a token's address and endpoint
//   number with its CRC5 checked, a data packet's payload with its CRC16
//   checked, and how long that payload was.
//
//   This is the part of a USB device that is **the same for every
//   endpoint**, and that is why it is a module of its own. It used to be
//   the first third of `usb_ctrl_ep`, which was the whole device when
//   endpoint 0 was the only endpoint there was. A second endpoint beside
//   it would have needed its own copy of a CRC16 generator, a byte
//   counter and a PID check — three chances for two statements of one
//   thing to drift apart, and about a hundred LUTs of duplication. So the
//   decoder came out and both endpoints read the same pulse.
//
//   `pkt` is high for one cycle when a packet has ended and its PID check
//   nibble was right. Everything else is valid in that cycle:
//
//     pkt_is_token   the PID is OUT, IN or SETUP
//     tok_ok         it was three bytes and its CRC5 checks
//     tok_addr       the device address it names
//     tok_endp       the endpoint number it names
//     pkt_is_data    the PID is DATA0 or DATA1
//     dat_ok         its CRC16 checks and it was not too long
//     dat_len        the payload's length in bytes, 0 to 8
//     dat            the payload, byte 0 in the low eight bits
//
//   A handshake is neither, so an endpoint reads `pkt` and `pkt_pid` for
//   those. `pkt` fires for a packet with a bad CRC too — `tok_ok` and
//   `dat_ok` are what say the packet is usable — because a packet that
//   arrived at all, right or wrong, is a reason to stop waiting for a
//   handshake, and an endpoint needs to know that.
//
// What it does not do
//   Eight bytes of payload at most, which is what a maximum packet size
//   of eight needs, and `too_long` withdraws `dat_ok` from anything
//   longer rather than truncating it silently. Nothing here knows about
//   addresses, endpoints, toggles or requests: the decoder does not care
//   who a packet is for.
//
//   Nothing here knows about NRZI, bit stuffing, SYNC, EOP or line
//   states either. A packet whose PID check fails, whose bit stuffing is
//   broken or which ends off a byte boundary never reaches `rx_eop` at
//   all, so the link layer below has already dropped it.
module usb_pkt_rx (
    input  wire       clk,
    input  wire       rst_n,

    // The bytes of a received packet, from the link layer.
    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    input  wire       rx_eop,
    input  wire       rx_active,

    // One whole packet, for one cycle.
    output wire        pkt,
    output wire [3:0]  pkt_pid,
    output wire        pkt_is_token,
    output wire        pkt_is_data,
    output wire        tok_ok,
    output wire [6:0]  tok_addr,
    output wire [3:0]  tok_endp,
    output wire        dat_ok,
    output wire [3:0]  dat_len,
    output wire [63:0] dat
);
    // PIDs, the low nibble as it appears on the wire.
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_SETUP = 4'b1101;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;

    // The residues a correct CRC leaves in these reflected registers.
    localparam [4:0]  CRC5_RESIDUE  = 5'h06;
    localparam [15:0] CRC16_RESIDUE = 16'hB001;

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

    wire [3:0] pid    = pid_byte[3:0];
    wire       pid_ok = (pid_byte[7:4] == ~pid_byte[3:0]);

    assign pkt          = rx_eop & pid_ok;
    assign pkt_pid      = pid;
    assign pkt_is_token = (pid == PID_OUT) | (pid == PID_IN) | (pid == PID_SETUP);
    assign pkt_is_data  = (pid == PID_DATA0) | (pid == PID_DATA1);
    assign tok_addr     = tok0[6:0];
    assign tok_endp     = {tok1[2:0], tok0[7]};
    assign tok_ok       = (n == 4'd3) & (crc5 == CRC5_RESIDUE);
    assign dat_ok       = (n >= 4'd3) & ~too_long & (crc16 == CRC16_RESIDUE);
    assign dat_len      = n - 4'd3;
    assign dat          = {d7, d6, d5, d4, d3, d2, d1, d0};

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            n        <= 4'd0;
            pid_byte <= 8'd0;
            tok0     <= 8'd0;
            tok1     <= 8'd0;
            d0 <= 8'd0; d1 <= 8'd0; d2 <= 8'd0; d3 <= 8'd0;
            d4 <= 8'd0; d5 <= 8'd0; d6 <= 8'd0; d7 <= 8'd0;
            crc5     <= 5'h1F;
            crc16    <= 16'hFFFF;
            too_long <= 1'b0;
        end else begin
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
        end
    end
endmodule
