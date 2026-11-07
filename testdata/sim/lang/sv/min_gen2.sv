`timescale 1ns/1ps
module leaf #(parameter int K = 0) (output logic [7:0] o);
  assign o = 8'(K * K);
endmodule

module tb;
  logic [7:0] outs [4];
  logic [3:0] bits_;
  for (genvar i = 0; i < 4; i++) begin : g_leaf
    leaf #(.K(i + 1)) u (.o(outs[i]));
    assign bits_[i] = outs[i][0];
  end
  initial begin
    #1;
    $display("G1 outs=%0d %0d %0d %0d bits=%b", outs[0], outs[1], outs[2], outs[3], bits_);
    $finish;
  end
endmodule
