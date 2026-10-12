// Which address does a `DP16KD` in a narrow mode actually write, and what
// does it keep there?
//
// **Why this exists, and what it settled.** `src/fpga/devices/ecp5.dev`
// described the 4-, 2- and 1-bit layouts as Reticle's reading of how the
// narrow modes use an 18 kbit array rather than anything measured, and for
// a day the 4-bit one was recorded as wrong on a part and all three were
// quarantined. That was an error -- the comparison behind it put the
// 4-bit build on a part that had just run a 123-case corpus and the 9-bit
// build on a fresh load, and the design in question fails according to
// what decoded before it. This is the measurement that should have been
// taken first.
//
// **CHECKED on a Cynthion, 13 October 2026: all three modes are right.**
// A memory of each width deep enough to fill exactly one block --
// 16384 x 1, 8192 x 2, 4096 x 4 -- answers both questions below
// perfectly: every one-hot write lands at the address asked for and
// nowhere else, and every address holds every value unpermuted. So
// `addr 1`, `addr 2`, the `DATA_WIDTH_A` encodings and the data pins are
// all correct. `docs/fpga-trellis.md` has the table and the mistake.
//
//     host -> device   cmd addr_hi addr_lo
//     device -> host   one byte per address, each a zero-extended word
//
//     cmd 0        write all ones at `addr` and nothing else
//     cmd 1        write `a[WIDTH-1:0]` at every address `a`
//     cmd 2 + v    write the value `v` at `addr` and nothing else
//
// **Two questions, because neither alone is enough.** A one-hot write is
// the only way to see the address: a word this narrow has at most sixteen
// values, so any pattern repeats within sixteen addresses and a read that
// came back shifted or aliased would look like a read that came back
// right. Writing one address and dumping the whole block cannot be
// confused that way. But every bit of an all-ones word is the same, so a
// one-hot probe cannot see a data-pin permutation -- which is what the
// fill pass is for, giving each address its own low bits and so exercising
// every value at once.
//
// What a wrong answer would have meant, kept because it is the reason the
// probe is shaped like this:
//
//   * **one byte set, at a different address.** The word address reaches
//     the wrong pins -- the `addr` shift is wrong, and the difference
//     between the two addresses is the error.
//   * **several bytes set, at a fixed stride.** Fewer address pins reach
//     the array than the mode needs, so addresses alias. The stride is
//     2^(pins that do reach).
//   * **no byte set, or all of them.** The block is not in the mode the
//     parameters asked for, which would point at `DATA_WIDTH_A` rather
//     than the address.
//   * **the fill pass permuted.** The data pins are not the ones
//     `init low`'s first word names, and that list is the fix.
//
// Building this needs every `mode` line reachable, which they now are.
//
// What this would catch: an address alignment or aliasing fault, a mode
// that is not configured at all, and a data-pin permutation. What it would
// not, and this is the gap that remains: **the initial contents.**
// `init low` is only read for a memory that has them, and this one writes
// at run time, so where a word sits in a row of `INITVAL_nn` in these
// three modes is still a reading. Settling that wants this probe with
// contents instead of writes, and nothing in this repository builds a ROM
// in a narrow mode today.

