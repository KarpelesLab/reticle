`timescale 1ns/1ps
module tb;
  real r;
  initial begin
    r = $sqrt(2.0);
    $display("M1 sqrt=%f pow=%f ln=%f exp=%f", r, $pow(2.0, 0.5), $ln(10.0), $exp(1.0));
    $finish;
  end
endmodule
