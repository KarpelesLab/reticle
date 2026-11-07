`timescale 1ns/1ps
module tb;
  reg e, d;
  triand ta;
  trior to;
  tri0 t0;
  tri1 t1;
  assign ta = d; assign ta = 1'b1;
  assign to = d; assign to = 1'b0;
  assign t0 = e ? d : 1'bz;
  assign t1 = e ? d : 1'bz;
  initial begin
    e = 0; d = 1; #1 $display("ta=%b to=%b t0=%b t1=%b", ta, to, t0, t1);
    e = 1; d = 0; #1 $display("ta=%b to=%b t0=%b t1=%b", ta, to, t0, t1);
    $finish;
  end
endmodule
