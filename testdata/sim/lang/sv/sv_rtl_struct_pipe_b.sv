`timescale 1ns/1ps
// sv_rtl_struct_pipe with whole-element writes instead of stage[i].val <= ...
typedef struct packed {
  logic valid;
  logic [3:0] id;
  logic [11:0] val;
} beat_t;

module pipe #(parameter int STAGES = 3) (
  input logic clk,
  input logic rst,
  input beat_t din,
  output beat_t dout
);
  beat_t stage [STAGES];
  always_ff @(posedge clk) begin
    if (rst) begin
      for (int i = 0; i < STAGES; i++) stage[i] <= '0;
    end else begin
      stage[0] <= {din.valid, din.id, din.val << 1};
      for (int i = 1; i < STAGES; i++)
        stage[i] <= {stage[i-1].valid, stage[i-1].id, stage[i-1].val + 12'(i)};
    end
  end
  assign dout = stage[STAGES-1];
endmodule

module tb;
  logic clk = 0, rst = 1;
  beat_t din, dout;
  pipe #(.STAGES(3)) dut (.*);
  always #5 clk = ~clk;
  initial begin
    din = '0;
    repeat (2) @(negedge clk);
    rst = 0;
    for (int k = 0; k < 6; k++) begin
      din.valid = 1'b1;
      din.id = 4'(k);
      din.val = 12'(k * 100);
      @(negedge clk);
      $display("SP k=%0d dout valid=%b id=%0d val=%0d raw=%h", k, dout.valid, dout.id, dout.val, dout);
    end
    din = '0;
    repeat (3) begin
      @(negedge clk);
      $display("SP drain valid=%b id=%0d val=%0d", dout.valid, dout.id, dout.val);
    end
    $finish;
  end
endmodule
