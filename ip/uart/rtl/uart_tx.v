// uart_tx — an 8N1 UART transmitter with a ready/valid input.
//
// What it does
//   Shifts out one start bit (low), eight data bits least significant
//   first, and one stop bit (high), each held for CLK_DIV cycles of
//   `clk`. CLK_DIV is the baud divisor: for a 12 MHz clock and 115200
//   baud it is 12_000_000 / 115_200 = 104.
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
//   clock is approximated by CLK_DIV and the error is the caller's to
//   check (under about 2% for 8N1 to survive).
module uart_tx #(
    // Clock cycles per bit. At least two.
    parameter CLK_DIV = 16
) (
    input  wire       clk,
    input  wire       rst_n,

    input  wire [7:0] tx_data,
    input  wire       tx_valid,
    output wire       tx_ready,

    output wire       tx
);
    localparam [15:0] DIV_LAST = CLK_DIV - 1;

    // {stop, data[7:0], start}, shifted right, ones fed in so the line
    // returns to idle by itself.
    reg  [9:0]  shift_q;
    reg  [3:0]  bit_cnt;
    reg  [15:0] div_cnt;
    reg         busy_q;

    assign tx_ready = !busy_q;
    assign tx       = shift_q[0];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            shift_q <= 10'h3FF;
            bit_cnt <= 4'd0;
            div_cnt <= 16'd0;
            busy_q  <= 1'b0;
        end else if (!busy_q) begin
            div_cnt <= 16'd0;
            bit_cnt <= 4'd0;
            if (tx_valid) begin
                shift_q <= {1'b1, tx_data, 1'b0};
                busy_q  <= 1'b1;
            end else begin
                shift_q <= 10'h3FF;
            end
        end else if (div_cnt == DIV_LAST) begin
            div_cnt <= 16'd0;
            shift_q <= {1'b1, shift_q[9:1]};
            bit_cnt <= bit_cnt + 4'd1;
            if (bit_cnt == 4'd9) busy_q <= 1'b0;
        end else begin
            div_cnt <= div_cnt + 16'd1;
        end
    end
endmodule
