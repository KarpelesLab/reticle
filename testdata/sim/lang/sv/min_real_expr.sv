`timescale 1ns/1ps
module tb;
  real r, s;
  initial begin
    $display("M1 direct: sqrt=%f ln=%f exp=%f", $sqrt(2.0), $ln(10.0), $exp(1.0));
    r = $ln(10.0);
    s = $exp(1.0);
    $display("M2 via var: ln=%f exp=%f", r, s);
    r = real'(7) / 2;
    $display("M3 r = real'(7)/2 -> %f", r);
    r = $itor(7) / 2;
    $display("M4 r = $itor(7)/2 -> %f", r);
    r = $sqrt(2.0) * 2;
    $display("M5 r = sqrt(2)*2 -> %f", r);
    $finish;
  end
endmodule
