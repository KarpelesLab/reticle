`timescale 1ns/1ps
module tb;
  logic [7:0] a, b, y, z;
  logic [7:0] mem [4];
  logic [1:0] idx;
  logic [7:0] rd;
  int runs = 0;
  function automatic logic [7:0] f(logic [7:0] x);
    return x + b;
  endfunction
  always_comb begin
    y = f(a);
    runs++;
  end
  always_comb z = a & 8'h0F;
  always_comb rd = mem[idx];
  initial begin
    #1 $display("AC1 runs=%0d y=%b z=%b", runs, y, z);
    a = 8'd1; b = 8'd2;
    #1 $display("AC2 y=%0d z=%0d runs=%0d", y, z, runs);
    b = 8'd10;
    #1 $display("AC3 y=%0d (function-read b) runs=%0d", y, runs);
    mem[0] = 8'h10; mem[1] = 8'h11; mem[2] = 8'h12; mem[3] = 8'h13;
    idx = 2;
    #1 $display("AC4 rd=%h", rd);
    mem[2] = 8'hEE;
    #1 $display("AC5 rd after mem write=%h", rd);
    $finish;
  end
endmodule
