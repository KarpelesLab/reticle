`timescale 1ns/1ps
module tb;
  for (genvar i = 0; i < 2; i++) begin : g_outer
    for (genvar j = 0; j < 2; j++) begin : g_inner
      logic [3:0] w;
      assign w = 4'(i * 2 + j + 5);
    end
  end
  initial begin
    #1;
    $display("G3 nested w=%0d %0d %0d %0d", g_outer[0].g_inner[0].w, g_outer[0].g_inner[1].w, g_outer[1].g_inner[0].w, g_outer[1].g_inner[1].w);
    $finish;
  end
endmodule
