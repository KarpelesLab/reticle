// Verilog-1995 style port lists: names only, with the directions and
// types declared in the body.
module adder(a, b, cin, sum, cout);
    input  [3:0] a, b;
    input        cin;
    output [3:0] sum;
    output       cout;
    assign {cout, sum} = a + b + cin;
endmodule

module regs(clk, d, q);
    input clk;
    input [7:0] d;
    output [7:0] q;
    reg [7:0] q;
    always @(posedge clk) q <= d;
endmodule

module regs2(clk, d, q);
    input clk;
    input [7:0] d;
    output reg [7:0] q;
    always @(posedge clk) q <= d;
endmodule

module none;
endmodule

module empty_parens();
endmodule
