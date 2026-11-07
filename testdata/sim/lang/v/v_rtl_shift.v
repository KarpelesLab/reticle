`timescale 1ns/1ps
module shreg (input clk, input rst_n, input [1:0] mode, input sin, input [7:0] pin,
              output reg [7:0] q, output sout);
  // mode: 0 hold, 1 shift left, 2 shift right arithmetic, 3 parallel load
  assign sout = q[7];
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) q <= 8'h00;
    else case (mode)
      2'd0: q <= q;
      2'd1: q <= {q[6:0], sin};
      2'd2: q <= {q[7], q[7:1]};
      2'd3: q <= pin;
    endcase
  end
endmodule
module lfsr (input clk, input rst, output reg [15:0] s);
  wire fb = s[15] ^ s[13] ^ s[12] ^ s[10];
  always @(posedge clk) if (rst) s <= 16'hACE1; else s <= {s[14:0], fb};
endmodule
module tb;
  reg clk, rst_n, sin;
  reg [1:0] mode;
  reg [7:0] pin;
  wire [7:0] q;
  wire sout;
  wire [15:0] s;
  integer i, errors;
  reg [7:0] exp;
  shreg u (.clk(clk), .rst_n(rst_n), .mode(mode), .sin(sin), .pin(pin), .q(q), .sout(sout));
  lfsr l (.clk(clk), .rst(~rst_n), .s(s));
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin
    errors = 0; mode = 0; sin = 0; pin = 0;
    rst_n = 0; #12 rst_n = 1;
    $display("%0t reset q=%h", $time, q);
    mode = 3; pin = 8'b1011_0010; @(posedge clk); #1;
    $display("%0t load q=%b", $time, q);
    exp = q;
    mode = 1;
    for (i = 0; i < 6; i = i + 1) begin
      sin = i[0];
      @(posedge clk); #1;
      exp = {exp[6:0], sin};
      if (q !== exp) errors = errors + 1;
      $display("%0t shl q=%b sout=%b lfsr=%h", $time, q, sout, s);
    end
    mode = 2;
    for (i = 0; i < 4; i = i + 1) begin
      @(posedge clk); #1;
      exp = {exp[7], exp[7:1]};
      if (q !== exp) errors = errors + 1;
      $display("%0t sra q=%b lfsr=%h", $time, q, s);
    end
    mode = 0; @(posedge clk); #1 $display("%0t hold q=%b", $time, q);
    #3 rst_n = 0; #1 $display("%0t async reset q=%h", $time, q);
    rst_n = 1;
    $display("errors=%0d", errors);
    $finish;
  end
endmodule
