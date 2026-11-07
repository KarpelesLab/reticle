`timescale 1ns/1ps
interface ch_if;
  logic [3:0] v;
endinterface
module drv #(parameter int K = 0) (ch_if c);
  assign c.v = 4'(K);
endmodule
module tb;
  ch_if ch [2] ();
  drv #(.K(3)) d0 (.c(ch[0]));
  drv #(.K(5)) d1 (.c(ch[1]));
  initial begin
    #1 $display("M1 ch0=%0d ch1=%0d", ch[0].v, ch[1].v);
    $finish;
  end
endmodule
