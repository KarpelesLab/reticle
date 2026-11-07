`timescale 1ns/1ps
module tb;
  reg [7:0] src;
  wire [7:0] w;
  assign w = src;
  initial begin
    src = 8'd4;
    force w = src * 2;
    #1 $display("force expr w=%0d", w);
    src = 8'd3;
    #1 $display("force expr follows w=%0d (want 6)", w);
    release w;
    src = 8'h0f;
    force w[0] = 1'b0;
    #1 $display("force bit w=%b", w);
    release w[0];
    #1 $display("release bit w=%b", w);
    $finish;
  end
endmodule
