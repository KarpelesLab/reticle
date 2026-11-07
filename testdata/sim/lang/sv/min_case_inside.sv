`timescale 1ns/1ps
module tb;
  logic [7:0] x = 8'd5;
  initial begin
    case (x) inside
      [8'd1:8'd9]: $display("M1 in range");
      default: $display("M1 default");
    endcase
    $finish;
  end
endmodule
