`timescale 1ns/1ps
module tb;
  logic [7:0] arr [2];
  function automatic int sum2(input logic [7:0] a [2]);
    return a[0] + a[1];
  endfunction
  initial begin
    arr[0] = 8'd3; arr[1] = 8'd4;
    $display("M1 sum=%0d", sum2(arr));
    $finish;
  end
endmodule
