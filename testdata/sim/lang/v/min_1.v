`timescale 1ns/1ps
module tb;
  reg signed [3:0] s; reg [3:0] u; reg [7:0] w; reg signed [7:0] sw;
  initial begin
    s = -4'sd1; u = 4'd15;
    w = s + u;   $display("a %h", w);
    sw = s + u;  $display("b %h", sw);
    w = s + 1'b1; $display("c %h", w);
    w = u + u;   $display("d %h", w);
    w = s + s;   $display("e %h", w);
    w = s;       $display("f %h", w);
    w = s + 4'd0; $display("g %h", w);
    $finish;
  end
endmodule
