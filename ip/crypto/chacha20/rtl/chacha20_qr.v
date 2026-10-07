// chacha20_qr — one ChaCha quarter round, combinational.
//
// What it does
//   RFC 8439 §2.1, exactly as the specification writes it:
//
//       a += b; d ^= a; d <<<= 16;
//       c += d; b ^= c; b <<<= 12;
//       a += b; d ^= a; d <<<= 8;
//       c += d; b ^= c; b <<<= 7;
//
//   where `+` is addition modulo 2^32 and `<<<` is a left rotation.
//   Nothing is clocked and nothing is stored; four words in, four words
//   out.
//
// Why it is its own module
//   Because RFC 8439 §2.1.1 is a test vector for *this* function, and a
//   module is the only thing a testbench can reach. `tests/ip_crypto.rs`
//   drives §2.1.1's four numbers straight into these ports and compares
//   the four that come out, which localises a fault to the rotation
//   amounts and the addition order rather than to "the keystream is
//   wrong".
//
//   `chacha20_core` instantiates four of these per round. That is the
//   area/throughput decision of this package and the argument is in that
//   file's header: the four quarter-rounds of a ChaCha round touch
//   disjoint words, so four copies run in the same cycle with the same
//   critical path one copy would have, and a round takes one cycle
//   instead of four.
//
// What it does not do
//   No pipeline register. The path through it is four 32-bit additions
//   deep — a += b, c += d, a += b, c += d — and that is the critical
//   path of `chacha20_core`. Breaking it in half would double the cycles
//   per block; that trade is `chacha20_core`'s to state and it does not
//   take it either.
//
//   The rotations are written as concatenations and not as shift-or
//   pairs, so each is wiring and no width inference has to be trusted.
module chacha20_qr (
    input  wire [31:0] a_in,
    input  wire [31:0] b_in,
    input  wire [31:0] c_in,
    input  wire [31:0] d_in,

    output wire [31:0] a_out,
    output wire [31:0] b_out,
    output wire [31:0] c_out,
    output wire [31:0] d_out
);
    // a += b; d ^= a; d <<<= 16;
    wire [31:0] a1 = a_in + b_in;
    wire [31:0] dx = d_in ^ a1;
    wire [31:0] d1 = {dx[15:0], dx[31:16]};

    // c += d; b ^= c; b <<<= 12;
    wire [31:0] c1 = c_in + d1;
    wire [31:0] bx = b_in ^ c1;
    wire [31:0] b1 = {bx[19:0], bx[31:20]};

    // a += b; d ^= a; d <<<= 8;
    wire [31:0] a2 = a1 + b1;
    wire [31:0] dy = d1 ^ a2;
    wire [31:0] d2 = {dy[23:0], dy[31:24]};

    // c += d; b ^= c; b <<<= 7;
    wire [31:0] c2 = c1 + d2;
    wire [31:0] by = b1 ^ c2;
    wire [31:0] b2 = {by[24:0], by[31:25]};

    assign a_out = a2;
    assign b_out = b2;
    assign c_out = c2;
    assign d_out = d2;
endmodule
