// sha256_core — the SHA-256 compression function over a byte stream of
// already-padded 512-bit blocks.
//
// What it does
//   Absorbs bytes. Every 64 bytes it runs the sixty-four rounds of
//   FIPS 180-4 §6.2.2 and adds the result into the chaining value, which
//   is on `state` and is the digest once the last padded block has gone
//   through. `start` reloads the chaining value with H(0) — the eight
//   words of FIPS 180-4 §5.3.3 — and throws away any partial block.
//
//   A block is fed through the same ready/valid handshake the rest of
//   this library uses: a byte moves on a rising edge where `in_valid`
//   and `in_ready` are both high. `in_ready` drops for the sixty-five
//   cycles a block takes to compress, so a producer that cannot wait
//   needs a FIFO (`fifo_sync`) in front.
//
//   The byte order is FIPS 180-4's: the first byte of the message is the
//   most significant byte of M(0), and `state[255:248]` is the first
//   byte of the digest. A byte string in this package always has its
//   first byte at the most significant end.
//
// What it does not do
//   **No padding.** This module compresses whole 512-bit blocks and
//   counts nothing; the length field and the `1` bit are `sha256`'s job,
//   one level up. Feeding it a number of bytes that is not a multiple of
//   64 leaves the last partial block sitting in `w_q` and never
//   compresses it — no error is raised, because this module has no idea
//   where a message ends.
//
//   No bit-length messages. FIPS 180-4 §5.1.1 pads a bit string; this
//   interface is a byte string, so a message whose length is not a whole
//   number of bytes cannot be expressed. Nothing in practice wants one.
//
//   No SHA-224. It is the same compression function with a different
//   H(0) and a truncated digest, and it would be an initial-value
//   parameter and a mux; nothing here needs it yet, and adding it later
//   changes no port.
//
// Why one round per cycle, and why a byte-wide port
//   The round is already five 32-bit additions deep — T1 is
//   `h + Σ1(e) + Ch(e,f,g) + K(t) + W(t)` — and two rounds per cycle
//   would make it ten, which is the whole critical path of this block
//   for a factor of two in cycles. One round per cycle it is: 64 cycles
//   of rounds, one to add into the chaining value, 65 in all.
//
//   That sets the port width, and this is the pleasing part: sixty-four
//   bytes arrive in sixty-four cycles at one byte per cycle, which is
//   exactly as long as the rounds take. A wider port would make the
//   load half as long and leave the rounds where they are — thirteen
//   per cent off a block, for four times the input wiring and a
//   byte-enable encoding on the last word to get wrong. Eight bits.
//
//   A block therefore costs 64 + 64 + 1 = **129 cycles**, and the
//   sustained rate is 64 bytes per 129 cycles, 0.496 bytes per cycle.
//   A second 512-bit buffer would let the next block load while this
//   one compresses and take that to 64 per 65, 0.985 — within two per
//   cent of twice as fast, for 512 more flip-flops, which is more than
//   half of everything this module holds. `docs/ip-library.md` has the
//   measured footprint; the trade is stated there and not taken.
//
// Constant time
//   The cycle count is 129 per block whatever the bytes are: no loop
//   here is bounded by data, and the one table — K(t) — is indexed by
//   the round counter, which counts 0 to 63 and depends on nothing.
//   `tests/ip_crypto.rs` measures that rather than asserting it, and
//   `ip/crypto/sha256/README.md` §4 says what it does and does not
//   establish. It does **not** establish anything about power or
//   electromagnetic emission.
module sha256_core (
    input  wire         clk,
    input  wire         rst_n,

    // Abandon whatever is in progress and start a new message: the
    // chaining value goes back to H(0) and any partial block is
    // dropped. A design that only ever hashes from reset can tie it to
    // zero.
    input  wire         start,

    // One byte of an already-padded message.
    input  wire [7:0]   in_byte,
    input  wire         in_valid,
    output wire         in_ready,

    // One cycle, as the sixty-fourth round's result lands in the
    // chaining value: `state` below is now the chaining value after the
    // block that just finished.
    output wire         block_valid,

    // The chaining value, H(0) after reset or `start`. After the last
    // padded block of a message it is the digest, first byte at the
    // most significant end.
    output wire [255:0] state
);
    // FIPS 180-4 §5.3.3: the first thirty-two bits of the fractional
    // parts of the square roots of the first eight primes.
    localparam [255:0] H_INIT = {
        32'h6a09e667, 32'hbb67ae85, 32'h3c6ef372, 32'ha54ff53a,
        32'h510e527f, 32'h9b05688c, 32'h1f83d9ab, 32'h5be0cd19
    };

    // The three things this module can be doing. Three values, two bits.
    localparam [1:0] S_LOAD  = 2'd0;
    localparam [1:0] S_ROUND = 2'd1;
    localparam [1:0] S_ADD   = 2'd2;

    // FIPS 180-4 (4.4) and (4.5): the big sigmas, over the working
    // variables. Written as concatenations rather than shift-or pairs so
    // that every one of them is wiring and no width inference has to be
    // trusted.
    function [31:0] bsig0;
        input [31:0] x;
        bsig0 = {x[1:0], x[31:2]} ^ {x[12:0], x[31:13]} ^ {x[21:0], x[31:22]};
    endfunction

    function [31:0] bsig1;
        input [31:0] x;
        bsig1 = {x[5:0], x[31:6]} ^ {x[10:0], x[31:11]} ^ {x[24:0], x[31:25]};
    endfunction

    // FIPS 180-4 (4.6) and (4.7): the small sigmas, over the message
    // schedule. Both end in a shift and not a rotate.
    function [31:0] ssig0;
        input [31:0] x;
        ssig0 = {x[6:0], x[31:7]} ^ {x[17:0], x[31:18]} ^ {3'b000, x[31:3]};
    endfunction

    function [31:0] ssig1;
        input [31:0] x;
        ssig1 = {x[16:0], x[31:17]} ^ {x[18:0], x[31:19]} ^ {10'b0, x[31:10]};
    endfunction

    // FIPS 180-4 (4.2) and (4.3).
    function [31:0] ch;
        input [31:0] x, y, z;
        ch = (x & y) ^ (~x & z);
    endfunction

    function [31:0] maj;
        input [31:0] x, y, z;
        maj = (x & y) ^ (x & z) ^ (y & z);
    endfunction

    // FIPS 180-4 §4.2.2: the first thirty-two bits of the fractional
    // parts of the cube roots of the first sixty-four primes.
    //
    // A case over the round counter and not a memory, on purpose. The
    // index is `round_q`, which counts 0 to 63 and is a function of
    // nothing but the clock, so this table is addressed by a public
    // value; synthesis turns it into logic and `tests/ip_crypto.rs`
    // asserts that neither crypto block contains a memory array at all.
    function [31:0] k_of;
        input [5:0] t;
        case (t)
            6'd0:  k_of = 32'h428a2f98;
            6'd1:  k_of = 32'h71374491;
            6'd2:  k_of = 32'hb5c0fbcf;
            6'd3:  k_of = 32'he9b5dba5;
            6'd4:  k_of = 32'h3956c25b;
            6'd5:  k_of = 32'h59f111f1;
            6'd6:  k_of = 32'h923f82a4;
            6'd7:  k_of = 32'hab1c5ed5;
            6'd8:  k_of = 32'hd807aa98;
            6'd9:  k_of = 32'h12835b01;
            6'd10: k_of = 32'h243185be;
            6'd11: k_of = 32'h550c7dc3;
            6'd12: k_of = 32'h72be5d74;
            6'd13: k_of = 32'h80deb1fe;
            6'd14: k_of = 32'h9bdc06a7;
            6'd15: k_of = 32'hc19bf174;
            6'd16: k_of = 32'he49b69c1;
            6'd17: k_of = 32'hefbe4786;
            6'd18: k_of = 32'h0fc19dc6;
            6'd19: k_of = 32'h240ca1cc;
            6'd20: k_of = 32'h2de92c6f;
            6'd21: k_of = 32'h4a7484aa;
            6'd22: k_of = 32'h5cb0a9dc;
            6'd23: k_of = 32'h76f988da;
            6'd24: k_of = 32'h983e5152;
            6'd25: k_of = 32'ha831c66d;
            6'd26: k_of = 32'hb00327c8;
            6'd27: k_of = 32'hbf597fc7;
            6'd28: k_of = 32'hc6e00bf3;
            6'd29: k_of = 32'hd5a79147;
            6'd30: k_of = 32'h06ca6351;
            6'd31: k_of = 32'h14292967;
            6'd32: k_of = 32'h27b70a85;
            6'd33: k_of = 32'h2e1b2138;
            6'd34: k_of = 32'h4d2c6dfc;
            6'd35: k_of = 32'h53380d13;
            6'd36: k_of = 32'h650a7354;
            6'd37: k_of = 32'h766a0abb;
            6'd38: k_of = 32'h81c2c92e;
            6'd39: k_of = 32'h92722c85;
            6'd40: k_of = 32'ha2bfe8a1;
            6'd41: k_of = 32'ha81a664b;
            6'd42: k_of = 32'hc24b8b70;
            6'd43: k_of = 32'hc76c51a3;
            6'd44: k_of = 32'hd192e819;
            6'd45: k_of = 32'hd6990624;
            6'd46: k_of = 32'hf40e3585;
            6'd47: k_of = 32'h106aa070;
            6'd48: k_of = 32'h19a4c116;
            6'd49: k_of = 32'h1e376c08;
            6'd50: k_of = 32'h2748774c;
            6'd51: k_of = 32'h34b0bcb5;
            6'd52: k_of = 32'h391c0cb3;
            6'd53: k_of = 32'h4ed8aa4a;
            6'd54: k_of = 32'h5b9cca4f;
            6'd55: k_of = 32'h682e6ff3;
            6'd56: k_of = 32'h748f82ee;
            6'd57: k_of = 32'h78a5636f;
            6'd58: k_of = 32'h84c87814;
            6'd59: k_of = 32'h8cc70208;
            6'd60: k_of = 32'h90befffa;
            6'd61: k_of = 32'ha4506ceb;
            6'd62: k_of = 32'hbef9a3f7;
            default: k_of = 32'hc67178f2;
        endcase
    endfunction

    // The message schedule, which is also the block as it arrives: a
    // byte shifts in at the bottom during S_LOAD, so after sixty-four
    // bytes the first four are the top word, which is M(0) big-endian.
    // During S_ROUND the same register shifts a word at a time and the
    // schedule's recurrence fills the hole, so W(t) is always the top
    // word and the sixteen live words are all there is.
    reg [511:0] w_q;
    // The chaining value H(i-1), eight words, H0 at the top.
    reg [255:0] h_q;
    // The working variables a..h of §6.2.2, a at the top.
    reg [255:0] wk_q;
    // Bytes of this block loaded so far, 0 to 63: the sixty-fourth byte
    // takes the state to S_ROUND and this back to zero, so it never
    // holds 64 and six bits is all it is.
    reg [5:0]   fill_q;
    // The round, 0 to 63.
    reg [5:0]   round_q;
    reg [1:0]   state_q;
    reg         done_q;

    assign in_ready    = (state_q == S_LOAD);
    assign block_valid = done_q;
    assign state       = h_q;

    wire load = in_valid && in_ready;

    // W(j) is w_q[511-32j -: 32]: the recurrence of FIPS 180-4 §6.2.2
    // wants W(t), W(t+1), W(t+9) and W(t+14), which are j = 0, 1, 9, 14.
    wire [31:0] w0  = w_q[511:480];
    wire [31:0] w1  = w_q[479:448];
    wire [31:0] w9  = w_q[223:192];
    wire [31:0] w14 = w_q[63:32];
    wire [31:0] w_next = ssig1(w14) + w9 + ssig0(w1) + w0;

    wire [31:0] a = wk_q[255:224];
    wire [31:0] b = wk_q[223:192];
    wire [31:0] c = wk_q[191:160];
    wire [31:0] d = wk_q[159:128];
    wire [31:0] e = wk_q[127:96];
    wire [31:0] f = wk_q[95:64];
    wire [31:0] g = wk_q[63:32];
    wire [31:0] h = wk_q[31:0];

    wire [31:0] t1 = h + bsig1(e) + ch(e, f, g) + k_of(round_q) + w0;
    wire [31:0] t2 = bsig0(a) + maj(a, b, c);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            w_q     <= 512'd0;
            h_q     <= H_INIT;
            wk_q    <= 256'd0;
            fill_q  <= 6'd0;
            round_q <= 6'd0;
            state_q <= S_LOAD;
            done_q  <= 1'b0;
        end else if (start) begin
            // A new message: the chaining value goes back and the partial
            // block goes away. `w_q` is left alone — it is overwritten by
            // the bytes of the next block before anything reads it.
            h_q     <= H_INIT;
            fill_q  <= 6'd0;
            round_q <= 6'd0;
            state_q <= S_LOAD;
            done_q  <= 1'b0;
        end else begin
            done_q <= 1'b0;
            case (state_q)
                S_LOAD: begin
                    if (load) begin
                        w_q <= {w_q[503:0], in_byte};
                        if (fill_q == 6'd63) begin
                            // The block is complete. a..h start at the
                            // chaining value, §6.2.2 step 2.
                            fill_q  <= 6'd0;
                            wk_q    <= h_q;
                            round_q <= 6'd0;
                            state_q <= S_ROUND;
                        end else begin
                            fill_q <= fill_q + 6'd1;
                        end
                    end
                end
                S_ROUND: begin
                    wk_q <= {t1 + t2, a, b, c, d + t1, e, f, g};
                    w_q  <= {w_q[479:0], w_next};
                    if (round_q == 6'd63) begin
                        state_q <= S_ADD;
                    end else begin
                        round_q <= round_q + 6'd1;
                    end
                end
                default: begin
                    // S_ADD, §6.2.2 step 4.
                    h_q <= {h_q[255:224] + wk_q[255:224],
                            h_q[223:192] + wk_q[223:192],
                            h_q[191:160] + wk_q[191:160],
                            h_q[159:128] + wk_q[159:128],
                            h_q[127:96]  + wk_q[127:96],
                            h_q[95:64]   + wk_q[95:64],
                            h_q[63:32]   + wk_q[63:32],
                            h_q[31:0]    + wk_q[31:0]};
                    done_q  <= 1'b1;
                    state_q <= S_LOAD;
                end
            endcase
        end
    end
endmodule
