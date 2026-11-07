`timescale 1ns/1ps
// sv_event_iff with each `iff` rewritten as an if inside the process.
module tb;
  logic clk = 0;
  logic en = 0;
  int cnt_all = 0, cnt_en = 0, cnt_neg = 0, cnt_edge = 0;
  always #5 clk = ~clk;
  always @(posedge clk) cnt_all++;
  always @(posedge clk) if (en) cnt_en++;
  always @(negedge clk) if (cnt_all > 2) cnt_neg++;
  always @(edge clk) cnt_edge++;
  initial begin
    #12 en = 1;
    #20 en = 0;
    #30;
    $display("EI1 all=%0d en=%0d neg=%0d edge=%0d", cnt_all, cnt_en, cnt_neg, cnt_edge);
    do @(posedge clk); while (cnt_all != 9);
    $display("EI2 t=%0t all=%0d", $time, cnt_all);
    wait (cnt_all == 11);
    $display("EI3 t=%0t", $time);
    $finish;
  end
endmodule
