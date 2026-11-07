`timescale 1ns/1ps
module adder #(parameter int W = 4, parameter logic [W-1:0] OFF = '0) (input logic [W-1:0] a, output logic [W:0] y);
  localparam int W2 = W * 2;
  assign y = a + OFF;
  initial $display("PO inst %m W=%0d W2=%0d OFF=%0d", W, W2, OFF);
endmodule

module tb;
  logic [3:0] a4;
  logic [4:0] y4;
  logic [7:0] a8;
  logic [8:0] y8;
  logic [5:0] a6;
  logic [6:0] y6;
  adder u4 (.a(a4), .y(y4));
  adder #(.W(8), .OFF(8'd200)) u8 (.a(a8), .y(y8));
  adder #(6, 6'd1) u6 (.a(a6), .y(y6));
  defparam u4.OFF = 4'd3;
  initial begin
    a4 = 4'd15; a8 = 8'd100; a6 = 6'd63;
    #1 $display("PO1 y4=%0d y8=%0d y6=%0d", y4, y8, y6);
    $finish;
  end
endmodule
