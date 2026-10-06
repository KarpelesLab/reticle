// uart_rx — an 8N1 UART receiver with a valid strobe.
//
// What it does
//   Waits for the falling edge that starts a character, waits half a bit
//   period and checks the line is still low (so a glitch does not start a
//   character), then samples eight data bits and the stop bit at the
//   middle of each bit period. `rx_valid` is high for one cycle when a
//   character has been assembled, with the byte on `rx_data` and
//   `rx_error` high in that same cycle if the stop bit was not high,
//   which is a framing error.
//
//   `rx` is asynchronous to `clk` by definition, so it goes through a
//   two-flop synchroniser before anything looks at it. That costs two
//   cycles of latency and is not optional.
//
// What it does not do
//   8N1 only, one sample per bit at the nominal centre: no parity, no
//   majority vote over three samples, no oversampling clock recovery, no
//   break or overrun detection, and no FIFO — a character not consumed in
//   the cycle `rx_valid` is high is lost, so put `fifo_sync` behind it if
//   the consumer cannot keep up. It does not resynchronise mid-character,
//   so the clock error budget is the usual half a bit over ten bits,
//   about 5% in theory and under 2% in practice.
//
//   `div` is read **once per character**, in the cycle the start edge is
//   found, and latched with the half-bit period derived from it. A
//   character being received therefore keeps the rate it started at
//   however `div` moves, and a change takes effect from the next start
//   edge. `uart_tx` does the same and its header says what that costs and
//   saves.
module uart_rx #(
    // Clock cycles per bit when `div` does not give one. At least four,
    // so half a bit is countable.
    parameter CLK_DIV = 16
) (
    input  wire       clk,
    input  wire       rst_n,

    // Clock cycles per bit, read when a start edge is found. Zero, or
    // anything below DIV_MIN, means "use CLK_DIV": a rate that cannot be
    // expressed has to leave the port working rather than stop it. Tie
    // it to zero for a fixed rate.
    input  wire [15:0] div,

    input  wire       rx,

    output reg  [7:0] rx_data,
    output reg        rx_valid,
    output reg        rx_error
);
    // Four clocks, so half a bit is a countable number of them. The
    // comparison is against a constant, so it is the low bits of `div`
    // being zero and not a subtraction.
    localparam [15:0] DIV_MIN = 16'd4;

    wire [15:0] div_used = (div < DIV_MIN) ? CLK_DIV[15:0] : div;
    // The bit period of the character being received, and half of it,
    // each one short. Latched at the start edge; `div_half` is a shift
    // rather than a divide, because a divide by a run-time value is what
    // this whole arrangement exists not to need twice.
    reg [15:0] div_last;
    reg [15:0] div_half;

    localparam [1:0] S_IDLE  = 2'd0;
    localparam [1:0] S_START = 2'd1;
    localparam [1:0] S_DATA  = 2'd2;
    localparam [1:0] S_STOP  = 2'd3;

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

    reg [1:0]  state;
    reg [15:0] div_cnt;
    reg [2:0]  bit_idx;
    reg [7:0]  shift_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state    <= S_IDLE;
            div_cnt  <= 16'd0;
            div_last <= CLK_DIV[15:0] - 16'd1;
            div_half <= {1'b0, CLK_DIV[15:1]} - 16'd1;
            bit_idx  <= 3'd0;
            shift_q  <= 8'd0;
            rx_data  <= 8'd0;
            rx_valid <= 1'b0;
            rx_error <= 1'b0;
        end else begin
            rx_valid <= 1'b0;
            rx_error <= 1'b0;
            case (state)
                S_IDLE: begin
                    div_cnt <= 16'd0;
                    bit_idx <= 3'd0;
                    if (!rx_sync_q) begin
                        state <= S_START;
                        // The rate this character will be timed at. Taken
                        // here and nowhere else, so the eight samples of
                        // one frame are all the same distance apart.
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
                        if (bit_idx == 3'd7) state <= S_STOP;
                        else                 bit_idx <= bit_idx + 3'd1;
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
                default: begin
                    if (div_cnt == div_last) begin
                        div_cnt  <= 16'd0;
                        state    <= S_IDLE;
                        rx_data  <= shift_q;
                        rx_valid <= 1'b1;
                        rx_error <= !rx_sync_q;
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
            endcase
        end
    end
endmodule
