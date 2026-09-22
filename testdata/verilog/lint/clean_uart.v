// UART transmitter and receiver, 8N1, with a baud-rate divider.
module uart_tx #(
    parameter CLK_DIV = 868
) (
    input  wire       clk,
    input  wire       rst,
    input  wire [7:0] data,
    input  wire       valid,
    output reg        ready,
    output reg        txd
);
    localparam IDLE  = 2'd0,
               START = 2'd1,
               DATA  = 2'd2,
               STOP  = 2'd3;

    reg [1:0]  state;
    reg [15:0] baud_cnt;
    reg [2:0]  bit_idx;
    reg [7:0]  shreg;
    wire       tick = (baud_cnt == CLK_DIV - 1);

    always @(posedge clk) begin
        if (rst) begin
            state    <= IDLE;
            baud_cnt <= 16'd0;
            bit_idx  <= 3'd0;
            txd      <= 1'b1;
            ready    <= 1'b1;
        end else begin
            if (state == IDLE)
                baud_cnt <= 16'd0;
            else
                baud_cnt <= tick ? 16'd0 : baud_cnt + 16'd1;

            case (state)
                IDLE: begin
                    txd <= 1'b1;
                    if (valid && ready) begin
                        shreg <= data;
                        ready <= 1'b0;
                        state <= START;
                    end
                end
                START: begin
                    txd <= 1'b0;
                    if (tick) state <= DATA;
                end
                DATA: begin
                    txd <= shreg[bit_idx];
                    if (tick) begin
                        if (bit_idx == 3'd7) begin
                            bit_idx <= 3'd0;
                            state   <= STOP;
                        end else
                            bit_idx <= bit_idx + 3'd1;
                    end
                end
                STOP: begin
                    txd <= 1'b1;
                    if (tick) begin
                        state <= IDLE;
                        ready <= 1'b1;
                    end
                end
                default: state <= IDLE;
            endcase
        end
    end
endmodule

module uart_rx #(
    parameter CLK_DIV = 868
) (
    input  wire       clk,
    input  wire       rst,
    input  wire       rxd,
    output reg  [7:0] data,
    output reg        valid
);
    reg [1:0]  sync;
    reg [15:0] cnt;
    reg [3:0]  bit_cnt;
    reg        busy;
    wire       rx = sync[1];

    always @(posedge clk) sync <= {sync[0], rxd};

    always @(posedge clk) begin
        valid <= 1'b0;
        if (rst) begin
            busy    <= 1'b0;
            cnt     <= 0;
            bit_cnt <= 0;
        end else if (!busy) begin
            if (!rx) begin
                busy <= 1'b1;
                cnt  <= CLK_DIV / 2;
            end
        end else if (cnt == CLK_DIV - 1) begin
            cnt <= 0;
            if (bit_cnt < 4'd8) begin
                data    <= {rx, data[7:1]};
                bit_cnt <= bit_cnt + 1;
            end else begin
                busy    <= 1'b0;
                bit_cnt <= 0;
                valid   <= rx;
            end
        end else
            cnt <= cnt + 1;
    end
endmodule
