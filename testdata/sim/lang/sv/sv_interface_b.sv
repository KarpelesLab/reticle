`timescale 1ns/1ps
// sv_interface with every interface signal driven by a continuous assignment
// from a local register, instead of procedurally through the modport.
interface bus_if (input logic clk);
  logic [7:0] data;
  logic valid;
  logic ready;
  modport master (output data, output valid, input ready, input clk);
  modport slave (input data, input valid, output ready, input clk);
endinterface

module producer (bus_if.master b);
  logic [7:0] cnt = 8'd0;
  logic [7:0] d_r = 8'd0;
  logic v_r = 1'b0;
  always_ff @(posedge b.clk) begin
    v_r <= 1'b1;
    if (b.ready) cnt <= cnt + 8'd3;
    d_r <= cnt;
  end
  assign b.data = d_r;
  assign b.valid = v_r;
endmodule

module consumer (bus_if.slave b, output logic [7:0] last);
  initial last = 8'd0;
  assign b.ready = 1'b1;
  always @(posedge b.clk)
    if (b.valid) last <= b.data;
endmodule

module tb;
  logic clk = 0;
  logic [7:0] last;
  bus_if bi (.clk(clk));
  producer p (.b(bi));
  consumer c (.b(bi.slave), .last(last));
  always #5 clk = ~clk;
  initial begin
    repeat (5) begin
      @(posedge clk); #1;
      $display("I1 t=%0t data=%0d valid=%b ready=%b last=%0d", $time, bi.data, bi.valid, bi.ready, last);
    end
    $finish;
  end
endmodule
