`timescale 1ns / 1ps
module counter (
  input wire clk,
  input wire rst,
  input wire en,
  (* keep = 1 *) output reg [7:0] q
);
  parameter WIDTH = 8;
  always @(posedge clk) begin : count
    if (rst) begin
      q <= 8'h00;
    end else if (en) begin
      q <= q + 8'h01;
    end
  end
endmodule
