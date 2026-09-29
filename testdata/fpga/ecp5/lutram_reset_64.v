// Distributed RAMs and flip-flops with **two different resets** in one
// design, which is the shape that made the router give up.
//
// An ECP5 logic tile has two set/reset wires, `LSR0` and `LSR1`, for its
// four slices — and a `TRELLIS_DPR16X4` spends `LSR1` on its write enable,
// because `WRE0_SLICE` and `WRE1_SLICE` are joined to it by a
// `.fixed_conn` with no mux to choose with. So a tile holding a RAM has
// **one** reset wire left, and a placer that does not know it will happily
// put flip-flops of two reset domains into one RAM tile. The router then
// cannot realise the placement and says so:
//
// ```text
// routing did not converge: 1 node(s) are still oversubscribed after
// 40 iteration(s), worst at X24Y3/LSR1 (2 signals)
// ```
//
// Four FIFOs of 64 words, 32 `TRELLIS_DPR16X4` and about ninety
// flip-flops between them: two reset from `rst_a_n` and two from
// `rst_b_n`, so the design has two reset nets for tiles that have one wire
// to spare. **Three FIFOs is not enough and sixteen RAMs at depth 32 is
// not enough** — both of those place and route without the rule, because
// the placer has room to keep the two domains apart by accident. That is
// worth knowing about this file: it is at the size where accident stops
// working.
//
// The data inputs differ per instance so synthesis cannot fold the four
// into one, and every output is combined so none of them is dead logic.
//
// This is **not** a board design: nothing has this on it, and the balls in
// the `.rcf` beside it are chosen only so the placement is the same on
// every run.
module lutram_reset_64 (
    input  wire       clk,
    input  wire       rst_a_n,
    input  wire       rst_b_n,
    input  wire       wr_en,
    input  wire [7:0] wr_data,
    input  wire       rd_en,
    output wire [7:0] rd_data,
    output wire       full,
    output wire       empty
);
    wire [7:0] q0, q1, q2, q3;
    wire f0, f1, f2, f3;
    wire e0, e1, e2, e3;

    fifo_sync #(.WIDTH(8), .DEPTH(64), .FWFT(0), .ADDR_WIDTH(6), .CNT_WIDTH(7)) a0 (
        .clk(clk), .rst_n(rst_a_n), .wr_en(wr_en), .wr_data(wr_data ^ 8'h01),
        .full(f0), .rd_en(rd_en), .rd_data(q0), .empty(e0), .count());
    fifo_sync #(.WIDTH(8), .DEPTH(64), .FWFT(0), .ADDR_WIDTH(6), .CNT_WIDTH(7)) a1 (
        .clk(clk), .rst_n(rst_a_n), .wr_en(wr_en), .wr_data(wr_data ^ 8'h02),
        .full(f1), .rd_en(rd_en), .rd_data(q1), .empty(e1), .count());
    fifo_sync #(.WIDTH(8), .DEPTH(64), .FWFT(0), .ADDR_WIDTH(6), .CNT_WIDTH(7)) b0 (
        .clk(clk), .rst_n(rst_b_n), .wr_en(wr_en), .wr_data(wr_data ^ 8'h04),
        .full(f2), .rd_en(rd_en), .rd_data(q2), .empty(e2), .count());
    fifo_sync #(.WIDTH(8), .DEPTH(64), .FWFT(0), .ADDR_WIDTH(6), .CNT_WIDTH(7)) b1 (
        .clk(clk), .rst_n(rst_b_n), .wr_en(wr_en), .wr_data(wr_data ^ 8'h08),
        .full(f3), .rd_en(rd_en), .rd_data(q3), .empty(e3), .count());

    assign rd_data = q0 ^ q1 ^ q2 ^ q3;
    assign full    = f0 | f1 | f2 | f3;
    assign empty   = e0 & e1 & e2 & e3;
endmodule
