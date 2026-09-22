// A parameterised FIFO with a generate-selected output register,
// instantiated twice with different parameters so two variants are
// elaborated.
module small_fifo #(
    parameter int WIDTH      = 8,
    parameter int DEPTH      = 4,
    parameter bit OUTPUT_REG = 1,
    localparam int AW        = $clog2(DEPTH)
) (
    input  logic             clk,
    input  logic             rst,
    input  logic             push,
    input  logic [WIDTH-1:0] din,
    input  logic             pop,
    output logic [WIDTH-1:0] dout,
    output logic [AW:0]      level
);
    logic [WIDTH-1:0] mem [DEPTH];
    logic [AW:0]      wr_ptr, rd_ptr;
    logic [WIDTH-1:0] word;

    assign level = wr_ptr - rd_ptr;
    assign word  = mem[rd_ptr[AW-1:0]];

    always_ff @(posedge clk) begin
        if (rst) begin
            wr_ptr <= '0;
            rd_ptr <= '0;
        end else begin
            if (push) begin
                mem[wr_ptr[AW-1:0]] <= din;
                wr_ptr <= wr_ptr + 1'b1;
            end
            if (pop)
                rd_ptr <= rd_ptr + 1'b1;
        end
    end

    generate
        if (OUTPUT_REG) begin : g_reg
            always_ff @(posedge clk) dout <= word;
        end else begin : g_comb
            assign dout = word;
        end
    endgenerate
endmodule

module fifo_two(
    input  logic        clk,
    input  logic        rst,
    input  logic        push,
    input  logic        pop,
    input  logic [7:0]  narrow_in,
    input  logic [15:0] wide_in,
    output logic [7:0]  narrow_out,
    output logic [15:0] wide_out,
    output logic [2:0]  narrow_level,
    output logic [3:0]  wide_level
);
    small_fifo #(.WIDTH(8), .DEPTH(4), .OUTPUT_REG(1)) u_narrow (
        .clk   (clk),
        .rst   (rst),
        .push  (push),
        .din   (narrow_in),
        .pop   (pop),
        .dout  (narrow_out),
        .level (narrow_level)
    );

    small_fifo #(.WIDTH(16), .DEPTH(8), .OUTPUT_REG(0)) u_wide (
        .clk   (clk),
        .rst   (rst),
        .push  (push),
        .din   (wide_in),
        .pop   (pop),
        .dout  (wide_out),
        .level (wide_level)
    );
endmodule
