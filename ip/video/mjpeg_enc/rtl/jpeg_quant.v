// jpeg_quant — quantisation and the zig-zag of ITU T.81 §A.3.4 and
// Figure A.6.
//
// What it does
//   Takes a block's 64 coefficients in row-major order, divides each by
//   its entry in the quantisation table, and gives back the 64 results
//   in **zig-zag order**, which is the order the entropy coder wants
//   and the order a JPEG file stores a table in.
//
//   The table is written through a port of its own, 64 bytes before the
//   first block, and **indexed exactly as `DQT` stores it** — in
//   zig-zag order. So output position `p` divides the coefficient at
//   natural position `zigzag[p]` by `table[p]`, and a caller can hand
//   this block the same 64 bytes it writes into the file without
//   permuting them. Getting that round the wrong way is the classic
//   way to produce a file that decodes to something recognisable but
//   wrong, which is why it is stated here twice.
//
//   A zero in the table would divide by zero. It is treated as a one,
//   which is the only answer that is not a lie; `DQT` forbids zero and
//   nothing here can police what a caller writes.
//
// Division, and why it is a loop
//   There is no multiplier and no divider: a `*` in this library is
//   lookup tables, and `docs/fpga-trellis.md` has what four of them
//   cost. So the quotient is built a bit at a time by restoring
//   division — compare, subtract, shift — one bit a cycle, which is
//   eleven cycles for a quotient that cannot exceed eleven bits.
//
//   **Most coefficients take two cycles instead of eleven.** After the
//   transform and a realistic table, most of the 63 AC coefficients
//   quantise to zero, and a magnitude smaller than the divisor is
//   recognised in one comparison. A block with ten non-zero
//   coefficients takes about 240 cycles rather than 830, so the timing
//   is data-dependent and the handshake is what a caller should pace
//   itself by, not a cycle count.
//
// Rounding is a choice
//   `round(coefficient / q)`, ties away from zero, which this does by
//   adding `q / 2` to the magnitude before dividing. **A decoder
//   multiplies by `q` and cannot tell**, so the rounding is the
//   encoder's to choose, exactly as the nibble is in
//   `ip/audio/adpcm_ima`: any rule here produces a legal file and the
//   difference is quality. Truncation — what a bare divide gives — is
//   measurably worse, and this costs one adder.
//
// What it does not do
//   No table arithmetic: nothing here scales a table by a quality
//   setting or checks it against Annex K's examples. A caller computes
//   the 64 bytes it wants, writes them here and writes the same bytes
//   into `DQT`.
//
//   No clamping to the eleven bits baseline JPEG allows. With a legal
//   table and 8-bit samples the quotient cannot reach them; with a
//   table of ones it can, and the entropy coder beside this is what
//   would notice.
module jpeg_quant (
    input  wire               clk,
    input  wire               rst_n,

    // The quantisation table, 64 bytes, in the order `DQT` stores them.
    input  wire        [5:0]  tbl_addr,
    input  wire        [7:0]  tbl_data,
    input  wire               tbl_we,

    // A block's coefficients, row-major, 64 of them.
    input  wire signed [11:0] in_coef,
    input  wire               in_valid,
    output wire               in_ready,

    // Quantised, in zig-zag order.
    output wire signed [11:0] out_level,
    output wire               out_valid,
    input  wire               out_ready
);
    localparam [1:0] S_IN   = 2'd0;   // 64 coefficients into the buffer
    localparam [1:0] S_PICK = 2'd1;   // fetch one, and its divisor
    localparam [1:0] S_DIV  = 2'd2;   // a quotient bit a cycle
    localparam [1:0] S_PUT  = 2'd3;   // hand it over

    reg [1:0]  state;
    reg [6:0]  ix;                    // which coefficient, 0..64
    reg [3:0]  bit_n;                 // which quotient bit is next

    // The block, and the table. One read port and one write port each,
    // which is the shape this flow builds a block RAM from.
    reg signed [11:0] blk [0:63];
    reg signed [11:0] blk_q;
    reg        [7:0]  tbl [0:63];
    reg        [7:0]  tbl_q;

    // Figure A.6, walked rather than recalled: `nat` is the natural
    // position of the coefficient that belongs at zig-zag position `p`.
    reg [5:0] nat;
    always @* begin
        case (ix[5:0])
            6'd0: nat = 6'd0;
            6'd1: nat = 6'd1;
            6'd2: nat = 6'd8;
            6'd3: nat = 6'd16;
            6'd4: nat = 6'd9;
            6'd5: nat = 6'd2;
            6'd6: nat = 6'd3;
            6'd7: nat = 6'd10;
            6'd8: nat = 6'd17;
            6'd9: nat = 6'd24;
            6'd10: nat = 6'd32;
            6'd11: nat = 6'd25;
            6'd12: nat = 6'd18;
            6'd13: nat = 6'd11;
            6'd14: nat = 6'd4;
            6'd15: nat = 6'd5;
            6'd16: nat = 6'd12;
            6'd17: nat = 6'd19;
            6'd18: nat = 6'd26;
            6'd19: nat = 6'd33;
            6'd20: nat = 6'd40;
            6'd21: nat = 6'd48;
            6'd22: nat = 6'd41;
            6'd23: nat = 6'd34;
            6'd24: nat = 6'd27;
            6'd25: nat = 6'd20;
            6'd26: nat = 6'd13;
            6'd27: nat = 6'd6;
            6'd28: nat = 6'd7;
            6'd29: nat = 6'd14;
            6'd30: nat = 6'd21;
            6'd31: nat = 6'd28;
            6'd32: nat = 6'd35;
            6'd33: nat = 6'd42;
            6'd34: nat = 6'd49;
            6'd35: nat = 6'd56;
            6'd36: nat = 6'd57;
            6'd37: nat = 6'd50;
            6'd38: nat = 6'd43;
            6'd39: nat = 6'd36;
            6'd40: nat = 6'd29;
            6'd41: nat = 6'd22;
            6'd42: nat = 6'd15;
            6'd43: nat = 6'd23;
            6'd44: nat = 6'd30;
            6'd45: nat = 6'd37;
            6'd46: nat = 6'd44;
            6'd47: nat = 6'd51;
            6'd48: nat = 6'd58;
            6'd49: nat = 6'd59;
            6'd50: nat = 6'd52;
            6'd51: nat = 6'd45;
            6'd52: nat = 6'd38;
            6'd53: nat = 6'd31;
            6'd54: nat = 6'd39;
            6'd55: nat = 6'd46;
            6'd56: nat = 6'd53;
            6'd57: nat = 6'd60;
            6'd58: nat = 6'd61;
            6'd59: nat = 6'd54;
            6'd60: nat = 6'd47;
            6'd61: nat = 6'd55;
            6'd62: nat = 6'd62;
            6'd63: nat = 6'd63;
            default: nat = 6'd0;
        endcase
    end

    always @(posedge clk) begin
        if (state == S_IN) begin
            if (in_valid && in_ready) blk[ix[5:0]] <= in_coef;
        end else begin
            blk_q <= blk[nat];
        end
        if (tbl_we) tbl[tbl_addr] <= tbl_data;
        tbl_q <= tbl[ix[5:0]];
    end

    // -----------------------------------------------------------------
    // The divider
    // -----------------------------------------------------------------
    // A zero in the table divides by one instead, which is the only
    // answer that is not a lie.
    wire [7:0]  qv    = (tbl_q == 8'd0) ? 8'd1 : tbl_q;
    wire        neg   = blk_q[11];
    wire [11:0] mag   = neg ? (~blk_q + 12'd1) : blk_q;
    // Rounded before the divide: `|c| + q/2` over `q` is `round(c/q)`
    // with ties away from zero, and it cannot overflow because the
    // magnitude is at most 2048 and half a divisor at most 127.
    wire [12:0] num   = {1'b0, mag} + {5'd0, qv[7:1]};

    reg  [12:0] rem;                  // what is left of the numerator
    reg  [11:0] quot;

    // One restoring step: bring down a bit, compare, subtract.
    wire [12:0] shifted = {rem[11:0], num[bit_n]};
    wire        fits    = (shifted >= {5'd0, qv});
    wire [12:0] nextrem = fits ? (shifted - {5'd0, qv}) : shifted;

    // The whole quotient is zero when the rounded numerator is below
    // the divisor, which is most coefficients and is one comparison.
    wire        tiny    = (num < {5'd0, qv});

    assign in_ready  = (state == S_IN);
    assign out_valid = (state == S_PUT);
    assign out_level = neg ? (~quot + 12'd1) : quot;

    wire took = in_valid && in_ready;
    wire gave = out_valid && out_ready;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state <= S_IN;
            ix    <= 7'd0;
            bit_n <= 4'd0;
            rem   <= 13'd0;
            quot  <= 12'd0;
        end else begin
            case (state)
                S_IN: if (took) begin
                    if (ix == 7'd63) begin
                        ix    <= 7'd0;
                        state <= S_PICK;
                    end else begin
                        ix <= ix + 7'd1;
                    end
                end
                // One cycle for the two memories to answer.
                S_PICK: begin
                    rem   <= 13'd0;
                    quot  <= 12'd0;
                    bit_n <= 4'd11;
                    state <= S_DIV;
                end
                S_DIV: begin
                    if (tiny) begin
                        quot  <= 12'd0;
                        state <= S_PUT;
                    end else begin
                        rem        <= nextrem;
                        quot[bit_n[3:0]] <= fits;
                        if (bit_n == 4'd0) state <= S_PUT;
                        else bit_n <= bit_n - 4'd1;
                    end
                end
                S_PUT: if (gave) begin
                    if (ix == 7'd63) begin
                        ix    <= 7'd0;
                        state <= S_IN;
                    end else begin
                        ix    <= ix + 7'd1;
                        state <= S_PICK;
                    end
                end
                default: state <= S_IN;
            endcase
        end
    end
endmodule
