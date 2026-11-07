`timescale 1ns/1ps
interface axis_if #(parameter int W = 8) (input logic clk);
  logic [W-1:0] tdata;
  logic tvalid;
  logic tready;
  logic tlast;
  modport src (output tdata, tvalid, tlast, input tready);
  modport snk (input tdata, tvalid, tlast, output tready);
endinterface

module axis_inc #(parameter int W = 8) (
  input logic clk,
  input logic rst,
  axis_if.snk s,
  axis_if.src m
);
  assign s.tready = !m.tvalid || m.tready;
  always_ff @(posedge clk) begin
    if (rst) begin
      m.tvalid <= 1'b0;
      m.tdata <= '0;
      m.tlast <= 1'b0;
    end else if (s.tready) begin
      m.tvalid <= s.tvalid;
      m.tdata <= s.tdata + 1'b1;
      m.tlast <= s.tlast;
    end
  end
endmodule

module tb;
  logic clk = 0, rst = 1;
  axis_if #(8) a (clk);
  axis_if #(8) b (clk);
  axis_inc #(8) dut (.clk, .rst, .s(a), .m(b));
  always #5 clk = ~clk;
  int got = 0, sum = 0;
  always @(posedge clk)
    if (b.tvalid && b.tready) begin
      got++;
      sum += b.tdata;
      $display("AX out data=%0d last=%b t=%0t", b.tdata, b.tlast, $time);
    end
  initial begin
    a.tvalid = 0; a.tdata = 0; a.tlast = 0; b.tready = 1;
    repeat (2) @(negedge clk);
    rst = 0;
    for (int i = 0; i < 5; i++) begin
      @(negedge clk);
      a.tvalid = 1; a.tdata = 8'(i * 10); a.tlast = (i == 4);
      b.tready = (i != 2);
    end
    @(negedge clk);
    a.tvalid = 0; b.tready = 1;
    repeat (3) @(negedge clk);
    $display("AX got=%0d sum=%0d", got, sum);
    $finish;
  end
endmodule
