`timescale 1ns/1ps
module tb;
  reg [7:0] r;
  wire [7:0] w;
  reg [7:0] src;
  reg [7:0] q;
  reg clk;
  assign w = src + 1;
  always @(posedge clk) q <= src;
  initial begin
    src = 8'd10; r = 8'd1; clk = 0;
    #1 $display("%0t w=%0d r=%0d", $time, w, r);
    force w = 8'd99; force r = 8'd77;
    #1 $display("%0t forced w=%0d r=%0d", $time, w, r);
    src = 8'd20; r = 8'd5;
    #1 $display("%0t forced still w=%0d r=%0d", $time, w, r);
    release w; release r;
    #1 $display("%0t released w=%0d r=%0d", $time, w, r);
    r = 8'd6;
    #1 $display("%0t r assigned %0d", $time, r);
    force w = src * 2;
    #1 $display("%0t force expr w=%0d", $time, w);
    src = 8'd3;
    #1 $display("%0t force expr follows w=%0d", $time, w);
    release w;
    #1 $display("%0t w=%0d", $time, w);
    assign q = 8'hAA;
    #1 clk = 1; #1 clk = 0;
    $display("%0t assign q=%h", $time, q);
    deassign q;
    #1 $display("%0t deassign q=%h", $time, q);
    #1 clk = 1; #1 clk = 0;
    $display("%0t after clk q=%h", $time, q);
    force q = 8'h55;
    #1 clk = 1; #1 clk = 0;
    $display("%0t force over q=%h", $time, q);
    release q;
    #1 $display("%0t release q=%h", $time, q);
    force w[0] = 1'b0;
    #1 $display("%0t force bit w=%b", $time, w);
    release w[0];
    #1 $display("%0t release bit w=%b", $time, w);
    $finish;
  end
endmodule
