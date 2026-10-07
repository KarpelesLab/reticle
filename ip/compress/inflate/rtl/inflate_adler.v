// inflate_adler — the Adler-32 checksum of RFC 1950 §9.
//
// What it does
//   Two sixteen-bit sums over a byte stream, each reduced modulo 65521:
//   `a` starts at one and accumulates the bytes, `b` accumulates `a`.
//   `sum` is the pair as RFC 1950 §9 writes it — `b` in the high half,
//   `a` in the low half — which is also the order the four trailer bytes
//   appear in on the wire, most significant first.
//
//   One byte a cycle, no back-pressure and no handshake: `in_valid` says
//   there is a byte and the sums move. `start` returns to a = 1, b = 0.
//
// Why it is its own module
//   For the same reason `chacha20_qr` is: RFC 1950 §9 is a *testable*
//   statement on its own, and a module boundary is what lets a test drive
//   it. `inflate_adler_matches_rfc_1950` feeds it byte strings and
//   compares against the sums computed in Rust, so a wrong reduction is
//   found here rather than as a mysterious `E_ADLER` out of a whole
//   decompressor.
//
// Why one conditional subtract is enough
//   65521 is prime and the reduction is the usual trick, but the bound is
//   worth writing down because the registers' widths depend on it.
//
//   `a` is always less than 65521 and a byte is at most 255, so
//   `a + byte` is at most 65775 — less than 2 x 65521 = 131042 — and one
//   subtract of 65521 brings it back under. `b` is also less than 65521
//   and so is the reduced `a`, so `b + a` is at most 131040, again under
//   131042, and again one subtract is enough. Both sums fit in seventeen
//   bits, and both registers hold values under 65521 and so are sixteen
//   bits wide and not seventeen.
//
// What it does not do
//   No CRC-32, which is what a gzip (RFC 1952) or a zip trailer carries;
//   that is a different algorithm with a different table and belongs to
//   whatever block parses those containers. No byte-count: RFC 1950's
//   trailer is the checksum alone, and the caller that wants a length
//   counts its own output.
module inflate_adler (
    input  wire        clk,
    input  wire        rst_n,

    // Return to a = 1, b = 0, which is RFC 1950 §9's initial value and
    // the checksum of the empty byte string.
    input  wire        start,

    input  wire [7:0]  in_byte,
    input  wire        in_valid,

    // {b, a}, updated on the edge that takes a byte.
    output wire [31:0] sum
);
    // RFC 1950 §9: "BASE is the largest prime smaller than 65536".
    localparam [16:0] BASE = 17'd65521;

    reg [15:0] a_q;
    reg [15:0] b_q;

    wire [16:0] a_add = {1'b0, a_q} + {9'd0, in_byte};
    wire [16:0] a_mod = (a_add >= BASE) ? (a_add - BASE) : a_add;
    wire [16:0] b_add = {1'b0, b_q} + a_mod;
    wire [16:0] b_mod = (b_add >= BASE) ? (b_add - BASE) : b_add;

    assign sum = {b_q, a_q};

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            a_q <= 16'd1;
            b_q <= 16'd0;
        end else if (start) begin
            a_q <= 16'd1;
            b_q <= 16'd0;
        end else if (in_valid) begin
            a_q <= a_mod[15:0];
            b_q <= b_mod[15:0];
        end
    end
endmodule
