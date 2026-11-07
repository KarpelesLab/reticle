`timescale 1ns/1ps
// sv_interface_param with the interface signals driven by continuous assignments
// from tb variables instead of procedural hierarchical writes, and no interface function.
interface reg_if #(parameter int W = 8) ();
  logic [W-1:0] q;
  logic [W-1:0] d;
  modport rd (input q);
  modport wr (output d);
endinterface

module copier #(parameter int W = 8) (reg_if.rd r, reg_if.wr w);
  assign w.d = r.q + 1'b1;
endmodule

module tb;
  logic [3:0] aq;
  logic [15:0] cq;
  reg_if #(.W(4)) a ();
  reg_if #(.W(4)) b ();
  reg_if #(16) c ();
  reg_if d ();
  assign a.q = aq;
  assign c.q = cq;
  copier #(.W(4)) cp (.r(a), .w(b));
  initial begin
    aq = 4'd9;
    #1 $display("IP1 b.d=%0d bits=%0d", b.d, $bits(b.d));
    aq = 4'hF;
    #1 $display("IP2 b.d=%0d", b.d);
    cq = 16'h1234;
    #1 $display("IP3 c.q=%h bits c.q=%0d d.q bits=%0d", c.q, $bits(c.q), $bits(d.q));
    $finish;
  end
endmodule
