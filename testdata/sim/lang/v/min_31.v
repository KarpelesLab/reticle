`timescale 1ns/1ps
module leaf;
  reg [7:0] val;
  initial val = 8'd1;
endmodule
module mid;
  leaf l0 ();
endmodule
module tb;
  mid m ();
  initial begin
    #1 $display("m.l0.val=%0d", m.l0.val);
    $display("tb.m.l0.val=%0d", tb.m.l0.val);
    begin : named
      reg [3:0] local_r;
      local_r = 4'd7;
      $display("named.local_r=%0d", named.local_r);
    end
    $finish;
  end
endmodule
