`timescale 1ns/1ps
module tb;
  reg [1:0] d1, d2;
  reg e1, e2;
  wire [1:0] t;
  wand [1:0] wa;
  wor [1:0] wo;
  tri [1:0] tr;
  triand ta;
  trior to;
  tri0 t0;
  tri1 t1;
  wire pu, pd;
  wire undriven;
  wire [3:0] multi;
  assign t = d1; assign t = d2;
  assign wa = d1; assign wa = d2;
  assign wo = d1; assign wo = d2;
  assign tr = e1 ? d1 : 2'bzz;
  assign tr = e2 ? d2 : 2'bzz;
  assign ta = d1[0]; assign ta = d2[0];
  assign to = d1[0]; assign to = d2[0];
  assign t0 = e1 ? d1[0] : 1'bz;
  assign t1 = e1 ? d1[0] : 1'bz;
  pullup (pu);
  pulldown (pd);
  assign pu = e1 ? 1'b0 : 1'bz;
  assign pd = e1 ? 1'b1 : 1'bz;
  assign multi[1:0] = d1;
  assign multi[3:2] = d2;
  integer i, j;
  reg [1:0] vals [0:3];
  initial begin
    vals[0] = 2'b00; vals[1] = 2'b01; vals[2] = 2'b1x; vals[3] = 2'bz1;
    for (i = 0; i < 4; i = i + 1)
      for (j = 0; j < 4; j = j + 1) begin
        d1 = vals[i]; d2 = vals[j]; e1 = 0; e2 = 0;
        #1 $display("d1=%b d2=%b wire=%b wand=%b wor=%b triand=%b trior=%b", d1, d2, t, wa, wo, ta, to);
      end
    d1 = 2'b10; d2 = 2'b01;
    e1 = 0; e2 = 0; #1 $display("tri none %b t0=%b t1=%b pu=%b pd=%b", tr, t0, t1, pu, pd);
    e1 = 1; e2 = 0; #1 $display("tri e1 %b t0=%b t1=%b pu=%b pd=%b", tr, t0, t1, pu, pd);
    e1 = 0; e2 = 1; #1 $display("tri e2 %b", tr);
    e1 = 1; e2 = 1; #1 $display("tri both %b", tr);
    $display("undriven %b multi %b", undriven, multi);
    $finish;
  end
endmodule
