`timescale 1ns/1ps
module fifo #(
  parameter int W = 8,
  parameter int DEPTH = 8,
  localparam int AW = $clog2(DEPTH)
) (
  input  logic         clk,
  input  logic         rst,
  input  logic         wr_en,
  input  logic [W-1:0] wr_data,
  input  logic         rd_en,
  output logic [W-1:0] rd_data,
  output logic         full,
  output logic         empty,
  output logic [AW:0]  count
);
  logic [W-1:0] mem [DEPTH];
  logic [AW-1:0] wp, rp;
  wire do_wr = wr_en && !full;
  wire do_rd = rd_en && !empty;
  assign full = (count == (AW+1)'(DEPTH));
  assign empty = (count == '0);
  assign rd_data = mem[rp];
  always_ff @(posedge clk) begin
    if (rst) begin
      wp <= '0;
      rp <= '0;
      count <= '0;
    end else begin
      if (do_wr) begin
        mem[wp] <= wr_data;
        wp <= wp + 1'b1;
      end
      if (do_rd) rp <= rp + 1'b1;
      case ({do_wr, do_rd})
        2'b10: count <= count + 1'b1;
        2'b01: count <= count - 1'b1;
        default: ;
      endcase
    end
  end
endmodule

module tb;
  logic clk = 0, rst = 1, wr_en = 0, rd_en = 0;
  logic [7:0] wr_data = 0, rd_data;
  logic full, empty;
  logic [3:0] count;
  int errors = 0;
  logic [7:0] expect_q [$];
  fifo #(.W(8), .DEPTH(8)) dut (.*);
  always #5 clk = ~clk;
  task automatic push(input logic [7:0] v);
    @(negedge clk);
    wr_en = 1; wr_data = v;
    @(negedge clk);
    wr_en = 0;
  endtask
  task automatic pop(input logic [7:0] exp);
    @(negedge clk);
    if (rd_data !== exp) begin
      errors++;
      $display("FF pop mismatch got=%h exp=%h", rd_data, exp);
    end else
      $display("FF pop ok %h count=%0d", rd_data, count);
    rd_en = 1;
    @(negedge clk);
    rd_en = 0;
  endtask
  initial begin
    repeat (2) @(posedge clk);
    rst = 0;
    $display("FF1 empty=%b full=%b count=%0d", empty, full, count);
    for (int i = 0; i < 8; i++) push(8'(i * 17 + 3));
    $display("FF2 empty=%b full=%b count=%0d", empty, full, count);
    push(8'hEE);
    $display("FF3 after overflow push count=%0d", count);
    for (int i = 0; i < 8; i++) pop(8'(i * 17 + 3));
    $display("FF4 empty=%b full=%b count=%0d", empty, full, count);
    $display("FF5 errors=%0d", errors);
    $finish;
  end
endmodule