module bram_probe #(
    // The mode follows the width: a memory `WIDTH` bits wide and deep
    // enough to fill one block reaches the mode of that width on block
    // count alone.
    //
    //     WIDTH 1, ADDR_BITS 14    1-bit mode, 1 block (9-bit needs 8)
    //     WIDTH 2, ADDR_BITS 13    2-bit mode, 1 block (9-bit needs 4)
    //     WIDTH 4, ADDR_BITS 12    4-bit mode, 1 block (9-bit needs 2)
    parameter integer WIDTH     = 4,
    parameter integer ADDR_BITS = 12
) (
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
    localparam integer DEPTH = 1 << ADDR_BITS;

    reg [WIDTH-1:0] mem [0:DEPTH-1];
    reg [WIDTH-1:0] rd_q;

    localparam [2:0] S_CMD  = 3'd0, S_HI  = 3'd1, S_LO   = 3'd2,
                     S_ZERO = 3'd3, S_SET = 3'd4, S_DUMP = 3'd5;

    reg [2:0]           state  = S_CMD;
    reg [ADDR_BITS-1:0] target = {ADDR_BITS{1'b0}};
    reg [ADDR_BITS-1:0] scan   = {ADDR_BITS{1'b0}};
    reg [7:0]           cmd    = 8'd0;
    reg [7:0]           hi     = 8'd0;
    reg                 armed  = 1'b0;

    // `cmd 1` writes each address its own low nibble, so the zeroing pass
    // is the filling pass and there is no one-hot write after it.
    wire filling = (cmd == 8'd1);
    // `cmd 2 + v` writes the nibble `v`; `cmd 0` writes all ones.
    wire [WIDTH-1:0] one_hot_value = (cmd >= 8'd2)
                                     ? (cmd[WIDTH-1:0] - 2)
                                     : {WIDTH{1'b1}};

    wire want_cmd = (state == S_CMD) || (state == S_HI) || (state == S_LO);
    assign host_ready = want_cmd;
    wire took = host_valid & host_ready;

    // One write port, its address and data muxed outside the block: an
    // `if`/`else if` over two assignments becomes two `$memwr` cells, and
    // three ports on a block that has two is refused by name.
    wire                 wr_en   = (state == S_ZERO) || (state == S_SET);
    wire [ADDR_BITS-1:0] wr_addr = (state == S_ZERO) ? scan : target;
    wire [WIDTH-1:0]     wr_d    = (state == S_ZERO)
                                   ? (filling ? scan[WIDTH-1:0] : {WIDTH{1'b0}})
                                   : one_hot_value;

    // Gated, so the value the issuing cycle captured holds while the host
    // has not taken the byte yet. `bram_rw.v`'s header says what an
    // ungated read costs.
    wire rd_en = (state == S_DUMP) && !armed;

    assign dev_data   = {{(8 - WIDTH){1'b0}}, rd_q};
    assign dev_valid  = (state == S_DUMP) && armed;
    assign dev_commit = dev_valid;
    wire gave = dev_valid & dev_ready;

    always @(posedge clk) begin
        if (wr_en) mem[wr_addr] <= wr_d;
        if (rd_en) rd_q <= mem[scan];
    end

    always @(posedge clk) begin
        if (!rst_n) begin
            state  <= S_CMD;
            target <= {ADDR_BITS{1'b0}};
            scan   <= {ADDR_BITS{1'b0}};
            hi     <= 8'd0;
            armed  <= 1'b0;
        end else case (state)
            S_CMD:
                if (took) begin
                    cmd   <= host_data;
                    state <= S_HI;
                end
            S_HI:
                if (took) begin
                    hi    <= host_data;
                    state <= S_LO;
                end
            S_LO:
                if (took) begin
                    target <= {hi, host_data}[ADDR_BITS-1:0];
                    scan   <= {ADDR_BITS{1'b0}};
                    state  <= S_ZERO;
                end
            S_ZERO: begin
                scan <= scan + 1'b1;
                if (scan == {ADDR_BITS{1'b1}}) begin
                    scan  <= {ADDR_BITS{1'b0}};
                    armed <= 1'b0;
                    // A fill pass has already written every address, so
                    // there is no single write to do after it.
                    state <= filling ? S_DUMP : S_SET;
                end
            end
            S_SET: begin
                scan  <= {ADDR_BITS{1'b0}};
                armed <= 1'b0;
                state <= S_DUMP;
            end
            S_DUMP:
                if (!armed) begin
                    armed <= 1'b1;
                end else if (gave) begin
                    armed <= 1'b0;
                    if (scan == {ADDR_BITS{1'b1}}) state <= S_CMD;
                    else                           scan  <= scan + 1'b1;
                end
            default: state <= S_CMD;
        endcase
    end
endmodule
