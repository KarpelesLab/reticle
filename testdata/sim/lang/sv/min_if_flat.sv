`timescale 1ns/1ps
interface simple_if;
  logic [7:0] data;
  logic valid;
  modport source (output data, output valid);
  modport sink (input data, input valid);
endinterface
module producer (simple_if.source bus, input logic [7:0] d);
  assign bus.data = d;
  assign bus.valid = 1'b1;
endmodule
module consumer (simple_if.sink bus, output logic [7:0] q);
  assign q = bus.valid ? bus.data : 8'h00;
endmodule
module tb;
  logic [7:0] d, q;
  simple_if bus ();
  producer u_src (.bus(bus), .d(d));
  consumer u_sink (.bus(bus), .q(q));
  initial begin
    d = 8'h3C;
    #1 $display("M1 q=%h", q);
    $finish;
  end
endmodule
