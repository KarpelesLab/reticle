// jpeg_entropy — the entropy coder of ITU T.81 §F.1.2: one block's 64
// quantised levels in, entropy-coded bytes out.
//
// What it does
//   Takes the 64 levels of a block in zig-zag order, as `jpeg_quant`
//   hands them over, and produces the bytes that go between `SOS` and
//   `EOI`. Three things happen to them:
//
//   * **The DC level is differenced** against the previous block's
//     (§F.1.2.1). `start` clears that predictor, which is what the
//     beginning of a scan and a restart interval both want.
//   * **The AC levels are run-length coded** (§F.1.2.2): a non-zero
//     level is a symbol `(run << 4) | size`, where `run` is how many
//     zeros preceded it and `size` how many bits its magnitude needs;
//     sixteen or more zeros emit `ZRL` first, and a tail of zeros
//     emits `EOB`.
//   * **Each symbol becomes a Huffman code** followed by the magnitude
//     bits of §F.1.2.1.1: the low `size` bits of the value, or of the
//     value minus one when it is negative.
//
//   **Unlike the transform, this is exact.** Given the same levels and
//   the same code tables the bitstream is completely determined by
//   T.81, so there is a right answer to compare against, and
//   `jpeg_entropy_codes_the_corpus_byte_for_byte` compares the whole
//   of it. That is the difference between this module and `jpeg_fdct`,
//   whose arithmetic the standard leaves open; the account is in
//   `ip/video/mjpeg_enc/README.md`.
//
// The tables are the caller's
//   Nothing here knows Annex K. The DC and AC code tables are written
//   through a port of their own, as a code and a length per symbol, so
//   a caller writes the same table it puts in the file's `DHT` and no
//   copy of Annex K lives in this package. A symbol whose length is
//   zero was never given a code; emitting one would produce a file
//   nothing can read, and this block cannot tell — `DHT` is the
//   caller's to get right.
//
// Byte stuffing
//   §F.1.2.3: a `FF` in the entropy-coded data is followed by a `00`,
//   because `FF` begins a marker. The final partial byte is padded
//   with ones by `flush`, which §F.1.2.3 also asks for and which is
//   not optional: a decoder reads whole bytes.
//
// A bit a cycle
//   The bits are shifted out one at a time rather than packed by a
//   barrel shifter. Everything else in this package is bit-serial for
//   want of a multiplier — the transform's arithmetic, the quantiser's
//   division — and a code is at most sixteen bits and a magnitude at
//   most eleven, so a block of ten non-zero levels is some ninety
//   cycles. That is less than the quantiser spends on it and far less
//   than the transform, so the packing was never the thing to optimise.
//
// What it does not do
//   No framing: no `SOI`, `DQT`, `SOF0`, `DHT`, `SOS` or `EOI`. A
//   caller writes those around the bytes this produces.
//
//   One component. A colour image needs a DC predictor and a pair of
//   tables per component; this has one of each, which is a greyscale
//   baseline scan and a legal, decodable file.
//
//   No restart markers, and no count of the bytes emitted.
module jpeg_entropy (
    input  wire               clk,
    input  wire               rst_n,

    // The code tables: `tbl_ac` picks the AC table over the DC one.
    // `tbl_code` is right-justified in its `tbl_len` bits.
    input  wire        [7:0]  tbl_addr,
    input  wire        [15:0] tbl_code,
    input  wire        [4:0]  tbl_len,
    input  wire               tbl_ac,
    input  wire               tbl_we,

    // Clear the DC predictor: the start of a scan.
    input  wire               start,

    // A block's levels, zig-zag order, 64 of them.
    input  wire signed [11:0] in_level,
    input  wire               in_valid,
    output wire               in_ready,

    // Pad and emit the final partial byte.
    input  wire               flush,

    output wire        [7:0]  out_byte,
    output wire               out_valid,
    input  wire               out_ready
);
    // One state a job, and one shift mechanism they all share: a `ret`
    // register says where `S_SHIFT` goes when the bits have gone out.
    // Written with the run-length loop open-coded instead, it was
    // wrong twice over -- two assignments to `ret` in one arm, and an
    // end-of-block test that read `run` before the cycle that clears
    // it.
    localparam [3:0] S_IDLE  = 4'd0;   // waiting for a level
    localparam [3:0] S_RUN   = 4'd1;   // ZRL, or the symbol
    localparam [3:0] S_LOOK  = 4'd2;   // a table is answering
    localparam [3:0] S_CODE  = 4'd10;  // and has answered
    localparam [3:0] S_SHIFT = 4'd3;   // bits going out, one a cycle
    localparam [3:0] S_RSUB  = 4'd4;   // sixteen zeros accounted for
    localparam [3:0] S_VAL   = 4'd5;   // the magnitude bits
    localparam [3:0] S_AFTER = 4'd6;   // bookkeeping after a level
    localparam [3:0] S_EOBQ  = 4'd7;   // ask for EOB's code
    localparam [3:0] S_END   = 4'd8;   // the block is over
    localparam [3:0] S_PAD   = 4'd9;   // flush

    reg [3:0]  state;
    reg [3:0]  ret;                    // where S_SHIFT goes next
    reg [6:0]  pos;                    // which level, 0..63
    reg [6:0]  run;                    // zeros since the last level
    reg signed [11:0] dc_prev;
    reg signed [11:0] level;           // the level being coded
    reg signed [12:0] diff;            // the DC difference
    reg [3:0]  size;                   // its magnitude's bits

    // -----------------------------------------------------------------
    // The tables
    // -----------------------------------------------------------------
    // **One memory, not four.** The DC and AC tables each hold a code
    // and a length, which is the obvious four arrays — and read that
    // way, two arrays drive each of `code_q` and `len_q`, which is not
    // the bare `q <= mem[a]` this flow builds a memory from. The result
    // was 102 distributed RAMs on an ECP5 and, on an iCE40, **5712
    // flip-flops and 11617 lookup tables**: the tables became registers,
    // nine times the part.
    //
    // So: one 512-word memory of 21 bits, addressed by the table bit
    // and the symbol. The DC half uses sixteen of its 256 words and the
    // waste is not worth a second memory and a multiplexer to save.
    reg [20:0] tbl [0:511];
    reg [20:0] tbl_q;

    reg [7:0] look;                    // the symbol being looked up
    reg       look_ac;

    always @(posedge clk) begin
        if (tbl_we) tbl[{tbl_ac, tbl_addr}] <= {tbl_len, tbl_code};
        tbl_q <= tbl[{look_ac, look}];
    end

    wire [15:0] code_q = tbl_q[15:0];
    wire [4:0]  len_q  = tbl_q[20:16];

    // -----------------------------------------------------------------
    // The bit shifter and the byte packer
    // -----------------------------------------------------------------
    reg [15:0] sh;                     // left-justified: bit 15 goes next
    reg [4:0]  sh_n;
    reg [7:0]  acc;                    // the byte being filled
    reg [3:0]  bn;                     // how many of its bits are valid
    reg [7:0]  obyte;
    reg        ov;
    reg        pend;                   // a stuffed 00 is owed

    // A bit may go in unless it would finish a byte with nowhere to
    // put it; `bn == 7` is the bit that finishes one.
    wire can_push = (bn != 4'd7) || (!ov && !pend);
    wire [7:0] filled = {acc[6:0], sh[15]};

    assign out_byte  = obyte;
    assign out_valid = ov;
    assign in_ready  = (state == S_IDLE) && !flush;

    wire took = in_valid && in_ready;
    wire gave = ov && out_ready;

    // The magnitude's bits, left-justified: the value when positive and
    // one less when negative, which is F.1.2.1.1.
    wire signed [12:0] vsrc  = (pos == 7'd0) ? diff : {level[11], level};
    wire        [12:0] vbits = vsrc[12] ? (vsrc - 13'sd1) : vsrc;
    wire        [15:0] vwide = {vbits, 3'd0};
    reg  [15:0] valigned;
    always @* begin
        case (size)
            4'd1:  valigned = {vbits[0],    15'd0};
            4'd2:  valigned = {vbits[1:0],  14'd0};
            4'd3:  valigned = {vbits[2:0],  13'd0};
            4'd4:  valigned = {vbits[3:0],  12'd0};
            4'd5:  valigned = {vbits[4:0],  11'd0};
            4'd6:  valigned = {vbits[5:0],  10'd0};
            4'd7:  valigned = {vbits[6:0],   9'd0};
            4'd8:  valigned = {vbits[7:0],   8'd0};
            4'd9:  valigned = {vbits[8:0],   7'd0};
            4'd10: valigned = {vbits[9:0],   6'd0};
            4'd11: valigned = {vbits[10:0],  5'd0};
            4'd12: valigned = {vbits[11:0],  4'd0};
            default: valigned = vwide;
        endcase
    end

    // A code is right-justified in `len_q` bits and is shifted up so
    // its first bit is bit 15.
    reg [15:0] caligned;
    always @* begin
        case (len_q)
            5'd1:  caligned = {code_q[0],    15'd0};
            5'd2:  caligned = {code_q[1:0],  14'd0};
            5'd3:  caligned = {code_q[2:0],  13'd0};
            5'd4:  caligned = {code_q[3:0],  12'd0};
            5'd5:  caligned = {code_q[4:0],  11'd0};
            5'd6:  caligned = {code_q[5:0],  10'd0};
            5'd7:  caligned = {code_q[6:0],   9'd0};
            5'd8:  caligned = {code_q[7:0],   8'd0};
            5'd9:  caligned = {code_q[8:0],   7'd0};
            5'd10: caligned = {code_q[9:0],   6'd0};
            5'd11: caligned = {code_q[10:0],  5'd0};
            5'd12: caligned = {code_q[11:0],  4'd0};
            5'd13: caligned = {code_q[12:0],  3'd0};
            5'd14: caligned = {code_q[13:0],  2'd0};
            5'd15: caligned = {code_q[14:0],  1'd0};
            5'd16: caligned = code_q;
            default: caligned = 16'd0;
        endcase
    end

    // How many bits a magnitude needs: where its highest one is, plus
    // one.
    //
    // **A binary search and not a priority chain.** Written as thirteen
    // `else if`s it is thirteen levels of logic in front of the
    // register that holds the symbol, and the block measured depth 16.
    // Halving the field four times is four levels of OR and four
    // multiplexers, and it does the same thing.
    function [3:0] category;
        input [12:0] m;
        reg [15:0] v;
        reg [7:0]  a;
        reg [3:0]  b;
        reg [1:0]  c;
        reg        h8, h4, h2;
        begin
            v  = {3'd0, m};
            h8 = |v[15:8];
            a  = h8 ? v[15:8] : v[7:0];
            h4 = |a[7:4];
            b  = h4 ? a[7:4] : a[3:0];
            h2 = |b[3:2];
            c  = h2 ? b[3:2] : b[1:0];
            category = (|v) ? ({h8, h4, h2, c[1]} + 4'd1) : 4'd0;
        end
    endfunction

    wire signed [12:0] dcdiff = {in_level[11], in_level} - {dc_prev[11], dc_prev};
    wire        [12:0] dcmag  = dcdiff[12] ? (~dcdiff + 13'sd1) : dcdiff;
    wire        [12:0] aclev  = {in_level[11], in_level};
    wire        [12:0] acmag  = aclev[12] ? (~aclev + 13'sd1) : aclev;

    // What `run` becomes once this level is accounted for: a level that
    // was coded consumed the run, a zero added to it. The end-of-block
    // test uses **this** and not `run`, which was the second of the two
    // bugs the restructuring fixed.
    wire [6:0] run_next = (pos != 7'd0 && level == 12'sd0) ? run : 7'd0;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state   <= S_IDLE;
            ret     <= S_IDLE;
            pos     <= 7'd0;
            run     <= 7'd0;
            dc_prev <= 12'sd0;
            level   <= 12'sd0;
            diff    <= 13'sd0;
            size    <= 4'd0;
            sh      <= 16'd0;
            sh_n    <= 5'd0;
            acc     <= 8'd0;
            bn      <= 4'd0;
            obyte   <= 8'd0;
            ov      <= 1'b0;
            pend    <= 1'b0;
            look    <= 8'd0;
            look_ac <= 1'b0;
        end else begin
            if (start) begin
                dc_prev <= 12'sd0;
                pos     <= 7'd0;
                run     <= 7'd0;
            end
            // The output register, and the stuffed zero that follows a
            // `FF`.
            if (gave) begin
                if (pend) begin
                    obyte <= 8'd0;
                    pend  <= 1'b0;
                end else begin
                    ov <= 1'b0;
                end
            end
            case (state)
                S_IDLE: begin
                    if (took) begin
                        level <= in_level;
                        if (pos == 7'd0) begin
                            diff    <= dcdiff;
                            size    <= category(dcmag);
                            dc_prev <= in_level;
                            look    <= {4'd0, category(dcmag)};
                            look_ac <= 1'b0;
                            ret     <= S_VAL;
                            state   <= S_LOOK;
                        end else if (in_level == 12'sd0) begin
                            run   <= run + 7'd1;
                            state <= S_AFTER;
                        end else begin
                            size  <= category(acmag);
                            state <= S_RUN;
                        end
                    end else if (flush && bn != 4'd0) begin
                        state <= S_PAD;
                    end
                end
                // Sixteen zeros at a time, then the symbol itself.
                S_RUN: begin
                    look_ac <= 1'b1;
                    if (run > 7'd15) begin
                        look <= 8'hF0;
                        ret  <= S_RSUB;
                    end else begin
                        look <= {run[3:0], size};
                        ret  <= S_VAL;
                    end
                    state <= S_LOOK;
                end
                S_RSUB: begin
                    run   <= run - 7'd16;
                    state <= S_RUN;
                end
                // **Two states and not one.** `look` is a register, so
                // the memory reads it during `S_LOOK` and `code_q` is
                // only good in the state after that. Loading `sh` in
                // `S_LOOK` loaded the *previous* symbol's code, which
                // put `0x24 0xD7` where `0xF4 0x9A` belonged on a block
                // with nothing but a DC level in it.
                S_LOOK: state <= S_CODE;
                S_CODE: begin
                    sh    <= caligned;
                    sh_n  <= len_q;
                    state <= S_SHIFT;
                end
                S_SHIFT: if (sh_n == 5'd0) begin
                    state <= ret;
                end else if (can_push) begin
                    sh   <= {sh[14:0], 1'b0};
                    sh_n <= sh_n - 5'd1;
                    acc  <= filled;
                    if (bn == 4'd7) begin
                        bn    <= 4'd0;
                        obyte <= filled;
                        ov    <= 1'b1;
                        pend  <= (filled == 8'hFF);
                    end else begin
                        bn <= bn + 4'd1;
                    end
                end
                S_VAL: if (size == 4'd0) begin
                    state <= S_AFTER;
                end else begin
                    sh    <= valigned;
                    sh_n  <= {1'b0, size};
                    ret   <= S_AFTER;
                    state <= S_SHIFT;
                end
                S_AFTER: begin
                    run <= run_next;
                    if (pos == 7'd63) begin
                        // A tail of zeros nothing coded is `EOB`.
                        if (run_next != 7'd0) begin
                            look    <= 8'h00;
                            look_ac <= 1'b1;
                            ret     <= S_END;
                            state   <= S_EOBQ;
                        end else begin
                            state <= S_END;
                        end
                    end else begin
                        pos   <= pos + 7'd1;
                        state <= S_IDLE;
                    end
                end
                S_EOBQ: state <= S_LOOK;
                S_END: begin
                    pos   <= 7'd0;
                    run   <= 7'd0;
                    state <= S_IDLE;
                end
                // Pad the final byte with ones, which F.1.2.3 asks for
                // and a decoder reading whole bytes needs.
                S_PAD: begin
                    sh    <= 16'hFFFF;
                    sh_n  <= 5'd8 - {1'b0, bn};
                    ret   <= S_IDLE;
                    state <= S_SHIFT;
                end
                default: state <= S_IDLE;
            endcase
        end
    end
endmodule
