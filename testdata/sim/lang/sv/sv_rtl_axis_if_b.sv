`timescale 1ns/1ps
// sv_rtl_axis_if with the output stage held in local registers and driven onto
// the interface by continuous assignments; the tb drives through assigns too.
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
  logic [W-1:0] d_r;
  logic v_r, l_r;
  assign s.tready = !v_r || m.tready;
  always_ff @(posedge clk) begin
    if (rst) begin
      v_r <= 1'b0;
      d_r <= '0;
      l_r <= 1'b0;
    end else if (s.tready) begin
      v_r <= s.tvalid;
      d_r <= s.tdata + 1'b1;
      l_r <= s.tlast;
    end
  end
  assign m.tvalid = v_r;
  assign m.tdata = d_r;
  assign m.tlast = l_r;
endmodule

module tb;
  logic clk = 0, rst = 1;
  logic a_valid, a_last, b_ready;
  logic [7:0] a_data;
  axis_if #(8) a (clk);
  axis_if #(8) b (clk);
  assign a.tvalid = a_valid;
  assign a.tdata = a_data;
  assign a.tlast = a_last;
  assign b.tready = b_ready;
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
    a_valid = 0; a_data = 0; a_last = 0; b_ready = 1;
    repeat (2) @(negedge clk);
    rst = 0;
    for (int i = 0; i < 5; i++) begin
      @(negedge clk);
      a_valid = 1; a_data = 8'(i * 10); a_last = (i == 4);
      b_ready = (i != 2);
    end
    @(negedge clk);
    a_valid = 0; b_ready = 1;
    repeat (3) @(negedge clk);
    $display("AX got=%0d sum=%0d", got, sum);
    $finish;
  end
endmodule
