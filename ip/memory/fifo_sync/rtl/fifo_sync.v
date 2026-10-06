// fifo_sync — a synchronous FIFO with full, empty and an occupancy count.
//
// What it does
//   DEPTH words of WIDTH bits in one clock domain. A word is written on a
//   rising `clk` edge when `wr_en` is high and `full` is low, and removed
//   when `rd_en` is high and `empty` is low; a write and a read in the
//   same cycle both happen. `count` is the number of words held, from 0
//   to DEPTH inclusive, which is why it is one bit wider than an address.
//
//   FWFT selects the read interface:
//
//     FWFT = 0  the classic FIFO. `rd_en` pops a word and `rd_data` holds
//               it from the *next* cycle, because the read is registered.
//               That is the variant to give a block RAM: the memory has
//               one read address and one clocked destination for it.
//
//     FWFT = 1  first word fall through. The oldest word is already on
//               `rd_data` whenever `empty` is low, with no `rd_en` needed
//               to fetch it; `rd_en` acknowledges it and the next word
//               appears in the following cycle. The read is combinational
//               out of the memory, so this variant wants distributed
//               (LUT) RAM, and it is the one to use in front of an
//               AXI-Stream or ready/valid consumer, where `empty` is the
//               inverse of `valid`.
//
// What it does not do
//   DEPTH must be a power of two: the pointers wrap by themselves and
//   there is no modulo. It does not cross clock domains — `fifo_async` is
//   that block. It has no almost-full or almost-empty threshold, no
//   programmable watermark and no data-count output for the other side
//   (there is only one side). `wr_en` while `full` and `rd_en` while
//   `empty` are ignored rather than reported: there is no overflow or
//   underflow flag, so a producer that cannot take back-pressure needs
//   one built around it.
//
//   ADDR_WIDTH and CNT_WIDTH are derived from DEPTH. Verilog-2005 has no
//   way to hide a parameter, so they are listed; never override them.
module fifo_sync #(
    // Bits per word.
    parameter WIDTH      = 8,
    // Words held. Must be a power of two, at least two.
    parameter DEPTH      = 16,
    // 1 selects first word fall through, 0 the classic read port.
    parameter FWFT       = 0,
    // Derived from DEPTH; do not override.
    parameter ADDR_WIDTH = $clog2(DEPTH),
    // Derived from DEPTH; do not override.
    parameter CNT_WIDTH  = $clog2(DEPTH) + 1
) (
    input  wire                 clk,
    input  wire                 rst_n,

    input  wire                 wr_en,
    input  wire [WIDTH-1:0]     wr_data,
    output wire                 full,

    input  wire                 rd_en,
    output wire [WIDTH-1:0]     rd_data,
    output wire                 empty,

    // Words held, 0 to DEPTH.
    output wire [CNT_WIDTH-1:0] count
);
    reg [WIDTH-1:0]     mem [0:DEPTH-1];
    reg [CNT_WIDTH-1:0] wr_ptr;
    reg [CNT_WIDTH-1:0] rd_ptr;

    wire [ADDR_WIDTH-1:0] wr_addr = wr_ptr[ADDR_WIDTH-1:0];
    wire [ADDR_WIDTH-1:0] rd_addr = rd_ptr[ADDR_WIDTH-1:0];

    wire do_write = wr_en && !full;
    wire do_read  = rd_en && !empty;

    // One extra pointer bit distinguishes full from empty: the addresses
    // are equal in both cases, the wrap bit differs only when full.
    assign empty = (wr_ptr == rd_ptr);
    assign full  = (wr_addr == rd_addr) && (wr_ptr[ADDR_WIDTH] != rd_ptr[ADDR_WIDTH]);
    assign count = wr_ptr - rd_ptr;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            wr_ptr <= {CNT_WIDTH{1'b0}};
            rd_ptr <= {CNT_WIDTH{1'b0}};
        end else begin
            if (do_write) wr_ptr <= wr_ptr + 1'b1;
            if (do_read)  rd_ptr <= rd_ptr + 1'b1;
        end
    end

    always @(posedge clk) begin
        if (do_write) mem[wr_addr] <= wr_data;
    end

    generate
        if (FWFT != 0) begin : g_fwft
            // The word at the read pointer is always on the output.
            assign rd_data = mem[rd_addr];
        end else begin : g_registered
            reg [WIDTH-1:0] rd_data_q;
            always @(posedge clk or negedge rst_n) begin
                if (!rst_n)        rd_data_q <= {WIDTH{1'b0}};
                else if (do_read)  rd_data_q <= mem[rd_addr];
            end
            assign rd_data = rd_data_q;
        end
    endgenerate
endmodule
