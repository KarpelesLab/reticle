// chacha20_core — the ChaCha20 block function of RFC 8439 §2.3.
//
// What it does
//   Builds the sixteen-word state from the constants, the key, the block
//   counter and the nonce; runs twenty rounds, ten column rounds
//   interleaved with ten diagonal rounds; adds the original state back
//   in; and presents the serialisation of the result on `block`. That is
//   the whole of §2.3, and `block` is one 64-byte keystream block.
//
//   `start` begins it. `valid` is one cycle, when `block` becomes the
//   answer. `busy` is high from `start` until then.
//
//   **Byte order.** `key` and `nonce` are byte strings with their first
//   byte at the most significant end, so `key[255:248]` is key byte 0
//   and `nonce[95:88]` is nonce byte 0 — the order the RFC prints them
//   in. `block[511:504]` is keystream byte 0, likewise. The little-endian
//   word loads of §2.3 happen inside, as wiring, which is where they
//   belong: a caller holding a key as thirty-two bytes should not have
//   to byte-swap it to use this.
//
//   `counter` is **not** a byte string. §2.3 calls word 12 "a block
//   counter" and the RFC's own test vectors give it as a number ("Block
//   Counter = 1"), so this port is that number and no swap is applied to
//   it.
//
//   `advance` rotates `block` one 32-bit word towards the most
//   significant end, so a consumer can take the keystream a word at a
//   time without a sixteen-way multiplexer; sixteen pulses bring it back
//   where it started. A consumer that reads all 512 bits at once leaves
//   it tied low and never notices. `chacha20` uses it, and the fact that
//   it costs nothing is why that module has no keystream register of its
//   own.
//
// What the caller must hold still
//   `key`, `nonce` and `counter` must not change between `start` and
//   `valid`. Nothing here latches them: the final addition of §2.3 reads
//   the initial state back off the ports, which is how this module
//   avoids a second 512-bit register holding a copy of the key. That is
//   384 flip-flops not spent, and it means the key exists in this design
//   in one place and not two.
//
// Why one round per cycle
//   A ChaCha round is four quarter-rounds on disjoint words — columns,
//   then diagonals — so four `chacha20_qr` instances compute a whole
//   round in one cycle and the critical path is one quarter-round, four
//   32-bit additions. The alternatives, with the numbers:
//
//     * one quarter-round per cycle, one instance: 80 cycles a block
//       instead of 20, for the **same** critical path. Four times slower
//       to save three quarter-rounds of logic, and the measured cost of
//       one is in `docs/ip-library.md` — `chacha20_qr` has a row of its
//       own there for exactly this comparison.
//     * one double-round per cycle, eight instances: 10 cycles a block,
//       and eight 32-bit additions of critical path instead of four.
//       That halves the clock this block closes at, which gives back
//       most of what it bought.
//
//   One round per cycle is the one that is four times faster than the
//   cheap option and costs nothing in path depth. Twenty rounds, one
//   cycle to add the initial state back, and one to start: a block is
//   **22 cycles** from `start` to `valid`.
//
// What it does not do
//   No counter arithmetic — `counter` is used as given. `chacha20`
//   increments it between blocks and stops when it would wrap.
//
//   No XChaCha20, no HChaCha20, no reduced-round ChaCha8 or ChaCha12,
//   no Poly1305. This is §2.3 and nothing else.
//
// Constant time
//   Twenty rounds whatever the key is, no table of any kind, and no
//   index derived from anything but `round_q`, which counts 0 to 19.
//   `tests/ip_crypto.rs` measures the cycle count for different keys and
//   compares them. `ip/crypto/chacha20/README.md` §4 says what that does
//   and does not establish, and it does **not** establish anything about
//   power or electromagnetic emission.
module chacha20_core (
    input  wire         clk,
    input  wire         rst_n,

    // Compute the block for the key, nonce and counter on the ports.
    input  wire         start,
    // Rotate `block` one word, for a word-at-a-time consumer.
    input  wire         advance,

    input  wire [255:0] key,
    input  wire [95:0]  nonce,
    input  wire [31:0]  counter,

    output wire         busy,
    output wire         valid,
    output wire [511:0] block
);
    // RFC 8439 §2.3: "expand 32-byte k", as four little-endian words.
    localparam [31:0] C0 = 32'h61707865;
    localparam [31:0] C1 = 32'h3320646e;
    localparam [31:0] C2 = 32'h79622d32;
    localparam [31:0] C3 = 32'h6b206574;

    // Three values, two bits.
    localparam [1:0] S_IDLE = 2'd0;
    localparam [1:0] S_RUN  = 2'd1;
    localparam [1:0] S_ADD  = 2'd2;

    // A 32-bit chunk of a byte string, read as a little-endian word.
    function [31:0] le32;
        input [31:0] bs;
        le32 = {bs[7:0], bs[15:8], bs[23:16], bs[31:24]};
    endfunction

    // The sixteen words of the state, word j at st_q[511-32j -: 32].
    reg [511:0] st_q;
    // The round, 0 to 19. Its low bit chooses column or diagonal.
    reg [4:0]   round_q;
    reg [1:0]   state_q;
    reg         valid_q;

    assign busy  = (state_q != S_IDLE);
    assign valid = valid_q;

    // §2.3's initial state, straight off the ports.
    wire [31:0] i0  = C0;
    wire [31:0] i1  = C1;
    wire [31:0] i2  = C2;
    wire [31:0] i3  = C3;
    wire [31:0] i4  = le32(key[255:224]);
    wire [31:0] i5  = le32(key[223:192]);
    wire [31:0] i6  = le32(key[191:160]);
    wire [31:0] i7  = le32(key[159:128]);
    wire [31:0] i8  = le32(key[127:96]);
    wire [31:0] i9  = le32(key[95:64]);
    wire [31:0] i10 = le32(key[63:32]);
    wire [31:0] i11 = le32(key[31:0]);
    wire [31:0] i12 = counter;
    wire [31:0] i13 = le32(nonce[95:64]);
    wire [31:0] i14 = le32(nonce[63:32]);
    wire [31:0] i15 = le32(nonce[31:0]);

    wire [511:0] init = {i0, i1, i2,  i3,  i4,  i5,  i6,  i7,
                         i8, i9, i10, i11, i12, i13, i14, i15};

    wire [31:0] x0  = st_q[511:480];
    wire [31:0] x1  = st_q[479:448];
    wire [31:0] x2  = st_q[447:416];
    wire [31:0] x3  = st_q[415:384];
    wire [31:0] x4  = st_q[383:352];
    wire [31:0] x5  = st_q[351:320];
    wire [31:0] x6  = st_q[319:288];
    wire [31:0] x7  = st_q[287:256];
    wire [31:0] x8  = st_q[255:224];
    wire [31:0] x9  = st_q[223:192];
    wire [31:0] x10 = st_q[191:160];
    wire [31:0] x11 = st_q[159:128];
    wire [31:0] x12 = st_q[127:96];
    wire [31:0] x13 = st_q[95:64];
    wire [31:0] x14 = st_q[63:32];
    wire [31:0] x15 = st_q[31:0];

    // An even round is a column round, an odd one a diagonal round:
    // RFC 8439 §2.3's `inner_block` is the eight calls below, and ten of
    // those is twenty rounds.
    //
    //   column:   QR(0,4,8,12)  QR(1,5,9,13)  QR(2,6,10,14) QR(3,7,11,15)
    //   diagonal: QR(0,5,10,15) QR(1,6,11,12) QR(2,7,8,13)  QR(3,4,9,14)
    //
    // The `a` input is word n either way; only b, c and d rotate, which
    // is why one set of four instances does both kinds.
    wire diag = round_q[0];

    wire [31:0] b0_in = diag ? x5  : x4;
    wire [31:0] c0_in = diag ? x10 : x8;
    wire [31:0] d0_in = diag ? x15 : x12;
    wire [31:0] b1_in = diag ? x6  : x5;
    wire [31:0] c1_in = diag ? x11 : x9;
    wire [31:0] d1_in = diag ? x12 : x13;
    wire [31:0] b2_in = diag ? x7  : x6;
    wire [31:0] c2_in = diag ? x8  : x10;
    wire [31:0] d2_in = diag ? x13 : x14;
    wire [31:0] b3_in = diag ? x4  : x7;
    wire [31:0] c3_in = diag ? x9  : x11;
    wire [31:0] d3_in = diag ? x14 : x15;

    wire [31:0] qa0, qb0, qc0, qd0;
    wire [31:0] qa1, qb1, qc1, qd1;
    wire [31:0] qa2, qb2, qc2, qd2;
    wire [31:0] qa3, qb3, qc3, qd3;

    chacha20_qr u_qr0 (
        .a_in (x0),    .b_in (b0_in), .c_in (c0_in), .d_in (d0_in),
        .a_out(qa0),   .b_out(qb0),   .c_out(qc0),   .d_out(qd0)
    );
    chacha20_qr u_qr1 (
        .a_in (x1),    .b_in (b1_in), .c_in (c1_in), .d_in (d1_in),
        .a_out(qa1),   .b_out(qb1),   .c_out(qc1),   .d_out(qd1)
    );
    chacha20_qr u_qr2 (
        .a_in (x2),    .b_in (b2_in), .c_in (c2_in), .d_in (d2_in),
        .a_out(qa2),   .b_out(qb2),   .c_out(qc2),   .d_out(qd2)
    );
    chacha20_qr u_qr3 (
        .a_in (x3),    .b_in (b3_in), .c_in (c3_in), .d_in (d3_in),
        .a_out(qa3),   .b_out(qb3),   .c_out(qc3),   .d_out(qd3)
    );

    // Where each quarter-round's outputs go back. On a column round
    // instance n wrote words n, 4+n, 8+n, 12+n; on a diagonal round the
    // b, c and d positions are one, two and three places round.
    wire [31:0] n0  = qa0;
    wire [31:0] n1  = qa1;
    wire [31:0] n2  = qa2;
    wire [31:0] n3  = qa3;
    wire [31:0] n4  = diag ? qb3 : qb0;
    wire [31:0] n5  = diag ? qb0 : qb1;
    wire [31:0] n6  = diag ? qb1 : qb2;
    wire [31:0] n7  = diag ? qb2 : qb3;
    wire [31:0] n8  = diag ? qc2 : qc0;
    wire [31:0] n9  = diag ? qc3 : qc1;
    wire [31:0] n10 = diag ? qc0 : qc2;
    wire [31:0] n11 = diag ? qc1 : qc3;
    wire [31:0] n12 = diag ? qd1 : qd0;
    wire [31:0] n13 = diag ? qd2 : qd1;
    wire [31:0] n14 = diag ? qd3 : qd2;
    wire [31:0] n15 = diag ? qd0 : qd3;

    wire [511:0] next = {n0, n1, n2,  n3,  n4,  n5,  n6,  n7,
                         n8, n9, n10, n11, n12, n13, n14, n15};

    // §2.3's `state += initial_state`, word by word so no carry crosses
    // a word boundary.
    wire [511:0] added = {x0  + i0,  x1  + i1,  x2  + i2,  x3  + i3,
                          x4  + i4,  x5  + i5,  x6  + i6,  x7  + i7,
                          x8  + i8,  x9  + i9,  x10 + i10, x11 + i11,
                          x12 + i12, x13 + i13, x14 + i14, x15 + i15};

    // `serialize(state)`: each word little-endian, so the whole thing is
    // a byte string with keystream byte 0 at the most significant end.
    assign block = {le32(x0),  le32(x1),  le32(x2),  le32(x3),
                    le32(x4),  le32(x5),  le32(x6),  le32(x7),
                    le32(x8),  le32(x9),  le32(x10), le32(x11),
                    le32(x12), le32(x13), le32(x14), le32(x15)};

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            st_q    <= 512'd0;
            round_q <= 5'd0;
            state_q <= S_IDLE;
            valid_q <= 1'b0;
        end else if (start) begin
            // `start` is honoured from **any** state, not only from idle:
            // it abandons a block in progress, the way `sha256_core`'s
            // does. `chacha20` relies on that — a stream whose last block
            // ended on a block boundary has already asked for the next
            // one, and a new `start` arrives with that request still in
            // the air. Taking `start` only from idle left the old block
            // to finish and the wrapper to use it as the new stream's
            // first, which is a keystream from the wrong counter.
            // `chacha20_encrypts_the_text_of_rfc_8439_2_4_2`'s
            // round trip is what found that, and no vector in RFC 8439
            // would have.
            st_q    <= init;
            round_q <= 5'd0;
            state_q <= S_RUN;
            valid_q <= 1'b0;
        end else begin
            valid_q <= 1'b0;
            case (state_q)
                S_IDLE: begin
                    if (advance) begin
                        // A rotation, not a shift: sixteen pulses and the
                        // block is back as it was.
                        st_q <= {st_q[479:0], st_q[511:480]};
                    end
                end
                S_RUN: begin
                    st_q <= next;
                    if (round_q == 5'd19) begin
                        state_q <= S_ADD;
                    end else begin
                        round_q <= round_q + 5'd1;
                    end
                end
                default: begin
                    // S_ADD.
                    st_q    <= added;
                    valid_q <= 1'b1;
                    state_q <= S_IDLE;
                end
            endcase
        end
    end
endmodule
