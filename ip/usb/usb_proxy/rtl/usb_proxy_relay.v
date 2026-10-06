// usb_proxy_relay — the half of a USB hub that forwards: the PC's
// transactions on the upstream bus, run again on the downstream one, with the
// answer served back.
//
// WHAT IT IS AND WHY IT CANNOT BE A REPEATER
//   `ip/usb/usb_hub` is a hub's control endpoint: a host finds a hub, finds a
//   device on its port, resets it — and then cannot reach it, because nothing
//   carries a packet across. This is the thing that carries it, and it is a
//   **transaction proxy** and not a repeater.
//
//   A repeater was never available. A hub repeats a downstream packet within
//   about four bit times (USB 2.0 §11.1 and chapter 7's delay budget), and
//   through a ULPI transceiver the floor is roughly twenty-four one way: the
//   receive path reports no byte until the byte is whole, which is eight bit
//   times of SYNC plus a byte, and the transmit command prepends a fresh SYNC
//   of its own before the byte reaches the wire. The delay is in the
//   transceivers and no gateware moves it. `ip/usb/usb_hub/README.md` §2 is the
//   argument at length.
//
//   **NAK is what makes the slow path legal.** USB 2.0 §8.4.6: a device may
//   answer NAK for as long as it likes while it gets ready, and a host's
//   answer to a NAK is to ask again. So this block takes the PC's token,
//   answers NAK, runs the transaction on the other bus at its own pace, and
//   has the answer waiting for the retry. The PC's own retry is the clock
//   this runs on — see "NOTHING IS RETRIED HERE" below.
//
// PASS-THROUGH ADDRESSING, WHICH IS THE WHOLE DESIGN
//   There is **no address translation and no translation table**, and that is
//   the decision this block is built around.
//
//   A token that arrives on the upstream bus with an address that is not the
//   hub's own is a token for something behind the hub's port, because the only
//   devices reachable through that port are the hub and what is on it. So the
//   token's address and endpoint number are forwarded **verbatim**: `trn_addr`
//   is `tok_addr` and `trn_endp` is `tok_endp`, latched and otherwise
//   untouched.
//
//   That includes the PC's own SET_ADDRESS. It is a control transfer like any
//   other, it is relayed like any other, and the device takes the address the
//   PC chose — after its own status stage, as USB 2.0 §9.4.6 requires, which
//   is the device's business and not this block's. From then on the PC's
//   tokens carry that address and so do ours. **Both sides agree on the
//   address because there is only one authority for it.**
//
// The alternative was to let our own enumerator keep the address it assigns
// (`ip/usb/usb_host_ulpi`'s `usb_host_enum`, which assigns 1) and
// translate. That needs: a map from the PC's address to ours, a rule for
// what a GET_DESCRIPTOR answers while the two disagree, a second copy of
// every descriptor, and an answer to the question of which of two
// authorities owns the device's configuration. Every one of those is a
// place for the two views to drift apart, and the first thing to drift is
// `bMaxPacketSize0` — because the lengths a host may send depend on a
// descriptor, and with pass-through the descriptor the PC read **is the
// device's own**, so every length agrees for free. `usb_host_enum` is
// therefore not instantiated in a proxy at all; `README.md` §2 says what it
// is still for.
//
//   The one thing pass-through needs in exchange: **a port reset must reach
//   the real device.** Without it the device keeps an address from a previous
//   session while the PC is talking to address 0, and nothing answers.
//   `usb_proxy_dn` drives that reset and this block holds its state clear
//   while it is being driven.
//
// WHAT A TRANSACTION LOOKS LIKE FROM EACH SIDE
//   One PC transaction becomes at most one downstream transaction, and the
//   `job` below is that one. Four states: nothing wanted, wanted, running, and
//   an answer the PC has not been given yet.
//
//   **A SETUP** is acknowledged at once — a device must acknowledge a SETUP,
//   and this one has the eight bytes safely in `setup_q` — and a downstream
//   SETUP is started with those same eight bytes. Until it is acknowledged
//   downstream, `ct_setup_ok` is low and every later stage of the transfer is
//   NAKed.
//
//   **A data or status stage** is the token the PC sends, forwarded in the
//   same direction: an IN becomes an IN, an OUT becomes an OUT. Nothing here
//   needs to know which stage it is in to do that, and a `?:` is enough to
//   know which stage it is for the one thing that does depend on it, the data
//   toggle: with the transfer's direction in `ct_dir` and whether it has a data
//   stage at all in `ct_nodata`, an IN is the status stage exactly when
//   `~ct_dir | ct_nodata`, and an OUT exactly when `ct_dir | ct_nodata`
//   (USB 2.0 §9.3 for the direction bit, §8.5.3 for the stages).
//
//   **A short packet ends a transfer, and that is free here.** One PC IN
//   fetches exactly one downstream IN and forwards exactly the bytes that came
//   back. Nothing reads ahead, so a device that answers 8 bytes to a 64-byte
//   read sends the PC an 8-byte packet and the PC's transfer ends where the
//   device ended it. A proxy that fetched a whole transfer's worth before
//   answering would have to decide for itself where the end was, and a round
//   of this work got that wrong; this one cannot, because it never has more of
//   the answer than the PC has asked for.
//
// NOTHING IS RETRIED HERE
//   When a downstream transaction comes back with anything but a usable
//   answer — a NAK, a timeout, an unusable reply, or a data packet whose
//   toggle says the device never heard our acknowledgement of the last one —
//   the job is **abandoned** and the PC is NAKed. The PC asks again, and the
//   asking is what starts the next attempt.
//
//   That is deliberate and it is one register less than the alternative. A
//   retry loop in here would hammer the downstream bus at its own rate against
//   a device that is not answering, would need a bound and a counter, and
//   would still have to NAK the PC in the meantime. Driving it from the PC's
//   retry instead means the proxy is exactly as patient as the host is, which
//   is what a hub should be, and a device that has gone away produces the
//   PC's own transfer timeout rather than a storm on the other bus.
//   `usb_host_sie`'s `RETRIES` still covers a lost packet on the downstream
//   wire, which is the retry that belongs to a bus and not to a proxy.
//
// THE DATA TOGGLES: FOUR ARRAYS, BECAUSE THERE ARE TWO BUSES
//   The two buses are independent and their toggles do not stay in step by
//   themselves. A packet can be acknowledged downstream and then have to be
//   sent upstream more than once, or the other way round, and each time only
//   one side's toggle moves. So there are four, one bit per endpoint number:
//
//     up_tog_in   what the next IN packet this block sends the PC will carry
//     up_tog_out  what it expects the PC's next OUT packet to carry
//     dn_tog_in   what it expects the device's next IN packet to carry
//     dn_tog_out  what its next OUT packet to the device will carry
//
//   Each is sixteen bits and indexed by the endpoint number as it is, **not**
//   fifteen indexed by the number less one. Endpoint 0's bit is live: a
//   control transfer's data stage starts at DATA1 in whichever direction it
//   goes (§8.5.3), so a SETUP sets all four of that endpoint's bits, and the
//   data stage then alternates out of the same array every other endpoint
//   uses. A status stage is always DATA1 and is forced rather than taken from
//   an array. The result is that no bit of any of the four is a bit nothing
//   writes — which is the shape this family has cost this project eight rounds
//   of investigation over, and `CLAUDE.md` carries the rule.
//
//   Each side's toggle moves when **that side** acknowledges: `up_tog_in` when
//   the PC acknowledges a packet we sent, `up_tog_out` when we acknowledge a
//   packet the PC sent, `dn_tog_in` when the engine acknowledges a packet the
//   device sent, `dn_tog_out` when the device acknowledges one we sent. A
//   packet whose toggle is not the expected one is a copy the far end is
//   sending again because it did not hear the last acknowledgement: upstream
//   it is acknowledged and dropped, downstream the job is abandoned and the
//   PC asks again.
//
//   **Three things reset a toggle and two of them are requests this block
//   forwards.** A bus reset or a port reset clears all four arrays. So does
//   the PC's SET_CONFIGURATION, and CLEAR_FEATURE(ENDPOINT_HALT) clears the
//   one endpoint it names in the direction its endpoint address names — both
//   read out of `setup_q` when the transfer's status stage completes, because
//   the PC resets its toggle then and so does the device, and a proxy that did
//   not would be the only thing on the bus holding the old one. SET_INTERFACE
//   also resets the toggles of the interface it selects and this block
//   **does not** reflect that: which endpoints belong to which interface is in
//   a descriptor this block forwards without reading. README.md says so in the
//   list of what is not done.
//
// STALL PROPAGATES, IN BOTH DIRECTIONS
//   A device that refuses a request and a proxy that swallows the refusal look
//   completely different to a host: the first is a driver finding out it asked
//   for something that does not exist, the second is a transfer that never
//   ends. So a downstream STALL becomes an upstream STALL on the transaction
//   that caused it, and for a control transfer it also sets `ct_stall`, which
//   STALLs every later stage of that transfer until the next SETUP — which is
//   what USB 2.0 §8.5.3 asks of a device that has stalled a control transfer.
//
//   **This is the reason an OUT is NAKed and not acknowledged on sight.** It
//   would be easy to acknowledge the PC's OUT data, buffer it, and forward it
//   afterwards; that is one fewer transaction per packet. But then a device
//   that STALLs the packet has already had it acknowledged on the PC's behalf,
//   and the refusal can only be reported against some later transaction. So an
//   OUT is held, NAKed, forwarded, and acknowledged — or stalled — on the PC's
//   retry, which is a round trip of a few microseconds and makes the answer the
//   device's own.
//
// What it does not do
//   **One transaction at a time**, and anything else is NAKed while it runs.
//   A device with several endpoints is therefore polled in turn rather than in
//   parallel, which is a throughput limit and not a correctness one: a job is
//   only ever started by a token the PC sent, so a job that is waiting is
//   waiting for something the PC asked for and will ask for again.
//
//   **One device behind the port.** Several addresses would work — the
//   forwarding is per token and keeps no per-device state at all — but the
//   toggle arrays are one set, so two devices sharing an endpoint number would
//   share a toggle. A second device needs the arrays indexed by address too,
//   and there is nowhere for one to be: the hub has one port.
//
//   **No isochronous transfers** and no frame alignment. The downstream SOF
//   comes from `usb_host_sie`'s own free-running counter and its number is not
//   the PC's; a device that times anything from the frame number sees a
//   different one on each side. Bulk, control and interrupt transfers do not
//   care, which is what USB 2.0 §5.6 to §5.8 make the difference.
//
//   **No PING, no split transactions, no low speed and no high speed**, all of
//   which follow from `usb_host_sie` being a full-speed engine with no PRE
//   token.
//
//   **No suspend.** The PC's SetPortFeature(PORT_SUSPEND) moves a bit in
//   `usb_hub` and nothing downstream stops; the downstream SOFs keep the
//   device awake.
//
//   Nothing here decodes a packet or checks a CRC: `usb_pkt_rx` from
//   `ip/usb/usb_device_fs` does the upstream half and the same module inside
//   `usb_host_sie` does the downstream one.
module usb_proxy_relay #(
    // The largest packet relayed, in bytes, which must be a power of two and
    // at least as large as any `wMaxPacketSize` the device behind the port
    // declares. 64 is the largest a full-speed endpoint may have (USB 2.0
    // §5.8.3), so 64 relays anything; a smaller value is a smaller buffer and
    // a design that knows what is on its port.
    //
    // A power of two because the buffer's write pointer is masked to its own
    // width rather than compared against a bound, which is `usb_bulk_ep`'s
    // reason for the same restriction.
    parameter [6:0]  MAXPKT     = 7'd64,
    // Cycles of `line_idle` before an answer starts; `usb_ctrl_ep`'s parameter
    // of the same name says what it has to be and why it is seven bits.
    parameter [6:0]  TURNAROUND = 7'd9
) (
    input  wire        clk,
    input  wire        rst_n,

    // THE UPSTREAM BUS — the PC's. These are the same link layer's outputs the
    // hub beside this block reads, so a packet is decoded twice and the two
    // decoders agree because they are the same module.
    input  wire [7:0]  rx_data,
    input  wire        rx_valid,
    input  wire        rx_eop,
    input  wire        rx_active,
    input  wire        line_idle,
    input  wire        bus_reset,

    // One packet out, upstream. The design above multiplexes these with the
    // hub's on `owns`.
    output reg         tx_start,
    output wire [3:0]  tx_pid,
    output wire        tx_with_data,
    output wire [6:0]  tx_len,
    input  wire [6:0]  tx_index,
    output wire [7:0]  tx_byte,
    input  wire        tx_busy,

    // WHICH TOKENS ARE THIS BLOCK'S
    //
    // `hub_addr` is the hub's own address, from its endpoint 0, and `enabled`
    // is its port being enabled by the PC with the hub configured. A token for
    // any other address, while `enabled`, is for something behind the port.
    input  wire [6:0]  hub_addr,
    input  wire        enabled,
    // The PC is resetting the port, so whatever is behind it is being reset
    // too and everything this block knows about it is wrong.
    input  wire        port_reset,
    // The last token was one of this block's, so the design above should hand
    // it the transmitter.
    output wire        owns,

    // THE DOWNSTREAM TRANSACTION ENGINE — `usb_host_sie`'s ports of the same
    // names. `dn_ready` is `usb_proxy_dn` saying the downstream bus has a
    // device on it and is not being reset.
    input  wire        dn_ready,
    output wire        trn_start,
    output wire [1:0]  trn_kind,
    output wire [6:0]  trn_addr,
    output wire [3:0]  trn_endp,
    output wire        trn_toggle,
    output wire [6:0]  trn_len,
    output wire [7:0]  trn_byte,
    input  wire [6:0]  trn_index,
    input  wire        trn_busy,
    input  wire        trn_done,
    input  wire [2:0]  trn_status,
    input  wire [3:0]  trn_rx_pid,
    input  wire [6:0]  trn_rx_len,
    input  wire [7:0]  in_byte,
    input  wire        in_push,
    input  wire [6:0]  in_index,

    // WHAT IT IS DOING, for a design's console and for a test
    //
    // `proxied` latches that the PC has addressed something behind the port
    // at all, which is the one thing a board can show that says the
    // forwarding path was reached. `job` is the state below.
    output wire        proxied,
    output wire [1:0]  job,

    // TWO PULSES, EACH ANSWERING A QUESTION A CONSOLE CANNOT OTHERWISE ASK
    //
    // `setup_seen` is one cycle when an **upstream** SETUP has been taken and a
    // downstream one started with its eight bytes. Counted on a board, it says
    // whether the PC is **enumerating** the device behind the port rather than
    // merely reaching it: a kernel enumerating a device sends eight or so and
    // stops.
    //
    // A downstream SETUP offered **again** — the arm below that re-offers one
    // whose first attempt did not land — does **not** pulse it, because what is
    // being counted is what the PC sent and a retry is not a second request.
    // `usb_proxy_enumerates_the_device_behind_the_port` asserts exactly six for
    // the PC's six control transfers, which is what pins that.
    //
    // `data_fwd` is one cycle when a transaction that is **not** part of a
    // control transfer is handed to the engine — a bulk or an interrupt
    // one. Latched on a board, it is the only thing that tells a bulk
    // endpoint the proxy never reached from one the device NAKed: a host
    // turns a NAK for ever into a timeout and the two are indistinguishable
    // from the host's side.
    //
    // `ctrl_active` is a control transfer the relay holds a SETUP for,
    // which is from the SETUP until the port is reset and **not** until the
    // transfer ends: nothing here needs to know when a transfer is over, so
    // nothing tracks it.
    output wire        ctrl_active,
    output wire        setup_seen,
    output wire        data_fwd
);
    // PIDs, the low nibble as it appears on the wire (USB 2.0 Table 8-1).
    localparam [3:0] PID_OUT   = 4'b0001;
    localparam [3:0] PID_IN    = 4'b1001;
    localparam [3:0] PID_SETUP = 4'b1101;
    localparam [3:0] PID_DATA0 = 4'b0011;
    localparam [3:0] PID_DATA1 = 4'b1011;
    localparam [3:0] PID_ACK   = 4'b0010;
    localparam [3:0] PID_NAK   = 4'b1010;
    localparam [3:0] PID_STALL = 4'b1110;

    // What `usb_host_sie` is asked for, and what it reports. Both sets are
    // that module's own localparams and are repeated here rather than shared,
    // because Verilog has nowhere to put them: the two modules are in
    // different packages and a parameter is not a constant a port can carry.
    localparam [1:0] K_SETUP = 2'd0;
    localparam [1:0] K_IN    = 2'd1;
    localparam [1:0] K_OUT   = 2'd2;

    localparam [2:0] ST_ACK     = 3'd0;
    localparam [2:0] ST_NAK     = 3'd1;
    localparam [2:0] ST_STALL   = 3'd2;
    localparam [2:0] ST_DATA    = 3'd3;

    // WHAT THIS BLOCK ANSWERS THE PC, AND WHY IT IS NOT A PID
    //
    // Four answers, held as two bits with the nibble decoded as wires below.
    // `usb_bulk_ep` says why at length: of the five PIDs an endpoint can send,
    // bit 1 is a one in every one of them, so a registered nibble has a
    // flip-flop whose data input is a constant — and an unrouted slice input
    // on an ECP5 reads as a one. An answer is two bits of answer.
    localparam [1:0] A_NAK   = 2'd0;
    localparam [1:0] A_ACK   = 2'd1;
    localparam [1:0] A_STALL = 2'd2;
    localparam [1:0] A_DATA  = 2'd3;

    // What the PC's last token was waiting for.
    localparam [1:0] X_NONE  = 2'd0;
    localparam [1:0] X_SETUP = 2'd1;
    localparam [1:0] X_OUT   = 2'd2;

    // The job: one downstream transaction, from wanting it to having its
    // answer. Four states and two bits.
    localparam [1:0] J_IDLE = 2'd0;  // nothing wanted downstream
    localparam [1:0] J_WANT = 2'd1;  // wanted, and the engine has not taken it
    localparam [1:0] J_RUN  = 2'd2;  // the engine has it
    localparam [1:0] J_DONE = 2'd3;  // its answer is waiting for the PC

    // The widths, from `MAXPKT`. `PBITS` addresses a byte of the buffer and
    // `LBITS` holds a length, which is one value more.
    localparam integer PBITS = $clog2(MAXPKT);
    localparam integer LBITS = $clog2(MAXPKT + 1);

    // -----------------------------------------------------------------
    // The upstream packet, decoded once. `usb_pkt_rx` is
    // `ip/usb/usb_device_fs`'s, reached through this package's `depends` line.
    // -----------------------------------------------------------------
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

    usb_pkt_rx u_rx (
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
        .dat          (dat),
        .pay_byte     (pay_byte),
        .pay_push     (pay_push)
    );

    // -----------------------------------------------------------------
    // The state.
    // -----------------------------------------------------------------
    // The last token, latched. `pkt_is_token` is OUT, IN and SETUP and not
    // SOF — `usb_pkt_rx` says so — so a frame marker is never one of these.
    reg [6:0]  addr_q;
    reg [3:0]  endp_q;
    reg [1:0]  expect;
    reg        owns_q;
    reg        proxied_q;
    reg        setup_seen_q;
    reg        data_fwd_q;

    // The control transfer in progress, and the eight bytes that started it.
    reg [63:0] setup_q;
    reg        ct_active;
    reg [3:0]  ct_ep;
    reg        ct_dir;       // bmRequestType bit 7: 1 is device to host
    reg        ct_nodata;    // wLength is zero, so there is no data stage
    reg        ct_setup_ok;  // the device has acknowledged the SETUP
    reg        ct_stall;     // the device refused something in this transfer

    // The job.
    reg [1:0]  j_state;
    reg [1:0]  j_kind;
    reg [6:0]  j_addr;
    reg [3:0]  j_endp;
    reg        j_ct;         // it belongs to a control transfer
    reg        j_stat;       // ... and is its status stage
    reg        j_dn_tog;     // the PID sent to, or expected from, the device
    reg        j_up_tog;     // the PID a DATA answer carries to the PC
    reg [LBITS-1:0] j_len;   // bytes in the buffer, whichever way they go
    reg [1:0]  j_resp;       // what the PC is to be told

    // The four toggles, one bit an endpoint number. The header says why
    // sixteen bits indexed by the number itself.
    reg [15:0] up_tog_in;
    reg [15:0] up_tog_out;
    reg [15:0] dn_tog_in;
    reg [15:0] dn_tog_out;

    // The answer being sent upstream. `trn_busy` a cycle ago, for the
    // rising edge that says the engine has taken **this** job and not
    // the one before it.
    reg        busy_q;
    wire       trn_taken = trn_busy & ~busy_q;

    reg        pending;
    reg [1:0]  ans;
    reg        ans_tog;
    reg [LBITS-1:0] ans_len;
    reg [6:0]  turn;
    reg        await_ack;

    // The one packet buffer. A transaction moves bytes one way at a time — an
    // IN fills it from the device and drains it to the PC, an OUT the other
    // way round — so one buffer serves both and the two writers can never be
    // active in the same cycle.
    //
    // **Not cleared on reset**, for `usb_bulk_ep`'s reason: a distributed RAM
    // has no reset pin and an array cleared on `rst_n` cannot be one. Nothing
    // needs the clear — a byte is read only while a job holds a length, and
    // `j_state` resets to `J_IDLE`.
    reg [7:0]  pbuf [0:MAXPKT-1];
    reg [PBITS-1:0] wp;
    reg        take;   // the OUT packet arriving is being kept

    // -----------------------------------------------------------------
    // Which tokens are this block's.
    // -----------------------------------------------------------------
    // A token for an address that is not the hub's, while the hub's port is
    // enabled. "PASS-THROUGH ADDRESSING" above is why that is **nearly**
    // exactly the set of tokens meant for something behind the port.
    //
    // **AND IT IS TOO WIDE, WHICH WAS FOUND ON A BOARD.** It is exact on a
    // root port, where a host controller sends a packet only down the path
    // the address is on. It is **not** exact on a hub's downstream port,
    // because **HIGH** (USB 2.0 §11.1.2.1) a hub repeats downstream traffic
    // to all of its enabled ports — so a hub plugged into another hub can
    // see tokens addressed to its siblings, and this claims them: forwards
    // them to a device that ignores them, and answers NAK upstream while
    // another device is being addressed.
    //
    // The narrowing is one seven-bit register: claim the **one** address the PC
    // gave the device — zero from the port reset, and whatever a forwarded
    // SET_ADDRESS gave it after that, which is already in `setup_q` and is
    // already parsed there for SET_CONFIGURATION and CLEAR_FEATURE.
    // `README.md` §9's "A claim that is too wide" is the measurement that asked
    // for it, what it did and did not establish, and why it is reported here
    // rather than made: the round that found it had no measurement that would
    // catch getting it wrong, and the instrument it wants is a **count** of
    // forwarded transactions where `data_fwd` is only a latch.
    wire claim = enabled & ~port_reset & tok_ok & (tok_addr != hub_addr);

    assign owns        = owns_q;
    assign proxied     = proxied_q;
    assign ctrl_active = ct_active;
    assign job         = j_state;
    assign setup_seen  = setup_seen_q;
    assign data_fwd    = data_fwd_q;

    // -----------------------------------------------------------------
    // The answer, upstream.
    // -----------------------------------------------------------------
    assign tx_pid       = (ans == A_NAK)   ? PID_NAK
                        : (ans == A_ACK)   ? PID_ACK
                        : (ans == A_STALL) ? PID_STALL
                        : (ans_tog ? PID_DATA1 : PID_DATA0);
    assign tx_with_data = (ans == A_DATA);
    // Zero-extended by the assignment and not by a replication, because at
    // `MAXPKT = 64` the replication's width is zero and a zero-width
    // replication is not something to rely on a parser for.
    assign tx_len       = ans_len;

    // -----------------------------------------------------------------
    // The buffer's one read port.
    // -----------------------------------------------------------------
    // Two readers and one address: the downstream engine fetches while a job
    // is running and the upstream transmitter only ever fetches a payload
    // while a job is `J_DONE`, so the select cannot be wrong. A NAK, an ACK
    // and a STALL have no payload and the byte the transmitter fetches for one
    // goes nowhere.
    wire [PBITS-1:0] rd_at = (j_state == J_RUN) ? trn_index[PBITS-1:0]
                                                : tx_index[PBITS-1:0];
    wire [7:0] rd_byte = pbuf[rd_at];

    assign tx_byte = rd_byte;

    // THE TWO THINGS THAT FILL THE BUFFER, AND THE ONE PORT THEY SHARE
    //
    // The device's IN payload goes at the index the engine gives it, so a
    // packet that turns out to be unusable leaves the buffer no worse than
    // the job that abandoned it. The PC's OUT payload goes at a write
    // pointer, as the bytes arrive, and whether it is kept at all is
    // decided at the **token** for `usb_bulk_ep`'s reason: whether there is
    // room can change halfway through a packet, so the answer is latched
    // from the one moment at which the question can still be asked.
    //
    // A transaction moves bytes one way at a time, so the two are never active
    // in the same cycle and one port serves both.
    wire from_pc = take & pay_push;
    wire from_dev = in_push & (j_state == J_RUN) & (j_kind == K_IN);
    wire buf_wr  = from_pc | from_dev;
    wire [PBITS-1:0] buf_at = from_pc ? wp : in_index[PBITS-1:0];
    wire [7:0] buf_in = from_pc ? pay_byte : in_byte;
    // A SETUP's payload is the eight bytes of `setup_q` and not the buffer's;
    // everything else the engine sends is a packet the PC gave us.
    assign trn_byte = (j_kind == K_SETUP) ? setup_q[{trn_index[2:0], 3'b000} +: 8]
                                          : rd_byte;

    // -----------------------------------------------------------------
    // What the engine is asked for, and the two things a preempting SETUP
    // would otherwise break.
    // -----------------------------------------------------------------
    // `trn_start` is a level and not a pulse, which is the contract
    // `usb_host_sie` states: hold it until `trn_busy`. `J_WANT` is exactly
    // that level — **and it is gated on the engine being idle**, which is
    // not the same thing and is here because of a defect.
    //
    // **HIGH** (USB 2.0 §8.5.3), and it is not optional: a SETUP starts a
    // new control transfer whatever the last one was doing, because the
    // host has moved on. So the one job can be replaced while the
    // **engine** is still running the job it replaced, and two things
    // followed from that.
    //
    // The first: with `trn_start` ungated and `trn_busy` read as a level,
    // the new job saw the **old** transaction's `trn_busy` and called
    // itself running — and then read the old transaction's `trn_done` and
    // `trn_status` as its own answer. An old transaction that was
    // acknowledged would have set `ct_setup_ok` for a SETUP the device
    // never saw, and the data stage after it would have gone to a device
    // still in the previous transfer. So the request waits for the engine,
    // and `trn_taken` is the engine **taking** it rather than the engine
    // being busy.
    //
    // The second is one cycle wide: a `trn_done` for the abandoned
    // transaction arriving in the **same cycle** as the SETUP's data
    // packet. Verilog's last assignment wins and the `trn_done` arm is
    // written after the packet arm, so it would overwrite the job the SETUP
    // had just built — with `j_resp` of `A_DATA` and `j_kind` of `K_SETUP`,
    // which is a job no token matches and which `job_free` will not drop,
    // so the transfer would stall until the host gave up and sent another
    // SETUP. `job_replaced` is the guard, and a SETUP's data packet is the
    // **only** thing that replaces a running job: every other arm schedules
    // on `job_free`, which excludes `J_RUN`.
    //
    // **No test reaches either.** Making one would need a SETUP to
    // land inside the few hundred clocks a downstream transaction
    // takes, which is the host model's own packet timing;
    // `usb_proxy_takes_a_setup_that_preempts_a_transaction_in_flight`
    // reaches the preemption and says in its own comment why it would
    // pass against the broken version too.
    assign trn_start  = (j_state == J_WANT) & dn_ready & ~trn_busy;
    assign trn_kind   = j_kind;
    assign trn_addr   = j_addr;
    assign trn_endp   = j_endp;
    assign trn_toggle = j_dn_tog;
    assign trn_len    = j_len;

    // -----------------------------------------------------------------
    // Which stage of a control transfer a token belongs to.
    // -----------------------------------------------------------------
    // The header's two expressions. `ct_tok` is the token being part of the
    // control transfer in progress at all, and the two `stat` wires say
    // whether a token of that direction is its status stage. No register is
    // needed for the stage, because the direction of the token and the
    // direction of the transfer decide it between them.
    wire ct_tok_in  = ct_active & (tok_endp == ct_ep);
    wire ct_tok_out = ct_active & (endp_q   == ct_ep);
    wire stat_in    = ~ct_dir | ct_nodata;
    wire stat_out   =  ct_dir | ct_nodata;

    // A job may be started: nothing is in flight, or what is in flight is an
    // answer of no value. A `J_DONE` NAK is worth dropping — it says only that
    // the device was not ready — and a `J_DONE` DATA or STALL is not, because
    // one holds the device's bytes and the other its refusal.
    wire job_free = (j_state == J_IDLE)
                  | ((j_state == J_DONE) & (j_resp == A_NAK));

    // The toggle an endpoint's next packet carries in each direction, with a
    // status stage's forced to DATA1 (§8.5.3).
    //
    // Three and not four: the toggle an **upstream IN** packet carries is
    // decided when the downstream answer lands rather than when the token
    // arrives — the same expression over `j_endp` and `j_stat`, in the
    // `ST_DATA` arm below — because by then it is known whether there is a
    // packet to send at all.
    wire up_out_exp  = (ct_tok_out & stat_out) ? 1'b1 : up_tog_out[endp_q];
    wire dn_in_exp   = (ct_tok_in  & stat_in)  ? 1'b1 : dn_tog_in[tok_endp];
    wire dn_out_tog  = (ct_tok_out & stat_out) ? 1'b1 : dn_tog_out[endp_q];

    // A SETUP's data packet, which is the one thing that replaces a job
    // the engine is still running. "What the engine is asked for" above
    // says what it guards.
    wire job_replaced = pkt & pkt_is_data & dat_ok & (expect == X_SETUP);

    // The PC's OUT packet carries the toggle we are expecting, or it is a copy
    // of one we have already acknowledged and whose acknowledgement the PC did
    // not hear.
    wire out_fresh = ((pkt_pid == PID_DATA1) == up_out_exp);

    // The requests whose completion resets a toggle, read out of the SETUP
    // that is finishing. `setup_q` byte 0 is bmRequestType, byte 1 bRequest,
    // bytes 2 and 3 wValue and byte 4 the low half of wIndex (USB 2.0
    // Table 9-2).
    wire set_cfg_done = (setup_q[7:0] == 8'h00) & (setup_q[15:8] == 8'h09);
    wire clr_halt_done = (setup_q[7:0] == 8'h02) & (setup_q[15:8] == 8'h01)
                       & (setup_q[31:16] == 16'h0000);
    wire [3:0] halt_ep  = setup_q[35:32];
    wire       halt_in  = setup_q[39];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            addr_q      <= 7'd0;
            endp_q      <= 4'd0;
            expect      <= X_NONE;
            owns_q      <= 1'b0;
            proxied_q   <= 1'b0;
            setup_seen_q <= 1'b0;
            data_fwd_q  <= 1'b0;
            setup_q     <= 64'd0;
            ct_active   <= 1'b0;
            ct_ep       <= 4'd0;
            ct_dir      <= 1'b0;
            ct_nodata   <= 1'b0;
            ct_setup_ok <= 1'b0;
            ct_stall    <= 1'b0;
            j_state     <= J_IDLE;
            j_kind      <= K_SETUP;
            j_addr      <= 7'd0;
            j_endp      <= 4'd0;
            j_ct        <= 1'b0;
            j_stat      <= 1'b0;
            j_dn_tog    <= 1'b0;
            j_up_tog    <= 1'b0;
            j_len       <= 0;
            j_resp      <= A_NAK;
            up_tog_in   <= 16'd0;
            up_tog_out  <= 16'd0;
            dn_tog_in   <= 16'd0;
            dn_tog_out  <= 16'd0;
            pending     <= 1'b0;
            ans         <= A_NAK;
            ans_tog     <= 1'b0;
            ans_len     <= 0;
            turn        <= 7'd0;
            await_ack   <= 1'b0;
            busy_q      <= 1'b0;
            wp          <= 0;
            take        <= 1'b0;
            tx_start    <= 1'b0;
        end else begin
            tx_start     <= 1'b0;
            setup_seen_q <= 1'b0;
            data_fwd_q   <= 1'b0;
            busy_q       <= trn_busy;

            // ----------------------------------------------------------
            // The buffer's one write port, and the two things that fill it.
            // ----------------------------------------------------------
            // One port and not two, although the two can never be active in
            // the same cycle: a distributed RAM has one write port and two
            // written from two statements is a memory a backend has to take
            // apart again. The select is the decode above.
            if (buf_wr) pbuf[buf_at] <= buf_in;
            if (take && pay_push) wp <= wp + 1'b1;

            // ----------------------------------------------------------
            // One whole upstream packet.
            // ----------------------------------------------------------
            if (pkt) begin
                if (pkt_is_token) begin
                    // Whatever the last token left is finished with. An
                    // answer not yet sent belongs to a token the PC has moved
                    // past, and on a bus with two devices on it the next token
                    // may be the hub's.
                    await_ack <= 1'b0;
                    expect    <= X_NONE;
                    pending   <= 1'b0;
                    take      <= 1'b0;
                    owns_q    <= claim;
                    if (claim) begin
                        addr_q    <= tok_addr;
                        endp_q    <= tok_endp;
                        proxied_q <= 1'b1;
                        if (pkt_pid == PID_SETUP) begin
                            expect <= X_SETUP;
                        end else if (pkt_pid == PID_OUT) begin
                            expect <= X_OUT;
                            // Room for the packet about to arrive, and the
                            // write pointer moved only if it is being kept.
                            if (job_free) begin
                                take <= 1'b1;
                                wp   <= 0;
                            end
                        end else begin
                            // IN. The answer is whatever the job has for this
                            // endpoint, and a NAK while it has nothing.
                            pending <= 1'b1;
                            turn    <= 7'd0;
                            ans_len <= 0;
                            if (ct_tok_in & ct_stall) begin
                                // The device refused this control transfer.
                                ans <= A_STALL;
                            end else if ((j_state == J_DONE) && (j_kind == K_IN)
                                         && (j_endp == tok_endp)) begin
                                ans <= j_resp;
                                if (j_resp == A_DATA) begin
                                    ans_len   <= j_len;
                                    ans_tog   <= j_up_tog;
                                    await_ack <= 1'b1;
                                end else begin
                                    // A NAK or a STALL is said once and the
                                    // job is finished with.
                                    j_state <= J_IDLE;
                                end
                            end else begin
                                ans <= A_NAK;
                                // ... and ask the device, if there is room for
                                // a job.
                                if (job_free && dn_ready && ct_tok_in
                                    && !ct_setup_ok) begin
                                    // The SETUP has not landed — the device
                                    // NAKed it, or said nothing. **The stage
                                    // is not what is asked for again, the
                                    // SETUP is**: without this the transfer
                                    // NAKs for ever and nothing ever retries
                                    // the one transaction it is waiting on.
                                    j_state  <= J_WANT;
                                    j_kind   <= K_SETUP;
                                    j_addr   <= tok_addr;
                                    j_endp   <= ct_ep;
                                    j_ct     <= 1'b1;
                                    j_stat   <= 1'b0;
                                    j_dn_tog <= 1'b0;
                                    j_len    <= 8;
                                end else if (job_free && dn_ready
                                             && (tok_endp != 4'd0
                                                 || ct_tok_in)) begin
                                    data_fwd_q <= ~ct_tok_in;
                                    j_state  <= J_WANT;
                                    j_kind   <= K_IN;
                                    j_addr   <= tok_addr;
                                    j_endp   <= tok_endp;
                                    j_ct     <= ct_tok_in;
                                    j_stat   <= ct_tok_in & stat_in;
                                    j_dn_tog <= dn_in_exp;
                                    j_len    <= 0;
                                end
                            end
                        end
                    end
                end else if (pkt_is_data) begin
                    expect <= X_NONE;
                    take   <= 1'b0;
                    if (dat_ok && expect == X_SETUP) begin
                        // A SETUP is always acknowledged and starts a new
                        // transfer whatever the last one was doing, which is
                        // what `usb_ctrl_ep` does with one and for the same
                        // reason: the host has moved on.
                        pending   <= 1'b1;
                        turn      <= 7'd0;
                        ans       <= A_ACK;
                        ans_len   <= 0;
                        ct_active <= 1'b1;
                        ct_ep     <= endp_q;
                        if (pkt_pid == PID_DATA0 && dat_len == 7'd8) begin
                            setup_q     <= dat;
                            ct_dir      <= dat[7];
                            ct_nodata   <= (dat[63:48] == 16'd0);
                            ct_setup_ok <= 1'b0;
                            ct_stall    <= 1'b0;
                            // §8.5.3: a data stage starts at DATA1 whichever
                            // way it points, on both buses.
                            up_tog_in[endp_q]  <= 1'b1;
                            up_tog_out[endp_q] <= 1'b1;
                            dn_tog_in[endp_q]  <= 1'b1;
                            dn_tog_out[endp_q] <= 1'b1;
                            // And the same eight bytes go to the device.
                            setup_seen_q <= 1'b1;
                            j_state  <= J_WANT;
                            j_kind   <= K_SETUP;
                            j_addr   <= addr_q;
                            j_endp   <= endp_q;
                            j_ct     <= 1'b1;
                            j_stat   <= 1'b0;
                            j_dn_tog <= 1'b0;
                            j_len    <= 8;
                        end else begin
                            // Not eight bytes of DATA0, so not a SETUP this
                            // block can forward. Acknowledged, and every later
                            // stage of it stalled — which is what endpoint 0
                            // does with the same packet.
                            ct_stall    <= 1'b1;
                            ct_setup_ok <= 1'b0;
                            j_state     <= J_IDLE;
                        end
                    end else if (dat_ok && expect == X_OUT) begin
                        pending <= 1'b1;
                        turn    <= 7'd0;
                        ans_len <= 0;
                        if (ct_tok_out & ct_stall) begin
                            ans <= A_STALL;
                        end else if ((j_state == J_DONE) && (j_kind == K_OUT)
                                     && (j_endp == endp_q)) begin
                            // The device's own answer to this very packet.
                            ans     <= j_resp;
                            j_state <= J_IDLE;
                            if (j_resp == A_ACK && !j_stat)
                                up_tog_out[endp_q] <= ~up_tog_out[endp_q];
                        end else if (!out_fresh) begin
                            // A copy of a packet already acknowledged: said
                            // again and dropped, which is what a data toggle
                            // is for.
                            ans <= A_ACK;
                        end else if (job_free && dn_ready && ct_tok_out
                                     && !ct_setup_ok) begin
                            // As above: the SETUP first, and the PC offers
                            // this packet again once it has landed.
                            ans      <= A_NAK;
                            j_state  <= J_WANT;
                            j_kind   <= K_SETUP;
                            j_addr   <= addr_q;
                            j_endp   <= ct_ep;
                            j_ct     <= 1'b1;
                            j_stat   <= 1'b0;
                            j_dn_tog <= 1'b0;
                            j_len    <= 8;
                        end else if (take && (dat_len <= MAXPKT) && dn_ready
                                     && (endp_q != 4'd0 || ct_tok_out)) begin
                            // Held, and sent to the device. The PC is NAKed
                            // and will offer it again, and the answer to that
                            // one is the device's. "STALL PROPAGATES" above is
                            // why it is not acknowledged here.
                            ans      <= A_NAK;
                            data_fwd_q <= ~ct_tok_out;
                            j_state  <= J_WANT;
                            j_kind   <= K_OUT;
                            j_addr   <= addr_q;
                            j_endp   <= endp_q;
                            j_ct     <= ct_tok_out;
                            j_stat   <= ct_tok_out & stat_out;
                            j_dn_tog <= dn_out_tog;
                            j_len    <= dat_len[LBITS-1:0];
                        end else begin
                            ans <= A_NAK;
                        end
                    end
                end else if (pkt_pid == PID_ACK && await_ack) begin
                    // The PC has the packet. Its toggle moves, and so does the
                    // job: the next one is fetched when the PC asks for it.
                    await_ack <= 1'b0;
                    j_state   <= J_IDLE;
                    if (!j_stat) up_tog_in[j_endp] <= ~up_tog_in[j_endp];
                    // A transfer whose status stage the PC has just
                    // acknowledged may have reset a toggle at both ends.
                    if (j_stat && j_ct) begin
                        if (set_cfg_done) begin
                            up_tog_in  <= 16'd0;
                            up_tog_out <= 16'd0;
                            dn_tog_in  <= 16'd0;
                            dn_tog_out <= 16'd0;
                        end else if (clr_halt_done) begin
                            if (halt_in) begin
                                up_tog_in[halt_ep] <= 1'b0;
                                dn_tog_in[halt_ep] <= 1'b0;
                            end else begin
                                up_tog_out[halt_ep] <= 1'b0;
                                dn_tog_out[halt_ep] <= 1'b0;
                            end
                        end
                    end
                end else begin
                    await_ack <= 1'b0;
                end
            end

            // ----------------------------------------------------------
            // The answer goes out once the bus has been idle for the
            // turnaround. `owns_q` gates it because the transmitter is
            // shared with the hub beside this block.
            // ----------------------------------------------------------
            if (pending && !tx_busy && owns_q) begin
                if (!line_idle) begin
                    turn <= 7'd0;
                end else if (turn == TURNAROUND) begin
                    pending  <= 1'b0;
                    tx_start <= 1'b1;
                end else begin
                    turn <= turn + 7'd1;
                end
            end

            // ----------------------------------------------------------
            // The downstream engine.
            // ----------------------------------------------------------
            if (j_state == J_WANT && trn_taken) j_state <= J_RUN;

            if (trn_done && j_state == J_RUN && !job_replaced) begin
                if (trn_status == ST_ACK) begin
                    if (j_kind == K_SETUP) begin
                        // The device has the request. Its stages may go now.
                        ct_setup_ok <= 1'b1;
                        j_state     <= J_IDLE;
                    end else if (j_kind == K_OUT) begin
                        j_resp  <= A_ACK;
                        j_state <= J_DONE;
                        if (!j_stat) dn_tog_out[j_endp] <= ~dn_tog_out[j_endp];
                    end else begin
                        // An ACK where a data packet belonged. Nothing to
                        // forward, so the PC asks again.
                        j_state <= J_IDLE;
                    end
                end else if (trn_status == ST_STALL) begin
                    j_resp  <= A_STALL;
                    j_state <= J_DONE;
                    // §8.5.3: a control transfer the device has stalled stays
                    // stalled until the next SETUP.
                    if (j_ct) ct_stall <= 1'b1;
                end else if (trn_status == ST_DATA && j_kind == K_IN) begin
                    if (((trn_rx_pid == PID_DATA1) == j_dn_tog)
                        && (trn_rx_len <= MAXPKT)) begin
                        j_len    <= trn_rx_len[LBITS-1:0];
                        j_resp   <= A_DATA;
                        j_up_tog <= j_stat ? 1'b1 : up_tog_in[j_endp];
                        j_state  <= J_DONE;
                        if (!j_stat) dn_tog_in[j_endp] <= ~dn_tog_in[j_endp];
                    end else begin
                        // The device never heard the acknowledgement of its
                        // last packet and has sent that one again. The engine
                        // has acknowledged this copy; dropping it is what the
                        // toggle is for.
                        j_state <= J_IDLE;
                    end
                end else begin
                    // A NAK, a timeout, or something unusable. Nothing is
                    // retried here: "NOTHING IS RETRIED HERE" above.
                    if (j_kind == K_SETUP) begin
                        j_state <= J_IDLE;
                    end else begin
                        j_resp  <= A_NAK;
                        j_state <= J_DONE;
                    end
                end
            end

            // A job for a bus that has gone is abandoned. The engine may still
            // be finishing it; `j_state` is what decides whether its answer is
            // read, so there is nothing else to undo.
            if (!dn_ready && j_state != J_RUN) j_state <= J_IDLE;

            // ----------------------------------------------------------
            // Everything this block knows is about one device behind one
            // port, so the three things that take that away take all of it.
            // ----------------------------------------------------------
            if (bus_reset || !enabled || port_reset) begin
                expect      <= X_NONE;
                owns_q      <= 1'b0;
                pending     <= 1'b0;
                await_ack   <= 1'b0;
                take        <= 1'b0;
                ct_active   <= 1'b0;
                ct_setup_ok <= 1'b0;
                ct_stall    <= 1'b0;
                j_state     <= J_IDLE;
                up_tog_in   <= 16'd0;
                up_tog_out  <= 16'd0;
                dn_tog_in   <= 16'd0;
                dn_tog_out  <= 16'd0;
            end
        end
    end
endmodule
