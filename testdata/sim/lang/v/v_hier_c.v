`timescale 1ns/1ps
module leaf;
  reg [7:0] val;
  integer hits;
  initial begin val = 8'd1; hits = 0; end
endmodule
module mid;
  leaf l0 ();
  leaf l1 ();
  wire [7:0] sum = l0.val + l1.val;
endmodule
module tb;
  mid m ();
  reg [7:0] r;
  initial begin
    #2;
    $display("m.l0.val=%0d m.l1.val=%0d m.sum=%0d", m.l0.val, m.l1.val, m.sum);
    #1 $display("after write m.sum=%0d", m.sum);
    $finish;
  end
endmodule
