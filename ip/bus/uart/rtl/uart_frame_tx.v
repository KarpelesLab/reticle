// uart_frame_tx — a UART transmitter whose character format is a port.
//
// What it does
//   Shifts out one start bit (low), five to eight data bits least
//   significant first, an optional parity bit, and one or two stop bits
//   (high), each held for one baud divisor's worth of `clk`. The divisor
//   is clock cycles per bit, exactly as `uart_tx`'s header describes it,
//   and this block is where that transmitter actually lives: `uart_tx` is
//   this module with its three format ports tied to constants from its
//   parameters, which is why 8N1 costs what it always did.
//
//   The format is **three ports and not three parameters**, and the
//   reason is a host: `ip/usb/usb_cdc_acm` decodes SET_LINE_CODING at run
//   time and puts `char_format`, `parity` and `data_bits` on wires, so a
//   serial port that honours `stty` cannot have had them compiled in.
//   `uart_line_coding` in this package turns that host's three bytes into
//   the three values here, and says when it could not do so exactly.
//
//   The format ports are read **once per character**, in the cycle the
//   byte is accepted, and latched — the same rule `div` follows and for
//   the same reason: a comparison against a run-time value inside the
//   per-bit loop is what cost eleven levels of logic when the divisor
//   first became a port, and `uart_tx`'s header has those measurements.
//   A character in flight therefore keeps the format it started at, and a
//   change takes effect from the next character. Change the format while
//   the line is idle.
//
//   What is latched is the whole frame: the twelve-bit word to shift out
//   and how many of its bits belong to the character. The arithmetic that
//   places the parity bit and counts the frame's length happens **once,
//   outside the loop**, so the loop is still one sixteen-bit equality and
//   one four-bit equality.
//
// The encodings, and what an unimplemented value does
//   `cfg_data_bits` is a count: 5, 6, 7 or 8. **Anything else is eight**,
//   including 9 and 16. Nine data bits are not implementable here at any
//   price short of a wider datapath — `tx_data` is eight bits and so is
//   every FIFO that would sit in front of it — and sixteen is a value CDC
//   lists that this block cannot express; `uart_line_coding` is where a
//   host's 16 is caught and reported rather than silently narrowed.
//
//   `cfg_parity` follows PSTN 1.2's own `bParityType` so that nothing has
//   to be renumbered between a host and a wire: 0 none, 1 odd, 2 even,
//   3 mark, 4 space. All five are implemented, because a parity bit is
//   one bit and selecting between `^data`, `~^data`, 1 and 0 is a
//   four-way multiplexer on it — mark and space cost almost nothing once
//   odd and even exist, so there is no argument for leaving them out.
//   **5, 6 and 7 are treated as none**, which is the block's default and
//   the only safe answer; `uart_line_coding` never produces one.
//
//   `cfg_stop` follows `bCharFormat`: 0 is one stop bit, 2 is two.
//   **1 — one and a half stop bits — sends two**, and that is a decision
//   rather than an oversight:
//
//     * a receiver samples the *first* stop bit and nothing after it, so
//       1.5 and 2 are indistinguishable to anything listening;
//     * a far end expecting 1.5 bit periods of mark gets 2, which is
//       more idle than it asked for and never less, so no frame is lost;
//     * the cost of doing it properly is a second limit for the bit
//       timer — a sixteen-bit multiplexer selecting `div_last` or half of
//       it — and a state to spend it in, in a block whose whole loop is
//       one comparison;
//     * and the only machines that ever asked for 1.5 are five-bit
//       Baudot teleprinters, which is also why `bDataBits` 5 exists.
//
//   So the wire is right and the throughput is half a bit per character
//   short of optimal. `uart_line_coding` reports the substitution by
//   dropping `ok`, because a design that wants to know is entitled to,
//   and **3 is treated as two stop bits** — it is not a CDC value.
//
// What it does not do
//   No break generation (a design that wants one holds `tx` low itself,
//   and this block has no port for that), no flow control, no FIFO, and
//   no 9-bit or mark/space *address* framing beyond the parity bit above.
//   The bit period is a whole number of clocks; `uart_baud_div` computes
//   one from a bit rate and says when the answer is usable.
module uart_frame_tx #(
    // Clock cycles per bit when `div` does not give one. At least four.
    parameter CLK_DIV = 16
) (
    input  wire        clk,
    input  wire        rst_n,

    // Clock cycles per bit, taken in the cycle a byte is accepted. Zero,
    // or anything below DIV_MIN, means "use CLK_DIV". Tie it to zero for
    // a fixed rate.
    input  wire [15:0] div,

    // The character format, taken in the same cycle as `div`. See "The
    // encodings" above for every value and what it does.
    input  wire [3:0]  cfg_data_bits,
    input  wire [2:0]  cfg_parity,
    input  wire [1:0]  cfg_stop,

    input  wire [7:0]  tx_data,
    input  wire        tx_valid,
    output wire        tx_ready,

    output wire        tx
);
    // Two clocks is the least a transmitter can shift at; `uart_frame_rx`
    // needs four, so the two halves share four and `uart_frame` can hand
    // them one number. The comparison is against a constant, so it is the
    // low bits of `div` being zero and not a subtraction.
    localparam [15:0] DIV_MIN = 16'd4;

    // `bParityType` of PSTN 1.2 §6.3.11, used unrenumbered.
    localparam [2:0] P_NONE  = 3'd0;
    localparam [2:0] P_ODD   = 3'd1;
    localparam [2:0] P_EVEN  = 3'd2;
    localparam [2:0] P_MARK  = 3'd3;
    localparam [2:0] P_SPACE = 3'd4;

    wire [15:0] div_used = (div < DIV_MIN) ? CLK_DIV[15:0] : div;

    // Five to eight, and eight for anything else.
    wire [3:0] bits_used =
        ((cfg_data_bits >= 4'd5) && (cfg_data_bits <= 4'd8)) ? cfg_data_bits : 4'd8;

    // A parity bit is sent for the four modes that have one. Written as
    // four equalities rather than as a range so that the one value this
    // block refuses — anything above space — reads as `none` by
    // construction and not by an inequality somebody can get backwards.
    wire par_en = (cfg_parity == P_ODD) | (cfg_parity == P_EVEN)
                | (cfg_parity == P_MARK) | (cfg_parity == P_SPACE);

    // One stop bit for `bCharFormat` 0, two for everything else.
    wire two_stop = (cfg_stop != 2'd0);

    // The data bits, zero-extended to eight, so the parity is the parity
    // of the character and not of whatever was in the unused bits.
    reg [7:0] dat;
    always @* begin
        case (bits_used)
            4'd5:    dat = {3'b000, tx_data[4:0]};
            4'd6:    dat = {2'b00,  tx_data[5:0]};
            4'd7:    dat = {1'b0,   tx_data[6:0]};
            default: dat = tx_data;
        endcase
    end

    // Even parity is the exclusive or of the data bits; odd is its
    // complement. Mark and space are the constants their names are.
    reg par_bit;
    always @* begin
        case (cfg_parity)
            P_ODD:   par_bit = ~(^dat);
            P_EVEN:  par_bit =  (^dat);
            P_SPACE: par_bit = 1'b0;
            // Mark, and the no-parity case: a one in this slot *is* the
            // first stop bit, which is why no-parity needs no special
            // case anywhere below. `frame_bits` simply does not count it.
            default: par_bit = 1'b1;
        endcase
    end

    // The frame, least significant bit first: start, the data, the parity
    // slot, then ones, which is what makes the line return to idle by
    // itself however short the frame is. Twelve bits is the longest
    // frame — one start, eight data, one parity, two stop.
    reg [11:0] load;
    always @* begin
        case (bits_used)
            4'd5:    load = {5'b11111, par_bit, dat[4:0], 1'b0};
            4'd6:    load = {4'b1111,  par_bit, dat[5:0], 1'b0};
            4'd7:    load = {3'b111,   par_bit, dat[6:0], 1'b0};
            default: load = {2'b11,    par_bit, dat[7:0], 1'b0};
        endcase
    end

    // One start bit, the data, the parity if there is one, and one or two
    // stop bits: between seven and twelve, which is what four bits hold.
    wire [3:0] frame_bits = bits_used + 4'd2 + {3'd0, par_en} + {3'd0, two_stop};

    reg  [11:0] shift_q;
    reg  [3:0]  bit_cnt;
    reg  [15:0] div_cnt;
    // The bit period and the frame length this character is being sent
    // at, each one short, latched when the byte was accepted.
    reg  [15:0] div_last;
    reg  [3:0]  frame_last;
    reg         busy_q;

    assign tx_ready = !busy_q;
    assign tx       = shift_q[0];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            shift_q    <= 12'hFFF;
            bit_cnt    <= 4'd0;
            div_cnt    <= 16'd0;
            div_last   <= CLK_DIV[15:0] - 16'd1;
            // 8N1's ten bits. Only read while `busy_q`, which is low
            // here, so this is a defined value and not a used one.
            frame_last <= 4'd9;
            busy_q     <= 1'b0;
        end else if (!busy_q) begin
            div_cnt <= 16'd0;
            bit_cnt <= 4'd0;
            if (tx_valid) begin
                shift_q    <= load;
                busy_q     <= 1'b1;
                div_last   <= div_used - 16'd1;
                frame_last <= frame_bits - 4'd1;
            end else begin
                shift_q <= 12'hFFF;
            end
        end else if (div_cnt == div_last) begin
            div_cnt <= 16'd0;
            shift_q <= {1'b1, shift_q[11:1]};
            bit_cnt <= bit_cnt + 4'd1;
            if (bit_cnt == frame_last) busy_q <= 1'b0;
        end else begin
            div_cnt <= div_cnt + 16'd1;
        end
    end
endmodule
