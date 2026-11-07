`timescale 1ns/1ps
module tb;
  parameter int MODE = 2;
  logic [7:0] modev;
  generate
    if (MODE == 1) begin : g_m
      assign modev = 8'd11;
    end else if (MODE == 2) begin : g_m
      assign modev = 8'd22;
    end else begin : g_m
      assign modev = 8'd33;
    end
  endgenerate
  initial begin
    #1 $display("M1 modev=%0d", modev);
    $finish;
  end
endmodule
