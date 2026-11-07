`timescale 1ns/1ps
module leaf #(parameter int K = 0) (output logic [7:0] o);
  assign o = 8'(K * K);
endmodule
module tb;
  logic [7:0] outs [3];
  for (genvar i = 0; i < 3; i++) begin : g_leaf
    leaf #(.K(i + 1)) u (.o(outs[i]));
  end
  initial begin
    #1 $display("M1 leaf2.o=%0d", g_leaf[2].u.o);
    $finish;
  end
endmodule
