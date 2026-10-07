// inflate — a DEFLATE (RFC 1951) and zlib (RFC 1950) decompressor.
//
// What it does
//   Takes a byte stream in and gives the decompressed byte stream out,
//   through two ready/valid handshakes. All three of RFC 1951 §3.2.3's
//   block types are decoded — stored, fixed Huffman and dynamic Huffman —
//   and with `WRAPPER` set the RFC 1950 framing around them is parsed and
//   its Adler-32 checked.
//
//   Every way a stream can be malformed is **reported** on `error` with a
//   code, and never hung on and never decoded into plausible rubbish.
//   `ip/compress/inflate/README.md` §6 is the list and §7 is the argument
//   that there is no input this block can be made to hang on.
//
// The shape of the thing, and why it is one state machine
//   A decompressor is five jobs — pull bits, decode a Huffman symbol,
//   turn a length and a distance code into numbers, copy out of the
//   window, hand bytes to the consumer — and the tempting arrangement is
//   five pipeline stages with a handshake between each. That arrangement
//   has a trap `ip/crypto/chacha20` found the hard way and wrote down:
//   if each stage's `ready` is a function of the next stage's `ready`,
//   then two such blocks back to back build a combinational path from one
//   block's `in_valid` to the other's `in_ready`, and enough of them in a
//   row take the clock with them.
//
//   So this is **one sequential state machine over one datapath**, not a
//   pipeline. The five jobs are states, they take turns, and there is no
//   internal handshake to get wrong. That costs throughput — §8 of the
//   README has the measurement and names what a table-driven decoder
//   would buy — and it buys a decompressor with exactly two places where
//   a combinational readiness chain could form. Both are cut:
//
//   - **The input boundary.** `nxt_q` is a one-byte slack slot in front
//     of the bit reader, and `in_ready` is `!nxt_full_q` and three other
//     registers. Nothing a producer drives reaches it, so `in_valid` to
//     `in_ready` is not a path.
//   - **The output boundary.** `out_q`/`ov_q` is a one-byte slack slot,
//     and `out_valid` is `ov_q` alone. A consumer's `out_ready` reaches
//     register enables inside this block and stops there; it never
//     reaches `in_ready`.
//
//   The window's read port is registered as well, which is a third place
//   a path could have formed and does not: the byte a copy produces is
//   one cycle behind the address that asked for it, by construction,
//   because a block RAM has no other shape.
//
// The output asymmetry, and stalling in the middle of a copy
//   RFC 1951 §3.2.5's longest match is **258 bytes**, and the code for it
//   is as few as 25 bits. So a handful of input bits can owe the consumer
//   258 output bytes, and a decompressor has two ways to be honest about
//   that: keep 258 bytes of output headroom, or stop in the middle of the
//   copy.
//
//   This one stops. `S_COPY` holds `rem_q`, the bytes of the match still
//   to come, and a window read pointer, and it produces one byte a cycle
//   for as long as the output slot is free. When the consumer drops
//   `out_ready` and the slot fills, the copy simply does not advance: no
//   read is issued, no byte is written, and `rem_q` does not move. It
//   resumes on the cycle the slot frees. The whole cost of that is a
//   nine-bit counter and a pointer, against 258 bytes of buffer — which
//   on this part is two block RAMs and a second set of pointers — so the
//   choice is not close.
//
//   **What the input side does while a copy is stalled**: it stops too,
//   and the producer finds out within one cycle. A stalled copy takes no
//   bits, so the bit reader's one-byte slack slot stays full, so
//   `in_ready` is low on the next edge. Nothing is dropped, because a
//   byte only moves on a cycle where both sides agree. The block holds at
//   most one buffered byte and eight buffered bits while stalled, and
//   that is the entire amount of input in flight anywhere in it.
//
// Huffman decoding: sequential, and why
//   One bit a cycle, over the canonical-code walk that `zlib`'s own
//   `puff.c` uses: keep the first code of the current length and the
//   number of codes of it, and compare. That is the *small* answer — one
//   16-bit comparator, three counters and two tables — against the fast
//   answer, which is a lookup table indexed by the next nine or so bits
//   and rebuilt whenever a dynamic block changes the code.
//
//   It was chosen because **dynamic blocks are the common case** and the
//   expensive half of a table-driven decoder is not the lookup but the
//   build: a 512-entry table filled in from the code lengths before the
//   first symbol of every block. The sequential decoder needs the code
//   lengths sorted by length, which is a counting sort, which is also
//   exactly what a table build needs — so the sequential decoder is the
//   table build and then nothing else. README §8 prices the upgrade.
//
//   The counting sort is in `S_BZERO`, `S_BCNT`, `S_BSUM` and `S_BPUT`,
//   and runs three times per dynamic block: once for RFC 1951 §3.2.7's
//   code-length code, once for the literal/length code and once for the
//   distance code. A fixed block fills the code lengths from §3.2.6 and
//   runs the same sort, and `fixed_q` remembers that it did, so a run of
//   fixed blocks pays for it once.
//
//   `lcnt`/`dcnt` hold the **counts** during the sort and the **running
//   offsets** afterwards, in the same sixteen words. That is worth a
//   sentence because it is not obvious: the sort needs counts, then a
//   prefix sum of them, then a write pointer per length, and the decoder
//   needs the counts back. Writing the prefix sums over the counts leaves
//   `cnt[len]` holding the number of symbols of length `len` *or less*,
//   from which the decoder recovers the count as `cnt[len] - cnt[len-1]`
//   and the symbol index as `cnt[len-1]` — and its walk is over
//   increasing `len`, so it already has `cnt[len-1]` in a register. One
//   file of sixteen words instead of two, and no third pass.
//
// Narrow registers
//   `CLAUDE.md` says never to declare a register wider than the values it
//   holds, and this block has a lot of nearly-wide ones, so each is sized
//   against a documented bound:
//
//   - `rem_q` is 9 bits: §3.2.5's longest match is 258.
//   - `dist_q` is **16** bits, which is wider than any window below the
//     maximum needs, on purpose: §3.2.5's longest distance is 32768, and
//     a narrower register could not tell a legal distance this instance
//     cannot reach from one no stream may name. Reporting that difference
//     is the point.
//   - `first_q` is 16 bits. It is the smallest code of the current
//     length, doubled each time the walk gets longer, and a code set that
//     passes the Kraft check of `S_BSUM` keeps it at or under 32768.
//   - `left_q` is 16 bits for the same bound, which is `2**15` reached
//     only by a code with nothing in it.
//   - `nlen_q` is 9 bits (257..288), `ndist_q` 6 (1..32), `ncode_q` 5
//     (4..19), `total_q` 9 (258..320), `rep_q` 8 (0..138) — all straight
//     out of §3.2.7.
//   - `state_q` is 5 bits for 29 states, and every unused encoding falls
//     to the `default` arm, which reports rather than wanders.
//
// What it does not do
//   No gzip. RFC 1952's header is variable-length — an optional file
//   name, an optional comment, an optional extra field, an optional
//   header check — and its trailer is a CRC-32, which is a different
//   algorithm with its own table. None of that is decompression; it is a
//   container parser, and it belongs above this block rather than inside
//   it. README §2 says the same about why zlib *is* here.
//
//   No preset dictionary. RFC 1950 §2.2's FDICT asks for the window to
//   start loaded from an agreed string; a header that sets it is reported
//   as `E_HEADER` rather than decoded against an empty window.
//
//   No compression. README §9 says what a compressor needs that this does
//   not have, and why it is a separate round.
//
//   No stream concatenation and no trailing-garbage tolerance: when the
//   final block ends, this block is done, and whatever follows the
//   checksum is the caller's business.
//
//   No bit-exact resume. `start` abandons whatever was in progress and
//   discards the buffered byte with it, so a caller restarting a stream
//   must re-send from the beginning. Hold `in_valid` low in the cycle
//   `start` is high, for the reason `chacha20` gives: `in_ready` is a
//   function of registers and so can be high while the restart throws the
//   byte away.
module inflate #(
    // The sliding window, as a power of two. 15 is RFC 1951 §3.2.5's
    // largest distance, 32768, and so the only setting that decodes every
    // legal stream; 262 144 bits is sixteen `DP16KD` on an ECP5. Lower
    // settings are for designs that know their streams were compressed
    // with a smaller window, and report `E_DIST` for a distance they
    // cannot reach. See `inflate_window.v` and README §5.
    parameter WINDOW_BITS = 15,
    // 1 parses RFC 1950's two-byte header and checks its trailing
    // Adler-32. 0 decodes a raw RFC 1951 stream, which is what a
    // container that carries its own checksum (gzip, zip, PNG) wants.
    parameter WRAPPER     = 1
) (
    input  wire        clk,
    input  wire        rst_n,

    // Abandon whatever is in progress and begin a new stream. Not a
    // required step: out of reset the block is already at the start of
    // one.
    input  wire        start,

    input  wire [7:0]  in_byte,
    input  wire        in_valid,
    // The compressed stream ends at this point. Like `sha256`'s and not
    // like AXI4-Stream's `tlast`: `in_valid` says whether there is a byte
    // here, so `in_last` alone with `in_valid` low ends a stream with no
    // byte at this point, which is the only way to say "the stream is
    // empty". Hold it until `in_ready` is high. A stream that needs more
    // bits after `in_last` reports `E_TRUNC`.
    input  wire        in_last,
    // A function of registers only. See the header.
    output wire        in_ready,

    output wire [7:0]  out_byte,
    output wire        out_valid,
    input  wire        out_ready,

    // The stream finished and, with `WRAPPER`, its checksum matched. Goes
    // high only after the last output byte has been taken.
    output wire        done,
    // Latched. `error_code` says which; both hold until `start`.
    output wire        error,
    output wire [2:0]  error_code,
    // Neither finished nor failed.
    output wire        busy
);
    // -----------------------------------------------------------------
    // Errors
    // -----------------------------------------------------------------
    localparam [2:0] E_NONE   = 3'd0;
    // RFC 1950 §2.2: CM is not 8, CINFO is over 7, FDICT asks for a
    // preset dictionary, or CMF*256+FLG is not a multiple of 31.
    localparam [2:0] E_HEADER = 3'd1;
    // RFC 1951 §3.2.3: BTYPE 11, "reserved (error)".
    localparam [2:0] E_BTYPE  = 3'd2;
    // RFC 1951 §3.2.4: NLEN is not the one's complement of LEN.
    localparam [2:0] E_NLEN   = 3'd3;
    // RFC 1951 §3.2.2, §3.2.6, §3.2.7: a code set that is over- or
    // under-subscribed, fifteen bits that match no code, a code-length
    // run that has nothing to repeat or that runs off the end, or a
    // symbol the tables of §3.2.5 do not define.
    localparam [2:0] E_CODE   = 3'd4;
    // RFC 1951 §3.2.5: a distance of zero, or one reaching further back
    // than this stream has produced or this window holds.
    localparam [2:0] E_DIST   = 3'd5;
    // `in_last` arrived while more bits were needed.
    localparam [2:0] E_TRUNC  = 3'd6;
    // RFC 1950 §9: the trailing checksum did not match.
    localparam [2:0] E_ADLER  = 3'd7;

    // -----------------------------------------------------------------
    // States. 29 of them, 5 bits, `default` reports.
    // -----------------------------------------------------------------
    localparam [4:0] S_ZHDR    = 5'd0;   // RFC 1950 §2.2's two bytes
    localparam [4:0] S_BITS    = 5'd1;   // gather `nbit_q` bits into `acc_q`
    localparam [4:0] S_BLKDO   = 5'd2;   // RFC 1951 §3.2.3's BFINAL/BTYPE
    localparam [4:0] S_SALIGN  = 5'd3;   // §3.2.4: drop to a byte boundary
    localparam [4:0] S_SLEN    = 5'd4;
    localparam [4:0] S_SNLEN   = 5'd5;
    localparam [4:0] S_SDATA   = 5'd6;
    localparam [4:0] S_FIXFILL = 5'd7;   // §3.2.6's code lengths
    localparam [4:0] S_DHDRDO  = 5'd8;   // §3.2.7's HLIT/HDIST/HCLEN
    localparam [4:0] S_DCLZ    = 5'd9;
    localparam [4:0] S_DCLDO   = 5'd10;
    localparam [4:0] S_DREPDO  = 5'd11;
    localparam [4:0] S_DFILL   = 5'd12;
    localparam [4:0] S_BZERO   = 5'd13;  // the counting sort
    localparam [4:0] S_BCNT    = 5'd14;
    localparam [4:0] S_BSUM    = 5'd15;
    localparam [4:0] S_BPUT    = 5'd16;
    localparam [4:0] S_BNEXT   = 5'd17;
    localparam [4:0] S_DEC     = 5'd18;  // one Huffman symbol, a bit a cycle
    localparam [4:0] S_DECDO   = 5'd19;
    localparam [4:0] S_LIT     = 5'd20;
    localparam [4:0] S_LEXDO   = 5'd21;
    localparam [4:0] S_DEXDO   = 5'd22;
    localparam [4:0] S_COPEN   = 5'd23;
    localparam [4:0] S_COPY    = 5'd24;
    localparam [4:0] S_EALIGN  = 5'd25;
    localparam [4:0] S_ADLER   = 5'd26;
    localparam [4:0] S_DONE    = 5'd27;
    localparam [4:0] S_ERR     = 5'd28;

    // Which table the decoder and the sort are working on.
    localparam [1:0] T_CL   = 2'd0;  // §3.2.7's code-length code
    localparam [1:0] T_LIT  = 2'd1;  // the literal/length code
    localparam [1:0] T_DIST = 2'd2;  // the distance code

    // -----------------------------------------------------------------
    // The tables of RFC 1951 §3.2.5, §3.2.6 and §3.2.7, as logic
    // -----------------------------------------------------------------

    // §3.2.5's length table, indexed by symbol - 257.
    function [8:0] len_base;
        input [4:0] s;
        case (s)
            5'd0:  len_base = 9'd3;    5'd1:  len_base = 9'd4;
            5'd2:  len_base = 9'd5;    5'd3:  len_base = 9'd6;
            5'd4:  len_base = 9'd7;    5'd5:  len_base = 9'd8;
            5'd6:  len_base = 9'd9;    5'd7:  len_base = 9'd10;
            5'd8:  len_base = 9'd11;   5'd9:  len_base = 9'd13;
            5'd10: len_base = 9'd15;   5'd11: len_base = 9'd17;
            5'd12: len_base = 9'd19;   5'd13: len_base = 9'd23;
            5'd14: len_base = 9'd27;   5'd15: len_base = 9'd31;
            5'd16: len_base = 9'd35;   5'd17: len_base = 9'd43;
            5'd18: len_base = 9'd51;   5'd19: len_base = 9'd59;
            5'd20: len_base = 9'd67;   5'd21: len_base = 9'd83;
            5'd22: len_base = 9'd99;   5'd23: len_base = 9'd115;
            5'd24: len_base = 9'd131;  5'd25: len_base = 9'd163;
            5'd26: len_base = 9'd195;  5'd27: len_base = 9'd227;
            5'd28: len_base = 9'd258;
            default: len_base = 9'd0;
        endcase
    endfunction

    function [2:0] len_extra;
        input [4:0] s;
        case (s)
            5'd0, 5'd1, 5'd2, 5'd3, 5'd4, 5'd5, 5'd6, 5'd7:
                len_extra = 3'd0;
            5'd8, 5'd9, 5'd10, 5'd11:   len_extra = 3'd1;
            5'd12, 5'd13, 5'd14, 5'd15: len_extra = 3'd2;
            5'd16, 5'd17, 5'd18, 5'd19: len_extra = 3'd3;
            5'd20, 5'd21, 5'd22, 5'd23: len_extra = 3'd4;
            5'd24, 5'd25, 5'd26, 5'd27: len_extra = 3'd5;
            default: len_extra = 3'd0;  // 28 is the bare 258
        endcase
    endfunction

    // §3.2.5's distance table. 24577 is the largest base, so 15 bits.
    function [14:0] dist_base;
        input [4:0] s;
        case (s)
            5'd0:  dist_base = 15'd1;      5'd1:  dist_base = 15'd2;
            5'd2:  dist_base = 15'd3;      5'd3:  dist_base = 15'd4;
            5'd4:  dist_base = 15'd5;      5'd5:  dist_base = 15'd7;
            5'd6:  dist_base = 15'd9;      5'd7:  dist_base = 15'd13;
            5'd8:  dist_base = 15'd17;     5'd9:  dist_base = 15'd25;
            5'd10: dist_base = 15'd33;     5'd11: dist_base = 15'd49;
            5'd12: dist_base = 15'd65;     5'd13: dist_base = 15'd97;
            5'd14: dist_base = 15'd129;    5'd15: dist_base = 15'd193;
            5'd16: dist_base = 15'd257;    5'd17: dist_base = 15'd385;
            5'd18: dist_base = 15'd513;    5'd19: dist_base = 15'd769;
            5'd20: dist_base = 15'd1025;   5'd21: dist_base = 15'd1537;
            5'd22: dist_base = 15'd2049;   5'd23: dist_base = 15'd3073;
            5'd24: dist_base = 15'd4097;   5'd25: dist_base = 15'd6145;
            5'd26: dist_base = 15'd8193;   5'd27: dist_base = 15'd12289;
            5'd28: dist_base = 15'd16385;  5'd29: dist_base = 15'd24577;
            default: dist_base = 15'd0;
        endcase
    endfunction

    function [3:0] dist_extra;
        input [4:0] s;
        case (s)
            5'd0, 5'd1, 5'd2, 5'd3:     dist_extra = 4'd0;
            5'd4, 5'd5:                 dist_extra = 4'd1;
            5'd6, 5'd7:                 dist_extra = 4'd2;
            5'd8, 5'd9:                 dist_extra = 4'd3;
            5'd10, 5'd11:               dist_extra = 4'd4;
            5'd12, 5'd13:               dist_extra = 4'd5;
            5'd14, 5'd15:               dist_extra = 4'd6;
            5'd16, 5'd17:               dist_extra = 4'd7;
            5'd18, 5'd19:               dist_extra = 4'd8;
            5'd20, 5'd21:               dist_extra = 4'd9;
            5'd22, 5'd23:               dist_extra = 4'd10;
            5'd24, 5'd25:               dist_extra = 4'd11;
            5'd26, 5'd27:               dist_extra = 4'd12;
            5'd28, 5'd29:               dist_extra = 4'd13;
            default: dist_extra = 4'd0;
        endcase
    endfunction

    // §3.2.7: "the code lengths for the code length alphabet are given
    // in the order" — the permutation that puts the common lengths first.
    function [4:0] cl_order;
        input [4:0] i;
        case (i)
            5'd0:  cl_order = 5'd16;  5'd1:  cl_order = 5'd17;
            5'd2:  cl_order = 5'd18;  5'd3:  cl_order = 5'd0;
            5'd4:  cl_order = 5'd8;   5'd5:  cl_order = 5'd7;
            5'd6:  cl_order = 5'd9;   5'd7:  cl_order = 5'd6;
            5'd8:  cl_order = 5'd10;  5'd9:  cl_order = 5'd5;
            5'd10: cl_order = 5'd11;  5'd11: cl_order = 5'd4;
            5'd12: cl_order = 5'd12;  5'd13: cl_order = 5'd3;
            5'd14: cl_order = 5'd13;  5'd15: cl_order = 5'd2;
            5'd16: cl_order = 5'd14;  5'd17: cl_order = 5'd1;
            5'd18: cl_order = 5'd15;
            default: cl_order = 5'd0;
        endcase
    endfunction

    // §3.2.6's fixed code, as code lengths: literal/length first, then
    // the 32 five-bit distance codes at 288. 32 and not 30, which is the
    // choice `zlib`'s own `fixedtables()` makes and which makes the code
    // complete; symbols 30 and 31 then decode and are reported as
    // `E_CODE` at the point of use, which §3.2.6 says "will never
    // actually occur".
    function [3:0] fixed_len;
        input [8:0] i;
        if (i < 9'd144)      fixed_len = 4'd8;
        else if (i < 9'd256) fixed_len = 4'd9;
        else if (i < 9'd280) fixed_len = 4'd7;
        else if (i < 9'd288) fixed_len = 4'd8;
        else                 fixed_len = 4'd5;
    endfunction

    localparam [8:0] FIXED_NLEN  = 9'd288;
    localparam [8:0] FIXED_TOTAL = 9'd320;

    // -----------------------------------------------------------------
    // The bit reader: one byte of slack, one bit or one whole byte a
    // cycle
    // -----------------------------------------------------------------
    reg [7:0] cur_q;      // the byte being consumed, shifted right
    reg [3:0] cnt_q;      // bits of it left, 0..8
    reg [7:0] nxt_q;      // the slack slot
    reg       nxt_full_q;
    reg       last_q;     // `in_last` has been taken

    // -----------------------------------------------------------------
    // Stream state
    // -----------------------------------------------------------------
    reg [4:0]  state_q;
    reg [4:0]  ret_q;     // where `S_BITS` goes when it is done
    reg        err_q;
    reg [2:0]  ecode_q;
    reg        done_q;
    reg        bfinal_q;

    reg [15:0] acc_q;     // the bit and byte accumulator
    reg [3:0]  pos_q;     // next bit position in `acc_q`
    reg [4:0]  nbit_q;    // bits still wanted, 1..16
    reg [1:0]  tix_q;     // a two-bit sub-counter: header and trailer bytes

    reg [7:0]  cmf_q;     // RFC 1950 §2.2's first byte
    reg [15:0] len_q;     // §3.2.4's LEN, then the bytes of it left

    reg [8:0]  nlen_q;    // §3.2.7: HLIT + 257
    reg [5:0]  ndist_q;   // HDIST + 1
    reg [4:0]  ncode_q;   // HCLEN + 4
    reg [8:0]  total_q;   // nlen + ndist
    reg [8:0]  lix_q;     // where the next code length goes
    reg [3:0]  prev_len_q;
    reg [7:0]  rep_q;     // §3.2.7's run length, 0..138
    reg [3:0]  rep_val_q;
    reg        rep18_q;   // the run is code 18, whose base is 11

    reg [1:0]  bphase_q;  // which table the sort is building
    reg [8:0]  bix_q;
    reg [8:0]  bn_q;
    reg [8:0]  bbase_q;
    reg [8:0]  bsym_q;    // the symbol `clen_q` belongs to
    reg        bpipe_q;   // `clen_q` holds something
    reg [3:0]  bl_q;      // the length being swept
    reg [15:0] left_q;    // §3.2.2's Kraft slack
    reg        over_q;    // it went negative: over-subscribed
    reg        only1_q;   // every code used is one bit long
    reg [8:0]  psum_q;

    reg [14:0] code_q;    // the bits of the code so far
    reg [15:0] first_q;   // the smallest code of this length
    reg [8:0]  prev_q;    // cnt[len-1]: the symbol index of this length
    reg [3:0]  dlen_q;    // the length being tried, 1..15
    reg [1:0]  dret_q;    // which table, and so what the symbol means

    // Not reset, and they must not be: a block RAM has no reset pin, and
    // this flow builds one only from a bare `q <= mem[a]` with a single
    // driver. Nothing reads either before `dec_hit` has loaded it.
    reg [8:0]  lsym_q;
    reg [4:0]  dsym_q;
    reg [4:0]  xcode_q;   // the length or distance symbol being extended
    reg [8:0]  rem_q;     // a match's bytes still to copy, 1..258
    reg [15:0] dist_q;
    reg [7:0]  lit_q;     // a literal waiting for the output slot
    reg        fixed_q;   // the tables in hand are §3.2.6's

    reg [7:0]  out_q;
    reg        ov_q;

    // -----------------------------------------------------------------
    // Memories
    // -----------------------------------------------------------------
    // §3.2.7's code lengths, literal/length then distance, contiguous
    // because the run-length codes may cross the boundary between them.
    reg [3:0] clen [0:319];
    reg [3:0] clen_q;
    // The symbols of the literal/length code sorted by code length, and
    // of the distance code. The first also holds §3.2.7's nineteen
    // code-length symbols while they are needed, which is before the
    // literal/length code overwrites it.
    reg [8:0] lsym [0:287];
    reg [4:0] dsym [0:31];
    // Counts per code length while sorting, running offsets afterwards.
    // Word 0 is kept at zero so the decoder's `cnt[len-1]` is uniform at
    // length 1.
    reg [8:0] lcnt [0:15];
    reg [5:0] dcnt [0:15];

    // -----------------------------------------------------------------
    // Bit reader combinational
    // -----------------------------------------------------------------
    wire bit_avail  = (cnt_q != 4'd0);
    wire byte_avail = (cnt_q == 4'd8);
    wire cur_bit    = cur_q[0];

    wire out_free = !ov_q || out_ready;

    wire dec_lit  = (state_q == S_DECDO) && (dret_q == T_LIT);
    wire [8:0] symv = (dret_q == T_DIST) ? {4'd0, dsym_q} : lsym_q;
    // A length symbol is 257..285, and 256..287 all have the same high
    // bits, so the low five of `symv` are `symv - 256` and one less than
    // that is §3.2.5's table index.
    wire [4:0] lenx = symv[4:0] - 5'd1;

    wire put_lit    = dec_lit && (symv < 9'd256) && out_free;
    wire put_held   = (state_q == S_LIT) && out_free;
    wire put_stored = (state_q == S_SDATA) && (len_q != 16'd0) && byte_avail && out_free;

    wire        win_rd_valid;
    wire [7:0]  win_rd_data;
    wire        win_dist_bad;

    wire put_copy   = (state_q == S_COPY) && win_rd_valid && out_free;
    wire copy_issue = (state_q == S_COPY) && (rem_q != 9'd0)
                      && (!win_rd_valid || out_free);
    wire copy_last  = (state_q == S_COPY) && (rem_q == 9'd0)
                      && (!win_rd_valid || put_copy);

    wire emit = put_lit || put_held || put_stored || put_copy;
    wire [7:0] emit_byte = put_lit    ? symv[7:0]
                         : put_held   ? lit_q
                         : put_stored ? cur_q
                         :              win_rd_data;

    wire align = (state_q == S_SALIGN) || (state_q == S_EALIGN);

    wire take_bit = ((state_q == S_BITS) || (state_q == S_DEC)) && bit_avail;
    wire take_byte = byte_avail && ((state_q == S_ZHDR) || (state_q == S_SLEN)
                                 || (state_q == S_SNLEN) || (state_q == S_ADLER)
                                 || put_stored);

    // After this cycle's consumption. `align` keeps a byte that is
    // already whole and drops a partial one, which is §3.2.4's "skip any
    // remaining bits in current partially processed byte".
    wire [3:0] cnt_after = align     ? (byte_avail ? 4'd8 : 4'd0)
                         : take_byte ? 4'd0
                         : take_bit  ? (cnt_q - 4'd1)
                         :             cnt_q;
    wire refill = (cnt_after == 4'd0) && nxt_full_q;

    // Registers only: `nxt_full_q`, `last_q`, `done_q`, `err_q`.
    assign in_ready = !nxt_full_q && !last_q && !done_q && !err_q;
    wire   in_beat  = in_valid && in_ready;

    // What the stream still needs, and whether it can ever arrive. Every
    // byte-wide read is reached through an alignment or from the start of
    // the stream, so `byte_avail` is never waiting on a count stuck
    // between 1 and 7.
    wire need_bit  = (state_q == S_BITS) || (state_q == S_DEC);
    wire need_byte = (state_q == S_ZHDR) || (state_q == S_SLEN)
                  || (state_q == S_SNLEN) || (state_q == S_ADLER)
                  || ((state_q == S_SDATA) && (len_q != 16'd0));
    wire dry = !nxt_full_q && last_q;
    wire starved = (need_bit && !bit_avail && dry)
                || (need_byte && !byte_avail && dry);

    // -----------------------------------------------------------------
    // RFC 1950 §2.2's header check
    // -----------------------------------------------------------------
    // CMF*256 + FLG must be a multiple of 31. 32 is 1 modulo 31, so the
    // five-bit groups of the value sum to it modulo 31, twice folded.
    wire [15:0] zhv = {cmf_q, cur_q};
    wire [6:0]  zf1 = {2'd0, zhv[4:0]} + {2'd0, zhv[9:5]}
                    + {2'd0, zhv[14:10]} + {6'd0, zhv[15]};
    wire [5:0]  zf2 = {4'd0, zf1[6:5]} + {1'b0, zf1[4:0]};
    wire [5:0]  zf3 = {5'd0, zf2[5]} + {1'b0, zf2[4:0]};
    wire zhdr_ok = (cmf_q[3:0] == 4'd8)        // CM: deflate
                && (cmf_q[7:4] <= 4'd7)        // CINFO: window <= 32K
                && !cur_q[5]                   // FDICT: no preset dictionary
                && ((zf3 == 6'd0) || (zf3 == 6'd31));

    // -----------------------------------------------------------------
    // The counting sort and the decoder share one count file
    // -----------------------------------------------------------------
    wire sorting = (state_q == S_BZERO) || (state_q == S_BCNT)
                || (state_q == S_BSUM) || (state_q == S_BPUT);
    wire tbl_dist = sorting ? (bphase_q == T_DIST) : (dret_q == T_DIST);

    wire [3:0] cnt_addr = ((state_q == S_BCNT) || (state_q == S_BPUT)) ? clen_q
                        : ((state_q == S_BZERO) || (state_q == S_BSUM)) ? bl_q
                        : dlen_q;
    wire [8:0] lcnt_rd = lcnt[cnt_addr];
    wire [5:0] dcnt_rd = dcnt[cnt_addr];
    wire [8:0] cnt_rd  = tbl_dist ? {3'd0, dcnt_rd} : lcnt_rd;

    wire bputting = (state_q == S_BPUT) && bpipe_q && (clen_q != 4'd0);
    wire bcounting = (state_q == S_BCNT) && bpipe_q && (clen_q != 4'd0);
    wire [8:0] psum_next = psum_q + cnt_rd;

    wire cnt_we = (state_q == S_BZERO) || (state_q == S_BSUM)
               || bcounting || bputting;
    // `S_BSUM` leaves the *start* of each length's run of symbols, which
    // is the sum of the counts of every shorter length; `S_BPUT` then
    // advances it per symbol placed, so afterwards it is the start of the
    // next length. That is why the decoder's `cnt[len-1]` is the index
    // and `cnt[len] - cnt[len-1]` is the count.
    wire [8:0] cnt_wd = (state_q == S_BZERO) ? 9'd0
                      : (state_q == S_BSUM)  ? psum_q
                      :                        (cnt_rd + 9'd1);

    wire clen_rd = (state_q == S_BCNT) || (state_q == S_BPUT);
    wire [8:0] clen_raddr = bbase_q + bix_q;

    wire dfill_go = (state_q == S_DFILL) && (rep_q != 8'd0) && (lix_q != total_q);
    wire clen_we = (state_q == S_DCLZ) || (state_q == S_FIXFILL)
                || (state_q == S_DCLDO) || dfill_go;
    wire [8:0] clen_waddr = (state_q == S_DCLZ)    ? lix_q
                          : (state_q == S_FIXFILL) ? bix_q
                          : (state_q == S_DCLDO)   ? {4'd0, cl_order(lix_q[4:0])}
                          :                          lix_q;
    wire [3:0] clen_wd = (state_q == S_DCLZ)    ? 4'd0
                       : (state_q == S_FIXFILL) ? fixed_len(bix_q)
                       : (state_q == S_DCLDO)   ? {1'b0, acc_q[2:0]}
                       :                          rep_val_q;

    // -----------------------------------------------------------------
    // One Huffman symbol, a bit a cycle (zlib's `puff.c` walk)
    // -----------------------------------------------------------------
    wire [14:0] code_n  = {code_q[13:0], cur_bit};
    wire [8:0]  cnt_len = cnt_rd - prev_q;              // count[dlen]
    wire [15:0] limit   = first_q + {7'd0, cnt_len};
    wire [15:0] offset  = {1'b0, code_n} - first_q;
    wire [8:0]  sa      = prev_q + offset[8:0];
    wire dec_hit = (state_q == S_DEC) && bit_avail && ({1'b0, code_n} < limit);
    wire dec_run = (state_q == S_DEC) && bit_avail && !dec_hit;
    wire [15:0] first_n = (first_q + {7'd0, cnt_len}) << 1;

    // -----------------------------------------------------------------
    // The window and the checksum
    // -----------------------------------------------------------------
    wire [31:0] adler_sum;

    inflate_window #(
        .WINDOW_BITS (WINDOW_BITS)
    ) u_win (
        .clk       (clk),
        .rst_n     (rst_n),
        .start     (start),
        .wr_data   (emit_byte),
        .wr_en     (emit),
        .dist      (dist_q),
        .dist_bad  (win_dist_bad),
        .copy_open ((state_q == S_COPEN) && !win_dist_bad),
        .rd_en     (copy_issue),
        .rd_take   (put_copy),
        .rd_data   (win_rd_data),
        .rd_valid  (win_rd_valid)
    );

    generate
        if (WRAPPER != 0) begin : g_adler
            inflate_adler u_adler (
                .clk      (clk),
                .rst_n    (rst_n),
                .start    (start),
                .in_byte  (emit_byte),
                .in_valid (emit),
                .sum      (adler_sum)
            );
        end else begin : g_no_adler
            assign adler_sum = 32'd0;
        end
    endgenerate

    // The trailer, most significant byte first (RFC 1950 §9).
    wire [7:0] trail_want = (tix_q == 2'd0) ? adler_sum[31:24]
                          : (tix_q == 2'd1) ? adler_sum[23:16]
                          : (tix_q == 2'd2) ? adler_sum[15:8]
                          :                   adler_sum[7:0];

    assign out_byte   = out_q;
    assign out_valid  = ov_q;
    assign done       = done_q;
    assign error      = err_q;
    assign error_code = ecode_q;
    assign busy       = !done_q && !err_q;

    // -----------------------------------------------------------------
    // The memories. One write port and one read port each, written the
    // one way this flow builds a block RAM from: a bare `q <= mem[a]`
    // with no reset and one consumer.
    // -----------------------------------------------------------------
    always @(posedge clk) begin
        if (clen_we) clen[clen_waddr] <= clen_wd;
        if (clen_rd) clen_q <= clen[clen_raddr];
        if (bputting && !tbl_dist) lsym[lcnt_rd] <= bsym_q;
        if (bputting && tbl_dist) dsym[dcnt_rd[4:0]] <= bsym_q[4:0];
        if (dec_hit) lsym_q <= lsym[sa];
        if (dec_hit) dsym_q <= dsym[sa[4:0]];
        if (cnt_we && !tbl_dist) lcnt[cnt_addr] <= cnt_wd;
        if (cnt_we && tbl_dist) dcnt[cnt_addr] <= cnt_wd[5:0];
    end

    // -----------------------------------------------------------------
    // Everything else
    // -----------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            cur_q      <= 8'd0;
            cnt_q      <= 4'd0;
            nxt_q      <= 8'd0;
            nxt_full_q <= 1'b0;
            last_q     <= 1'b0;
            state_q    <= (WRAPPER != 0) ? S_ZHDR : S_BITS;
            ret_q      <= S_BLKDO;
            err_q      <= 1'b0;
            ecode_q    <= E_NONE;
            done_q     <= 1'b0;
            bfinal_q   <= 1'b0;
            acc_q      <= 16'd0;
            pos_q      <= 4'd0;
            nbit_q     <= 5'd3;
            tix_q      <= 2'd0;
            cmf_q      <= 8'd0;
            len_q      <= 16'd0;
            nlen_q     <= 9'd0;
            ndist_q    <= 6'd0;
            ncode_q    <= 5'd0;
            total_q    <= 9'd0;
            lix_q      <= 9'd0;
            prev_len_q <= 4'd0;
            rep_q      <= 8'd0;
            rep_val_q  <= 4'd0;
            rep18_q    <= 1'b0;
            bphase_q   <= T_CL;
            bix_q      <= 9'd0;
            bn_q       <= 9'd0;
            bbase_q    <= 9'd0;
            bsym_q     <= 9'd0;
            bpipe_q    <= 1'b0;
            bl_q       <= 4'd0;
            left_q     <= 16'd1;
            over_q     <= 1'b0;
            only1_q    <= 1'b1;
            psum_q     <= 9'd0;
            code_q     <= 15'd0;
            first_q    <= 16'd0;
            prev_q     <= 9'd0;
            dlen_q     <= 4'd1;
            dret_q     <= T_CL;
            xcode_q    <= 5'd0;
            rem_q      <= 9'd0;
            dist_q     <= 16'd0;
            lit_q      <= 8'd0;
            fixed_q    <= 1'b0;
            out_q      <= 8'd0;
            ov_q       <= 1'b0;
        end else if (start) begin
            cur_q      <= 8'd0;
            cnt_q      <= 4'd0;
            nxt_full_q <= 1'b0;
            last_q     <= 1'b0;
            state_q    <= (WRAPPER != 0) ? S_ZHDR : S_BITS;
            ret_q      <= S_BLKDO;
            err_q      <= 1'b0;
            ecode_q    <= E_NONE;
            done_q     <= 1'b0;
            bfinal_q   <= 1'b0;
            acc_q      <= 16'd0;
            pos_q      <= 4'd0;
            nbit_q     <= 5'd3;
            tix_q      <= 2'd0;
            fixed_q    <= 1'b0;
            bpipe_q    <= 1'b0;
            ov_q       <= 1'b0;
        end else begin
            // ---------------------------------------------------------
            // The bit reader, and the one byte of slack in front of it
            // ---------------------------------------------------------
            if (refill) begin
                cur_q      <= nxt_q;
                cnt_q      <= 4'd8;
                nxt_full_q <= 1'b0;
            end else begin
                cnt_q <= cnt_after;
                if (take_bit) cur_q <= {1'b0, cur_q[7:1]};
            end
            if (in_beat) begin
                nxt_q      <= in_byte;
                nxt_full_q <= 1'b1;
            end
            if (in_ready && in_last) last_q <= 1'b1;

            // ---------------------------------------------------------
            // The output slot
            // ---------------------------------------------------------
            if (ov_q && out_ready) ov_q <= 1'b0;
            if (emit) begin
                out_q <= emit_byte;
                ov_q  <= 1'b1;
            end

            // ---------------------------------------------------------
            // The state machine
            // ---------------------------------------------------------
            if (starved) begin
                err_q   <= 1'b1;
                ecode_q <= E_TRUNC;
                state_q <= S_ERR;
            end else begin
                case (state_q)
                    // RFC 1950 §2.2.
                    S_ZHDR: if (byte_avail) begin
                        if (tix_q == 2'd0) begin
                            cmf_q <= cur_q;
                            tix_q <= 2'd1;
                        end else if (zhdr_ok) begin
                            tix_q   <= 2'd0;
                            acc_q   <= 16'd0;
                            pos_q   <= 4'd0;
                            nbit_q  <= 5'd3;
                            ret_q   <= S_BLKDO;
                            state_q <= S_BITS;
                        end else begin
                            err_q   <= 1'b1;
                            ecode_q <= E_HEADER;
                            state_q <= S_ERR;
                        end
                    end

                    // Gather `nbit_q` bits, least significant first,
                    // which is RFC 1951 §3.1.1's order for everything
                    // that is not a Huffman code.
                    S_BITS: if (bit_avail) begin
                        if (cur_bit) acc_q <= acc_q | (16'd1 << pos_q);
                        pos_q  <= pos_q + 4'd1;
                        nbit_q <= nbit_q - 5'd1;
                        if (nbit_q == 5'd1) state_q <= ret_q;
                    end

                    // RFC 1951 §3.2.3.
                    S_BLKDO: begin
                        bfinal_q <= acc_q[0];
                        case (acc_q[2:1])
                            2'd0: state_q <= S_SALIGN;
                            2'd1: begin
                                if (fixed_q) begin
                                    code_q  <= 15'd0;
                                    first_q <= 16'd0;
                                    prev_q  <= 9'd0;
                                    dlen_q  <= 4'd1;
                                    dret_q  <= T_LIT;
                                    state_q <= S_DEC;
                                end else begin
                                    bix_q   <= 9'd0;
                                    state_q <= S_FIXFILL;
                                end
                            end
                            2'd2: begin
                                acc_q   <= 16'd0;
                                pos_q   <= 4'd0;
                                nbit_q  <= 5'd14;
                                ret_q   <= S_DHDRDO;
                                state_q <= S_BITS;
                            end
                            default: begin
                                err_q   <= 1'b1;
                                ecode_q <= E_BTYPE;
                                state_q <= S_ERR;
                            end
                        endcase
                    end

                    // RFC 1951 §3.2.4.
                    S_SALIGN: begin
                        tix_q   <= 2'd0;
                        acc_q   <= 16'd0;
                        state_q <= S_SLEN;
                    end

                    S_SLEN: if (byte_avail) begin
                        acc_q <= {cur_q, acc_q[15:8]};
                        if (tix_q == 2'd1) begin
                            tix_q   <= 2'd0;
                            len_q   <= {cur_q, acc_q[15:8]};
                            state_q <= S_SNLEN;
                        end else begin
                            tix_q <= 2'd1;
                        end
                    end

                    S_SNLEN: if (byte_avail) begin
                        acc_q <= {cur_q, acc_q[15:8]};
                        if (tix_q == 2'd1) begin
                            tix_q <= 2'd0;
                            if ({cur_q, acc_q[15:8]} == ~len_q) begin
                                state_q <= S_SDATA;
                            end else begin
                                err_q   <= 1'b1;
                                ecode_q <= E_NLEN;
                                state_q <= S_ERR;
                            end
                        end else begin
                            tix_q <= 2'd1;
                        end
                    end

                    S_SDATA: begin
                        if (len_q == 16'd0) begin
                            if (bfinal_q) begin
                                tix_q   <= 2'd0;
                                state_q <= (WRAPPER != 0) ? S_EALIGN : S_DONE;
                            end else begin
                                acc_q   <= 16'd0;
                                pos_q   <= 4'd0;
                                nbit_q  <= 5'd3;
                                ret_q   <= S_BLKDO;
                                state_q <= S_BITS;
                            end
                        end else if (put_stored) begin
                            len_q <= len_q - 16'd1;
                        end
                    end

                    // RFC 1951 §3.2.6, written out as code lengths so
                    // that one sort serves both block types.
                    S_FIXFILL: begin
                        bix_q <= bix_q + 9'd1;
                        if (bix_q == FIXED_TOTAL - 9'd1) begin
                            nlen_q   <= FIXED_NLEN;
                            ndist_q  <= 6'd32;
                            total_q  <= FIXED_TOTAL;
                            fixed_q  <= 1'b1;
                            bphase_q <= T_LIT;
                            bbase_q  <= 9'd0;
                            bn_q     <= FIXED_NLEN;
                            bl_q     <= 4'd0;
                            state_q  <= S_BZERO;
                        end
                    end

                    // RFC 1951 §3.2.7.
                    S_DHDRDO: begin
                        nlen_q  <= {4'd0, acc_q[4:0]} + 9'd257;
                        ndist_q <= {1'b0, acc_q[9:5]} + 6'd1;
                        ncode_q <= acc_q[13:10] + 5'd4;
                        total_q <= ({4'd0, acc_q[4:0]} + 9'd257)
                                 + ({4'd0, acc_q[9:5]} + 9'd1);
                        fixed_q <= 1'b0;
                        lix_q   <= 9'd0;
                        state_q <= S_DCLZ;
                    end

                    // The nineteen code-length code lengths start at
                    // zero, because HCLEN may name fewer than all of
                    // them.
                    S_DCLZ: begin
                        lix_q <= lix_q + 9'd1;
                        if (lix_q == 9'd18) begin
                            lix_q   <= 9'd0;
                            acc_q   <= 16'd0;
                            pos_q   <= 4'd0;
                            nbit_q  <= 5'd3;
                            ret_q   <= S_DCLDO;
                            state_q <= S_BITS;
                        end
                    end

                    S_DCLDO: begin
                        lix_q <= lix_q + 9'd1;
                        if (lix_q + 9'd1 == {4'd0, ncode_q}) begin
                            bphase_q <= T_CL;
                            bbase_q  <= 9'd0;
                            bn_q     <= 9'd19;
                            bl_q     <= 4'd0;
                            state_q  <= S_BZERO;
                        end else begin
                            acc_q   <= 16'd0;
                            pos_q   <= 4'd0;
                            nbit_q  <= 5'd3;
                            ret_q   <= S_DCLDO;
                            state_q <= S_BITS;
                        end
                    end

                    S_DREPDO: begin
                        rep_q   <= (rep18_q ? 8'd11 : 8'd3) + {1'b0, acc_q[6:0]};
                        state_q <= S_DFILL;
                    end

                    S_DFILL: begin
                        if (rep_q == 8'd0) begin
                            if (lix_q == total_q) begin
                                bphase_q <= T_LIT;
                                bbase_q  <= 9'd0;
                                bn_q     <= nlen_q;
                                bl_q     <= 4'd0;
                                state_q  <= S_BZERO;
                            end else begin
                                code_q  <= 15'd0;
                                first_q <= 16'd0;
                                prev_q  <= 9'd0;
                                dlen_q  <= 4'd1;
                                dret_q  <= T_CL;
                                state_q <= S_DEC;
                            end
                        end else if (lix_q == total_q) begin
                            // §3.2.7's run would write past the last
                            // code length the header declared.
                            err_q   <= 1'b1;
                            ecode_q <= E_CODE;
                            state_q <= S_ERR;
                        end else begin
                            lix_q <= lix_q + 9'd1;
                            rep_q <= rep_q - 8'd1;
                        end
                    end

                    // The counting sort: clear, count, prefix-sum with
                    // the Kraft check, place.
                    S_BZERO: begin
                        bl_q <= bl_q + 4'd1;
                        if (bl_q == 4'd15) begin
                            bix_q   <= 9'd0;
                            bpipe_q <= 1'b0;
                            only1_q <= 1'b1;
                            state_q <= S_BCNT;
                        end
                    end

                    S_BCNT: begin
                        if (bix_q != bn_q) begin
                            bix_q   <= bix_q + 9'd1;
                            bsym_q  <= bix_q;
                            bpipe_q <= 1'b1;
                        end else begin
                            bpipe_q <= 1'b0;
                        end
                        if (bpipe_q && (clen_q > 4'd1)) only1_q <= 1'b0;
                        if (!bpipe_q && (bix_q == bn_q)) begin
                            bl_q    <= 4'd1;
                            left_q  <= 16'd1;
                            over_q  <= 1'b0;
                            psum_q  <= 9'd0;
                            state_q <= S_BSUM;
                        end
                    end

                    // RFC 1951 §3.2.2's canonical code must be neither
                    // over- nor under-subscribed. `left` starts at one
                    // and doubles per length, less the codes used; a
                    // borrow means over-subscribed and a non-zero
                    // remainder means incomplete.
                    S_BSUM: begin
                        psum_q <= psum_next;
                        if ({left_q, 1'b0} < {8'd0, cnt_rd}) over_q <= 1'b1;
                        left_q <= {left_q[14:0], 1'b0} - {7'd0, cnt_rd};
                        bl_q   <= bl_q + 4'd1;
                        if (bl_q == 4'd15) begin
                            bix_q   <= 9'd0;
                            bpipe_q <= 1'b0;
                            state_q <= S_BPUT;
                        end
                    end

                    S_BPUT: begin
                        if (bix_q != bn_q) begin
                            bix_q   <= bix_q + 9'd1;
                            bsym_q  <= bix_q;
                            bpipe_q <= 1'b1;
                        end else begin
                            bpipe_q <= 1'b0;
                        end
                        if (!bpipe_q && (bix_q == bn_q)) state_q <= S_BNEXT;
                    end

                    S_BNEXT: begin
                        // The verdict on the code set. An incomplete one
                        // is refused, except that the literal/length and
                        // distance codes may have a single one-bit code,
                        // which is what a block with one distance or no
                        // matches at all produces and what `zlib` itself
                        // allows. §3.2.2 does not say; this follows the
                        // reference implementation.
                        if (over_q || ((left_q != 16'd0)
                                       && !(only1_q && (bphase_q != T_CL)))) begin
                            err_q   <= 1'b1;
                            ecode_q <= E_CODE;
                            state_q <= S_ERR;
                        end else begin
                            case (bphase_q)
                                T_CL: begin
                                    lix_q      <= 9'd0;
                                    prev_len_q <= 4'd0;
                                    code_q     <= 15'd0;
                                    first_q    <= 16'd0;
                                    prev_q     <= 9'd0;
                                    dlen_q     <= 4'd1;
                                    dret_q     <= T_CL;
                                    state_q    <= S_DEC;
                                end
                                T_LIT: begin
                                    bphase_q <= T_DIST;
                                    bbase_q  <= nlen_q;
                                    bn_q     <= {3'd0, ndist_q};
                                    bl_q     <= 4'd0;
                                    state_q  <= S_BZERO;
                                end
                                default: begin
                                    code_q  <= 15'd0;
                                    first_q <= 16'd0;
                                    prev_q  <= 9'd0;
                                    dlen_q  <= 4'd1;
                                    dret_q  <= T_LIT;
                                    state_q <= S_DEC;
                                end
                            endcase
                        end
                    end

                    S_DEC: if (bit_avail) begin
                        if (dec_hit) begin
                            state_q <= S_DECDO;
                        end else if (dlen_q == 4'd15) begin
                            // Fifteen bits and no code: §3.2.7 caps a
                            // code length at 15, so there is nothing
                            // left to try.
                            err_q   <= 1'b1;
                            ecode_q <= E_CODE;
                            state_q <= S_ERR;
                        end else begin
                            code_q  <= code_n;
                            first_q <= first_n;
                            prev_q  <= cnt_rd;
                            dlen_q  <= dlen_q + 4'd1;
                        end
                    end

                    S_DECDO: begin
                        case (dret_q)
                            // §3.2.7's code-length alphabet.
                            T_CL: begin
                                if (symv < 9'd16) begin
                                    rep_val_q  <= symv[3:0];
                                    prev_len_q <= symv[3:0];
                                    rep_q      <= 8'd1;
                                    state_q    <= S_DFILL;
                                end else if (symv == 9'd16) begin
                                    if (lix_q == 9'd0) begin
                                        err_q   <= 1'b1;
                                        ecode_q <= E_CODE;
                                        state_q <= S_ERR;
                                    end else begin
                                        rep_val_q <= prev_len_q;
                                        rep18_q   <= 1'b0;
                                        acc_q     <= 16'd0;
                                        pos_q     <= 4'd0;
                                        nbit_q    <= 5'd2;
                                        ret_q     <= S_DREPDO;
                                        state_q   <= S_BITS;
                                    end
                                end else if (symv == 9'd17) begin
                                    rep_val_q <= 4'd0;
                                    rep18_q   <= 1'b0;
                                    acc_q     <= 16'd0;
                                    pos_q     <= 4'd0;
                                    nbit_q    <= 5'd3;
                                    ret_q     <= S_DREPDO;
                                    state_q   <= S_BITS;
                                end else if (symv == 9'd18) begin
                                    rep_val_q <= 4'd0;
                                    rep18_q   <= 1'b1;
                                    acc_q     <= 16'd0;
                                    pos_q     <= 4'd0;
                                    nbit_q    <= 5'd7;
                                    ret_q     <= S_DREPDO;
                                    state_q   <= S_BITS;
                                end else begin
                                    err_q   <= 1'b1;
                                    ecode_q <= E_CODE;
                                    state_q <= S_ERR;
                                end
                            end

                            // The literal/length alphabet.
                            T_LIT: begin
                                if (symv < 9'd256) begin
                                    if (out_free) begin
                                        code_q  <= 15'd0;
                                        first_q <= 16'd0;
                                        prev_q  <= 9'd0;
                                        dlen_q  <= 4'd1;
                                        state_q <= S_DEC;
                                    end else begin
                                        lit_q   <= symv[7:0];
                                        state_q <= S_LIT;
                                    end
                                end else if (symv == 9'd256) begin
                                    if (bfinal_q) begin
                                        tix_q   <= 2'd0;
                                        state_q <= (WRAPPER != 0) ? S_EALIGN
                                                                  : S_DONE;
                                    end else begin
                                        acc_q   <= 16'd0;
                                        pos_q   <= 4'd0;
                                        nbit_q  <= 5'd3;
                                        ret_q   <= S_BLKDO;
                                        state_q <= S_BITS;
                                    end
                                end else if (symv < 9'd286) begin
                                    xcode_q <= lenx;
                                    acc_q   <= 16'd0;
                                    pos_q   <= 4'd0;
                                    if (len_extra(lenx) == 3'd0) begin
                                        state_q <= S_LEXDO;
                                    end else begin
                                        nbit_q  <= {2'd0, len_extra(lenx)};
                                        ret_q   <= S_LEXDO;
                                        state_q <= S_BITS;
                                    end
                                end else begin
                                    // §3.2.5: 286 and 287 "will never
                                    // actually occur in the compressed
                                    // data".
                                    err_q   <= 1'b1;
                                    ecode_q <= E_CODE;
                                    state_q <= S_ERR;
                                end
                            end

                            // The distance alphabet.
                            default: begin
                                if (symv < 9'd30) begin
                                    xcode_q <= symv[4:0];
                                    acc_q   <= 16'd0;
                                    pos_q   <= 4'd0;
                                    if (dist_extra(symv[4:0]) == 4'd0) begin
                                        state_q <= S_DEXDO;
                                    end else begin
                                        nbit_q  <= {1'b0, dist_extra(symv[4:0])};
                                        ret_q   <= S_DEXDO;
                                        state_q <= S_BITS;
                                    end
                                end else begin
                                    // §3.2.6 again: 30 and 31 cannot
                                    // occur, and the fixed code has
                                    // codes for them.
                                    err_q   <= 1'b1;
                                    ecode_q <= E_CODE;
                                    state_q <= S_ERR;
                                end
                            end
                        endcase
                    end

                    S_LIT: if (out_free) begin
                        code_q  <= 15'd0;
                        first_q <= 16'd0;
                        prev_q  <= 9'd0;
                        dlen_q  <= 4'd1;
                        dret_q  <= T_LIT;
                        state_q <= S_DEC;
                    end

                    S_LEXDO: begin
                        rem_q   <= len_base(xcode_q) + {4'd0, acc_q[4:0]};
                        code_q  <= 15'd0;
                        first_q <= 16'd0;
                        prev_q  <= 9'd0;
                        dlen_q  <= 4'd1;
                        dret_q  <= T_DIST;
                        state_q <= S_DEC;
                    end

                    S_DEXDO: begin
                        dist_q  <= {1'b0, dist_base(xcode_q)}
                                 + {3'd0, acc_q[12:0]};
                        state_q <= S_COPEN;
                    end

                    S_COPEN: begin
                        if (win_dist_bad) begin
                            err_q   <= 1'b1;
                            ecode_q <= E_DIST;
                            state_q <= S_ERR;
                        end else begin
                            state_q <= S_COPY;
                        end
                    end

                    S_COPY: begin
                        if (copy_issue) rem_q <= rem_q - 9'd1;
                        if (copy_last) begin
                            code_q  <= 15'd0;
                            first_q <= 16'd0;
                            prev_q  <= 9'd0;
                            dlen_q  <= 4'd1;
                            dret_q  <= T_LIT;
                            state_q <= S_DEC;
                        end
                    end

                    S_EALIGN: state_q <= S_ADLER;

                    // RFC 1950 §9's four bytes, most significant first.
                    S_ADLER: if (byte_avail) begin
                        if (cur_q != trail_want) begin
                            err_q   <= 1'b1;
                            ecode_q <= E_ADLER;
                            state_q <= S_ERR;
                        end else if (tix_q == 2'd3) begin
                            state_q <= S_DONE;
                        end else begin
                            tix_q <= tix_q + 2'd1;
                        end
                    end

                    // `done` waits for the consumer to take the last
                    // byte, so that it means "all of it is yours".
                    S_DONE: if (!ov_q) done_q <= 1'b1;

                    S_ERR: ;

                    default: begin
                        err_q   <= 1'b1;
                        ecode_q <= E_CODE;
                        state_q <= S_ERR;
                    end
                endcase
            end
        end
    end
endmodule
