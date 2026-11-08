// `force` and `release` of bit- and part-selects of a wire: two parts
// forced at once, one released while the other holds, a part forced over
// a whole force, and the whole released. (Of a variable they are refused,
// by xsim as by IEEE 1364-2005 §9.3.2.)
`timescale 1ns/1ps
module tb;
  reg  [7:0] src;
  wire [7:0] w;
  assign w = src;
  initial begin
    src = 8'h0f;
    #1 force w[7:6] = 2'b10; force w[1] = 1'b0;
    #1 $display("P1 w=%b", w);
    src = 8'hf0;
    #1 $display("P2 w=%b (forced bits hold)", w);
    release w[7:6];
    #1 $display("P3 w=%b (7:6 follow again)", w);
    release w[1];
    #1 $display("P4 w=%b", w);
    force w = 8'h55; force w[0] = 1'b0;
    #1 $display("P5 w=%b", w);
    release w[6:4];
    #1 $display("P6 w=%b", w);
    src = 8'h0a;
    #1 $display("P7 w=%b", w);
    release w;
    #1 $display("P8 w=%b", w);
    $finish;
  end
endmodule
