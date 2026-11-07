`timescale 1ns/1ps
module tb;
  logic [3:0] x = 4'd3;
  int i = -7;
  bit [7:0] b = 8'hA5;
  logic [7:0] dep = 8'd10 + 8'd5;
  logic [7:0] m [2] = '{8'h11, 8'h22};
  string s = "init";
  logic [3:0] y = x + 1;
  typedef enum {OFF, ON} st_t;
  st_t st = ON;
  initial begin
    $display("ID1 x=%0d i=%0d b=%h dep=%0d", x, i, b, dep);
    $display("ID2 m=%h %h s=%s st=%0d", m[0], m[1], s, st);
    $display("ID3 y=%0d", y);
    for (int k = 0; k < 2; k++) begin
      automatic int z = k * 5;
      $display("ID4 k=%0d z=%0d", k, z);
    end
    begin
      int local_v = 99;
      $display("ID5 local_v=%0d", local_v);
    end
    $finish;
  end
endmodule
