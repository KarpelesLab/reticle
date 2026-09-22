// usb_fs_rx — the receive half of a USB full-speed PHY.
//
// What it does
//   Recovers bytes from the D+ / D- pair at 12 Mbit/s with a 48 MHz
//   clock, four samples a bit. Both lines pass two flip-flops first,
//   since they change whenever the host likes. Every change of line
//   state restarts a four-step phase counter, and the bit is sampled in
//   the third of the four cycles the synchronised line holds it — the
//   first is spent noticing the change — which leaves room for one bit
//   in a run to come a cycle short or two to come a cycle long. With a
//   transition at least every seven bits (bit stuffing guarantees one)
//   that keeps the sampling point in the eye with room to spare over
//   the 0.25 % the two ends' clocks may differ by; the testbench runs a
//   host 0.4 % off either way.
//
//   On the sampled states: a K after idle starts the SYNC field, whose
//   zeros are counted until its closing one; NRZI is decoded (no change
//   is a one, a change a zero); after six ones in a row the next bit
//   must be a stuffed zero and is dropped, and a one there is a
//   bit-stuffing error. Bits are assembled least significant first and
//   each byte is strobed on `valid`. SE0 ends the packet: `eop` pulses
//   if it ended on a byte boundary with no error, `error` pulses if not.
//   Bit stuffing is counted from the SYNC field's closing one, as the
//   specification says.
//
//   `bus_reset` is high while SE0 has lasted longer than 2.5 us, which
//   is how a host resets a device.
//
// What it does not do
//   Full speed only: low-speed polarity and its keep-alive are not
//   recognised. The SYNC field is accepted with as few as three of its
//   seven zeros, since a hub may eat some; nothing checks that it had
//   all of them. `enable` low holds the receiver idle, which the device
//   uses while it is itself driving the pair.
module usb_fs_rx (
    input  wire       clk,
    input  wire       rst_n,
    input  wire       dp,
    input  wire       dn,
    input  wire       enable,
    output wire [7:0] data,
    output wire       valid,
    output wire       eop,
    output wire       error,
    output wire       active,
    output wire       idle_j,
    output wire       bus_reset
);
    // Line states as {D+, D-}.
    localparam [1:0] LINE_SE0 = 2'b00;
    localparam [1:0] LINE_K   = 2'b01;
    localparam [1:0] LINE_J   = 2'b10;

    localparam [1:0] S_IDLE = 2'd0;
    localparam [1:0] S_SYNC = 2'd1;
    localparam [1:0] S_DATA = 2'd2;
    localparam [1:0] S_EOP  = 2'd3;

    // 2.5 us at 48 MHz.
    localparam [7:0] RESET_CYCLES = 8'd120;

    // Two flip-flops per line before anything looks at it.
    reg dp_meta, dp_sync;
    reg dn_meta, dn_sync;
    wire [1:0] line = {dp_sync, dn_sync};

    reg [1:0] line_q;
    reg [1:0] phase;
    reg [1:0] state;
    reg [1:0] prev;
    reg [2:0] zeros;
    reg [2:0] ones;
    reg [2:0] bitcnt;
    reg [7:0] shift;
    reg [7:0] data_q;
    reg       valid_q;
    reg       eop_q;
    reg       error_q;
    reg [7:0] se0_cnt;

    wire strobe = (phase == 2'd1);
    wire bit_in = (line == prev);

    assign data      = data_q;
    assign valid     = valid_q;
    assign eop       = eop_q;
    assign error     = error_q;
    assign active    = (state == S_SYNC) | (state == S_DATA);
    assign idle_j    = (state == S_IDLE) & (line == LINE_J);
    assign bus_reset = (se0_cnt == RESET_CYCLES);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            dp_meta <= 1'b1;
            dp_sync <= 1'b1;
            dn_meta <= 1'b0;
            dn_sync <= 1'b0;
        end else begin
            dp_meta <= dp;
            dp_sync <= dp_meta;
            dn_meta <= dn;
            dn_sync <= dn_meta;
        end
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            line_q  <= LINE_J;
            phase   <= 2'd0;
            state   <= S_IDLE;
            prev    <= LINE_J;
            zeros   <= 3'd0;
            ones    <= 3'd0;
            bitcnt  <= 3'd0;
            shift   <= 8'd0;
            data_q  <= 8'd0;
            valid_q <= 1'b0;
            eop_q   <= 1'b0;
            error_q <= 1'b0;
            se0_cnt <= 8'd0;
        end else begin
            valid_q <= 1'b0;
            eop_q   <= 1'b0;
            error_q <= 1'b0;
            line_q  <= line;
            phase   <= (line != line_q) ? 2'd0 : phase + 2'd1;

            if (line != LINE_SE0)            se0_cnt <= 8'd0;
            else if (se0_cnt != RESET_CYCLES) se0_cnt <= se0_cnt + 8'd1;

            if (!enable) begin
                state <= S_IDLE;
                prev  <= LINE_J;
            end else if (strobe) begin
                case (state)
                    S_IDLE: begin
                        if (line == LINE_K) begin
                            // The first K of SYNC: a change from idle J,
                            // so a zero.
                            state <= S_SYNC;
                            prev  <= LINE_K;
                            zeros <= 3'd1;
                        end
                    end
                    S_SYNC: begin
                        prev <= line;
                        if (line == LINE_SE0) begin
                            state <= S_EOP;
                        end else if (!bit_in) begin
                            if (zeros != 3'd7) zeros <= zeros + 3'd1;
                        end else if (zeros >= 3'd3) begin
                            state  <= S_DATA;
                            ones   <= 3'd1;
                            bitcnt <= 3'd0;
                        end else begin
                            state <= S_EOP;
                        end
                    end
                    S_DATA: begin
                        prev <= line;
                        if (line == LINE_SE0) begin
                            state <= S_EOP;
                            if (bitcnt == 3'd0) eop_q <= 1'b1;
                            else                error_q <= 1'b1;
                        end else if (ones == 3'd6) begin
                            // The stuffed zero.
                            ones <= 3'd0;
                            if (bit_in) begin
                                error_q <= 1'b1;
                                state   <= S_EOP;
                            end
                        end else begin
                            shift  <= {bit_in, shift[7:1]};
                            ones   <= bit_in ? ones + 3'd1 : 3'd0;
                            bitcnt <= bitcnt + 3'd1;
                            if (bitcnt == 3'd7) begin
                                data_q  <= {bit_in, shift[7:1]};
                                valid_q <= 1'b1;
                            end
                        end
                    end
                    default: begin
                        // S_EOP: wait for the bus to be idle again.
                        if (line == LINE_J) state <= S_IDLE;
                    end
                endcase
            end
        end
    end
endmodule
