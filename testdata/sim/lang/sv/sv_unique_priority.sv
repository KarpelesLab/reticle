`timescale 1ns/1ps
module tb;
  logic [1:0] s;
  logic [3:0] o;
  task automatic t_unique(input logic [1:0] v);
    unique case (v)
      2'd0: o = 4'h1;
      2'd1: o = 4'h2;
      2'd2: o = 4'h4;
    endcase
    $display("UP unique case v=%0d o=%h", v, o);
  endtask
  task automatic t_priority(input logic [1:0] v);
    priority case (1'b1)
      v[1]: o = 4'hA;
      v[0]: o = 4'hB;
    endcase
    $display("UP priority case v=%0d o=%h", v, o);
  endtask
  task automatic t_uif(input logic [1:0] v);
    unique if (v == 2'd0) o = 4'h3;
    else if (v == 2'd1) o = 4'h5;
    $display("UP unique if v=%0d o=%h", v, o);
  endtask
  task automatic t_pif(input logic [1:0] v);
    priority if (v[1]) o = 4'h6;
    else if (v[0]) o = 4'h7;
    $display("UP priority if v=%0d o=%h", v, o);
  endtask
  task automatic t_u0(input logic [1:0] v);
    unique0 case (v)
      2'd0: o = 4'hC;
      2'd1: o = 4'hD;
    endcase
    $display("UP unique0 v=%0d o=%h", v, o);
  endtask
  initial begin
    o = 0;
    t_unique(2'd1);
    t_unique(2'd3);
    t_priority(2'd3);
    t_priority(2'd0);
    t_uif(2'd1);
    t_uif(2'd2);
    t_pif(2'd1);
    t_pif(2'd0);
    t_u0(2'd0);
    t_u0(2'd3);
    $finish;
  end
endmodule
