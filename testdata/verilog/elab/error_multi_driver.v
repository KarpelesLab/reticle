// Two procedural blocks driving the same register.
module error_multi_driver(input wire clk, input wire a, input wire b, output reg q);
    always @(posedge clk)
        q <= a;

    always @(posedge clk)
        q <= b;
endmodule
