`timescale 1ns/1ps
module tb;
  reg e;
  wire pu;
  pullup (pu);
  assign pu = e ? 1'b0 : 1'bz;
  initial begin
    e = 0; #1 $display("released %b", pu);
    e = 1; #1 $display("driven %b", pu);
    $finish;
  end
endmodule
