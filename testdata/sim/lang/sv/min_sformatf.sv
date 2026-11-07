`timescale 1ns/1ps
module tb;
  initial begin
    $display("M1 %s", $sformatf("v=%0d h=%h", 10, 8'hAB));
    $finish;
  end
endmodule
