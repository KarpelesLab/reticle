`timescale 1ns/1ps
module tb;
  reg [7:0] g;
  reg [7:0] o1, o2, io;
  integer cnt;
  task sum_diff(input [7:0] a, input [7:0] b, output [7:0] s, output [7:0] d);
    begin s = a + b; d = a - b; end
  endtask
  task incr(inout [7:0] v);
    v = v + 1;
  endtask
  task automatic delayed(input [7:0] v, input integer dly);
    begin
      #dly $display("%0t delayed %0d", $time, v);
    end
  endtask
  task global_mod;
    g = g ^ 8'hff;
  endtask
  task with_disable(input integer n);
    integer k;
    begin : body
      for (k = 0; k < 10; k = k + 1) begin
        cnt = cnt + 1;
      end
    end
  endtask
  task old_style;
    input [3:0] x;
    output [7:0] y;
    y = {x, x};
  endtask
  task early_ret(input integer n);
    begin
      $display("early_ret ran with %0d", n);
    end
  endtask
  task trunc_out(output [3:0] y);
    y = 4'hf;
  endtask
  initial begin
    sum_diff(8'd10, 8'd3, o1, o2); $display("sum %0d diff %0d", o1, o2);
    sum_diff(8'd3, 8'd10, o1, o2); $display("sum %0d diff %0d", o1, o2);
    io = 8'd41; incr(io); $display("incr %0d", io);
    io = 8'd255; incr(io); $display("incr wrap %0d", io);
    g = 8'h0f; global_mod; $display("global %h", g);
    cnt = 0; with_disable(3); $display("disable cnt %0d", cnt);
    cnt = 0; with_disable(20); $display("no disable cnt %0d", cnt);
    old_style(4'ha, o1); $display("old %h", o1);
    early_ret(3); early_ret(9);
    o1 = 8'haa; trunc_out(o1); $display("trunc_out %h", o1);
    begin
      delayed(8'd1, 3);
      delayed(8'd2, 1);
      delayed(8'd3, 2);
    end
    $display("%0t after fork", $time);
    begin : outer
      repeat (5) begin
        cnt = cnt + 1;
      end
    end
    $display("outer cnt %0d", cnt);
    $finish;
  end
endmodule
