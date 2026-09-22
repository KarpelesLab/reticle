// A small counter: exercises the everyday token mix.
module counter #(
    parameter WIDTH = 8
) (
    input  wire             clk,
    input  wire             rst_n,
    input  wire             en,
    output reg  [WIDTH-1:0] count
);
    /* Synchronous, active-low reset. */
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= {WIDTH{1'b0}};
        else if (en)
            count <= count + 1'b1;
    end

    wire overflow = &count;
    assign #1 done = overflow ? 1'b1 : 1'b0;
endmodule
