`timescale 1ns/1ps
module tb;
  for (genvar i = 0; i < 2; i++) begin : g
    localparam int IJ = i + 3;
  end
  initial begin
    $display("G4 IJ=%0d", g[1].IJ);
    $finish;
  end
endmodule
