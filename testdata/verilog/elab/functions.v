// Functions inlined into the process that calls them.
module functions(
    input  wire       clk,
    input  wire [7:0] a,
    input  wire [7:0] b,
    output reg  [8:0] sum,
    output reg  [7:0] big
);
    function [8:0] add9;
        input [7:0] x, y;
        begin
            add9 = {1'b0, x} + {1'b0, y};
        end
    endfunction

    function [7:0] maximum;
        input [7:0] x, y;
        begin
            if (x > y)
                maximum = x;
            else
                maximum = y;
        end
    endfunction

    always @(posedge clk) begin
        sum <= add9(a, b);
        big <= maximum(a, b);
    end
endmodule
