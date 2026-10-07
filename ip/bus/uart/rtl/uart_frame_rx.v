// uart_frame_rx — a UART receiver whose character format is a port, and
// which says *which* of the four things went wrong.
//
// What it does
//   Waits for the falling edge that starts a character, waits half a bit
//   period and checks the line is still low (so a glitch does not start a
//   character), then samples five to eight data bits, an optional parity
//   bit and the stop bit at the middle of each bit period. This is where
//   `uart_rx` actually lives: that module is this one with its two format
//   ports tied to constants from its parameters.
//
//   `rx` is asynchronous to `clk` by definition, so it goes through a
//   two-flop synchroniser before anything looks at it. That costs two
//   cycles of latency and is not optional.
//
//   The format is a pair of ports and not a pair of parameters for the
//   reason `uart_frame_tx`'s header gives: the host's line coding arrives
//   at run time.
//
// Why `div` is latched per character here and the format is not
//   `div` is read **once per character**, in the cycle the start edge is
//   found, so that every sample of one frame is the same distance from
//   the last. The format is read **continuously**, and the asymmetry is
//   deliberate and was measured.
//
//   Latching it costs real area in the module that does not want it.
//   `cfg_data_bits` drives the two multiplexers that right-align the
//   character and `cfg_parity` drives the expected-parity selector; with
//   both tied to constants — which is `uart_rx`, which is what every
//   existing design in this repository instantiates — all of that folds
//   away to nothing, *unless* the values have been through a register
//   first, because a constant does not propagate through a flip-flop in
//   this synthesiser. Latched, `uart` at CLK_DIV 104 is **275 LUT4**;
//   unlatched it is **237**, against 213 before any of this existed. The
//   configurable pair barely notices either way — 311 against 303 — so
//   the whole 38 lookup tables were being spent on behalf of designs
//   that had not asked for a configurable format at all.
//
//   What the latch would have bought is the guarantee that a format
//   changed mid-character does not garble that character, and the reason
//   that is worth little *here* is that a receiver does not own what it
//   is receiving: a host changing the line coding has changed its own
//   transmitter too, so the character crossing that change is lost at the
//   sender whatever this block latched. The transmitter is the other way
//   round — it owns the frame it started and must finish it with the
//   length it started with — and `uart_frame_tx` latches accordingly.
//
//   A format changed mid-character can therefore produce a wrong byte, a
//   framing error, or a parity error, and the state machine always
//   returns to idle: there is no setting of the two ports, and no instant
//   to change them at, that can leave this block stuck. Change them while
//   the line is idle, which is when a host changes them.
//
// Why there is no stop-bit setting here
//   A receiver samples the **first** stop bit and nothing after it: a
//   frame is over once the line has been high at that one sample, and the
//   second stop bit of a two-stop format is indistinguishable from idle.
//   The 16550 behaves the same way. So one stop bit, one and a half, and
//   two are the same configuration as far as this block is concerned, and
//   a `cfg_stop` port here would be a port that changed nothing. The
//   state machine returns to idle at the *middle* of the first stop bit,
//   which leaves half a bit period of mark before the earliest legal next
//   start edge whatever the transmitter was configured for.
//
// The four errors, how each is raised, and how it recovers
//   One wire cannot report four different events, so there are four.
//
//   * **`rx_frame_error`** — the stop bit was not high. A property of the
//     character, so it is held with `rx_valid` and cleared when the
//     character is taken. The byte the broken frame produced is still
//     delivered, because a wrong byte at a consumer is more informative
//     than silence. Recovery is automatic: the block returns to idle at
//     the middle of the stop bit and looks for the next start edge.
//   * **`rx_parity_error`** — the parity bit was not what the data and
//     `cfg_parity` say it should be. Also a property of the character,
//     also held with `rx_valid`, and the byte is delivered for the same
//     reason. Never raised when `cfg_parity` is none, because there is no
//     parity bit to disagree with.
//   * **`rx_break`** — the line was low for a whole frame: every data
//     bit zero, the parity bit zero if there is one, and the stop bit
//     low. That is a break and **not a character**, so no `rx_valid` and
//     no framing error accompany it — a bridge that echoed a break as a
//     0x00 would be reporting a byte nobody sent. It is one cycle wide
//     and it fires **once** per break however long the break is: the
//     block then waits in a state of its own for the line to return to
//     mark, which is the recovery, and only then looks for a start edge
//     again. Without that state a held-low line would produce a break
//     every frame period for ever.
//   * **`rx_overrun`** — a character was assembled while the previous one
//     had not been taken. One cycle wide, not a property of either
//     character. The **new** character is kept and the old one is lost,
//     which is what a 16550 does and what its datasheet means by "the
//     previous character is destroyed"; the alternative — dropping the
//     new one — keeps a stale byte and loses the fresher evidence of
//     whatever is going wrong. Recovery is automatic: `rx_data` and
//     `rx_valid` describe the new character and the block is already
//     idle.
//
//   `rx_valid` is a **handshake** here and not a strobe: it stays high
//   until `rx_ready` is high in the same cycle. That is what makes an
//   overrun detectable at all — a receiver with no holding register
//   cannot tell a byte nobody wanted from a byte nobody could take. A
//   consumer that is always ready ties `rx_ready` to one, and then
//   `rx_valid` is a one-cycle strobe exactly as `uart_rx`'s always was
//   and `rx_overrun` can never fire, because nothing is ever not taken.
//
// What it does not do
//   One sample per bit at the nominal centre: no majority vote over three
//   samples, no oversampling clock recovery, and no mid-character
//   resynchronisation, so the clock error budget is the usual half a bit
//   over a frame — about 5% in theory and under 2% in practice, and a
//   longer frame (eight data bits, parity, two stop) is the tighter case
//   because the last sample is further from the start edge.
//
//   No FIFO. One character of holding register is all there is, which is
//   why `rx_overrun` exists; put `ip/memory/fifo_sync` behind it if the
//   consumer cannot keep up with a character per frame.
//
//   No 9-bit or address-mark framing. `cfg_parity` can send and check a
//   mark or a space bit, which is the hardware half of 9-bit multidrop,
//   but nothing here treats such a bit as an address flag.
module uart_frame_rx #(
    // Clock cycles per bit when `div` does not give one. At least four,
    // so half a bit is countable.
    parameter CLK_DIV = 16
) (
    input  wire        clk,
    input  wire        rst_n,

    // Clock cycles per bit, read when a start edge is found. Zero, or
    // anything below DIV_MIN, means "use CLK_DIV". Tie it to zero for a
    // fixed rate.
    input  wire [15:0] div,

    // The character format, read continuously — see the header for why
    // this one is not latched when `div` is.
    // `cfg_data_bits` is 5, 6, 7 or 8 and anything else is 8;
    // `cfg_parity` is `bParityType` — 0 none, 1 odd, 2 even, 3 mark,
    // 4 space — and 5 to 7 are none. `uart_frame_tx`'s header has the
    // whole table and what it refuses.
    input  wire [3:0]  cfg_data_bits,
    input  wire [2:0]  cfg_parity,

    input  wire        rx,
    // High when the consumer can take the character on `rx_data`. Tie it
    // to one for the strobe behaviour `uart_rx` has.
    input  wire        rx_ready,

    output reg  [7:0]  rx_data,
    output reg         rx_valid,
    output reg         rx_frame_error,
    output reg         rx_parity_error,
    output reg         rx_break,
    output reg         rx_overrun
);
    // Four clocks, so half a bit is a countable number of them. The
    // comparison is against a constant, so it is the low bits of `div`
    // being zero and not a subtraction.
    localparam [15:0] DIV_MIN = 16'd4;

    // `bParityType` of PSTN 1.2 §6.3.11, used unrenumbered.
    localparam [2:0] P_NONE  = 3'd0;
    localparam [2:0] P_ODD   = 3'd1;
    localparam [2:0] P_EVEN  = 3'd2;
    localparam [2:0] P_MARK  = 3'd3;
    localparam [2:0] P_SPACE = 3'd4;

    localparam [2:0] S_IDLE   = 3'd0;
    localparam [2:0] S_START  = 3'd1;
    localparam [2:0] S_DATA   = 3'd2;
    localparam [2:0] S_PARITY = 3'd3;
    localparam [2:0] S_STOP   = 3'd4;
    localparam [2:0] S_BREAK  = 3'd5;

    wire [15:0] div_used = (div < DIV_MIN) ? CLK_DIV[15:0] : div;

    // Five to eight, and eight for anything else, as the transmitter
    // reads it. The index of the last data bit is what the loop wants,
    // and a case rather than a subtraction so that the three bits it is
    // carried in are the three bits it needs.
    wire [3:0] bits_used =
        ((cfg_data_bits >= 4'd5) && (cfg_data_bits <= 4'd8)) ? cfg_data_bits : 4'd8;
    reg [2:0] last_idx;
    always @* begin
        case (bits_used)
            4'd5:    last_idx = 3'd4;
            4'd6:    last_idx = 3'd5;
            4'd7:    last_idx = 3'd6;
            default: last_idx = 3'd7;
        endcase
    end

    // The input synchroniser. Two separately named registers, so the
    // flip-flop inference produces the two-flop chain a CDC checker
    // recognises rather than one two-bit register.
    reg rx_meta_q;
    reg rx_sync_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            rx_meta_q <= 1'b1;
            rx_sync_q <= 1'b1;
        end else begin
            rx_meta_q <= rx;
            rx_sync_q <= rx_meta_q;
        end
    end

    reg [2:0]  state;
    reg [15:0] div_cnt;
    // The bit period of the character being received, and half of it,
    // each one short. Latched at the start edge; `div_half` is a shift
    // rather than a divide, because a divide by a run-time value is what
    // this whole arrangement exists not to need twice.
    reg [15:0] div_last;
    reg [15:0] div_half;
    // The parity bit as it arrived, for the comparison at the stop bit.
    reg        par_got;
    reg [2:0]  bit_idx;
    reg [7:0]  shift_q;

    // A bit enters `shift_q` at the top and the register shifts right, so
    // after N data bits the character sits in the top N bits and has to
    // come down by 8 - N. Two stages of multiplexer, which fold away
    // entirely when `cfg_data_bits` is a constant eight — which is the
    // whole reason the format is not latched here. See the header.
    //
    // 8 - N is 7 - `last_idx`, and `last_idx` is 4 to 7, so its top bit
    // is always one and the subtraction is the complement of its low two
    // bits and nothing else.
    wire [1:0] amt = ~last_idx[1:0];
    wire [7:0] align1  = amt[0] ? {1'b0,  shift_q[7:1]} : shift_q;
    wire [7:0] aligned = amt[1] ? {2'b00, align1[7:2]}  : align1;

    wire par_en = (cfg_parity == P_ODD) | (cfg_parity == P_EVEN)
                | (cfg_parity == P_MARK) | (cfg_parity == P_SPACE);

    // What the parity bit should have been, given the data that arrived.
    reg par_exp;
    always @* begin
        case (cfg_parity)
            P_ODD:   par_exp = ~(^aligned);
            P_EVEN:  par_exp =  (^aligned);
            P_SPACE: par_exp = 1'b0;
            default: par_exp = 1'b1;
        endcase
    end

    // Read at the stop sample and nowhere else.
    wire frame_bad = !rx_sync_q;
    wire par_bad   = par_en & (par_got != par_exp);
    // Every bit of the frame low, which with a low stop bit is a break
    // and not a character. `par_got` is stale when there is no parity
    // bit, which is why it is masked by `par_en` rather than trusted.
    wire all_low   = (aligned == 8'd0) & (~par_en | ~par_got);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state           <= S_IDLE;
            div_cnt         <= 16'd0;
            div_last        <= CLK_DIV[15:0] - 16'd1;
            div_half        <= {1'b0, CLK_DIV[15:1]} - 16'd1;
            par_got         <= 1'b0;
            bit_idx         <= 3'd0;
            shift_q         <= 8'd0;
            rx_data         <= 8'd0;
            rx_valid        <= 1'b0;
            rx_frame_error  <= 1'b0;
            rx_parity_error <= 1'b0;
            rx_break        <= 1'b0;
            rx_overrun      <= 1'b0;
        end else begin
            // The two events that belong to no character.
            rx_break   <= 1'b0;
            rx_overrun <= 1'b0;

            // The character is let go when it is taken, with the two
            // flags that describe it. A character delivered in this same
            // cycle overrides this below, which is the right order: the
            // consumer took the old byte and the new one is now the one
            // being offered.
            if (rx_valid & rx_ready) begin
                rx_valid        <= 1'b0;
                rx_frame_error  <= 1'b0;
                rx_parity_error <= 1'b0;
            end

            case (state)
                S_IDLE: begin
                    div_cnt <= 16'd0;
                    bit_idx <= 3'd0;
                    if (!rx_sync_q) begin
                        state <= S_START;
                        // The rate this character will be timed at.
                        // Taken here and nowhere else, so every sample of
                        // one frame is the same distance from the last.
                        // The *format* is not latched; the header says
                        // what that saves and what it gives up.
                        div_last <= div_used - 16'd1;
                        div_half <= {1'b0, div_used[15:1]} - 16'd1;
                    end
                end
                S_START: begin
                    if (div_cnt == div_half) begin
                        div_cnt <= 16'd0;
                        // Still low at the middle of the start bit: a
                        // real character. Otherwise it was a glitch.
                        state   <= rx_sync_q ? S_IDLE : S_DATA;
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
                S_DATA: begin
                    if (div_cnt == div_last) begin
                        div_cnt <= 16'd0;
                        shift_q <= {rx_sync_q, shift_q[7:1]};
                        if (bit_idx == last_idx) begin
                            state <= par_en ? S_PARITY : S_STOP;
                        end else begin
                            bit_idx <= bit_idx + 3'd1;
                        end
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
                S_PARITY: begin
                    if (div_cnt == div_last) begin
                        div_cnt <= 16'd0;
                        par_got <= rx_sync_q;
                        state   <= S_STOP;
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
                S_STOP: begin
                    if (div_cnt == div_last) begin
                        div_cnt <= 16'd0;
                        if (frame_bad & all_low) begin
                            // A break. Not a character, and the line is
                            // still low: wait for it to be let go.
                            rx_break <= 1'b1;
                            state    <= S_BREAK;
                        end else begin
                            rx_data         <= aligned;
                            rx_valid        <= 1'b1;
                            rx_frame_error  <= frame_bad;
                            rx_parity_error <= par_bad;
                            // The one the consumer never took.
                            rx_overrun      <= rx_valid & ~rx_ready;
                            state           <= S_IDLE;
                        end
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
                S_BREAK: begin
                    div_cnt <= 16'd0;
                    if (rx_sync_q) state <= S_IDLE;
                end
                // Six states in three bits leaves two that nothing
                // reaches. An unrouted slice input on an ECP5 reads as a
                // one, so a state register that could ever hold one of
                // them has to have somewhere to go: here.
                default: state <= S_IDLE;
            endcase
        end
    end
endmodule
