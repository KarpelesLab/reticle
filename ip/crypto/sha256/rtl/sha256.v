// sha256 — SHA-256 over a byte stream, padding included.
//
// What it does
//   Takes a message a byte at a time and produces its SHA-256 digest.
//   `sha256_core` does the compression; everything here is the padding
//   of FIPS 180-4 §5.1.1 — the `1` bit, the zeros, and the sixty-four
//   bit length — generated as bytes and fed to the core in front of
//   nothing and behind the message.
//
//   The interface is the library's ready/valid handshake with one
//   addition, and the addition is worth reading because it is not what
//   AXI4-Stream means by `tlast`:
//
//     * a byte moves on a rising edge where `in_valid` and `in_ready`
//       are both high;
//     * **`in_last` ends the message at this point in the stream, and
//       `in_valid` says whether there is a byte at this point.** So the
//       usual case is `in_valid` and `in_last` together on the final
//       byte, and `in_last` alone — with `in_valid` low — ends a message
//       with no byte here, which is the only way to express the empty
//       message. `in_last` must be held until `in_ready` is high, like
//       any other input of a handshake.
//
//   `digest_valid` is one cycle. `digest` holds the digest it announced
//   until the next message's digest replaces it, so a consumer does not
//   have to catch the pulse. Before the first message it is zero.
//
//   `start` abandons a message in progress and begins again with an
//   empty one. A design that only ever hashes whole messages ties it to
//   zero; nothing needs to be pulsed to begin.
//
//   Byte order is FIPS 180-4's throughout: the first byte of the message
//   is the most significant byte of M(0), and `digest[255:248]` is the
//   first byte of the digest.
//
// Why the padding is in here and not in the caller
//   It is a real choice. A core that takes already-padded 512-bit blocks
//   is smaller — no length counter, no pad state machine, 70-odd
//   flip-flops and a handful of lookup tables less — and `sha256_core`
//   is exactly that core, so a caller who already has padded blocks (a
//   Merkle tree walking fixed-size nodes, say) instantiates it directly
//   and pays nothing for this file.
//
//   But padding is where SHA-256 implementations go wrong, and they go
//   wrong in one specific place: a message whose length is 56 bytes more
//   than a multiple of 64 leaves no room for the length field and the
//   padding spills into an extra block. FIPS 180-4's own Appendix B.2
//   example is 56 bytes long, which is not a coincidence. A library
//   block that leaves that to every caller ships that bug once per
//   caller; a library block that does it itself ships it once, and
//   `tests/ip_crypto.rs` drives every message length from 0 to 129 and
//   192 through it.
//
//   So: both, and the default — `top` in the manifest — is the one that
//   pads.
//
// What it does not do
//   No bit-granular messages: the stream is bytes, so FIPS 180-4's bit
//   strings of non-multiple-of-eight length cannot be expressed.
//
//   No length limit beyond LEN_BITS, and **no complaint if you exceed
//   it**. The byte counter is LEN_BITS wide; a message longer than
//   2^LEN_BITS - 1 bytes wraps it and the digest is of a different
//   length field, which is wrong and silent. At the default of 61 the
//   limit is the 2^64 - 1 bits FIPS 180-4 §1 states, so the only way to
//   reach it is to lower the parameter on purpose.
//
//   No HMAC, no multi-message pipelining, no interleaved messages. One
//   message at a time.
//
//   Nothing here is clocked faster than one message byte per cycle, and
//   the core stops accepting for the 65 cycles a block compresses in, so
//   `in_ready` is low about half the time. `docs/ip-library.md` has the
//   numbers and `sha256_core`'s header has the reasoning.
module sha256 #(
    // Bits of the message byte counter. The length field FIPS 180-4
    // §5.1.1 appends is 64 bits of *bit* length; this counts *bytes*, so
    // 61 bits is the whole standard range and the three low bits of the
    // field are hard zeros rather than three flip-flops that could only
    // ever hold zero. Lower it to buy the counter back: 32 bits hashes
    // messages up to four gigabytes and costs 29 flip-flops less.
    parameter LEN_BITS = 61
) (
    input  wire         clk,
    input  wire         rst_n,

    // Abandon any message in progress; the next byte starts a new one.
    input  wire         start,

    input  wire [7:0]   in_byte,
    input  wire         in_valid,
    // The message ends here. See the header: `in_valid` says whether
    // there is a byte at this point, and `in_last` without it is how the
    // empty message is spelled.
    input  wire         in_last,
    output wire         in_ready,

    // One cycle, when `digest` becomes the digest of the message that
    // just ended.
    output wire         digest_valid,
    output wire [255:0] digest,

    // High whenever a message is in progress or being finished.
    output wire         busy
);
    // Where the padding has got to. Five values, three bits.
    localparam [2:0] P_MSG  = 3'd0;  // passing message bytes through
    localparam [2:0] P_ONE  = 3'd1;  // the single 1 bit, as 0x80
    localparam [2:0] P_ZERO = 3'd2;  // zeros up to byte 56 of a block
    localparam [2:0] P_LEN  = 3'd3;  // the eight bytes of the bit length
    localparam [2:0] P_WAIT = 3'd4;  // the last block is compressing

    reg [2:0]           state_q;
    // Message bytes seen. Not the same counter as the core's `fill_q`:
    // that one counts bytes of the current block and this one counts
    // bytes of the message.
    reg [LEN_BITS-1:0]  len_q;
    // Bytes fed to the core in the current block, 0 to 63. It tracks the
    // core's own `fill_q` exactly — both advance on the same condition —
    // and is here because the padding needs to know where byte 56 of a
    // block is and the core does not publish it.
    reg [5:0]           pos_q;
    // Which of the eight length bytes is next.
    reg [2:0]           lenix_q;
    reg [255:0]         dg_q;
    reg                 dv_q;
    // One cycle of `start` to the core, the cycle after a digest is
    // taken. `in_ready` is low in it, so the reload cannot eat a byte.
    reg                 init_q;

    wire         core_ready;
    wire         core_block;
    wire [255:0] core_state;

    // The 1 bit of FIPS 180-4 §5.1.1, as the byte the padding of a
    // byte-granular message always begins with.
    localparam [7:0] PAD_ONE = 8'h80;

    // The length field: the message length in bits, which is the byte
    // count times eight. The assignment zero-extends, so LEN_BITS below
    // 61 simply leaves more leading zeros and LEN_BITS = 61 fills it
    // exactly.
    wire [63:0] bitlen = {len_q, 3'b000};
    // Big-endian: byte 0 of the field is bits 63..56.
    wire [7:0]  len_byte = bitlen[{(3'd7 - lenix_q), 3'b000} +: 8];

    // What the core is being fed this cycle, and whether it is being fed
    // at all.
    wire        pad_phase = (state_q != P_MSG) && (state_q != P_WAIT);
    wire [7:0]  core_byte = (state_q == P_MSG) ? in_byte
                          : (state_q == P_ONE) ? PAD_ONE
                          : (state_q == P_LEN) ? len_byte
                          :                      8'h00;
    wire        core_valid = pad_phase || ((state_q == P_MSG) && in_valid);
    wire        fed        = core_valid && core_ready;

    assign in_ready     = (state_q == P_MSG) && core_ready && !init_q;
    assign digest_valid = dv_q;
    assign digest       = dg_q;
    assign busy         = (state_q != P_MSG) || !core_ready;

    // The message ends on this edge: either the final byte moves, or
    // `in_last` arrives on its own.
    wire msg_end = in_ready && in_last;
    // The 1 bit lands on byte 56 of a block exactly when the byte before
    // it was byte 55, which is the case that needs no zeros at all.
    wire at_55 = (pos_q == 6'd55);

    sha256_core u_core (
        .clk         (clk),
        .rst_n       (rst_n),
        .start       (start || init_q),
        .in_byte     (core_byte),
        .in_valid    (core_valid),
        .in_ready    (core_ready),
        .block_valid (core_block),
        .state       (core_state)
    );

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state_q <= P_MSG;
            len_q   <= {LEN_BITS{1'b0}};
            pos_q   <= 6'd0;
            lenix_q <= 3'd0;
            dg_q    <= 256'd0;
            dv_q    <= 1'b0;
            init_q  <= 1'b0;
        end else if (start) begin
            state_q <= P_MSG;
            len_q   <= {LEN_BITS{1'b0}};
            pos_q   <= 6'd0;
            lenix_q <= 3'd0;
            dv_q    <= 1'b0;
            init_q  <= 1'b0;
        end else begin
            dv_q   <= 1'b0;
            init_q <= 1'b0;

            // One counter pair for every byte that reaches the core,
            // whichever state put it there. `pos_q` is six bits and
            // wraps at the block boundary by itself.
            if (fed) begin
                pos_q <= pos_q + 6'd1;
                if (state_q == P_MSG) len_q <= len_q + 1'b1;
            end

            case (state_q)
                P_MSG: begin
                    if (msg_end) state_q <= P_ONE;
                end
                P_ONE: begin
                    if (fed) state_q <= at_55 ? P_LEN : P_ZERO;
                end
                P_ZERO: begin
                    if (fed && at_55) state_q <= P_LEN;
                end
                P_LEN: begin
                    if (fed) begin
                        if (lenix_q == 3'd7) begin
                            lenix_q <= 3'd0;
                            state_q <= P_WAIT;
                        end else begin
                            lenix_q <= lenix_q + 3'd1;
                        end
                    end
                end
                default: begin
                    // P_WAIT. The length field always ends on a block
                    // boundary, so the next block the core finishes is
                    // the last one of this message.
                    if (core_block) begin
                        dg_q    <= core_state;
                        dv_q    <= 1'b1;
                        init_q  <= 1'b1;
                        len_q   <= {LEN_BITS{1'b0}};
                        state_q <= P_MSG;
                    end
                end
            endcase
        end
    end
endmodule
