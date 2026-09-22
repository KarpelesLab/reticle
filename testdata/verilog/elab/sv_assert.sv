// Immediate assertions become IR assertions.
module sv_assert(
    input  logic clk,
    input  logic req,
    input  logic ack
);
    always_ff @(posedge clk) begin
        assert (!(req && ack)) else $error("req and ack at the same time");
        assert (req || !ack);
    end
endmodule
