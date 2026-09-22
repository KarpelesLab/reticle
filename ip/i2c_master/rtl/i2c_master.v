// i2c_master — a byte-level I²C master with start, stop, acknowledge and
// clock stretching.
//
// What it does
//   One command per `start` pulse, and a command is: optionally a START
//   (or a repeated START), then one byte written or read, then optionally
//   a STOP. `done` is high for one cycle when the command has finished.
//   Stringing the commands together is the caller's job, and that is what
//   makes a seven-bit addressed transfer ordinary:
//
//       start + write {addr[6:0], 1'b0}          address for writing
//       write byte                                ... as many as wanted
//       write byte + stop
//
//       start + write {addr[6:0], 1'b0}          set the register pointer
//       write byte
//       start + write {addr[6:0], 1'b1}          repeated start, reading
//       read byte with ack_in = 1                 more to come
//       read byte with ack_in = 0 + stop          the last one, NACK
//
//   `ack_out` is the acknowledgement the slave gave to a written byte: 1
//   means it pulled SDA low, which is an ACK. `ack_in` is what the master
//   sends after a read byte: 1 sends an ACK, 0 a NACK, which is how a
//   master tells a slave the read is over.
//
//   The bus is open drain, so the pins are split into a drive and a sense
//   half rather than being `inout`: `scl_o` and `sda_o` are 0 to pull the
//   line low and 1 to release it, `scl_i` and `sda_i` are what the line
//   actually reads. A top level joins each pair to a pad with the pull-up
//   on it; in simulation the wired AND is one `&` per line.
//
//   **Clock stretching is tolerated.** Whenever the master has released
//   SCL but `scl_i` still reads low, the quarter-bit timer is frozen, so
//   a slave may hold the clock down for as long as it likes and the bit
//   simply takes longer. That is why `scl_i` exists and why it must be
//   wired to the real pin rather than tied high.
//
//   SCL is built from four quarter periods of CLK_DIV clock cycles each,
//   so its period is 4 * CLK_DIV cycles: 100 kHz from a 12 MHz clock is
//   CLK_DIV = 30. SDA changes a quarter period after SCL falls and a
//   quarter before it rises, which is the data setup and hold the bus
//   specification asks for.
//
// What it does not do
//   No multi-master arbitration: it never checks that the level it reads
//   back is the level it drove, so two masters on one bus will corrupt
//   each other. No bus-busy detection or timeout, so a slave that holds
//   SCL low forever hangs the transfer rather than reporting an error —
//   watchdog it outside. No ten-bit addressing, no general call, no SMBus
//   packet error checking, no repeated-byte or DMA interface, and no
//   glitch filter on `scl_i` or `sda_i`, which a long or noisy bus wants.
//   High-speed mode (with its current-source SCL driver) is out of reach
//   of any open-drain-only block, this one included.
module i2c_master #(
    // Clock cycles per quarter of an SCL period; the SCL period is
    // 4 * CLK_DIV cycles.
    parameter CLK_DIV = 4
) (
    input  wire       clk,
    input  wire       rst_n,

    // Command, captured on the cycle `start` is accepted.
    input  wire       start,
    input  wire       cmd_start,
    input  wire       cmd_stop,
    input  wire       cmd_read,
    input  wire [7:0] wr_data,
    // Acknowledgement to send after a read byte: 1 = ACK, 0 = NACK.
    input  wire       ack_in,

    output wire       busy,
    output reg        done,
    output wire [7:0] rd_data,
    // 1 when the slave acknowledged a written byte.
    output wire       ack_out,

    output wire       scl_o,
    input  wire       scl_i,
    output wire       sda_o,
    input  wire       sda_i
);
    localparam [15:0] DIV_LAST = CLK_DIV - 1;

    localparam [1:0] S_IDLE  = 2'd0;
    localparam [1:0] S_START = 2'd1;
    localparam [1:0] S_BIT   = 2'd2;
    localparam [1:0] S_ACK   = 2'd3;
    // STOP reuses S_START's slot with `stopping_q` set, so the state
    // register stays two bits wide.

    reg  [1:0]  state;
    reg  [1:0]  phase;
    reg  [3:0]  bit_cnt;
    reg  [7:0]  shift_q;
    reg  [15:0] div_cnt;

    reg         scl_q;
    reg         sda_q;
    reg         busy_q;
    reg         ack_q;
    reg         is_read_q;
    reg         do_stop_q;
    reg         ack_snd_q;
    reg         stopping_q;

    assign busy    = busy_q;
    assign rd_data = shift_q;
    assign ack_out = ack_q;
    assign scl_o   = scl_q;
    assign sda_o   = sda_q;

    // The slave is stretching the clock: the master has released SCL and
    // the line is still low. Freeze everything until it lets go.
    wire stretch = scl_q && !scl_i;
    wire tick    = (div_cnt == DIV_LAST) && !stretch;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            div_cnt <= 16'd0;
        end else if (!busy_q) begin
            div_cnt <= 16'd0;
        end else if (stretch) begin
            div_cnt <= div_cnt;
        end else if (div_cnt == DIV_LAST) begin
            div_cnt <= 16'd0;
        end else begin
            div_cnt <= div_cnt + 16'd1;
        end
    end

    // The bit grid. Every state runs on the same four quarters:
    //
    //   phase 3  SCL low, SDA takes the value for the coming bit
    //   phase 0  SCL low, SDA settled
    //   phase 1  SCL high, the line is sampled
    //   phase 2  SCL high, then SCL falls at the end
    //
    // START and STOP move SDA during phase 1 instead, which is exactly
    // what makes them conditions rather than data.
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state      <= S_IDLE;
            phase      <= 2'd3;
            bit_cnt    <= 4'd0;
            shift_q    <= 8'd0;
            scl_q      <= 1'b1;
            sda_q      <= 1'b1;
            busy_q     <= 1'b0;
            ack_q      <= 1'b0;
            is_read_q  <= 1'b0;
            do_stop_q  <= 1'b0;
            ack_snd_q  <= 1'b0;
            stopping_q <= 1'b0;
            done       <= 1'b0;
        end else begin
            done <= 1'b0;

            if (state == S_IDLE) begin
                // SCL and SDA keep whatever the last command left them
                // at: between a START and its STOP the bus is held.
                phase      <= 2'd3;
                bit_cnt    <= 4'd0;
                stopping_q <= 1'b0;
                if (start && !busy_q) begin
                    busy_q    <= 1'b1;
                    is_read_q <= cmd_read;
                    do_stop_q <= cmd_stop;
                    ack_snd_q <= ack_in;
                    shift_q   <= wr_data;
                    state     <= cmd_start ? S_START : S_BIT;
                end
            end else if (tick) begin
                case (phase)
                    2'd3: begin
                        phase <= 2'd0;
                        case (state)
                            // Release SDA before SCL goes up, so the
                            // START can be a falling edge.
                            S_START: sda_q <= stopping_q ? 1'b0 : 1'b1;
                            S_BIT:   sda_q <= is_read_q ? 1'b1 : shift_q[7];
                            S_ACK:   sda_q <= (is_read_q && ack_snd_q) ? 1'b0 : 1'b1;
                            default: sda_q <= sda_q;
                        endcase
                    end
                    2'd0: begin
                        phase <= 2'd1;
                        scl_q <= 1'b1;
                    end
                    2'd1: begin
                        phase <= 2'd2;
                        case (state)
                            // SDA moves while SCL is high: down is a
                            // START, up is a STOP.
                            S_START: sda_q <= stopping_q ? 1'b1 : 1'b0;
                            S_BIT:   if (is_read_q) shift_q <= {shift_q[6:0], sda_i};
                            S_ACK:   if (!is_read_q) ack_q <= !sda_i;
                            default: shift_q <= shift_q;
                        endcase
                    end
                    default: begin
                        phase <= 2'd3;
                        case (state)
                            S_START: begin
                                if (stopping_q) begin
                                    // The bus is idle again: both lines
                                    // high, and SCL stays there.
                                    state  <= S_IDLE;
                                    busy_q <= 1'b0;
                                    done   <= 1'b1;
                                end else begin
                                    scl_q   <= 1'b0;
                                    state   <= S_BIT;
                                    bit_cnt <= 4'd0;
                                end
                            end
                            S_BIT: begin
                                scl_q <= 1'b0;
                                if (!is_read_q) shift_q <= {shift_q[6:0], 1'b0};
                                if (bit_cnt == 4'd7) state   <= S_ACK;
                                else                 bit_cnt <= bit_cnt + 4'd1;
                            end
                            S_ACK: begin
                                scl_q <= 1'b0;
                                if (do_stop_q) begin
                                    state      <= S_START;
                                    stopping_q <= 1'b1;
                                end else begin
                                    state  <= S_IDLE;
                                    busy_q <= 1'b0;
                                    done   <= 1'b1;
                                end
                            end
                            default: state <= S_IDLE;
                        endcase
                    end
                endcase
            end
        end
    end
endmodule
