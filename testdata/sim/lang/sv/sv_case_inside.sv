`timescale 1ns/1ps
module tb;
  logic [7:0] v;
  int r;
  function automatic int classify(logic [7:0] x);
    case (x) inside
      8'd0: return 0;
      [8'd1:8'd9]: return 1;
      8'd10, 8'd20, 8'd30: return 2;
      8'b1???_????: return 3;
      [8'd40:8'd50], 8'd99: return 4;
      default: return -1;
    endcase
  endfunction
  initial begin
    $display("CI1 0->%0d", classify(8'd0));
    $display("CI2 5->%0d", classify(8'd5));
    $display("CI3 9->%0d", classify(8'd9));
    $display("CI4 20->%0d", classify(8'd20));
    $display("CI5 200->%0d", classify(8'd200));
    $display("CI6 45->%0d", classify(8'd45));
    $display("CI7 99->%0d", classify(8'd99));
    $display("CI8 77->%0d", classify(8'd77));
    v = 8'b0000_x000;
    $display("CI9 x->%0d", classify(v));
    v = 8'b1x00_0000;
    $display("CI10 1x->%0d", classify(v));
    v = 8'd3;
    casez (v)
      8'b0000_00??: r = 1;
      default: r = 0;
    endcase
    $display("CI11 casez r=%0d", r);
    casex (8'b0000_x011)
      8'b0000_1011: r = 5;
      default: r = 6;
    endcase
    $display("CI12 casex r=%0d", r);
    $finish;
  end
endmodule
