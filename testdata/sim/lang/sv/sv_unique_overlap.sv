`timescale 1ns/1ps
module tb;
  logic [2:0] v;
  logic [3:0] o;
  initial begin
    v = 3'b011;
    unique case (1'b1)
      v[0]: o = 4'h1;
      v[1]: o = 4'h2;
      v[2]: o = 4'h4;
    endcase
    $display("UO1 overlap unique o=%h", o);
    #1;
    unique if (v[0]) o = 4'h8;
    else if (v[1]) o = 4'h9;
    $display("UO2 overlap unique if o=%h", o);
    #1;
    v = 3'bx00;
    unique case (v)
      3'b000: o = 4'h5;
      3'b100: o = 4'h6;
      default: o = 4'hF;
    endcase
    $display("UO3 x selector o=%h", o);
    #1;
    $finish;
  end
endmodule
