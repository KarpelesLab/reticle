// A top level that asks `ip/fifo_sync` for DEPTH = 64 words, so a test can
// take the library block through place, route and a bitstream at a fixed
// depth. Verilog-2005 has no way to override a parameter from outside the
// source and `tests/fpga_trellis.rs` elaborates with the defaults, so the
// override lives here.
//
// Every port is brought straight out. That is the point: the FIFO's memory
// becomes `TRELLIS_DPR16X4` cells, 8 of them, and nothing else in
// the design may absorb them.
module fifo_sync_64 (
    input  wire       clk,
    input  wire       rst_n,
    input  wire       wr_en,
    input  wire [7:0] wr_data,
    output wire       full,
    input  wire       rd_en,
    output wire [7:0] rd_data,
    output wire       empty,
    output wire [6:0] count
);
    fifo_sync #(
        .WIDTH(8),
        .DEPTH(64),
        .FWFT(0),
        .ADDR_WIDTH(6),
        .CNT_WIDTH(7)
    ) fifo (
        .clk(clk),
        .rst_n(rst_n),
        .wr_en(wr_en),
        .wr_data(wr_data),
        .full(full),
        .rd_en(rd_en),
        .rd_data(rd_data),
        .empty(empty),
        .count(count)
    );
endmodule
