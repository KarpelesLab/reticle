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
module uart_rx #(
    // Clock cycles per bit. At least four, so half a bit is countable.
    parameter CLK_DIV = 16
) (
    input  wire       clk,
    input  wire       rst_n,

    input  wire       rx,

    output reg  [7:0] rx_data,
    output reg        rx_valid,
    output reg        rx_error
);
    localparam [15:0] DIV_LAST = CLK_DIV - 1;
    localparam [15:0] DIV_HALF = (CLK_DIV / 2) - 1;

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
                    if (!rx_sync_q) state <= S_START;
                end
                S_START: begin
                    if (div_cnt == DIV_HALF) begin
                        div_cnt <= 16'd0;
                        // Still low at the middle of the start bit: a
                        // real character. Otherwise it was a glitch.
                        state   <= rx_sync_q ? S_IDLE : S_DATA;
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
                S_DATA: begin
                    if (div_cnt == DIV_LAST) begin
                        div_cnt <= 16'd0;
                        shift_q <= {rx_sync_q, shift_q[7:1]};
                        if (bit_idx == 3'd7) state <= S_STOP;
                        else                 bit_idx <= bit_idx + 3'd1;
                    end else begin
                        div_cnt <= div_cnt + 16'd1;
                    end
                end
                default: begin
                    if (div_cnt == DIV_LAST) begin
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
