// fifo_async — the classic gray-pointer asynchronous FIFO.
//
// What it does
//   Moves WIDTH-bit words from the `wr_clk` domain to the `rd_clk`
//   domain through DEPTH words of memory. The two clocks are unrelated:
//   any frequency, any phase, no common edge.
//
//   This is the design from Clifford Cummings, "Simulation and Synthesis
//   Techniques for Asynchronous FIFO Design" (SNUG 2002), and the reason
//   it is worth copying exactly is that every piece of it is load
//   bearing:
//
//   * Each pointer is one bit wider than an address, so `wr_ptr` and
//     `rd_ptr` equal means empty while a differing top bit means full.
//   * Each pointer is *also* kept gray coded. Exactly one bit of a gray
//     code changes per increment, so a pointer sampled by the other
//     domain mid-change is either the old value or the new one, never a
//     mixture. That is what makes the crossing safe, and it is why the
//     binary and gray forms are both registered rather than converted
//     combinationally on the far side.
//   * Each gray pointer crosses through its own two-flop synchroniser
//     (`cdc_sync`), so `full` and `empty` are each computed from a
//     pessimistic view of the other side: `full` may be asserted when the
//     reader has already advanced, `empty` when the writer has already
//     written, but never the other way round. Pessimism is safe; the FIFO
//     stalls a cycle or two early rather than losing a word.
//   * `full` and `empty` are registered, from the *next* pointer value,
//     so they are flip-flop outputs rather than a comparison in the
//     middle of a path — and so the enable that feeds the pointer
//     increment does not depend combinationally on the pointer.
//
//   `rd_data` is combinational out of the memory: the oldest word is on
//   it whenever `rd_empty` is low, and `rd_en` acknowledges it. That is
//   first-word-fall-through, which is what makes `rd_empty` the inverse
//   of a `valid`.
//
// What it does not do
//   DEPTH must be a power of two, at least four: the pointers wrap by
//   themselves, and the full comparison needs two pointer bits above the
//   addresses. There is no occupancy count — a count would have to be
//   computed from two pointers that are never simultaneously valid in one
//   domain, so anything reported would be a guess; ask the writer how
//   full it looks and the reader how empty, which is what `wr_full` and
//   `rd_empty` are.
//
//   There is no almost-full or almost-empty, no reset synchroniser (each
//   domain's `rst_n` is asynchronous and must be released in its own
//   domain — tie them together at your peril: one side out of reset while
//   the other is held leaves the synchronised pointers disagreeing), and
//   no protection against a memory whose write and read collide on the
//   same address, which cannot happen while `wr_full` and `rd_empty` are
//   obeyed.
//
//   Reticle's `timing::analyze_cdc` reports the pointer crossings as a
//   gray bus it *cannot verify*, and is right to: nothing in a netlist
//   says a value is gray coded. That the source is gray is this file's
//   responsibility, not the checker's.
module fifo_async #(
    // Bits per word.
    parameter WIDTH      = 8,
    // Words held. Must be a power of two, at least four.
    parameter DEPTH      = 16,
    // Derived from DEPTH; do not override.
    parameter ADDR_WIDTH = $clog2(DEPTH)
) (
    input  wire             wr_clk,
    input  wire             wr_rst_n,
    input  wire             wr_en,
    input  wire [WIDTH-1:0] wr_data,
    output wire             wr_full,

    input  wire             rd_clk,
    input  wire             rd_rst_n,
    input  wire             rd_en,
    output wire [WIDTH-1:0] rd_data,
    output wire             rd_empty
);
    localparam AW = ADDR_WIDTH;

    reg [WIDTH-1:0] mem [0:DEPTH-1];

    // Write domain.
    reg  [AW:0] wr_bin;
    reg  [AW:0] wr_gray;
    reg         wr_full_q;
    wire [AW:0] rd_gray_in_wr;

    wire [AW:0] wr_bin_next  = wr_bin + {{AW{1'b0}}, (wr_en && !wr_full_q)};
    wire [AW:0] wr_gray_next = wr_bin_next ^ (wr_bin_next >> 1);

    // Full when the next write pointer would reach the read pointer from
    // behind: same address, both top bits inverted in gray.
    wire wr_full_next =
        (wr_gray_next == {~rd_gray_in_wr[AW:AW-1], rd_gray_in_wr[AW-2:0]});

    assign wr_full = wr_full_q;

    always @(posedge wr_clk or negedge wr_rst_n) begin
        if (!wr_rst_n) begin
            wr_bin    <= {(AW+1){1'b0}};
            wr_gray   <= {(AW+1){1'b0}};
            wr_full_q <= 1'b0;
        end else begin
            wr_bin    <= wr_bin_next;
            wr_gray   <= wr_gray_next;
            wr_full_q <= wr_full_next;
        end
    end

    always @(posedge wr_clk) begin
        if (wr_en && !wr_full_q) mem[wr_bin[AW-1:0]] <= wr_data;
    end

    // Read domain.
    reg  [AW:0] rd_bin;
    reg  [AW:0] rd_gray;
    reg         rd_empty_q;
    wire [AW:0] wr_gray_in_rd;

    wire [AW:0] rd_bin_next  = rd_bin + {{AW{1'b0}}, (rd_en && !rd_empty_q)};
    wire [AW:0] rd_gray_next = rd_bin_next ^ (rd_bin_next >> 1);

    wire rd_empty_next = (rd_gray_next == wr_gray_in_rd);

    assign rd_empty = rd_empty_q;
    assign rd_data  = mem[rd_bin[AW-1:0]];

    always @(posedge rd_clk or negedge rd_rst_n) begin
        if (!rd_rst_n) begin
            rd_bin     <= {(AW+1){1'b0}};
            rd_gray    <= {(AW+1){1'b0}};
            rd_empty_q <= 1'b1;
        end else begin
            rd_bin     <= rd_bin_next;
            rd_gray    <= rd_gray_next;
            rd_empty_q <= rd_empty_next;
        end
    end

    // The two crossings, each a bank of two-flop synchronisers.
    cdc_sync #(
        .WIDTH  (AW + 1),
        .STAGES (2),
        .INIT   (0)
    ) u_wr_gray_to_rd (
        .clk   (rd_clk),
        .rst_n (rd_rst_n),
        .d     (wr_gray),
        .q     (wr_gray_in_rd)
    );

    cdc_sync #(
        .WIDTH  (AW + 1),
        .STAGES (2),
        .INIT   (0)
    ) u_rd_gray_to_wr (
        .clk   (wr_clk),
        .rst_n (wr_rst_n),
        .d     (rd_gray),
        .q     (rd_gray_in_wr)
    );
endmodule
