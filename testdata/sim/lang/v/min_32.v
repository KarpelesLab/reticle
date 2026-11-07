`timescale 1ns/1ps
module leaf;
  reg [7:0] val;
  integer hits;
  initial begin val = 8'd1; hits = 0; end
  task bump; hits = hits + 1; endtask
  function [7:0] twice(input [7:0] x); twice = 2 * x; endfunction
endmodule
module tb;
  leaf l ();
  initial begin
    #1 l.val = 8'd50;
    $display("write l.val=%0d", l.val);
    l.bump;
    $display("bump hits=%0d", l.hits);
    $display("twice %0d", l.twice(8'd21));
    $finish;
  end
endmodule
