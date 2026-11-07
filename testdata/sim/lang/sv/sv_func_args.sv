`timescale 1ns/1ps
module tb;
  int g;
  logic [7:0] arr [4];
  function automatic int add3(input int a, input int b = 10, input int c = 100);
    return a + b + c;
  endfunction
  function automatic void split(input logic [15:0] w, output logic [7:0] hi, output logic [7:0] lo);
    hi = w[15:8];
    lo = w[7:0];
  endfunction
  function automatic int bump(inout int x);
    x = x + 1;
    return x * 2;
  endfunction
  task automatic tref(ref int x, input int by);
    x += by;
  endtask
  function automatic int sumarr(const ref logic [7:0] a [4]);
    int s = 0;
    foreach (a[i]) s += a[i];
    return s;
  endfunction
  function automatic int early(int x);
    if (x < 0) return -1;
    if (x == 0) return 0;
    return 1;
  endfunction
  function int implicit_ret(int x);
    implicit_ret = x + 5;
  endfunction
  task automatic delay_print(input int d);
    #d $display("FA delay_print d=%0d t=%0t", d, $time);
  endtask
  logic [7:0] h, l;
  int x;
  initial begin
    $display("FA1 add3(1)=%0d", add3(1));
    $display("FA2 add3(1,2)=%0d", add3(1, 2));
    $display("FA3 add3(1,2,3)=%0d", add3(1, 2, 3));
    $display("FA4 named=%0d", add3(.c(1), .a(2)));
    $display("FA5 skip=%0d", add3(1, , 5));
    split(16'hABCD, h, l);
    $display("FA6 split h=%h l=%h", h, l);
    x = 4;
    g = bump(x);
    $display("FA7 bump x=%0d g=%0d", x, g);
    tref(x, 10);
    $display("FA8 tref x=%0d", x);
    arr = '{8'd1, 8'd2, 8'd3, 8'd250};
    $display("FA9 sumarr=%0d", sumarr(arr));
    $display("FA10 early %0d %0d %0d", early(-5), early(0), early(9));
    $display("FA11 implicit_ret=%0d", implicit_ret(2));
    void'(bump(x));
    $display("FA12 void' x=%0d", x);
    delay_print(3);
    $display("FA13 t=%0t", $time);
    $finish;
  end
endmodule
