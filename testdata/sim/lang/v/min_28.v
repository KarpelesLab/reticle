`timescale 1ns/1ps
module tb;
  reg [7:0] q;
  initial begin
    q = 0;
    assign q = 8'hAA;
    #1 $display("assign q=%h", q);
    q = 8'h11;
    #1 $display("blocked q=%h", q);
    deassign q;
    q = 8'h22;
    #1 $display("deassign q=%h", q);
    $finish;
  end
endmodule
