`timescale 1ns/1ps
module leaf;
  reg [7:0] val;
  integer hits;
  initial begin val = 8'd1; hits = 0; end
  task bump; begin hits = hits + 1; val = val + 1; end endtask
  function [7:0] twice(input [7:0] x); twice = 2 * x; endfunction
endmodule
module mid;
  leaf l0 ();
  leaf l1 ();
  wire [7:0] sum = l0.val + l1.val;
  initial #1 l1.val = 8'd10;
endmodule
module tb;
  mid m ();
  reg [7:0] r;
  initial begin
    #2;
    $display("m.l0.val=%0d m.l1.val=%0d m.sum=%0d", m.l0.val, m.l1.val, m.sum);
    m.l0.val = 8'd50;
    #1 $display("after write m.sum=%0d", m.sum);
    m.l0.bump; m.l0.bump;
    $display("bump hits=%0d val=%0d", m.l0.hits, m.l0.val);
    r = m.l1.twice(8'd21);
    $display("twice %0d", r);
    $display("upward %0d", tb.m.l1.val);
    begin : named
      reg [3:0] local_r;
      local_r = 4'd7;
      $display("named.local_r=%0d", named.local_r);
      $display("tb.named.local_r=%0d", tb.named.local_r);
    end
    $finish;
  end
endmodule
