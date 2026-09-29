// uart_tx — an 8N1 UART transmitter with a ready/valid input.
//
// What it does
//   Shifts out one start bit (low), eight data bits least significant
//   first, and one stop bit (high), each held for one baud divisor's
//   worth of `clk`. The divisor is clock cycles per bit: for a 12 MHz
//   clock and 115200 baud it is 12_000_000 / 115_200 = 104.
//
//   The bit period is `div` clocks when the caller drives a usable one,
//   and CLK_DIV when it drives zero — see `div`'s own comment. A design
//   with a fixed rate ties `div` to zero and nothing changes.
//
//   The input is a ready/valid handshake. `tx_ready` is high while the
//   shifter is idle; a transfer happens on the rising edge where
//   `tx_valid` and `tx_ready` are both high, and `tx_data` is captured
//   then. `tx` idles high, so a line left alone reads as idle.
//
// What it does not do
//   8N1 only: no parity, no 5/6/7-bit words, no two-stop-bit mode, no
//   break generation, no flow control and no FIFO — put `fifo_sync` in
//   front of it if the producer cannot wait. The bit period is an exact
//   whole number of clock cycles, so a baud rate that does not divide the
//   clock is approximated by the divisor and the error is the caller's to
//   check (under about 2% for 8N1 to survive) — `uart_baud_div` computes
//   one from a bit rate and says when the answer is usable.
//
//   `div` is read **once per character**, in the cycle the byte is
//   accepted, and latched. A character in flight therefore keeps the rate
//   it started at however `div` moves, and a change takes effect from the
//   next character — which is the only definition under which a rate
//   change never corrupts a byte.
//
//   Reading it once per character is also what keeps it cheap. Comparing
//   a counter against a run-time value every bit needs a sixteen-bit
//   magnitude comparison in the loop, and it cost 109 more lookup tables
//   and took the block's logic depth from 6 to 19 — measured, in
//   `docs/ip-library.md`'s footprint table, before this comment was
//   written. Latching the limit makes the loop a sixteen-bit *equality*
//   against a register, which is a four-deep AND tree.
module uart_tx #(
    // Clock cycles per bit when `div` does not give one. At least four.
    parameter CLK_DIV = 16
) (
    input  wire       clk,
    input  wire       rst_n,

    // Clock cycles per bit, taken in the cycle a byte is accepted. Zero,
    // or anything below DIV_MIN, means "use CLK_DIV" — a rate has to mean
    // something even when it cannot be expressed, and the divisor the
    // design was built with is the only answer that leaves the port
    // working. Tie it to zero for a fixed rate.
    input  wire [15:0] div,

    input  wire [7:0] tx_data,
    input  wire       tx_valid,
    output wire       tx_ready,

    output wire       tx
);
    // Two clocks is the least a transmitter can shift at; `uart_rx`
    // needs four, so the two halves share four and `uart` can hand them
    // one number. The comparison is against a constant, so it is the low
    // bits of `div` being zero and not a subtraction.
    localparam [15:0] DIV_MIN = 16'd4;

    wire [15:0] div_used = (div < DIV_MIN) ? CLK_DIV[15:0] : div;

    // {stop, data[7:0], start}, shifted right, ones fed in so the line
    // returns to idle by itself.
    reg  [9:0]  shift_q;
    reg  [3:0]  bit_cnt;
    reg  [15:0] div_cnt;
    // The bit period this character is being sent at, one short, latched
    // when the byte was accepted.
    reg  [15:0] div_last;
    reg         busy_q;

    assign tx_ready = !busy_q;
    assign tx       = shift_q[0];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            shift_q  <= 10'h3FF;
            bit_cnt  <= 4'd0;
            div_cnt  <= 16'd0;
            div_last <= CLK_DIV[15:0] - 16'd1;
            busy_q   <= 1'b0;
        end else if (!busy_q) begin
            div_cnt <= 16'd0;
            bit_cnt <= 4'd0;
            if (tx_valid) begin
                shift_q  <= {1'b1, tx_data, 1'b0};
                busy_q   <= 1'b1;
                div_last <= div_used - 16'd1;
            end else begin
                shift_q <= 10'h3FF;
            end
        end else if (div_cnt == div_last) begin
            div_cnt <= 16'd0;
            shift_q <= {1'b1, shift_q[9:1]};
            bit_cnt <= bit_cnt + 4'd1;
            if (bit_cnt == 4'd9) busy_q <= 1'b0;
        end else begin
            div_cnt <= div_cnt + 16'd1;
        end
    end
endmodule
