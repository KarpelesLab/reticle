// A task inlined into the process that calls it, with an output
// argument copied back.
module tasks(
    input  wire       clk,
    input  wire [7:0] d,
    output reg  [7:0] q,
    output reg        parity
);
    task compute;
        input  [7:0] value;
        output [7:0] result;
        output       odd;
        begin
            result = value + 8'd1;
            odd    = ^value;
        end
    endtask

    always @(posedge clk) begin
        compute(d, q, parity);
    end
endmodule
