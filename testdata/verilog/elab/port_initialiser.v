// top: ansi
// An initialiser on an output variable port is its time-zero value, as
// the same initialiser on a body variable is. Both header styles: the
// simulator once dropped it, so `v` read z until first assigned.
module ansi (
    input  wire       clk,
    input  wire       d,
    output reg        v = 1'b1,
    output reg  [3:0] q = 4'd9
);
    non_ansi u (.clk(clk), .d(d));
    always @(posedge clk) begin
        v <= d;
        q <= {3'd0, d};
    end
endmodule

module non_ansi (clk, d, a);
    input clk;
    input d;
    output reg [3:0] a = 4'd5;
    always @(posedge clk) a <= {d, 3'd0};
endmodule
