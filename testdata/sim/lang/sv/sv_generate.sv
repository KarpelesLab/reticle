`timescale 1ns/1ps
module leaf #(parameter int K = 0) (output logic [7:0] o);
  assign o = 8'(K * K);
endmodule

module tb;
  parameter int N = 4;
  parameter int MODE = 2;
  logic [7:0] outs [N];
  logic [N-1:0] bits_;
  logic [7:0] modev;
  for (genvar i = 0; i < N; i++) begin : g_leaf
    leaf #(.K(i + 1)) u (.o(outs[i]));
    assign bits_[i] = outs[i][0];
  end
  generate
    if (MODE == 1) begin : g_m
      assign modev = 8'd11;
    end else if (MODE == 2) begin : g_m
      assign modev = 8'd22;
    end else begin : g_m
      assign modev = 8'd33;
    end
  endgenerate
  logic [7:0] cv;
  generate
    case (N)
      2: begin : g_c assign cv = 8'hA2; end
      4: begin : g_c assign cv = 8'hA4; end
      default: begin : g_c assign cv = 8'hAF; end
    endcase
  endgenerate
  for (genvar i = 0; i < 2; i++) begin : g_outer
    for (genvar j = 0; j < 2; j++) begin : g_inner
      localparam int IJ = i * 2 + j;
      logic [3:0] w;
      assign w = 4'(IJ + 5);
    end
  end
  initial begin
    #1;
    $display("G1 outs=%0d %0d %0d %0d bits=%b", outs[0], outs[1], outs[2], outs[3], bits_);
    $display("G2 modev=%0d cv=%h", modev, cv);
    $display("G3 nested w=%0d %0d %0d %0d", g_outer[0].g_inner[0].w, g_outer[0].g_inner[1].w, g_outer[1].g_inner[0].w, g_outer[1].g_inner[1].w);
    $display("G4 IJ=%0d leaf2.o=%0d", g_outer[1].g_inner[1].IJ, g_leaf[2].u.o);
    $finish;
  end
endmodule
