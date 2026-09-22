// always_ff, always_comb and logic, with a synchronous reset.
module sv_always(
    input  logic       clk,
    input  logic       rst,
    input  logic [7:0] a,
    input  logic [7:0] b,
    output logic [7:0] q,
    output logic [7:0] sum
);
    logic [7:0] next;

    always_comb begin
        next = a + b;
    end

    always_ff @(posedge clk) begin
        if (rst) q <= '0;
        else     q <= next;
    end

    assign sum = next;
endmodule
