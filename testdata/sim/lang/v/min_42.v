`timescale 1ns/1ps
module tb;
  reg a, en0, en1;
  wire ob, o0, o1;
  buf (ob, a);
  bufif0 (o0, a, en0);
  bufif1 (o1, a, en1);
  initial begin
    a = 1'bz; en0 = 0; en1 = 1;
    #1 $display("buf(z)=%b bufif0(z,0)=%b bufif1(z,1)=%b", ob, o0, o1);
    $finish;
  end
endmodule
