// What does a distributed RAM do when one address is read and written in
// the same cycle, with the read feeding the write?
//
// **Why this exists.** `bram_rw.v` asked this of a `DP16KD` and the answer
// was read-before-write, as modelled. This asks it of a
// `TRELLIS_DPR16X4`, where the read is **asynchronous** -- a lookup table's
// output, not a register -- and where `ip/compress/inflate` does the
// sharpest version of it:
//
//     wire [8:0] lcnt_rd = lcnt[cnt_addr];        // combinational
//     ...
//     if (cnt_we) lcnt[cnt_addr] <= cnt_rd + 9'd1; // same address, same cycle
//
// A counting sort's increment pass is a read-modify-write of one address in
// one cycle, and the read is combinational, so the value written is the one
// the lookup table is presenting *while* the write is being set up.
//
// That matters because of what sits downstream. `inflate`'s `lsym` is never
// cleared between streams: the decoder only ever indexes the slots this
// table's counts say were filled, so stale entries are unreachable -- **if
// the counts are right**. A count wrong by anything reads a symbol the
// previous stream left, which is invisible while `lsym` is still all zeros
// and wrong the moment a different stream has written it. On a Cynthion a
// dynamic-Huffman stream decodes correctly only if no *different* stream has
// decoded since configuration, which is that shape exactly.
//
//     host -> device   len_hi len_lo, then that many bytes (at most 16)
//     device -> host   those addresses after one increment pass
//
// The expected answer is each byte plus one. Getting the byte back unchanged
// would mean the write never landed; getting plus two would mean the read
// already saw the new value; anything else says the address decoding moves
// under a simultaneous write.
//
// What this would catch: a read-modify-write of one address that does not
// come out as read-then-add-then-write. What it would not: anything about
// the two *ports* of a `TRELLIS_DPR16X4` at different addresses, and nothing
// about a count file deeper than one lookup table's sixteen words -- which
// is the shape `inflate` actually uses, and a reason to read this result as
// one piece of evidence rather than a verdict.

module lutram_rw (
    input  wire       clk,
    input  wire       rst_n,
    input  wire [7:0] host_data,
    input  wire       host_valid,
    output wire       host_ready,
    output wire [7:0] dev_data,
    output wire       dev_valid,
    input  wire       dev_ready,
    output wire       dev_commit
);
    // Sixteen words of eight bits is 128, below the 256-bit threshold that
    // sends a memory to block RAM, and an asynchronous read is what a
    // `DP16KD` cannot serve at all -- so this is a `TRELLIS_DPR16X4` for
    // two independent reasons.
    reg [7:0] mem [0:15];

    localparam [1:0] S_LEN_HI = 2'd0, S_LEN_LO = 2'd1,
                     S_FILL   = 2'd2, S_BUMP   = 2'd3;
    // The dump is not a state of its own: `S_LEN_HI` with `left` non-zero
    // would be ambiguous, so the pass counter says which half we are in.
    reg [1:0]  state = S_LEN_HI;
    reg        dumping = 1'b0;
    reg [15:0] len   = 16'd0;
    reg [3:0]  addr  = 4'd0;
    reg [15:0] left  = 16'd0;

    wire want_len = (state == S_LEN_HI) || (state == S_LEN_LO);
    assign host_ready = want_len || (state == S_FILL);
    wire took = host_valid & host_ready;

    // The asynchronous read. This is the wire whose value the write below
    // uses, in the same cycle, at the same address.
    wire [7:0] rd = mem[addr];

    wire bumping = (state == S_BUMP) && !dumping;

    assign dev_data   = rd;
    assign dev_valid  = (state == S_BUMP) && dumping;
    assign dev_commit = dev_valid;
    wire gave = dev_valid & dev_ready;

    // One write port with its data muxed outside the block. An
    // `if`/`else if` over two assignments becomes two `$memwr` cells, and
    // this memory then comes out as 128 flip-flops instead of a
    // distributed RAM -- which would answer a question about registers
    // rather than the one being asked.
    wire       wr_en = (state == S_FILL && took) || bumping;
    wire [7:0] wr_d  = (state == S_FILL) ? host_data : (rd + 8'd1);
    always @(posedge clk) begin
        if (wr_en) mem[addr] <= wr_d;
    end

    always @(posedge clk) begin
        if (!rst_n) begin
            state   <= S_LEN_HI;
            dumping <= 1'b0;
            len     <= 16'd0;
            addr    <= 4'd0;
            left    <= 16'd0;
        end else case (state)
            S_LEN_HI:
                if (took) begin
                    len[15:8] <= host_data;
                    state     <= S_LEN_LO;
                end
            S_LEN_LO:
                if (took) begin
                    len[7:0] <= host_data;
                    left     <= {len[15:8], host_data};
                    addr     <= 4'd0;
                    dumping  <= 1'b0;
                    state    <= ({len[15:8], host_data} == 16'd0)
                                ? S_LEN_HI : S_FILL;
                end
            S_FILL:
                if (took) begin
                    addr <= addr + 4'd1;
                    if (left == 16'd1) begin
                        addr  <= 4'd0;
                        left  <= len;
                        state <= S_BUMP;
                    end else begin
                        left <= left - 16'd1;
                    end
                end
            S_BUMP:
                if (!dumping) begin
                    // One increment per cycle, one address per cycle.
                    addr <= addr + 4'd1;
                    if (left == 16'd1) begin
                        addr    <= 4'd0;
                        left    <= len;
                        dumping <= 1'b1;
                    end else begin
                        left <= left - 16'd1;
                    end
                end else if (gave) begin
                    addr <= addr + 4'd1;
                    if (left == 16'd1) begin
                        addr    <= 4'd0;
                        dumping <= 1'b0;
                        state   <= S_LEN_HI;
                    end else begin
                        left <= left - 16'd1;
                    end
                end
            default: state <= S_LEN_HI;
        endcase
    end
endmodule
