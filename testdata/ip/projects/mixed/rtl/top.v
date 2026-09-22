// The top is Verilog; the counter it instantiates is VHDL.
`timescale 1ns / 1ps
`default_nettype none

module top (
    input  wire       clk,
    input  wire       rst,
    output wire [3:0] gray
);
  gray_counter u_gray (
      .clk (clk),
      .rst (rst),
      .gray(gray)
  );
endmodule
