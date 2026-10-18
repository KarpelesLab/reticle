// mjpeg_enc — a baseline greyscale JPEG encoder: pixels in, a whole
// file out.
//
// What it does
//   Puts `jpeg_fdct`, `jpeg_quant` and `jpeg_entropy` in a row, writes
//   the markers of ITU T.81 §B.2 around them, and hands back every byte
//   of a `.jpg` from its `SOI` to its `EOI`. One scan, one component,
//   eight bits a sample: a greyscale baseline file, which is legal and
//   which every decoder reads.
//
//   The three blocks compose by their handshakes and nothing else is
//   needed between them — a coefficient leaves the transform and enters
//   the quantiser, a level leaves the quantiser and enters the coder.
//   What this module adds is the configuration, the markers, and the
//   order of events.
//
// One source of truth for each table
//   A JPEG carries its quantisation table in `DQT` and its Huffman
//   tables in `DHT`, and the encoder needs the same numbers. **The
//   caller writes each table once, here**, in exactly the form the file
//   carries it: 64 bytes in zig-zag order for `DQT`, and for each `DHT`
//   the sixteen counts of codes per length followed by the symbols.
//   This module writes them into the file *and* derives what the blocks
//   need from them — the quantiser's divisors verbatim, and the coder's
//   codes by the canonical procedure of §C.2.
//
//   That is the whole reason `jpeg_quant` and `jpeg_entropy` take
//   tables rather than containing Annex K: a table written twice is a
//   table that can disagree with itself, and a file whose `DHT` does
//   not match the codes used to write it decodes to nothing.
//
// How a frame goes
//   `start` begins one. First a **prepare** phase, some seven hundred
//   cycles, which derives the Huffman codes, copies the divisors into
//   the quantiser, and assembles the header into the spare half of the
//   configuration memory. Then the header is emitted as one run, then
//   the pixels are taken and the coded bytes come out behind them, then
//   the coder is flushed and `EOI` goes out and `done` rises.
//
//   **Pixels are not accepted until the header is out.** They could be,
//   with another buffer; they are not, because the coded bytes would
//   then have to queue behind a header that is still going and the
//   buffer would have to be as deep as the header is long.
//
// The pixel order is block order
//   64 samples a block, row-major inside the block, and the blocks in
//   raster order across the image. **Not raster order over the whole
//   picture**, which would need eight lines of it buffered — 15 kilobytes
//   at 640 wide, four block RAMs, and a module about line buffering
//   rather than about JPEG. A design with a frame buffer has blocks for
//   the asking; one with a camera needs those line buffers, and they
//   are not here.
//
// What it does not do
//   No colour. Three components want a DC predictor and a pair of
//   tables each, and 4:2:0 wants the subsampling and the macroblock
//   order on top of that.
//
//   No restart markers, no `APP0`/JFIF segment (a decoder does not need
//   one), no comment, and no arithmetic coding or progressive mode.
//
//   Nothing checks the tables. A `DHT` that gives some symbol no code
//   makes a file nothing can read, and a zero in `DQT` is forbidden by
//   the standard; `jpeg_quant` treats a zero as a one and this module
//   passes both through as the caller wrote them.
module mjpeg_enc #(
    // Both must be multiples of eight: this encoder does not pad an
    // edge block, so a picture that is not a whole number of blocks
    // cannot be expressed.
    parameter integer WIDTH  = 640,
    parameter integer HEIGHT = 480
) (
    input  wire       clk,
    input  wire       rst_n,

    // The tables, each in the form the file carries it.
    //   `cfg_sel` 0 — `DQT`'s 64 bytes, zig-zag order
    //   `cfg_sel` 1 — the DC `DHT`: 16 counts, then the symbols
    //   `cfg_sel` 2 — the AC `DHT`: the same
    input  wire [1:0] cfg_sel,
    input  wire [7:0] cfg_addr,
    input  wire [7:0] cfg_data,
    input  wire       cfg_we,

    // Begin a frame.
    input  wire       start,

    // The samples: 64 a block, row-major in the block, blocks in
    // raster order.
    input  wire [7:0] in_pixel,
    input  wire       in_valid,
    output wire       in_ready,

    output wire [7:0] out_byte,
    output wire       out_valid,
    input  wire       out_ready,

    // The frame is finished and every byte of it has been taken.
    output wire       done
);
    localparam integer BLOCKS = (WIDTH / 8) * (HEIGHT / 8);

    // The configuration memory, and the header built in its upper half.
    // 0x000..0x03F  the quantisation table
    // 0x100..0x1FF  the DC `DHT`
    // 0x200..0x2FF  the AC `DHT`
    // 0x400..0x7FF  the header, assembled by `start`
    //
    // **The header needs more than 256 bytes.** Greyscale with Annex
    // K's tables is 310, so a header region at `0x300` ran off the end
    // of a 1024-byte memory and wrapped onto the quantisation table.
    // The first frame survived it -- the quantiser had already been
    // loaded from the table by then -- and the second read the wreckage
    // and wrote a file whose `DQT` was the tail of the AC symbol list.
    localparam [10:0] HDR_BASE = 11'h400;

    localparam [3:0] S_CFG   = 4'd0;   // taking tables, waiting for start
    localparam [3:0] S_CNTRD = 4'd1;   // a DHT count is being read
    localparam [3:0] S_CNTUS = 4'd2;   // and has arrived
    localparam [3:0] S_SYMRD = 4'd3;   // a DHT symbol is being read
    localparam [3:0] S_SYMUS = 4'd4;   // and has arrived: one code
    localparam [3:0] S_QRD   = 4'd5;   // a divisor, into the quantiser
    localparam [3:0] S_QUS   = 4'd6;
    localparam [3:0] S_BUILD = 4'd7;   // assemble the header
    localparam [3:0] S_COPY  = 4'd8;   // a run of it comes from a table
    localparam [3:0] S_HEAD  = 4'd9;   // emit the header
    localparam [3:0] S_DATA  = 4'd10;  // pixels in, coded bytes out
    localparam [3:0] S_FLUSH = 4'd11;  // the coder's last byte
    localparam [3:0] S_EOI   = 4'd12;  // FF D9
    localparam [3:0] S_DONE  = 4'd13;
    localparam [3:0] S_CWAIT = 4'd14;  // the copy's first byte is in flight

    reg [3:0] state;

    // -----------------------------------------------------------------
    // The memory: one write port, one read port
    // -----------------------------------------------------------------
    reg [7:0] cfg [0:2047];
    reg [7:0] cfg_q;
    reg [10:0] rd_addr;
    reg [10:0] wr_addr;
    reg [7:0] wr_data;
    reg       wr_en;

    // **One write port, and the multiplexing outside the memory.**
    // Written as two arms -- `cfg_we` to one address, `wr_en` to
    // another -- inference saw two write ports and left a generic
    // `memory` in the netlist that neither backend could map:
    // `docs/ip-library.md` recorded `1 x memory 1024x8, 1 x memrd,
    // 2 x memwr` beside the block RAMs it *did* infer, which is a
    // netlist that would not place. The caller's
    // writes and the header builder's never collide, because the
    // builder only runs between `start` and the first byte out.
    wire [10:0] mem_wa = cfg_we ? {1'b0, cfg_sel, cfg_addr} : wr_addr;
    wire [7:0] mem_wd = cfg_we ? cfg_data : wr_data;
    wire       mem_we = cfg_we | wr_en;

    always @(posedge clk) begin
        if (mem_we) cfg[mem_wa] <= mem_wd;
        cfg_q <= cfg[rd_addr];
    end

    // -----------------------------------------------------------------
    // Deriving the Huffman codes: T.81 §C.2
    // -----------------------------------------------------------------
    reg        p_ac;                   // which table is being walked
    reg [10:0]  p_base;                 // where that table lives
    reg [4:0]  p_len;                  // 1..16
    reg [7:0]  p_left;                 // codes of this length still to do
    reg [7:0]  p_k;                    // which symbol
    reg [15:0] p_code;                 // the running code
    reg [7:0]  nsym_dc, nsym_ac;

    // -----------------------------------------------------------------
    // Building and emitting the header
    // -----------------------------------------------------------------
    reg [3:0]  b_step;
    reg [8:0]  b_idx;                  // within the step
    reg [10:0]  b_hp;                   // where the next header byte goes
    reg [10:0]  b_src;                  // where a copied run comes from
    reg [8:0]  b_run;                  // how many bytes it is
    reg [10:0]  hdr_len;                // the header, once built
    reg [10:0]  h_ix;                   // emitting it

    wire [10:0] dht_dc_len = 11'd19 + {3'd0, nsym_dc};
    wire [10:0] dht_ac_len = 11'd19 + {3'd0, nsym_ac};

    // The literal bytes of each marker segment. The computed ones --
    // the dimensions and the two `DHT` lengths -- are here too, which
    // is why this is logic and not a ROM.
    reg [7:0] lit;
    reg [8:0] lit_n;                   // how many bytes this step has
    always @* begin
        case (b_step)
            4'd0: begin                // SOI, and DQT's header
                lit_n = 9'd7;
                case (b_idx)
                    9'd0: lit = 8'hFF;
                    9'd1: lit = 8'hD8;
                    9'd2: lit = 8'hFF;
                    9'd3: lit = 8'hDB;
                    9'd4: lit = 8'h00;
                    9'd5: lit = 8'h43;   // 67: two, one, and 64
                    default: lit = 8'h00;   // Pq = 0, Tq = 0
                endcase
            end
            4'd2: begin                // SOF0
                lit_n = 9'd13;
                case (b_idx)
                    9'd0:  lit = 8'hFF;
                    9'd1:  lit = 8'hC0;
                    9'd2:  lit = 8'h00;
                    9'd3:  lit = 8'h0B;
                    9'd4:  lit = 8'h08;              // eight bits a sample
                    9'd5:  lit = HEIGHT[15:8];
                    9'd6:  lit = HEIGHT[7:0];
                    9'd7:  lit = WIDTH[15:8];
                    9'd8:  lit = WIDTH[7:0];
                    9'd9:  lit = 8'h01;              // one component
                    9'd10: lit = 8'h01;              // its identifier
                    9'd11: lit = 8'h11;              // no subsampling
                    default: lit = 8'h00;            // table zero
                endcase
            end
            4'd3: begin                // the DC DHT's header
                lit_n = 9'd5;
                case (b_idx)
                    9'd0: lit = 8'hFF;
                    9'd1: lit = 8'hC4;
                    9'd2: lit = {6'd0, dht_dc_len[9:8]};
                    9'd3: lit = dht_dc_len[7:0];
                    default: lit = 8'h00;            // class 0, table 0
                endcase
            end
            4'd5: begin                // the AC DHT's header
                lit_n = 9'd5;
                case (b_idx)
                    9'd0: lit = 8'hFF;
                    9'd1: lit = 8'hC4;
                    9'd2: lit = {6'd0, dht_ac_len[9:8]};
                    9'd3: lit = dht_ac_len[7:0];
                    default: lit = 8'h10;            // class 1, table 0
                endcase
            end
            4'd7: begin                // SOS
                lit_n = 9'd10;
                case (b_idx)
                    9'd0: lit = 8'hFF;
                    9'd1: lit = 8'hDA;
                    9'd2: lit = 8'h00;
                    9'd3: lit = 8'h08;
                    9'd4: lit = 8'h01;               // one component
                    9'd5: lit = 8'h01;               // its identifier
                    9'd6: lit = 8'h00;               // DC table 0, AC table 0
                    9'd7: lit = 8'h00;               // Ss
                    9'd8: lit = 8'h3F;               // Se: 63
                    default: lit = 8'h00;            // Ah, Al
                endcase
            end
            default: begin lit = 8'h00; lit_n = 9'd0; end
        endcase
    end

    // -----------------------------------------------------------------
    // The three blocks
    // -----------------------------------------------------------------
    wire signed [11:0] coef;
    wire               coef_v, coef_r;
    wire signed [11:0] lev;
    wire               lev_v, lev_r;
    wire        [7:0]  ent_byte;
    wire               ent_v;
    wire               fdct_ready;

    reg        [5:0]  q_addr;
    reg        [5:0]  q_wa;             // see `S_QUS`
    reg               q_we;
    reg        [7:0]  tbl_addr_r;
    reg        [15:0] tbl_code_r;
    reg        [4:0]  tbl_len_r;
    reg               tbl_ac_r;
    reg               tbl_we_r;
    reg               ent_start;
    reg               ent_flush;
    reg               pix_gate;
    reg               head_armed;
    reg        [1:0]  eoi_ix;
    reg        [6:0]  quiet;            // cycles with nothing coming out
    reg        [19:0] lev_left;         // levels still to reach the coder

    jpeg_fdct u_fdct (
        .clk(clk), .rst_n(rst_n),
        .in_sample(in_pixel), .in_valid(in_valid && pix_gate),
        .in_ready(fdct_ready),
        .out_coef(coef), .out_valid(coef_v), .out_ready(coef_r));

    jpeg_quant u_quant (
        .clk(clk), .rst_n(rst_n),
        .tbl_addr(q_wa), .tbl_data(cfg_q), .tbl_we(q_we),
        .in_coef(coef), .in_valid(coef_v), .in_ready(coef_r),
        .out_level(lev), .out_valid(lev_v), .out_ready(lev_r));

    jpeg_entropy u_ent (
        .clk(clk), .rst_n(rst_n),
        .tbl_addr(tbl_addr_r), .tbl_code(tbl_code_r), .tbl_len(tbl_len_r),
        .tbl_ac(tbl_ac_r), .tbl_we(tbl_we_r),
        .start(ent_start),
        .in_level(lev), .in_valid(lev_v), .in_ready(lev_r),
        .flush(ent_flush),
        .out_byte(ent_byte), .out_valid(ent_v), .out_ready(out_ready));

    assign in_ready = pix_gate && fdct_ready;

    // -----------------------------------------------------------------
    // The output: the header, then the coded bytes, then EOI
    // -----------------------------------------------------------------
    assign out_byte  = (state == S_HEAD) ? cfg_q
                     : (state == S_EOI)  ? (eoi_ix == 2'd0 ? 8'hFF : 8'hD9)
                     :                     ent_byte;
    assign out_valid = (state == S_HEAD)  ? head_armed
                     : (state == S_EOI)   ? 1'b1
                     : ((state == S_DATA || state == S_FLUSH) && ent_v);
    assign done      = (state == S_DONE);

    wire gave = out_valid && out_ready;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state      <= S_CFG;
            p_ac       <= 1'b0;
            p_base     <= 11'h100;
            p_len      <= 5'd1;
            p_left     <= 8'd0;
            p_k        <= 8'd0;
            p_code     <= 16'd0;
            nsym_dc    <= 8'd0;
            nsym_ac    <= 8'd0;
            b_step     <= 4'd0;
            b_idx      <= 9'd0;
            b_hp       <= 11'd0;
            b_src      <= 11'd0;
            b_run      <= 9'd0;
            hdr_len    <= 11'd0;
            h_ix       <= 11'd0;
            head_armed <= 1'b0;
            rd_addr    <= 11'd0;
            wr_addr    <= 11'd0;
            wr_data    <= 8'd0;
            wr_en      <= 1'b0;
            q_addr     <= 6'd0;
            q_wa       <= 6'd0;
            q_we       <= 1'b0;
            tbl_we_r   <= 1'b0;
            tbl_ac_r   <= 1'b0;
            tbl_addr_r <= 8'd0;
            tbl_code_r <= 16'd0;
            tbl_len_r  <= 5'd0;
            ent_start  <= 1'b0;
            ent_flush  <= 1'b0;
            pix_gate   <= 1'b0;
            eoi_ix     <= 2'd0;
            quiet      <= 7'd0;
            lev_left   <= 20'd0;
        end else begin
            wr_en    <= 1'b0;
            q_we     <= 1'b0;
            tbl_we_r <= 1'b0;
            // **`start` is a pulse, not a level.** The coder clears its
            // predictor and its coefficient position whenever `start`
            // is high, so held high it codes every level as a DC
            // coefficient and emits bytes without end: a quarter of a
            // million of them, for an image of 256 pixels.
            ent_start <= 1'b0;
            case (state)
                // **`S_DONE` shares this arm.** One `start` has to be
                // enough to begin the next frame: with `S_DONE` only
                // falling back to `S_CFG`, a second frame needed two
                // pulses and a caller that sent one waited for ever.
                // Motion JPEG is this module run again, so that is the
                // ordinary case and not an edge of one.
                S_CFG, S_DONE: if (start) begin
                    p_ac    <= 1'b0;
                    p_base  <= 11'h100;
                    p_len   <= 5'd1;
                    p_k     <= 8'd0;
                    p_code  <= 16'd0;
                    nsym_dc <= 8'd0;
                    nsym_ac <= 8'd0;
                    rd_addr <= 11'h100;
                    state   <= S_CNTRD;
                end
                S_CNTRD: state <= S_CNTUS;
                S_CNTUS: begin
                    p_left <= cfg_q;
                    if (p_ac) nsym_ac <= nsym_ac + cfg_q;
                    else      nsym_dc <= nsym_dc + cfg_q;
                    if (cfg_q == 8'd0) begin
                        // No codes of this length: shift and move on.
                        p_code <= {p_code[14:0], 1'b0};
                        state  <= S_CNTUS;          // overwritten below
                        if (p_len == 5'd16) begin
                            if (p_ac) begin
                                q_addr  <= 6'd0;
                                rd_addr <= 11'h000;
                                state   <= S_QRD;
                            end else begin
                                p_ac    <= 1'b1;
                                p_base  <= 11'h200;
                                p_len   <= 5'd1;
                                p_k     <= 8'd0;
                                p_code  <= 16'd0;
                                rd_addr <= 11'h200;
                                state   <= S_CNTRD;
                            end
                        end else begin
                            p_len   <= p_len + 5'd1;
                            rd_addr <= p_base + {6'd0, p_len};
                            state   <= S_CNTRD;
                        end
                    end else begin
                        rd_addr <= p_base + 11'd16 + {3'd0, p_k};
                        state   <= S_SYMRD;
                    end
                end
                S_SYMRD: state <= S_SYMUS;
                S_SYMUS: begin
                    tbl_we_r   <= 1'b1;
                    tbl_ac_r   <= p_ac;
                    tbl_addr_r <= cfg_q;
                    tbl_code_r <= p_code;
                    tbl_len_r  <= p_len;
                    p_k        <= p_k + 8'd1;
                    p_left     <= p_left - 8'd1;
                    if (p_left == 8'd1) begin
                        // The last of this length: C.2's increment and
                        // then its shift, which is `(code + 1) << 1`.
                        p_code <= {p_code[14:0], 1'b0} + 16'd2;
                        if (p_len == 5'd16) begin
                            if (p_ac) begin
                                q_addr  <= 6'd0;
                                rd_addr <= 11'h000;
                                state   <= S_QRD;
                            end else begin
                                p_ac    <= 1'b1;
                                p_base  <= 11'h200;
                                p_len   <= 5'd1;
                                p_k     <= 8'd0;
                                p_code  <= 16'd0;
                                rd_addr <= 11'h200;
                                state   <= S_CNTRD;
                            end
                        end else begin
                            p_len   <= p_len + 5'd1;
                            rd_addr <= p_base + {6'd0, p_len};
                            state   <= S_CNTRD;
                        end
                    end else begin
                        p_code  <= p_code + 16'd1;
                        rd_addr <= p_base + 11'd17 + {3'd0, p_k};
                        state   <= S_SYMRD;
                    end
                end
                // The divisors, verbatim, into the quantiser.
                S_QRD: state <= S_QUS;
                // **`q_we` is registered, so the divisor it writes is
                // the one `cfg_q` holds a cycle later -- still this
                // entry's, because the next read has not landed -- but
                // `q_addr` has already moved on. So the address goes
                // into `q_wa` beside the enable. Driving the quantiser
                // from `q_addr` instead wrote divisor `k` into entry
                // `k + 1`: entry 0 was never written at all, the DC
                // coefficient was divided by an undefined value, and
                // the coder's table lookup never resolved.
                S_QUS: begin
                    q_we <= 1'b1;
                    q_wa <= q_addr;
                    if (q_addr == 6'd63) begin
                        b_step <= 4'd0;
                        b_idx  <= 9'd0;
                        b_hp   <= HDR_BASE;
                        state  <= S_BUILD;
                    end else begin
                        q_addr  <= q_addr + 6'd1;
                        rd_addr <= {5'd0, q_addr} + 11'd1;
                        state   <= S_QRD;
                    end
                end
                // -------------------------------------------------
                // Assembling the header into the memory's upper half
                // -------------------------------------------------
                // Eight steps. **Three of them are copies and they are
                // not the odd ones**: 1 the divisors, 4 the DC table, 6
                // the AC table. Dispatching on `b_step[0]` instead made
                // steps 3 and 5 both literal and copy, and the build
                // looped for ever on a step whose literal length was
                // zero -- 0 pixels in and 0 bytes out.
                //
                //   0  SOI and DQT's header      7 literal bytes
                //   1  the divisors             64 copied
                //   2  SOF0                     13 literal
                //   3  the DC DHT's header       5 literal
                //   4  the DC table        16 + n copied
                //   5  the AC DHT's header       5 literal
                //   6  the AC table        16 + n copied
                //   7  SOS                      10 literal
                S_BUILD: begin
                    if (b_step == 4'd1 || b_step == 4'd4 || b_step == 4'd6) begin
                        case (b_step)
                            4'd1: begin
                                b_src <= 11'h000;
                                b_run <= 9'd64;
                            end
                            4'd4: begin
                                b_src <= 11'h100;
                                b_run <= 9'd16 + {1'd0, nsym_dc};
                            end
                            default: begin
                                b_src <= 11'h200;
                                b_run <= 9'd16 + {1'd0, nsym_ac};
                            end
                        endcase
                        b_idx   <= 9'd0;
                        rd_addr <= (b_step == 4'd1) ? 11'h000
                                 : (b_step == 4'd4) ? 11'h100 : 11'h200;
                        // **One cycle before the first write.** The read
                        // issued here lands in `cfg_q` two cycles from
                        // now, so going straight to `S_COPY` wrote
                        // whatever the previous read left there and
                        // shifted the whole run by a byte: a header of
                        // the right length whose every table was one
                        // entry out.
                        state   <= S_CWAIT;
                    end else if (lit_n == 9'd0) begin
                        // A literal step with no bytes would loop here
                        // for ever. There is none, and this says so.
                        state   <= S_CFG;
                    end else if (b_idx == lit_n - 9'd1) begin
                        wr_en   <= 1'b1;
                        wr_addr <= b_hp;
                        wr_data <= lit;
                        b_hp    <= b_hp + 11'd1;
                        b_idx   <= 9'd0;
                        if (b_step == 4'd7) begin
                            hdr_len    <= b_hp + 11'd1 - HDR_BASE;
                            h_ix       <= 11'd0;
                            head_armed <= 1'b0;
                            rd_addr    <= HDR_BASE;
                            ent_start  <= 1'b1;
                            state      <= S_HEAD;
                        end else begin
                            b_step <= b_step + 4'd1;
                        end
                    end else begin
                        wr_en   <= 1'b1;
                        wr_addr <= b_hp;
                        wr_data <= lit;
                        b_hp    <= b_hp + 11'd1;
                        b_idx   <= b_idx + 9'd1;
                    end
                end
                // One byte a cycle: the read issued last cycle is in
                // `cfg_q` now, so the write trails the read by one.
                // The read has to run **two** cycles ahead of the
                // write, not one: `cfg_q` holds what `rd_addr` named a
                // cycle ago, and `rd_addr` holds what was assigned to
                // it a cycle ago. One ahead made every byte of a run
                // the one before it.
                S_CWAIT: begin
                    rd_addr <= b_src + 11'd1;
                    state   <= S_COPY;
                end
                S_COPY: begin
                    wr_en   <= 1'b1;
                    wr_addr <= b_hp;
                    wr_data <= cfg_q;
                    b_hp    <= b_hp + 11'd1;
                    if (b_idx == b_run - 9'd1) begin
                        b_step <= b_step + 4'd1;
                        b_idx  <= 9'd0;
                        // **And back to S_BUILD.** Without this it sat
                        // in S_COPY writing the same byte for ever, and
                        // `b_hp` ran off the end of the header and
                        // wrapped.
                        state  <= S_BUILD;
                    end else begin
                        b_idx   <= b_idx + 9'd1;
                        rd_addr <= b_src + {2'd0, b_idx} + 11'd2;
                    end
                end
                // -------------------------------------------------
                // Out it goes
                // -------------------------------------------------
                S_HEAD: begin
                    if (!head_armed) begin
                        head_armed <= 1'b1;
                    end else if (gave) begin
                        head_armed <= 1'b0;
                        if (h_ix == hdr_len - 11'd1) begin
                            pix_gate <= 1'b1;
                            lev_left <= BLOCKS * 64;
                            state    <= S_DATA;
                        end else begin
                            h_ix    <= h_ix + 11'd1;
                            rd_addr <= HDR_BASE + h_ix + 11'd1;
                        end
                    end
                end
                S_DATA: begin
                    if (lev_v && lev_r) begin
                        lev_left <= lev_left - 20'd1;
                        if (lev_left == 20'd1) begin
                            pix_gate  <= 1'b0;
                            ent_flush <= 1'b1;
                            quiet     <= 7'd0;
                            state     <= S_FLUSH;
                        end
                    end
                end
                // Every level is in. The coder still owes the last
                // level's code, `EOB`, and a byte padded with ones, and
                // it has no "finished" line -- so it is taken to be
                // finished when nothing has come out for long enough
                // that nothing can be in flight.
                //
                // **Sixty-four is four times what the corpus needs.**
                // Measured by shrinking it: the six cases pass at 16
                // and fail at 15, so the longest gap the coder leaves
                // between two bytes is sixteen cycles -- a code and its
                // magnitude bits shifted a bit a cycle with a lookup in
                // front of them. The margin is not a proof, because a
                // table with longer codes than Annex K's would widen
                // that gap; what would catch a count too small is the
                // corpus, where it truncates the scan and no byte of
                // the file matches.
                S_FLUSH: begin
                    if (ent_v) begin
                        quiet <= 7'd0;
                    end else if (out_ready) begin
                        quiet <= quiet + 7'd1;
                        if (quiet == 7'd63) begin
                            ent_flush <= 1'b0;
                            eoi_ix    <= 2'd0;
                            state     <= S_EOI;
                        end
                    end
                end
                S_EOI: if (gave) begin
                    if (eoi_ix == 2'd1) state <= S_DONE;
                    else eoi_ix <= eoi_ix + 2'd1;
                end
                default: state <= S_CFG;
            endcase
        end
    end
endmodule
