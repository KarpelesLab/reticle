// usb_bulk_ep — the endpoint that moves bytes: one bulk OUT and one bulk IN
// on the same endpoint number, with a byte interface for whatever is above
// them.
//
// What it does
//   Once a host has enumerated a device it stops asking questions and
//   starts moving data, and this is the part that answers that. It reads
//   the same `usb_pkt_rx` pulse `usb_ctrl_ep` reads, answers tokens for
//   endpoint `ENDP` only, and presents each direction as a byte stream:
//
//     OUT, host to device        `out_data` with `out_valid`, taken when
//                                `out_ready`, and `out_last` on the last
//                                byte of the packet the host sent
//     IN, device to host         `in_data` with `in_valid`, taken when
//                                `in_ready`; `in_commit` sends what has
//                                been given, however short
//
//   One packet of each direction is in flight at a time, which is what
//   makes the endpoint small: a buffer of MAXPKT bytes each way and no
//   FIFO. `fifo_sync` is a block in this library already, so a design that
//   wants depth puts one on either side rather than paying for it here.
//
//   **Flow control is the host's problem, which is what bulk means.** An
//   OUT packet that arrives while the last one has not been drained is
//   answered with NAK and the host sends it again; an IN token that
//   arrives with nothing ready is answered with NAK and the host asks
//   again. Neither loses a byte and neither needs the logic above to be
//   fast.
//
//   The data toggle is kept per direction, as USB 2.0 §8.6 asks. An OUT
//   packet whose PID is not the toggle expected is the host not having
//   heard the last ACK, so it is acknowledged again and **discarded**
//   rather than delivered twice. An IN packet the host does not
//   acknowledge is sent again with the same toggle, because nothing is
//   released until the ACK arrives. `ep_reset` puts both toggles back to
//   DATA0, which is what SET_CONFIGURATION means, and `ep_clear` does it
//   for one direction, which is what CLEAR_FEATURE(ENDPOINT_HALT) means.
//
//   The turnaround — `TURNAROUND` cycles of `line_idle` after the host's
//   packet before the answer goes out — is counted here and also in
//   `usb_ctrl_ep`. That is two statements of one rule, and deliberately:
//   the alternative is a shared counter driven by whichever endpoint is
//   answering, and then an endpoint has to be told that its packet went
//   out a cycle after it did. Ten lines of counter in each endpoint is
//   cheaper to read and to be sure of than a handshake between three
//   modules.
//
// What it does not do
//   One endpoint number, one packet deep, `MAXPKT` of at most 8 bytes —
//   which is what the four-bit length the transmitters take allows, and
//   which USB 2.0 §5.8.3 lists as a legal full-speed bulk size beside 16,
//   32 and 64. No isochronous and no interrupt endpoint, though an
//   interrupt endpoint is this module with a different bmAttributes in the
//   descriptor and nothing else: the packets are identical and only the
//   host's scheduling differs.
//
//   No STALL of its own: nothing here halts, so there is nothing to clear
//   except the toggle. A SETUP addressed to a bulk endpoint is ignored,
//   since a bulk endpoint has no control pipe and a host that sends one
//   has made a mistake no answer would tell it about.
//
//   Nothing here decodes a packet or checks a CRC — `usb_pkt_rx` does —
//   and nothing here knows the device's address: `address` comes from
//   `usb_ctrl_ep`, which is what SET_ADDRESS moved.
module usb_bulk_ep #(
    // The endpoint number both directions use. Endpoint 0 is the control
    // endpoint's and must not be given here.
    parameter [3:0]  ENDP       = 4'd1,
    // Bytes in a packet, 1 to 8.
    parameter [3:0]  MAXPKT     = 4'd8,
    // Cycles of `line_idle` before an answer starts; `usb_ctrl_ep`'s
    // parameter of the same name says what it has to be and why.
    parameter [6:0]  TURNAROUND = 7'd8
) (
    input  wire       clk,
    input  wire       rst_n,

    // One whole packet, from `usb_pkt_rx`.
    input  wire        pkt,
    input  wire [3:0]  pkt_pid,
    input  wire        pkt_is_token,
    input  wire        pkt_is_data,
    input  wire        tok_ok,
    input  wire [6:0]  tok_addr,
    input  wire [3:0]  tok_endp,
    input  wire        dat_ok,
    input  wire [3:0]  dat_len,
    input  wire [63:0] dat,

    // The device's address, from the control endpoint.
    input  wire [6:0] address,
    // The bus is idle, so an answer may be timed from now.
    input  wire       line_idle,
    // The host has reset the bus.
    input  wire       bus_reset,
    // Both toggles back to DATA0, and one direction's.
    input  wire       ep_reset,
    input  wire       ep_clear,
    input  wire [7:0] ep_clear_ep,
    // This endpoint owns the transmitter.
    input  wire       sel,

    // One packet out.
    output reg        tx_start,
    output reg  [3:0] tx_pid,
    output reg        tx_with_data,
    output reg  [3:0] tx_len,
    input  wire [3:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,

    // The bytes the host sent.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,

    // The bytes to send it.
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit
);
    // PIDs, the low nibble as it appears on the wire.
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;
    localparam [3:0] PID_ACK   = 4'b0010;
    localparam [3:0] PID_NAK   = 4'b1010;

    // The endpoint addresses this endpoint answers CLEAR_FEATURE for: the
    // direction bit and the number.
    localparam [7:0] ADDR_OUT = {4'h0, ENDP};
    localparam [7:0] ADDR_IN  = {4'h8, ENDP};

    // -----------------------------------------------------------------
    // Host to device.
    // -----------------------------------------------------------------
    // The byte index is scaled to a bit index by concatenation and not by
    // `* 8`. A multiply by a power of two is a shift, but this compiler's
    // synthesis does not strength-reduce one: `ordx * 8` became a `mul`
    // cell, and on the ECP5 a `MULT18X18D`. Three of them, across the two
    // buffers and the descriptor, cost 640 LUT4 and two hard multipliers.
    reg [63:0] obuf;        // the packet, byte 0 in the low eight bits
    reg [3:0]  olen;        // bytes in it, 0 when it has been drained
    reg [2:0]  ordx;        // the byte being handed over
    reg        out_toggle;  // the PID the next packet should carry
    reg        expect_out;  // an OUT token has been seen and its data is next

    assign out_valid = (olen != 4'd0);
    assign out_data  = obuf[{ordx, 3'b000} +: 8];
    assign out_last  = (({1'b0, ordx} + 4'd1) == olen);

    // -----------------------------------------------------------------
    // Device to host.
    // -----------------------------------------------------------------
    // Eight byte registers and a `case`, rather than one wide register and
    // a part-select: an assignment to a part-select whose bound is not
    // constant is not something every tool takes, and a decoder is what
    // this becomes either way.
    reg [7:0] i0, i1, i2, i3, i4, i5, i6, i7;
    reg [3:0] ilen;         // bytes given so far
    reg       armed;        // the packet is ready to go out
    reg       in_toggle;    // the PID it will carry
    reg       in_await;     // it has gone out and its ACK has not come back

    wire [63:0] ibuf = {i7, i6, i5, i4, i3, i2, i1, i0};

    // Room while no packet is waiting to be sent. `armed` covers the full
    // buffer too, since the eighth byte arms it.
    assign in_ready = ~armed;
    assign tx_byte  = ibuf[{tx_index[2:0], 3'b000} +: 8];

    // -----------------------------------------------------------------
    // The answer, and when it may go out.
    // -----------------------------------------------------------------
    reg       pending;
    reg [3:0] pend_pid;
    reg       pend_data;
    reg [3:0] pend_len;
    reg [6:0] turn;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            obuf       <= 64'd0;
            olen       <= 4'd0;
            ordx       <= 3'd0;
            out_toggle <= 1'b0;
            expect_out <= 1'b0;
            i0 <= 8'd0; i1 <= 8'd0; i2 <= 8'd0; i3 <= 8'd0;
            i4 <= 8'd0; i5 <= 8'd0; i6 <= 8'd0; i7 <= 8'd0;
            ilen       <= 4'd0;
            armed      <= 1'b0;
            in_toggle  <= 1'b0;
            in_await   <= 1'b0;
            pending    <= 1'b0;
            pend_pid   <= 4'd0;
            pend_data  <= 1'b0;
            pend_len   <= 4'd0;
            turn       <= 7'd0;
            tx_start   <= 1'b0;
            tx_pid     <= 4'd0;
            tx_with_data <= 1'b0;
            tx_len     <= 4'd0;
        end else begin
            tx_start <= 1'b0;

            // ---------------------------------------------------------
            // A whole packet from the host.
            // ---------------------------------------------------------
            if (pkt) begin
                if (pkt_is_token) begin
                    // A token ends any wait for a handshake, whoever it
                    // was for.
                    in_await   <= 1'b0;
                    expect_out <= 1'b0;
                    if (tok_ok && tok_addr == address && tok_endp == ENDP) begin
                        if (pkt_pid == PID_OUT) begin
                            expect_out <= 1'b1;
                        end else if (pkt_pid == PID_IN) begin
                            pending <= 1'b1;
                            turn    <= 7'd0;
                            if (armed) begin
                                pend_pid  <= in_toggle ? PID_DATA1 : PID_DATA0;
                                pend_data <= 1'b1;
                                pend_len  <= ilen;
                                in_await  <= 1'b1;
                            end else begin
                                pend_pid  <= PID_NAK;
                                pend_data <= 1'b0;
                            end
                        end
                        // A SETUP is not answered at all. A bulk endpoint
                        // has no control pipe, and a token is not a thing
                        // a device answers: what it would answer is the
                        // data packet behind the token, and a STALL sent
                        // in the token's turnaround would land on top of
                        // that packet — which is how this was wrong once,
                        // and what `the host saw the protocol broken: the
                        // device drives the pair while the host does`
                        // said about it.
                    end
                end else if (pkt_is_data) begin
                    expect_out <= 1'b0;
                    if (dat_ok && expect_out) begin
                        pending   <= 1'b1;
                        turn      <= 7'd0;
                        pend_data <= 1'b0;
                        if (pkt_pid != (out_toggle ? PID_DATA1 : PID_DATA0)) begin
                            // The host did not hear the last ACK. Say it
                            // again and drop the copy.
                            pend_pid <= PID_ACK;
                        end else if (olen == 4'd0) begin
                            obuf       <= dat;
                            olen       <= dat_len;
                            ordx       <= 3'd0;
                            out_toggle <= ~out_toggle;
                            pend_pid   <= PID_ACK;
                        end else begin
                            // Nothing has taken the last packet yet.
                            pend_pid <= PID_NAK;
                        end
                    end
                end else if (pkt_pid == PID_ACK && in_await) begin
                    // The packet arrived. The buffer is free and the
                    // toggle moves on.
                    in_await  <= 1'b0;
                    ilen      <= 4'd0;
                    armed     <= 1'b0;
                    in_toggle <= ~in_toggle;
                end else begin
                    in_await <= 1'b0;
                end
            end

            // ---------------------------------------------------------
            // The bytes handed over, and the bytes given.
            // ---------------------------------------------------------
            if (out_valid && out_ready) begin
                if (out_last) begin
                    olen <= 4'd0;
                    ordx <= 3'd0;
                end else begin
                    ordx <= ordx + 3'd1;
                end
            end

            if (in_valid && in_ready) begin
                case (ilen)
                    4'd0:    i0 <= in_data;
                    4'd1:    i1 <= in_data;
                    4'd2:    i2 <= in_data;
                    4'd3:    i3 <= in_data;
                    4'd4:    i4 <= in_data;
                    4'd5:    i5 <= in_data;
                    4'd6:    i6 <= in_data;
                    default: i7 <= in_data;
                endcase
                ilen <= ilen + 4'd1;
                if ((ilen + 4'd1) == MAXPKT || in_commit) armed <= 1'b1;
            end else if (in_commit && !armed) begin
                // A short packet, or a zero-length one, on request.
                armed <= 1'b1;
            end

            // ---------------------------------------------------------
            // The answer, once the bus has been idle for the turnaround.
            // ---------------------------------------------------------
            if (pending && !tx_busy && sel) begin
                if (!line_idle) begin
                    turn <= 7'd0;
                end else if (turn == TURNAROUND) begin
                    pending      <= 1'b0;
                    tx_start     <= 1'b1;
                    tx_pid       <= pend_pid;
                    tx_with_data <= pend_data;
                    tx_len       <= pend_len;
                end else begin
                    turn <= turn + 7'd1;
                end
            end

            // ---------------------------------------------------------
            // The toggles, and a bus reset.
            // ---------------------------------------------------------
            if (ep_reset) begin
                out_toggle <= 1'b0;
                in_toggle  <= 1'b0;
            end
            if (ep_clear && ep_clear_ep == ADDR_OUT) out_toggle <= 1'b0;
            if (ep_clear && ep_clear_ep == ADDR_IN)  in_toggle  <= 1'b0;

            if (bus_reset) begin
                olen       <= 4'd0;
                ordx       <= 3'd0;
                out_toggle <= 1'b0;
                expect_out <= 1'b0;
                ilen       <= 4'd0;
                armed      <= 1'b0;
                in_toggle  <= 1'b0;
                in_await   <= 1'b0;
                pending    <= 1'b0;
            end
        end
    end
endmodule
