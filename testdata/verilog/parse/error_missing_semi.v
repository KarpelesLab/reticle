// Missing semicolons: each costs one diagnostic and the next item still
// parses.
module m(input a, output reg y);
    wire w = a
    wire v = ~a;
    reg r
    always @(a) begin
        y = w
        r = v;
    end
    assign y = r;
endmodule
