`timescale 1ns/1ps
module leaf;
  reg [7:0] tag;
  initial tag = 8'd9;
endmodule
module tb;
  generate
    if (1) begin : blk
      leaf u ();
    end
  endgenerate
  initial begin
    #1 $display("tag %0d", blk.u.tag);
    $finish;
  end
endmodule
