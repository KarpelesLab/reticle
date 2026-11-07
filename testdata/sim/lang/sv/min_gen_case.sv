`timescale 1ns/1ps
module tb;
  parameter int N = 4;
  logic [7:0] cv;
  generate
    case (N)
      2: begin : g_c assign cv = 8'hA2; end
      4: begin : g_c assign cv = 8'hA4; end
      default: begin : g_c assign cv = 8'hAF; end
    endcase
  endgenerate
  initial begin
    #1 $display("M1 cv=%h", cv);
    $finish;
  end
endmodule
