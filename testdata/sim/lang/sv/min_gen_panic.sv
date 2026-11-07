`timescale 1ns/1ps
// Reticle used to panic here, an index past the end of its net table.
// Needs: an instance inside a generate-for whose output is connected to an
// element of an unpacked array (outs[i]), plus a second, nested generate-for
// whose inner block declares a variable driven by a continuous assignment.
module leaf (output logic [7:0] o);
  assign o = 8'd1;
endmodule

module tb;
  logic [7:0] outs [4];
  for (genvar i = 0; i < 1; i++) begin : g_leaf
    leaf u (.o(outs[i]));
  end
  for (genvar i = 0; i < 2; i++) begin : g_outer
    for (genvar j = 0; j < 2; j++) begin : g_inner
      logic [3:0] w;
      assign w = 4'd3;
    end
  end
  initial begin
    #1 $display("M1 outs0=%0d w11=%0d", outs[0], g_outer[1].g_inner[1].w);
    $finish;
  end
endmodule
