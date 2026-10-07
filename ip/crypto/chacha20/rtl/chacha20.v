// chacha20 — the ChaCha20 encryption algorithm of RFC 8439 §2.4.
//
// What it does
//   Turns a key, a nonce and a starting block counter into a keystream
//   and exclusive-ors it with a stream of data, which is both encryption
//   and decryption. `chacha20_core` makes the blocks; this file counts
//   them and does the exclusive-or.
//
//   `start` opens a stream: the counter on the port is captured, the
//   first block is computed, and `in_ready` comes up when it is there.
//   After that a word moves on a rising edge where `in_valid` and
//   `in_ready` are both high; `out_data` and `out_valid` follow **one
//   cycle later**, registered.
//
//   Sixteen words is a block. After the sixteenth the counter advances
//   and `in_ready` drops for the 23 cycles it takes to ask for the next
//   block, compute it and notice it arrived, then comes back. Nothing has
//   to be re-started between blocks.
//
//   **Byte order.** `in_data[31:24]` is the first of the four bytes and
//   `out_data[31:24]` is its result, which is the same way round as
//   `key`, `nonce` and `chacha20_core`'s `block`: in this package a byte
//   string always has its first byte at the most significant end. No
//   byte swap happens anywhere on the data path — the keystream comes off
//   the core already serialised.
//
//   `exhausted` latches if the stream would need block 2^32, which is
//   where RFC 8439 §2.3's 32-bit counter runs out: 256 gibibytes under
//   one key and nonce. `in_ready` stays low from then on, so the block
//   **stops** rather than wrapping the counter and repeating keystream.
//   Only `start` clears it.
//
// Why 32 bits of data and not 8
//   Because 8 would make the port the bottleneck and 32 does not. The
//   core needs 22 cycles for a block of 64 bytes; a 32-bit port drains
//   those 64 bytes in 16 cycles, which fits inside the 22 with room, so
//   the port is free and the core is the limit. An 8-bit port would take
//   64 cycles to drain a block the core made in 22, and a block would
//   cost 87 cycles instead of 39 — well under half the rate for no
//   saving worth having. A 64-bit port would drain in 8 and buy nothing
//   at all, since the 22 are still there.
//
//   `sha256` in this same category is 8 bits wide, and for the same
//   reason read the other way: its compression needs 64 cycles for 64
//   bytes, so one byte per cycle is already as fast as the core. The
//   rule is the width at which the port stops being the limit, and the
//   two blocks land in different places because their cores do.
//
// Throughput, and what is not spent on it
//   16 cycles of data, 22 of computing the next block and one to notice
//   it arrived is **39 cycles for 64 bytes**, 1.64 bytes per cycle —
//   164 MB/s, 1.31 Gbit/s, at 100 MHz. That is measured and not
//   calculated: `chacha20_takes_the_same_cycles_whatever_the_key_is`
//   prints 157 cycles for 256 bytes, which is four whole blocks and the
//   one cycle `start` takes.
//
//   The obvious improvement is a second 512-bit register: copy the block
//   out, start the next one immediately, and the 16 cycles of draining
//   hide inside the 22 of computing, giving 64 bytes per 22 cycles —
//   2.91 per cycle, 2.33 Gbit/s, **1.77 times** this. It costs 512
//   flip-flops, which is more than this module and `chacha20_core` hold
//   together, so it is stated here and not taken — the same call
//   `sha256_core` makes about its own second buffer, and for the same
//   reason. `docs/ip-library.md` has the footprints both of those claims
//   rest on.
//
// What the caller must hold still
//   `key` and `nonce` must not change from `start` until the stream ends.
//   Nothing latches them — see `chacha20_core`'s header for why — so a
//   caller that changes a key mid-stream gets a keystream from neither.
//
// What it does not do
//   No resuming mid-word. The keystream position advances four bytes at
//   a time, so a caller that wants to encrypt five bytes and then
//   continue from byte 5 cannot say so here; `chacha20_core` gives the
//   whole block and such a caller does the exclusive-or itself. A message
//   whose length is not a multiple of four is handled the easy way
//   instead: present a final word padded however you like and use the
//   bytes you need. The keystream depends on the key, the nonce and the
//   position and on nothing in `in_data`, so the bytes discarded change
//   nothing.
//
//   No Poly1305, so no authentication, so **this is not an AEAD**. A
//   ciphertext from here is malleable: flipping a bit of it flips that
//   bit of the plaintext. RFC 8439 §4 is about exactly that and
//   `ip/crypto/chacha20/README.md` §6 says what would have to be built.
//
//   No nonce generation and no nonce checking. Repeating a (key, nonce)
//   pair destroys the security of both messages and this block cannot
//   tell. The counter guard above is the one case it can see.
//
//   No XChaCha20 and no 64-bit-nonce original ChaCha20.
module chacha20 (
    input  wire         clk,
    input  wire         rst_n,

    // Open a stream at `counter`. Clears `exhausted`. Leave `in_valid`
    // low in this one cycle: `in_ready` is a function of registers only
    // (see below), so it can be high while the restart discards the
    // word.
    input  wire         start,

    input  wire [255:0] key,
    input  wire [95:0]  nonce,
    input  wire [31:0]  counter,

    input  wire [31:0]  in_data,
    input  wire         in_valid,
    output wire         in_ready,

    // One cycle after each accepted word.
    output wire [31:0]  out_data,
    output wire         out_valid,

    // A stream is open: `start` has been seen and `exhausted` has not
    // latched.
    output wire         active,
    // The 32-bit block counter ran out. See the header.
    output wire         exhausted
);
    // Three values, two bits.
    localparam [1:0] C_IDLE = 2'd0;  // no stream, or the counter ran out
    localparam [1:0] C_FILL = 2'd1;  // the core is computing a block
    localparam [1:0] C_RUN  = 2'd2;  // a block is here, words can move

    reg [1:0]  state_q;
    // The block counter of the block in hand.
    reg [31:0] ctr_q;
    // Words of this block already consumed, 0 to 15.
    reg [3:0]  widx_q;
    reg [31:0] out_q;
    reg        ov_q;
    reg        req_q;
    reg        exh_q;

    wire         core_valid;
    wire [511:0] core_block;

    // Registers only, for the reason `sha256`'s does the same: nothing a
    // caller drives reaches it, so two blocks back to back cannot build
    // a combinational path between one's `in_valid` and the other's
    // `in_ready`.
    assign in_ready  = (state_q == C_RUN);
    assign out_data  = out_q;
    assign out_valid = ov_q;
    assign active    = (state_q != C_IDLE);
    assign exhausted = exh_q;

    wire beat = in_valid && in_ready;
    // The keystream word in front: `advance` below rotates the next one
    // into place, so this is always the top of the core's block.
    wire [31:0] ks = core_block[511:480];
    // Rotate for the next word, except on the sixteenth — the core is
    // about to be reloaded, so where its rotation got to does not matter.
    wire        last_word = (widx_q == 4'd15);
    wire        advance = beat && !last_word;
    // RFC 8439 §2.3's counter is 32 bits wide, so this is the last block
    // there is under this key and nonce.
    wire        ctr_last = (ctr_q == 32'hFFFFFFFF);

    chacha20_core u_core (
        .clk     (clk),
        .rst_n   (rst_n),
        .start   (req_q),
        .advance (advance),
        .key     (key),
        .nonce   (nonce),
        .counter (ctr_q),
        .busy    (),
        .valid   (core_valid),
        .block   (core_block)
    );

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state_q <= C_IDLE;
            ctr_q   <= 32'd0;
            widx_q  <= 4'd0;
            out_q   <= 32'd0;
            ov_q    <= 1'b0;
            req_q   <= 1'b0;
            exh_q   <= 1'b0;
        end else begin
            ov_q  <= 1'b0;
            req_q <= 1'b0;
            if (start) begin
                ctr_q   <= counter;
                widx_q  <= 4'd0;
                req_q   <= 1'b1;
                exh_q   <= 1'b0;
                state_q <= C_FILL;
            end else begin
                case (state_q)
                    C_FILL: begin
                        // Not while `req_q` is still up: the core takes
                        // the request a cycle after it is made, so a
                        // `valid` seen in that cycle belongs to the block
                        // the request is replacing and not to the one it
                        // asked for.
                        if (core_valid && !req_q) state_q <= C_RUN;
                    end
                    C_RUN: begin
                        if (beat) begin
                            out_q <= in_data ^ ks;
                            ov_q  <= 1'b1;
                            if (last_word) begin
                                widx_q <= 4'd0;
                                if (ctr_last) begin
                                    // No block 2^32. Stop here rather
                                    // than wrap and repeat keystream.
                                    exh_q   <= 1'b1;
                                    state_q <= C_IDLE;
                                end else begin
                                    ctr_q   <= ctr_q + 32'd1;
                                    req_q   <= 1'b1;
                                    state_q <= C_FILL;
                                end
                            end else begin
                                widx_q <= widx_q + 4'd1;
                            end
                        end
                    end
                    default: begin
                        // C_IDLE: nothing happens until `start`.
                        state_q <= C_IDLE;
                    end
                endcase
            end
        end
    end
endmodule
