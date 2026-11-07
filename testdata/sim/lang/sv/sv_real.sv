`timescale 1ns/1ps
module tb;
  real r, q;
  shortreal sr;
  realtime rt;
  int i;
  initial begin
    r = 3.75;
    $display("RE1 r=%f %0.2f %e %g", r, r, r, r);
    i = r;
    $display("RE2 int from 3.75 = %0d", i);
    r = -2.5;
    i = r;
    $display("RE3 int from -2.5 = %0d", i);
    q = r * 4 + 1;
    $display("RE4 q=%f", q);
    r = 1.0 / 3;
    $display("RE5 1/3=%.6f", r);
    i = 7;
    r = i / 2;
    $display("RE6 int div to real %f", r);
    r = i / 2.0;
    $display("RE7 real div %f", r);
    $display("RE8 $rtoi(2.9)=%0d $itor(5)=%f", $rtoi(2.9), $itor(5));
    $display("RE9 floor ceil %f %f", $floor(2.7), $ceil(2.1));
    $display("RE10 sqrt pow ln %f %f %f", $sqrt(2.0), $pow(2.0, 0.5), $ln(10.0));
    sr = 1.5;
    $display("RE11 shortreal %f", sr);
    rt = 12.5;
    $display("RE12 realtime %f", rt);
    $display("RE13 compare %b %b", 2.5 > 2, 0.1 + 0.2 == 0.3);
    r = 1e10;
    $display("RE14 big %e %0d", r, $rtoi(r / 1e5));
    $finish;
  end
endmodule
