// usb_proxy_dn — the downstream port of a USB proxy: whether a device is
// there, and the bus reset the PC asked for, driven at it.
//
// What it does
//   Three things, and `usb_proxy_relay` beside it does everything else.
//
//   **Is a device there.** `LineState` leaving SE0 is something plugged in,
//   and USB 2.0 §7.1.7.3 asks for 100 ms of stability first because a plug
//   being pushed in bounces. `DEBOUNCE_CYCLES` is that, and the **same**
//   counter debounces a detach: SE0 between two packets is an end of packet
//   and not an empty socket, so a device is gone only once the pair has been
//   SE0 for as long as it took to decide it had arrived.
//
//   **Which speed.** A full-speed device pulls D+ up through 1.5 kOhm and a
//   low-speed one pulls D- up (USB 2.0 §7.1.5), so `LineState` says which, and
//   `FS_LINE` says which of the two the board calls D+ — which is a property
//   of the board and not of ULPI, and `ip/usb_host_ulpi/README.md` §5 is why
//   it has to be a parameter. A low-speed device is **reported and not spoken
//   to**: there is no PRE token here and `usb_host_sie` is a full-speed engine.
//
//   **The reset the PC asked for.** This is the one thing in a proxy that the
//   hub half cannot do for itself, and the thing pass-through addressing needs
//   most: when the PC sets PORT_RESET, the device behind the port must really
//   be reset, so that it forgets its address and takes the one the PC is about
//   to assign. ULPI 1.1 §3.8.5.1 step 2 makes that a **register write** and
//   not a transmission:
//
//     "If a host detects a full speed peripheral, it resets the peripheral by
//      writing to the Function Control register and setting XcvrSelect = 00b
//      (HS) and TermSelect = 0b which drives SE0 on the bus (D+ and D-
//      connected to ground via 45 Ohm). The host also sets OpMode = 10b for
//      correct chirp transmit and receive."
//
//   so `50h` into `04h` drives SE0 and `45h` puts it back, which are the "Host
//   Chirp" and "Host Full Speed" rows of the USB334x's own Table 5-1 — and
//   §5.2.2 of that datasheet says a combination which is not a row of it is
//   not supported, which is why `OpMode = 10b` is written although nothing here
//   ever chirps. USB 2.0 §7.1.7.5 asks for at least 10 ms of SE0 and §7.1.7.3
//   gives the device 10 ms to recover; `RESET_HOLD` and `RESET_RECOVERY` are
//   15 ms and 20 ms, because a device that measures the reset meanly should not
//   be what decides whether this works.
//
// THE HANDSHAKE WITH THE HUB, AND WHY IT IS A LEVEL AND A PULSE
//   `reset_req` is `ip/usb_hub`'s `port_reset`, which is now a **level** held
//   for as long as the port is resetting, and `reset_done` is one cycle when
//   the reset and its recovery are over. The hub reports PORT_RESET set in
//   `wPortStatus` while the level is high and sets C_PORT_RESET and enables the
//   port when the pulse arrives, which is §11.5.1's **Resetting** state
//   properly rather than the one-cycle reset that block used to have.
//
//   **A port with nothing on it completes at once.** So does one with a
//   low-speed device, which this engine cannot talk to whatever it does to the
//   wire, and so does one whose transceiver is not up. The hub would otherwise
//   hold PORT_RESET for ever and the PC would wait for a reset of a port with
//   no device behind it, which is a worse answer than "done, and still nothing
//   there": §11.5.1 takes a port out of Enabled on a disconnect anyway, and the
//   hub's own `enabled <= connection` is what refuses to enable it.
//
//   **The request is taken on its edge in the two states that drive a reset**
//   and on its level in the four that complete one at once. The edge is there
//   because the hub clears `port_reset` in the cycle *after* it sees
//   `reset_done`, so a level test in the state the pulse leads to would start
//   the whole reset again, for ever.
//
// What it does not do
//   **No enumeration.** It does not read a descriptor, assign an address or
//   set a configuration: the PC does all three and `usb_proxy_relay` forwards
//   them. `ip/usb_host_ulpi`'s `usb_host_enum` is the block that does enumerate
//   and it is deliberately **not** instantiated in a proxy —
//   `usb_proxy_relay`'s "PASS-THROUGH ADDRESSING" is the whole argument, and
//   `README.md` §2 says what the enumerator is still for.
//
//   **No suspend or resume on the port.** The PC's
//   SetPortFeature(PORT_SUSPEND) moves a bit in `ip/usb_hub` and nothing
//   downstream stops; `sof_en` keeps the frames going and the device stays
//   awake. ULPI §3.8.5.3.2's suspend is the change, and it needs a decision
//   about what a resume does to a relay with a job in flight.
//
//   **No over-current and no VBUS.** `VbusState` is not even read here: on the
//   board this was written for the TARGET transceiver does not sense the
//   connector its power flows through, which `ip/usb_host_ulpi/README.md` has
//   traced from the published PCB design, so the one thing that register could
//   be used for would be wrong. Power is a switch in the design's own top
//   level and not a question a port asks.
//
//   **It does not check that the pair came back to J after the reset.**
//   `usb_host_enum` reads the Debug register at that point and fails if it is
//   not; here the next thing that happens is the PC's own enumeration, which is
//   a better test of the same thing and reports itself to the PC rather than to
//   a console. A device that left during the reset goes back to `D_IDLE` on the
//   detach debounce and the hub reports the disconnection.
module usb_proxy_dn #(
    // Clocks of a stable line before an attach or a detach is believed: 100 ms
    // at 60 MHz, which is USB 2.0 §7.1.7.3's figure.
    parameter integer DEBOUNCE_CYCLES = 6_000_000,
    // Clocks of SE0 driven downstream, and of recovery afterwards: 15 ms and
    // 20 ms at 60 MHz against the 10 ms each of §7.1.7.5 and §7.1.7.3.
    parameter integer RESET_HOLD      = 900_000,
    parameter integer RESET_RECOVERY  = 1_200_000,
    // Which `LineState` a full-speed device's idle J reads as, from this
    // transceiver's point of view. `01` is ULPI 1.1 Table 7's D+; a board that
    // exchanges DP and DM between the transceiver and its connector, and does
    // not undo it in the transceiver's own vendor register, reads `10`.
    parameter [1:0]   FS_LINE         = 2'b01
) (
    input  wire       clk,
    input  wire       rst_n,

    // The Link, as it found the transceiver.
    input  wire       phy_ready,
    input  wire [1:0] line_state,

    // The Link's register port, which is this block's alone.
    output wire       reg_start,
    output wire       reg_write,
    output wire [5:0] reg_addr,
    output wire [7:0] reg_wdata,
    input  wire       reg_done,
    input  wire       reg_ok,
    input  wire       reg_busy,

    // The hub's port reset: a level in, a pulse out.
    input  wire       reset_req,
    output reg        reset_done,

    // What is on the port, for the hub to report.
    output wire       attached,
    output wire       low_speed,

    // The downstream bus is usable: a full-speed device is there and has been
    // reset through this port, so its address is the one the PC gave it.
    output wire       dn_ready,
    // Frames, which `usb_host_sie` stops when this is low — and it must be low
    // while the terminations are driving SE0, because a full-speed token has
    // nowhere to go then.
    output wire       sof_en,

    // What it is doing, for a console and for a test.
    output wire [3:0] stage,
    // A register transaction the transceiver refused `REG_TRIES` times, which
    // is the Link giving up. Latched, because it is the one failure here that
    // nothing else would ever mention.
    output reg        reg_failed
);
    // ULPI 1.1 Table 24 and the USB334x's Table 5-1: the three register values
    // this block writes, and the one address it writes them to.
    localparam [5:0] REG_FUNC_CTRL = 6'h04;
    localparam [7:0] FUNC_CTRL_FS  = 8'h45;  // Host Full Speed
    localparam [7:0] FUNC_CTRL_SE0 = 8'h50;  // Host Chirp: HSTERM_EN, which is the SE0

    localparam [1:0] LINE_SE0 = 2'b00;

    // The states. **Four bits for eleven of them**, and the width is the count
    // and not a round number: a state register wide enough for states that do
    // not exist is a hostage to whatever a backend does with a flip-flop whose
    // data input is a constant, and on an ECP5 an unrouted slice input reads as
    // a one. `usb_ctrl_ep`'s header has the account of what that cost.
    localparam [3:0] D_OFF     = 4'd0;   // the transceiver is not up
    localparam [3:0] D_IDLE    = 4'd1;   // an empty port: SE0
    localparam [3:0] D_DEB     = 4'd2;   // something on the line, settling
    localparam [3:0] D_LOW     = 4'd3;   // a low-speed device; reported only
    localparam [3:0] D_PORT    = 4'd4;   // full speed, not yet reset by the PC
    localparam [3:0] D_RST_ON  = 4'd5;   // Function Control <- 50h
    localparam [3:0] D_RST_HLD = 4'd6;   // SE0 held
    localparam [3:0] D_RST_OFF = 4'd7;   // Function Control <- 45h
    localparam [3:0] D_RECOV   = 4'd8;   // the device coming back
    localparam [3:0] D_CLR     = 4'd9;   // waiting for the hub to take the pulse
    localparam [3:0] D_UP      = 4'd10;  // reset, and relayed to

    // One counter for all four waits, as wide as the longest of them.
    localparam integer WAIT_A   = (DEBOUNCE_CYCLES > RESET_HOLD)
                                      ? DEBOUNCE_CYCLES : RESET_HOLD;
    localparam integer WAIT_MAX = (WAIT_A > RESET_RECOVERY)
                                      ? WAIT_A : RESET_RECOVERY;
    localparam integer WAIT_W   = $clog2(WAIT_MAX + 1);

    reg [3:0]        stg;
    reg [WAIT_W-1:0] wait_cnt;
    reg              low_q;
    reg              req_q;
    reg              reg_start_q;
    reg              reg_write_q;
    reg [7:0]        reg_wdata_q;

    assign reg_start = reg_start_q;
    assign reg_write = reg_write_q;
    assign reg_addr  = REG_FUNC_CTRL;
    assign reg_wdata = reg_wdata_q;

    assign stage     = stg;
    // A device is there from the moment the debounce believed it, whatever
    // speed it is and whatever the PC has done to the port.
    assign attached  = (stg >= D_LOW);
    assign low_speed = low_q;
    assign dn_ready  = (stg == D_UP);
    // Frames once the device is there and the pair is not being held at SE0.
    assign sof_en    = (stg == D_PORT) | (stg == D_CLR) | (stg == D_UP);

    // The hub's request on its edge, for the two states that drive a reset.
    wire req_edge = reset_req & ~req_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            stg         <= D_OFF;
            wait_cnt    <= 0;
            low_q       <= 1'b0;
            req_q       <= 1'b0;
            reset_done  <= 1'b0;
            reg_failed  <= 1'b0;
            reg_start_q <= 1'b0;
            reg_write_q <= 1'b0;
            reg_wdata_q <= 8'h00;
        end else begin
            reset_done <= 1'b0;
            req_q      <= reset_req;
            // The request is **held** until the Link takes it, which is the
            // contract `usb_ulpi_host_link` states and the reason
            // `usb_host_enum` holds its own: the cycle a pulse lands in may be
            // one in which `dir` is high and the Link is not listening.
            if (reg_start_q && reg_busy) reg_start_q <= 1'b0;

            if (!phy_ready) begin
                stg <= D_OFF;
                // A port whose transceiver is not up cannot be reset, and
                // telling the hub so at once is better than leaving it holding
                // PORT_RESET for ever.
                if (reset_req) reset_done <= 1'b1;
            end else begin
                case (stg)
                    D_OFF: begin
                        wait_cnt <= 0;
                        stg      <= D_IDLE;
                        low_q    <= 1'b0;
                    end
                    D_IDLE: begin
                        if (reset_req) reset_done <= 1'b1;
                        if (line_state != LINE_SE0) begin
                            wait_cnt <= 0;
                            stg      <= D_DEB;
                        end
                    end
                    D_DEB: begin
                        if (reset_req) reset_done <= 1'b1;
                        if (line_state == LINE_SE0) begin
                            // It bounced back. Nothing is attached yet.
                            stg <= D_IDLE;
                        end else if (wait_cnt == DEBOUNCE_CYCLES[WAIT_W-1:0]) begin
                            wait_cnt <= 0;
                            if (line_state == FS_LINE) begin
                                stg <= D_PORT;
                            end else begin
                                low_q <= 1'b1;
                                stg   <= D_LOW;
                            end
                        end else begin
                            wait_cnt <= wait_cnt + 1'b1;
                        end
                    end
                    D_LOW: begin
                        // Reported, never spoken to, and a reset of it is
                        // answered at once: this engine has no PRE token, so
                        // SE0 on the pair would change nothing it could use.
                        if (reset_req) reset_done <= 1'b1;
                        if (line_state == LINE_SE0) begin
                            if (wait_cnt == DEBOUNCE_CYCLES[WAIT_W-1:0]) begin
                                low_q    <= 1'b0;
                                wait_cnt <= 0;
                                stg      <= D_IDLE;
                            end else begin
                                wait_cnt <= wait_cnt + 1'b1;
                            end
                        end else begin
                            wait_cnt <= 0;
                        end
                    end
                    D_PORT, D_UP: begin
                        // A device the PC may reset at any time, and the one
                        // place a detach is noticed. SE0 between two packets is
                        // an end of packet, so the same hundred milliseconds
                        // that decided it had arrived decide it has gone.
                        if (req_edge) begin
                            wait_cnt <= 0;
                            stg      <= D_RST_ON;
                        end else if (line_state == LINE_SE0) begin
                            if (wait_cnt == DEBOUNCE_CYCLES[WAIT_W-1:0]) begin
                                wait_cnt <= 0;
                                stg      <= D_IDLE;
                            end else begin
                                wait_cnt <= wait_cnt + 1'b1;
                            end
                        end else begin
                            wait_cnt <= 0;
                        end
                    end
                    D_RST_ON: begin
                        // **`reg_done` is tested first** and the order is not a
                        // style: `reg_busy` falls in the very cycle `reg_done`
                        // rises, because the Link clears the request it is
                        // reporting on the same edge. Issuing first would see an
                        // idle port in that cycle and send the write again, for
                        // ever. `usb_host_enum` carries the same comment.
                        if (reg_done) begin
                            wait_cnt   <= 0;
                            reg_failed <= reg_failed | ~reg_ok;
                            stg        <= D_RST_HLD;
                        end else if (!reg_start_q && !reg_busy) begin
                            reg_write_q <= 1'b1;
                            reg_wdata_q <= FUNC_CTRL_SE0;
                            reg_start_q <= 1'b1;
                        end
                    end
                    D_RST_HLD: begin
                        if (wait_cnt == RESET_HOLD[WAIT_W-1:0]) begin
                            stg <= D_RST_OFF;
                        end else begin
                            wait_cnt <= wait_cnt + 1'b1;
                        end
                    end
                    D_RST_OFF: begin
                        if (reg_done) begin
                            wait_cnt   <= 0;
                            reg_failed <= reg_failed | ~reg_ok;
                            stg        <= D_RECOV;
                        end else if (!reg_start_q && !reg_busy) begin
                            reg_write_q <= 1'b1;
                            reg_wdata_q <= FUNC_CTRL_FS;
                            reg_start_q <= 1'b1;
                        end
                    end
                    D_RECOV: begin
                        if (wait_cnt == RESET_RECOVERY[WAIT_W-1:0]) begin
                            wait_cnt   <= 0;
                            reset_done <= 1'b1;
                            stg        <= D_CLR;
                        end else begin
                            wait_cnt <= wait_cnt + 1'b1;
                        end
                    end
                    default: begin
                        // D_CLR: the hub clears `port_reset` in the cycle after
                        // it sees `reset_done`, so the port is not relayed to
                        // until it has. One state rather than an edge detector
                        // on the way out as well as on the way in.
                        if (!reset_req) stg <= D_UP;
                    end
                endcase
            end
        end
    end
endmodule
