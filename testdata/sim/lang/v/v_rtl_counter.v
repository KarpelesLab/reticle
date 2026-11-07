`timescale 1ns/1ps
module counter #(parameter W = 4) (
  input clk, input rst, input en, input load, input [W-1:0] din, input up,
  output reg [W-1:0] q, output wrap
);
  assign wrap = en & (up ? (q == {W{1'b1}}) : (q == {W{1'b0}}));
  always @(posedge clk) begin
    if (rst) q <= {W{1'b0}};
    else if (load) q <= din;
    else if (en) q <= up ? q + 1'b1 : q - 1'b1;
  end
endmodule
module tb;
  reg clk, rst, en, load, up;
  reg [3:0] din;
  wire [3:0] q;
  wire wrap;
  reg [3:0] model;
  integer cyc, errors, wraps;
  counter #(.W(4)) dut (.clk(clk), .rst(rst), .en(en), .load(load), .din(din), .up(up), .q(q), .wrap(wrap));
  initial begin clk = 0; forever #5 clk = ~clk; end
  always @(posedge clk) begin
    if (rst) model <= 0;
    else if (load) model <= din;
    else if (en) model <= up ? model + 1 : model - 1;
  end
  initial begin
    errors = 0; wraps = 0;
    rst = 1; en = 0; load = 0; up = 1; din = 0;
    @(posedge clk); @(posedge clk); #1;
    $display("%0t after reset q=%0d", $time, q);
    rst = 0; en = 1;
    for (cyc = 0; cyc < 40; cyc = cyc + 1) begin
      if (cyc == 20) up = 0;
      if (cyc == 25) begin load = 1; din = 4'd9; end
      if (cyc == 26) load = 0;
      if (cyc == 30) en = 0;
      if (cyc == 33) en = 1;
      @(posedge clk);
      if (wrap) wraps = wraps + 1;
      #1;
      if (q !== model) begin errors = errors + 1; $display("%0t MISMATCH q=%0d model=%0d", $time, q, model); end
      $display("%0t cyc=%0d q=%0d %b %h wrap=%b", $time, cyc, q, q, q, wrap);
    end
    $display("errors=%0d wraps=%0d", errors, wraps);
    $finish;
  end
endmodule
