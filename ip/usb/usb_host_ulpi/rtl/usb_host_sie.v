// usb_host_sie — the serial interface engine of a USB full-speed host:
// one transaction at a time, a frame every millisecond, and a timeout
// with retries when a device says nothing.
//
// What it does
//   A host's work on the wire is three things, and this module is all
//   three.
//
//   **A token.** Every transaction starts with one the host sends and
//   nobody answers: a PID and two bytes carrying seven bits of device
//   address, four of endpoint number and a CRC5 over the eleven of them
//   (USB 2.0 §8.4.1, §8.4.2, §8.3.5). The CRC5 is here and not in the
//   Link because it covers an address and an endpoint, which is what this
//   module knows; the Link's CRC16 is the Link's for the mirror reason.
//
//   **A frame.** A full-speed host sends a SOF token every millisecond
//   carrying an 11-bit frame number with its own CRC5 (§8.4.3), and the
//   number increments every frame. This is not decoration: a device that
//   sees no bus activity for 3 ms enters suspend (§7.1.7.6), and an
//   enumeration has gaps longer than that in it. `FRAME_CYCLES` is the
//   millisecond, in clocks.
//
//   **A reply, or the absence of one.** After a token — or after the data
//   packet that follows a SETUP or an OUT — the host listens. What comes
//   back is a handshake (ACK, NAK or STALL), a data packet, or nothing;
//   and "nothing" has to be a decision rather than a wait, because a
//   device that is not there never answers. `TIMEOUT_CYCLES` is that
//   decision and its comment says why it is far larger than the 16 bit
//   times USB 2.0 §7.1.19.1 gives a host.
//
//   `usb_pkt_rx` decodes the reply, and it is **`ip/usb/usb_device_fs`'s**,
//   reached through this package's `depends` line rather than copied. The
//   PID's check nibble, the CRC16 and a payload with its CRC taken out are
//   the same work in both directions, and that module has been doing it on
//   a board. Its `pkt` pulse fires for a handshake as well as for a data
//   packet — "a packet that arrived at all, right or wrong, is a reason to
//   stop waiting", its own header says — which is exactly what a host's
//   timeout needs to end on.
//
// WAITING FOR ITS OWN PACKET TO FINISH ON THE WIRE
//   This is the one thing a host must do that a peripheral's link layer
//   gets away with not doing, and it is worth stating at length because
//   getting it wrong is silent.
//
//   ULPI 1.1 §3.8.2.2: after `stp` the Link cannot transmit again until
//   the packet has finished on the wire. A peripheral never has a second
//   packet ready that soon. A host's SETUP token is followed
//   **immediately** by its data packet, so it does.
//
//   `tx_busy` is no help. It falls with the `stp` that ends the *bus*
//   transfer, and on the part this was written for the bus transfer
//   finishes long before the wire does: three bytes went across in 32
//   clocks where the wire needs 175, and "twelve or more receive commands
//   follow it inside 8.5 us" (`docs/fpga-trellis.md`). The transceiver
//   announces the real end with a receive command carrying the SE0-to-J
//   transition, and that one cannot be picked out of the others, because
//   "after STP is asserted each FS/LS bit transition will generate a
//   RXCMD" (USB334x DS00002646A §6.3.1) and the Link is handed a backlog.
//
//   So the wait is **counted**, and only in the three places where the
//   next thing this engine does is transmit:
//
//     after a SOF                     3 bytes on the wire
//     after a token with data to come 3 bytes
//     after the ACK for a packet      1 byte
//
//   `TX_PER_BYTE` clocks a byte and `TX_OVERHEAD` for the SYNC field, the
//   end of packet and the transceiver's own latency. At 60 MHz a
//   full-speed bit is 5 clocks, so a byte is 40 — and the default is 48,
//   which is 40 plus the worst case of a stuffed zero after every six ones
//   (a byte of all ones is 9⅓ bit times). Both are upper bounds on
//   purpose: too long only delays the next packet, too short puts a
//   transmit command onto a bus the transceiver is not listening to.
//
//   **There is no count after this host's own data packet**, and that is
//   not an omission. What follows a data packet is always listening, never
//   transmitting, and by the time the listening is over the wire is
//   provably clear: either a reply arrived, which the device could only
//   send after hearing the whole packet, or `TIMEOUT_CYCLES` went by,
//   which is longer than the longest packet this host sends.
//
//   **What this costs, and the measurement to make if a device will not
//   answer a SETUP.** The gap this leaves between the host's token and its
//   data packet is the count's slack plus `GAP_CYCLES`, which is about ten
//   bit times rather than the two to six and a half USB 2.0 §7.1.18 names
//   for a *response*. Nothing in USB 2.0 puts a ceiling on how long a host
//   may leave between two packets it sends itself, and a device's receiver
//   has no timer between a token and the data after it — it decodes
//   packets and reacts. `ip/usb/usb_device_fs`'s does not, and that is checked
//   in simulation. A device that did would be the first thing a ULPI trace
//   should be pointed at, and the fix is not a shorter count but a
//   measurement of the transceiver's transmit-command-to-wire latency,
//   which is what `TX_OVERHEAD` is standing in for.
//
// What it does not do
//   **One transaction at a time, and no schedule.** A host controller
//   proper keeps a list of endpoints and a budget per frame; this keeps
//   one transaction and a frame counter. Whatever drives `trn_start`
//   decides what to ask for and when, and `usb_host_enum` beside this is
//   the one thing in this package that does.
//
//   **A SOF may be late.** The frame counter never drifts — it is free
//   running and `sof_due` is sticky — but a SOF that falls due while a
//   transaction is in flight goes out when that transaction is over rather
//   than interrupting it. A transaction that succeeds is a few microseconds
//   and one retried to exhaustion is `RETRIES + 1` timeouts — 273 us at the
//   defaults, a quarter of a frame — against the sub-microsecond jitter USB
//   2.0 §7.1.12 allows a host. A device measuring its own frame interval
//   would see that; one that only needs to be told not to suspend would not.
//   Interrupting a transaction to keep the SOF on time is the change to
//   make, and it needs the retry to be able to resume rather than restart.
//
//   **No isochronous or interrupt transfers, no split transactions, no low
//   speed, no high speed and no hub.** There is no PRE token, so nothing
//   downstream of a hub at low speed can be reached, and no chirp, so
//   nothing negotiates high speed. This host talks to the one device on
//   its port.
//
//   **No data toggle of its own.** `trn_toggle` says which of DATA0 and
//   DATA1 the packet this transaction sends carries, and `trn_rx_pid`
//   reports which came back; keeping the two in step across a transfer
//   belongs to whatever knows what the transfer is. A toggle kept here
//   would be one toggle for every endpoint, which is wrong by
//   construction.
//
//   **A data packet whose CRC16 fails is treated as no answer**, which is
//   USB's own rule — the host sends no handshake and the device sends the
//   packet again — and it is why `ST_ERROR` only appears once the retries
//   have run out.
module usb_host_sie #(
    // Clocks in a frame: 1 ms at 60 MHz.
    parameter integer FRAME_CYCLES = 60000,
    // Clocks waited for a reply, counted from the start of the host's own
    // transmit.
    //
    // USB 2.0 §7.1.19.1 gives the host a limit of **16 bit times** after
    // the end of its own packet — 80 clocks at 60 MHz — and ULPI 1.1
    // Table 10 says the same in interface clocks. This is far larger than
    // either, and the asymmetry is the reason: a timeout too long only
    // delays the retry of a transaction that was going to fail anyway, and
    // a timeout too short abandons an answer that was on its way. Nothing
    // on this bus is more correct for giving up sooner.
    //
    // It is also measured from a different place. Counting from the end of
    // the host's packet on the wire would need to know where that is,
    // which is the whole subject of this file's header; counting from the
    // transmit instead means the packet's own wire time is inside the
    // window, so the window has to be longer than the longest packet.
    // A 64-byte data packet is 547 bit times, 2735 clocks.
    //
    // And it has to cover a **model** of a transceiver as well as one. The
    // model in `tests/ip_library.rs` is store and forward: it collects a
    // whole packet from its Link before any of it reaches the pair, so a
    // reply arrives a packet's length later there than on a part that
    // serialises as the bytes come in. 4096 clocks is 68 us, which is both
    // ends of that and still under a tenth of a frame.
    parameter integer TIMEOUT_CYCLES = 4096,
    // How many times a transaction is sent again when nothing usable came
    // back. USB 2.0 §8.6.4 leaves the number to the host; three is what a
    // bus error budget usually gets.
    parameter integer RETRIES = 3,
    // Clocks of a full-speed byte on the wire, and the clocks the SYNC
    // field, the EOP and the transceiver's own latency add. The header
    // says why 48 and not 40, and what the slack costs.
    parameter integer TX_PER_BYTE = 48,
    parameter integer TX_OVERHEAD = 128,
    // Clocks of an idle bus between the host's own two packets — a token
    // and the data that follows it — counted from the end of the first on
    // the wire. ULPI 1.1 Table 10 allows a full-speed Link 7 to 18.
    parameter integer GAP_CYCLES = 12,
    // Clocks of an idle bus after a received packet before the host's
    // handshake goes out, from the same table and the same window. Nine is
    // what `ip/usb/usb_device_ulpi` answers in and for the same reason: it
    // lands in the middle of the 2 to 6.5 bit times USB 2.0 §7.1.18
    // allows.
    parameter integer TURNAROUND = 9
) (
    input  wire       clk,
    input  wire       rst_n,

    // The ULPI Link. Every one of these is `usb_ulpi_host_link`'s port of
    // the same name.
    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    input  wire       rx_eop,
    input  wire       rx_active,
    output wire       tx_start,
    output wire [3:0] tx_pid,
    output wire [1:0] tx_mode,
    output wire [6:0] tx_len,
    input  wire [6:0] tx_index,
    output wire [7:0] tx_byte,
    input  wire       tx_busy,
    input  wire       tx_abort,

    // Frames. `sof_en` low stops them, which is what a host wants while it
    // is driving a bus reset: the transceiver is in high-speed mode then
    // and a full-speed token has nowhere to go.
    input  wire        sof_en,
    output wire [10:0] frame,
    output wire        sof_sent,

    // One transaction. Assert `trn_start` and hold it until `trn_busy`
    // goes high, then drop it and wait for `trn_done`. The address,
    // endpoint, kind, toggle and length are sampled when it is taken.
    input  wire        trn_start,
    input  wire [1:0]  trn_kind,
    input  wire [6:0]  trn_addr,
    input  wire [3:0]  trn_endp,
    input  wire        trn_toggle,
    input  wire [6:0]  trn_len,
    input  wire [7:0]  trn_byte,
    output wire [6:0]  trn_index,
    output wire        trn_busy,
    output wire        trn_done,
    output wire [2:0]  trn_status,
    output wire [3:0]  trn_rx_pid,
    output wire [6:0]  trn_rx_len,

    // The payload of a data packet the host received, a byte a cycle with
    // the PID and the CRC16 already taken out, and the byte's index in the
    // packet. A consumer that writes it at `base + in_index` and only
    // advances `base` when `trn_status` was `ST_DATA` never has to undo a
    // packet that turned out to be broken.
    output wire [7:0]  in_byte,
    output wire        in_push,
    output wire [6:0]  in_index
);
    // The kinds of transaction.
    localparam [1:0] K_SETUP = 2'd0;
    localparam [1:0] K_IN    = 2'd1;
    localparam [1:0] K_OUT   = 2'd2;

    // What `trn_status` says.
    localparam [2:0] ST_ACK     = 3'd0;  // the device acknowledged
    localparam [2:0] ST_NAK     = 3'd1;  // it has nothing, or is not ready
    localparam [2:0] ST_STALL   = 3'd2;  // it refuses the request
    localparam [2:0] ST_DATA    = 3'd3;  // a data packet arrived and was ACKed
    localparam [2:0] ST_TIMEOUT = 3'd4;  // nothing came back, after the retries
    localparam [2:0] ST_ERROR   = 3'd5;  // something came back, unusable

    // PIDs, the low nibble as it goes on the wire (USB 2.0 Table 8-1).
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_SOF   = 4'b0101;
    localparam [3:0] PID_SETUP = 4'b1101;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;
    localparam [3:0] PID_ACK   = 4'b0010;
    localparam [3:0] PID_NAK   = 4'b1010;
    localparam [3:0] PID_STALL = 4'b1110;

    // What the Link's `tx_mode` selects.
    localparam [1:0] TX_HANDSHAKE = 2'd0;
    localparam [1:0] TX_RAW       = 2'd1;
    localparam [1:0] TX_DATA      = 2'd2;

    // The states.
    localparam [3:0] H_IDLE   = 4'd0;   // nothing in flight
    localparam [3:0] H_SOF    = 4'd1;   // the frame's own token
    localparam [3:0] H_TOKEN  = 4'd2;   // the transaction's token
    localparam [3:0] H_DATA   = 4'd3;   // the data packet after it
    localparam [3:0] H_ACK    = 4'd4;   // the handshake for a packet received
    localparam [3:0] H_WIRE   = 4'd5;   // our packet finishing on the wire
    localparam [3:0] H_GAP    = 4'd6;   // the idle bus between two packets
    localparam [3:0] H_RXWAIT = 4'd7;   // listening for the reply
    localparam [3:0] H_RETRY  = 4'd8;   // decide whether to send it again
    localparam [3:0] H_END    = 4'd9;   // report it, and go idle

    // The two counts of a packet on the wire. Both are constants: the
    // header says why no length-dependent one is needed.
    localparam integer WIRE_TOKEN = 3 * TX_PER_BYTE + TX_OVERHEAD;
    localparam integer WIRE_HS    = 1 * TX_PER_BYTE + TX_OVERHEAD;

    localparam integer GAP_MAX = (GAP_CYCLES > TURNAROUND) ? GAP_CYCLES
                                                           : TURNAROUND;

    localparam integer FRAME_W = $clog2(FRAME_CYCLES);
    localparam integer TO_W    = $clog2(TIMEOUT_CYCLES + 1);
    localparam integer WIRE_W  = $clog2(WIRE_TOKEN + 1);
    localparam integer GAP_W   = $clog2(GAP_MAX + 1);
    localparam integer TRY_W   = $clog2(RETRIES + 1);

    reg [3:0]         state;
    // Where `H_WIRE` goes when the packet has had its time on the wire, and
    // where `H_GAP` goes when the gap is over. **Two registers and not one**:
    // the one sequence that uses both is a token followed by a data packet,
    // where the wire's own wait is followed by the gap and the gap is
    // followed by the data. One register for both means `H_GAP` sends itself
    // back to `H_GAP`, which is a token that goes out once and a transaction
    // that never ends.
    reg [3:0]         after;
    reg [3:0]         after_gap;
    reg [FRAME_W-1:0] frame_cnt;
    reg [10:0]        frame_q;
    reg               sof_due;
    reg               sof_sent_q;
    reg [TO_W-1:0]    to_cnt;
    reg [WIRE_W-1:0]  wire_left;
    reg [GAP_W-1:0]   gap_left;
    reg [TRY_W-1:0]   tries;

    // The transaction in flight.
    reg        trn_act;
    reg [1:0]  kind_q;
    reg [6:0]  addr_q;
    reg [3:0]  endp_q;
    reg        toggle_q;
    reg [6:0]  len_q;
    reg [2:0]  status_q;
    reg        done_q;
    reg [3:0]  rx_pid_q;
    reg [6:0]  rx_len_q;
    reg [6:0]  in_idx;
    // Anything at all came back during this transaction, which is the
    // difference between a device that is silent and one that is heard and
    // not understood. Per transaction, so it is cleared where one starts.
    reg        saw_reply;

    // What is being transmitted.
    reg        tx_start_q;
    reg [3:0]  tx_pid_q;
    reg [1:0]  tx_mode_q;
    reg [6:0]  tx_len_q;

    // The two bytes of the token being sent, latched when it starts so
    // that nothing downstream recomputes a CRC5 per cycle.
    reg [7:0]  tok0, tok1;

    // --------------------------------------------------------------
    // The reply, decoded. `usb_pkt_rx` is `ip/usb/usb_device_fs`'s.
    // --------------------------------------------------------------
    wire        pkt;
    wire [3:0]  pkt_pid;
    wire        pkt_is_data;
    wire        dat_ok;
    wire [6:0]  dat_len;
    wire [7:0]  pay_byte;
    wire        pay_push;

    usb_pkt_rx u_rx (
        .clk          (clk),
        .rst_n        (rst_n),
        .rx_data      (rx_data),
        .rx_valid     (rx_valid),
        .rx_eop       (rx_eop),
        .rx_active    (rx_active),
        .pkt          (pkt),
        .pkt_pid      (pkt_pid),
        .pkt_is_token (),
        .pkt_is_data  (pkt_is_data),
        .tok_ok       (),
        .tok_addr     (),
        .tok_endp     (),
        .dat_ok       (dat_ok),
        .dat_len      (dat_len),
        .dat          (),
        .pay_byte     (pay_byte),
        .pay_push     (pay_push)
    );

    wire is_handshake = (pkt_pid == PID_ACK) | (pkt_pid == PID_NAK)
                      | (pkt_pid == PID_STALL);

    assign frame      = frame_q;
    assign sof_sent   = sof_sent_q;
    assign trn_busy   = trn_act;
    assign trn_done   = done_q;
    assign trn_status = status_q;
    assign trn_rx_pid = rx_pid_q;
    assign trn_rx_len = rx_len_q;
    assign trn_index  = tx_index;

    assign tx_start = tx_start_q;
    assign tx_pid   = tx_pid_q;
    assign tx_mode  = tx_mode_q;
    assign tx_len   = tx_len_q;
    // A token's two bytes come from here; a data packet's come from above.
    assign tx_byte  = (tx_mode_q == TX_RAW) ? (tx_index[0] ? tok1 : tok0)
                                            : trn_byte;

    assign in_byte  = pay_byte;
    assign in_push  = pay_push & (state == H_RXWAIT) & (kind_q == K_IN);
    assign in_index = in_idx;

    // The CRC5 of the eleven bits a token carries: polynomial x^5 + x^2 + 1,
    // reflected, all ones to start and inverted at the end (USB 2.0 §8.3.5).
    function [4:0] crc5_11;
        input [10:0] d;
        integer      i;
        reg   [4:0]  r;
        begin
            r = 5'h1F;
            for (i = 0; i < 11; i = i + 1)
                r = (r[0] ^ d[i]) ? ((r >> 1) ^ 5'h14) : (r >> 1);
            crc5_11 = ~r;
        end
    endfunction

    // The eleven bits of this transaction's token, and of a SOF. USB 2.0
    // §8.4.1: the address is the low seven bits and the endpoint the next
    // four, least significant bit first on the wire, which is what putting
    // the address in the low bits of a little-endian pair of bytes does.
    wire [10:0] tok_field = {endp_q, addr_q};
    wire [4:0]  tok_crc   = crc5_11(tok_field);
    wire [10:0] sof_field = frame_q;
    wire [4:0]  sof_crc   = crc5_11(sof_field);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state      <= H_IDLE;
            after      <= H_IDLE;
            after_gap  <= H_IDLE;
            frame_cnt  <= 0;
            frame_q    <= 11'd0;
            sof_due    <= 1'b0;
            sof_sent_q <= 1'b0;
            to_cnt     <= 0;
            wire_left  <= 0;
            gap_left   <= 0;
            tries      <= 0;
            trn_act    <= 1'b0;
            kind_q     <= K_SETUP;
            addr_q     <= 7'd0;
            endp_q     <= 4'd0;
            toggle_q   <= 1'b0;
            len_q      <= 7'd0;
            status_q   <= ST_TIMEOUT;
            done_q     <= 1'b0;
            rx_pid_q   <= 4'd0;
            rx_len_q   <= 7'd0;
            in_idx     <= 7'd0;
            saw_reply  <= 1'b0;
            tx_start_q <= 1'b0;
            tx_pid_q   <= 4'd0;
            tx_mode_q  <= TX_HANDSHAKE;
            tx_len_q   <= 7'd0;
            tok0       <= 8'h00;
            tok1       <= 8'h00;
        end else begin
            done_q     <= 1'b0;
            sof_sent_q <= 1'b0;
            // `tx_start` is **held until the Link takes it** and not pulsed
            // for a cycle. The Link ignores `tx_start` in any cycle `dir`
            // is high, and `dir` rising is the transceiver's own business
            // and may land in exactly the cycle a pulse was in; a held
            // request goes out when the bus comes back instead of being
            // lost and waited out as a timeout. `tx_busy` going high is
            // the Link saying it has the packet.
            if (tx_start_q && tx_busy) tx_start_q <= 1'b0;

            // The frame clock, free running so that a SOF the engine could
            // not send on time does not move the next one.
            if (frame_cnt == FRAME_CYCLES[FRAME_W-1:0] - 1'b1) begin
                frame_cnt <= 0;
                sof_due   <= 1'b1;
            end else begin
                frame_cnt <= frame_cnt + 1'b1;
            end

            // The index of the payload byte arriving, which is what a
            // consumer writes at.
            if (pay_push && state == H_RXWAIT) in_idx <= in_idx + 7'd1;

            case (state)
                H_IDLE: begin
                    if (sof_due && sof_en && !tx_busy) begin
                        // The frame's token. The number increments first,
                        // so the one in the packet is this frame's.
                        frame_q <= frame_q + 11'd1;
                        sof_due <= 1'b0;
                        state   <= H_SOF;
                    end else if (trn_start && !trn_act) begin
                        kind_q    <= trn_kind;
                        addr_q    <= trn_addr;
                        endp_q    <= trn_endp;
                        toggle_q  <= trn_toggle;
                        len_q     <= trn_len;
                        tries     <= 0;
                        saw_reply <= 1'b0;
                        rx_pid_q  <= 4'd0;
                        rx_len_q  <= 7'd0;
                        trn_act   <= 1'b1;
                        state     <= H_TOKEN;
                    end
                end
                H_SOF: begin
                    if (!tx_busy) begin
                        tok0       <= sof_field[7:0];
                        tok1       <= {sof_crc, sof_field[10:8]};
                        tx_pid_q   <= PID_SOF;
                        tx_mode_q  <= TX_RAW;
                        tx_len_q   <= 7'd2;
                        tx_start_q <= 1'b1;
                        sof_sent_q <= 1'b1;
                        wire_left  <= WIRE_TOKEN[WIRE_W-1:0];
                        after      <= H_IDLE;
                        state      <= H_WIRE;
                    end
                end
                H_TOKEN: begin
                    if (!tx_busy) begin
                        tok0       <= tok_field[7:0];
                        tok1       <= {tok_crc, tok_field[10:8]};
                        tx_pid_q   <= (kind_q == K_IN)    ? PID_IN
                                    : (kind_q == K_SETUP) ? PID_SETUP
                                                          : PID_OUT;
                        tx_mode_q  <= TX_RAW;
                        tx_len_q   <= 7'd2;
                        tx_start_q <= 1'b1;
                        in_idx     <= 7'd0;
                        to_cnt     <= 0;
                        // An IN is answered by the device, so the listening
                        // starts at once and the token's own time on the
                        // wire is inside the timeout. A SETUP or an OUT is
                        // followed by this host's **own** data packet, and
                        // that one has to wait for the wire.
                        if (kind_q == K_IN) begin
                            state <= H_RXWAIT;
                        end else begin
                            wire_left <= WIRE_TOKEN[WIRE_W-1:0];
                            gap_left  <= GAP_CYCLES[GAP_W-1:0];
                            after     <= H_GAP;
                            after_gap <= H_DATA;
                            state     <= H_WIRE;
                        end
                    end
                end
                H_DATA: begin
                    if (!tx_busy) begin
                        tx_pid_q   <= toggle_q ? PID_DATA1 : PID_DATA0;
                        tx_mode_q  <= TX_DATA;
                        tx_len_q   <= len_q;
                        tx_start_q <= 1'b1;
                        in_idx     <= 7'd0;
                        to_cnt     <= 0;
                        state      <= H_RXWAIT;
                    end
                end
                H_ACK: begin
                    if (!tx_busy) begin
                        tx_pid_q   <= PID_ACK;
                        tx_mode_q  <= TX_HANDSHAKE;
                        tx_len_q   <= 7'd0;
                        tx_start_q <= 1'b1;
                        wire_left  <= WIRE_HS[WIRE_W-1:0];
                        after      <= H_END;
                        state      <= H_WIRE;
                    end
                end
                H_WIRE: begin
                    // The packet finishing on the wire; ULPI 1.1 §3.8.2.2
                    // forbids another transmit before it has. Nothing is
                    // listened for here, so the states this leads to are
                    // only ever ones that transmit.
                    if (tx_abort) begin
                        // The transceiver took the bus and the packet never
                        // went out (§3.8.4.1). Nothing was heard and
                        // nothing was said, so it is a retry.
                        state <= H_RETRY;
                    end else if (wire_left == 0) begin
                        state <= after;
                    end else begin
                        wire_left <= wire_left - 1'b1;
                    end
                end
                H_GAP: begin
                    if (gap_left == 0) begin
                        state <= after_gap;
                    end else begin
                        gap_left <= gap_left - 1'b1;
                    end
                end
                H_RXWAIT: begin
                    if (tx_abort) begin
                        state <= H_RETRY;
                    end else if (pkt && is_handshake) begin
                        saw_reply <= 1'b1;
                        rx_pid_q  <= pkt_pid;
                        rx_len_q  <= 7'd0;
                        status_q  <= (pkt_pid == PID_ACK) ? ST_ACK
                                   : (pkt_pid == PID_NAK) ? ST_NAK
                                                          : ST_STALL;
                        state     <= H_END;
                    end else if (pkt && pkt_is_data && dat_ok
                                 && kind_q == K_IN) begin
                        // A data packet whose CRC16 checks: acknowledge it
                        // after the turnaround USB 2.0 §7.1.18 allows.
                        saw_reply <= 1'b1;
                        rx_pid_q  <= pkt_pid;
                        rx_len_q  <= dat_len;
                        status_q  <= ST_DATA;
                        gap_left  <= TURNAROUND[GAP_W-1:0];
                        after_gap <= H_ACK;
                        state     <= H_GAP;
                    end else if (pkt) begin
                        // Something arrived and it cannot be used: a data
                        // packet with a broken CRC16, data where a
                        // handshake belonged, or a PID this transaction has
                        // no use for. USB's answer is **silence**, so the
                        // device sends it again.
                        saw_reply <= 1'b1;
                        state     <= H_RETRY;
                    end else if (to_cnt == TIMEOUT_CYCLES[TO_W-1:0]) begin
                        state <= H_RETRY;
                    end else begin
                        to_cnt <= to_cnt + 1'b1;
                    end
                end
                H_RETRY: begin
                    if (tries == RETRIES[TRY_W-1:0]) begin
                        // Out of tries. Whether anything came back at all
                        // is a real difference to whoever is listening: a
                        // device that answers nothing may not be there, and
                        // one that answers badly is.
                        status_q <= saw_reply ? ST_ERROR : ST_TIMEOUT;
                        state    <= H_END;
                    end else begin
                        tries     <= tries + 1'b1;
                        gap_left  <= GAP_CYCLES[GAP_W-1:0];
                        after_gap <= H_TOKEN;
                        state     <= H_GAP;
                    end
                end
                default: begin
                    // H_END: report it, and be ready for the next.
                    done_q  <= 1'b1;
                    trn_act <= 1'b0;
                    state   <= H_IDLE;
                end
            endcase
        end
    end
endmodule
