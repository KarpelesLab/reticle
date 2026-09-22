// A synchronous FIFO whose output register is optional, selected with a
// generate block, plus a generate-for building a parity tree. Taken from
// the parser corpus: a real design must lint silently.
module sync_fifo #(
    parameter int WIDTH = 8,
    parameter int DEPTH = 16,
    parameter bit OUTPUT_REG = 1,
    localparam int AW = $clog2(DEPTH)
) (
    input  logic             clk,
    input  logic             rst,
    input  logic             wr_en,
    input  logic [WIDTH-1:0] wr_data,
    output logic             full,
    input  logic             rd_en,
    output logic [WIDTH-1:0] rd_data,
    output logic             empty,
    output logic [AW:0]      count,
    output logic             parity_err
);
    logic [WIDTH-1:0] mem [DEPTH];
    logic [AW:0] wr_ptr, rd_ptr;
    logic [WIDTH-1:0] rd_word;

    assign full  = (wr_ptr[AW] != rd_ptr[AW]) && (wr_ptr[AW-1:0] == rd_ptr[AW-1:0]);
    assign empty = (wr_ptr == rd_ptr);
    assign count = wr_ptr - rd_ptr;

    always_ff @(posedge clk) begin
        if (rst) begin
            wr_ptr <= '0;
            rd_ptr <= '0;
        end else begin
            if (wr_en && !full) begin
                mem[wr_ptr[AW-1:0]] <= wr_data;
                wr_ptr <= wr_ptr + 1'b1;
            end
            if (rd_en && !empty)
                rd_ptr <= rd_ptr + 1'b1;
        end
    end

    assign rd_word = mem[rd_ptr[AW-1:0]];

    generate
        if (OUTPUT_REG) begin : g_reg
            always_ff @(posedge clk) rd_data <= rd_word;
        end else begin : g_comb
            assign rd_data = rd_word;
        end
    endgenerate

    // Byte-wise parity of the write data, one XOR tree per byte.
    localparam int BYTES = (WIDTH + 7) / 8;
    logic [BYTES-1:0] parity;
    genvar b;
    generate
        for (b = 0; b < BYTES; b = b + 1) begin : g_parity
            localparam int LO = b * 8;
            localparam int HI = (LO + 7 < WIDTH) ? LO + 7 : WIDTH - 1;
            assign parity[b] = ^wr_data[HI:LO];
        end
    endgenerate

    assign parity_err = |parity;
endmodule
