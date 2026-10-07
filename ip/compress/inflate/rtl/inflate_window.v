// inflate_window — DEFLATE's sliding window, and the distance check.
//
// What it does
//   `1 << WINDOW_BITS` bytes of history in a circular buffer, with a
//   write port that appends and a read port that reaches an arbitrary
//   distance back. RFC 1951 §3.2.5's copies are *random* reads at a
//   distance of 1 to 32768, which is why this is not a FIFO: a FIFO can
//   give back the oldest byte or the newest, and a match needs the one
//   `dist` bytes ago with `dist` arriving fresh out of the bit stream.
//
//   `wr_en` appends `wr_data` and advances the write pointer. Every byte
//   the decompressor produces goes through here, literals included,
//   because a later match may reach back to any of them.
//
//   `copy_open` latches a read pointer at `dist` bytes back. `rd_en`
//   then hands over one byte a cycle, `rd_valid` one cycle behind it,
//   advancing the pointer each time. `dist` must hold still for the one
//   cycle `copy_open` is asserted in and not after.
//
// Why it is its own module
//   Because `dist_bad` is the whole security of a decompressor's memory,
//   and a module boundary is what lets a test drive it directly rather
//   than through a Huffman decoder.
//   `inflate_window_refuses_a_distance_it_does_not_hold` writes n bytes
//   and then asks for n+1 back.
//
// The distance that reaches behind the start of the stream
//   A stream whose first match says "copy from 500 bytes ago" when 10
//   bytes have been produced is **malformed**, and the only two things a
//   decompressor may do about it are refuse it or invent bytes. Inventing
//   is what a buffer that is merely uninitialised does, and what it hands
//   back is whatever the last stream left in the block RAM — which is a
//   disclosure, not merely a wrong answer.
//
//   So the window counts. `full_q` latches when the write pointer first
//   wraps; before that the history is exactly `wptr_q` bytes long and
//   after it is the whole window. `dist_bad` is high whenever `dist` is
//   zero — which RFC 1951 §3.2.5 never encodes, the smallest distance
//   being 1 — or larger than that history. The caller must not assert
//   `copy_open` in a cycle where `dist_bad` is high, and `inflate.v`
//   reports `E_DIST` instead.
//
//   The same comparison does a second job for free. A design built with
//   `WINDOW_BITS` below 15 cannot decode every legal stream, because a
//   compressor with a 32 KiB window may name a distance this window does
//   not hold. That is not a malformed stream, but it is one this
//   instance cannot decode, and `dist_bad` catches it as the same
//   reported error rather than as a wrong answer. See
//   `ip/compress/inflate/README.md` §5 for why that was preferred to
//   rejecting the stream up front on RFC 1950 §2.2's CINFO field.
//
// Reading a byte in the cycle it is written
//   A copy at distance 1 is a run: every byte it reads is the byte it
//   wrote on the cycle before, and the read and write pointers advance
//   in lockstep one apart. On the cycle a byte is written to `wptr_q`
//   the read for the *next* output byte is issued at `wptr_q - 1 + 1`,
//   which is `wptr_q` itself, and a synchronous memory hands back the old
//   contents of an address it is writing.
//
//   `fwd_q` is the bypass. Whenever the issued read address is the
//   address being written in the same cycle, the written byte is
//   registered alongside and `rd_data` takes it instead of the memory's
//   output. Distance 1 is the only distance that can trigger it — the
//   two pointers stay `dist` apart — but the comparison is written on
//   the addresses rather than on `dist == 1` so that it stays correct if
//   the copy engine's pipeline ever changes.
//
// What it does not do
//   No preset dictionary. RFC 1950 §2.2's FDICT would pre-load this
//   window from an agreed string, and `inflate.v` reports a header that
//   asks for one rather than decoding it with an empty window.
//
//   No reset on the memory and none on the read register. A block RAM
//   has no reset pin and a read port whose output register is reset is
//   not a block RAM at all on this flow, which at 262 144 bits is the
//   difference between sixteen `DP16KD` and a design that does not fit a
//   part. `start` resets the *pointers*, which is what makes the stale
//   contents unreachable.
module inflate_window #(
    // 15 is RFC 1951 §3.2.5's largest distance, 32768, and so the only
    // setting that decodes every legal stream. Lower settings are for
    // designs that know their streams were compressed with a smaller
    // window; see the header and README §5.
    parameter WINDOW_BITS = 15
) (
    input  wire        clk,
    input  wire        rst_n,

    // Forget everything: the history becomes empty, so every distance is
    // out of range until bytes have been written.
    input  wire        start,

    input  wire [7:0]  wr_data,
    input  wire        wr_en,

    // A distance back from the next byte to be written, 1 being the byte
    // written last. `dist_bad` answers in the same cycle.
    input  wire [15:0] dist,
    output wire        dist_bad,
    // Latch a read pointer at `dist`. Must not be asserted together with
    // `wr_en`, which would leave it ambiguous which byte "the next one"
    // is; `inflate.v` always leaves a cycle between the two.
    input  wire        copy_open,

    input  wire        rd_en,
    // The byte on `rd_data` has been taken. Without it this module could
    // not tell a stalled consumer from a finished one, and `rd_valid`
    // would drop a byte nobody had taken yet — which is exactly the
    // defect `inflate_window_refuses_a_distance_it_does_not_hold` grew a
    // section for after `Sink::Alternate` found it on a four-byte stream.
    input  wire        rd_take,
    output wire [7:0]  rd_data,
    output wire        rd_valid
);
    localparam integer DEPTH = 1 << WINDOW_BITS;

    reg [7:0]             mem [0:DEPTH-1];

    reg [WINDOW_BITS-1:0] wptr_q;
    reg                   full_q;
    reg [WINDOW_BITS-1:0] src_q;
    reg                   rv_q;
    reg [7:0]             fwd_q;
    reg                   use_fwd_q;

    // The bare `q <= mem[a]` this flow needs to see to build a block
    // RAM: no reset, nothing combined into the right-hand side, and one
    // consumer.
    reg [7:0]             rd_q;

    // How many bytes of history there are. At most `1 << WINDOW_BITS`,
    // which is 32768 at the default and so wants sixteen bits — the same
    // width as `dist`, whose largest legal value it is.
    wire [15:0] have = full_q ? (16'd1 << WINDOW_BITS)
                              : {{(16 - WINDOW_BITS){1'b0}}, wptr_q};

    assign dist_bad = (dist == 16'd0) || (dist > have);

    // `dist` is at most `1 << WINDOW_BITS` when it is legal, so the low
    // `WINDOW_BITS` of it are the right subtrahend modulo the window: a
    // distance of exactly the window length wraps to the write pointer,
    // which is the oldest byte and is the right answer.
    wire [WINDOW_BITS-1:0] first_src = wptr_q - dist[WINDOW_BITS-1:0];

    wire collide = rd_en && wr_en && (src_q == wptr_q);

    assign rd_data  = use_fwd_q ? fwd_q : rd_q;
    assign rd_valid = rv_q;

    always @(posedge clk) begin
        if (wr_en) mem[wptr_q] <= wr_data;
        if (rd_en) rd_q <= mem[src_q];
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            wptr_q    <= {WINDOW_BITS{1'b0}};
            full_q    <= 1'b0;
            src_q     <= {WINDOW_BITS{1'b0}};
            rv_q      <= 1'b0;
            fwd_q     <= 8'd0;
            use_fwd_q <= 1'b0;
        end else begin
            if (start) begin
                wptr_q <= {WINDOW_BITS{1'b0}};
                full_q <= 1'b0;
            end else if (wr_en) begin
                wptr_q <= wptr_q + {{(WINDOW_BITS-1){1'b0}}, 1'b1};
                if (wptr_q == {WINDOW_BITS{1'b1}}) full_q <= 1'b1;
            end

            if (copy_open) src_q <= first_src;
            else if (rd_en) src_q <= src_q + {{(WINDOW_BITS-1){1'b0}}, 1'b1};

            // A one-deep valid: a read sets it and only a take clears
            // it, so a byte survives any number of stalled cycles. The
            // data survives with it because `rd_q` and `fwd_q` are
            // written only when a read is issued.
            if (start) rv_q <= 1'b0;
            else rv_q <= rd_en || (rv_q && !rd_take);
            if (rd_en) begin
                use_fwd_q <= collide;
                if (collide) fwd_q <= wr_data;
            end
        end
    end
endmodule
