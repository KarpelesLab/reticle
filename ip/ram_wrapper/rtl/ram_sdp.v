// ram_sdp — a portable simple dual-port synchronous RAM.
//
// What it does
//   DEPTH words of WIDTH bits with one write port and one read port, each
//   with its own clock. `wr_clk` writes `wr_data` at `wr_addr` when
//   `wr_en` is high; `rd_clk` presents the word at `rd_addr` on `rd_data`
//   one cycle later when `rd_en` is high, or two cycles later with
//   OUT_REG = 1.
//
//   Two clocks make this the shape a block RAM natively has and the one
//   an asynchronous FIFO's storage wants. Tie `wr_clk` and `rd_clk`
//   together for a same-clock dual-port RAM; nothing else changes.
//
// What it does not do
//   Simple dual port, not true dual port: the read port cannot write and
//   the write port cannot read, so a processor wanting two read/write
//   ports needs a different block. Reading an address in the cycle it is
//   written gives the *old* contents on a same-clock instance and, across
//   two clocks, whatever the device does — which is undefined, and is why
//   `fifo_async` never lets the two pointers meet. No byte enables, no
//   initial contents, no reset on the output and no collision flag.
module ram_sdp #(
    // Bits per word.
    parameter WIDTH      = 8,
    // Words.
    parameter DEPTH      = 256,
    // 1 adds an output register, so reads take two cycles.
    parameter OUT_REG    = 0,
    // Derived from DEPTH; do not override.
    parameter ADDR_WIDTH = $clog2(DEPTH)
) (
    input  wire                  wr_clk,
    input  wire                  wr_en,
    input  wire [ADDR_WIDTH-1:0] wr_addr,
    input  wire [WIDTH-1:0]      wr_data,

    input  wire                  rd_clk,
    input  wire                  rd_en,
    input  wire [ADDR_WIDTH-1:0] rd_addr,
    output wire [WIDTH-1:0]      rd_data
);
    reg [WIDTH-1:0] mem [0:DEPTH-1];
    reg [WIDTH-1:0] rd_data_q;

    always @(posedge wr_clk) begin
        if (wr_en) mem[wr_addr] <= wr_data;
    end

    always @(posedge rd_clk) begin
        if (rd_en) rd_data_q <= mem[rd_addr];
    end

    generate
        if (OUT_REG != 0) begin : g_pipelined
            reg [WIDTH-1:0] rd_data_q2;
            always @(posedge rd_clk) begin
                if (rd_en) rd_data_q2 <= rd_data_q;
            end
            assign rd_data = rd_data_q2;
        end else begin : g_direct
            assign rd_data = rd_data_q;
        end
    endgenerate
endmodule
