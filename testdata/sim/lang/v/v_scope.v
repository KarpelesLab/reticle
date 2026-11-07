`timescale 1ns/1ps
module tb;
  reg [7:0] x;
  integer cnt;
  (* keep = "true" *) reg [3:0] attr_r;
  function [7:0] f(input [7:0] v);
    reg [7:0] tmp;
    begin
      tmp = v ^ 8'h55;
      f = tmp + x;
    end
  endfunction
  task t(input [7:0] v);
    reg [7:0] tmp2;
    begin
      tmp2 = v + 1;
      x = tmp2;
    end
  endtask
  initial begin
    x = 8'd1;
    $display("f %h", f(8'h0f));
    t(8'd9); $display("t x=%0d", x);
    begin : b1
      reg [7:0] x;
      x = 8'd200;
      $display("shadow inner x=%0d", x);
    end
    $display("outer x=%0d", x);
    begin : b2
      integer k;
      for (k = 0; k < 3; k = k + 1) begin : b3
        reg [3:0] q;
        q = k * 3;
        $display("b3 q=%0d", q);
      end
    end
    cnt = 0;
    begin : b4
      cnt = cnt + 1;
      begin : b5
        cnt = cnt + 10;
      end
    end
    $display("cnt %0d", cnt);
    attr_r = 4'd3 + 4'd4;
    $display("attr %0d", attr_r);
    $display("%m");
    begin : b6
      $display("%m");
    end
    $finish;
  end
endmodule
