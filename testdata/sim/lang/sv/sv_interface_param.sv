`timescale 1ns/1ps
interface reg_if #(parameter int W = 8) ();
  logic [W-1:0] q;
  logic [W-1:0] d;
  function automatic logic [W-1:0] inv();
    return ~q;
  endfunction
  modport rd (input q, import inv);
  modport wr (output d);
endinterface

module copier #(parameter int W = 8) (reg_if.rd r, reg_if.wr w);
  assign w.d = r.q + 1'b1;
endmodule

module tb;
  reg_if #(.W(4)) a ();
  reg_if #(.W(4)) b ();
  reg_if #(16) c ();
  reg_if d ();
  copier #(.W(4)) cp (.r(a), .w(b));
  initial begin
    a.q = 4'd9;
    #1 $display("IP1 b.d=%0d bits=%0d", b.d, $bits(b.d));
    a.q = 4'hF;
    #1 $display("IP2 b.d=%0d", b.d);
    c.q = 16'h1234;
    $display("IP3 c.inv=%h bits c.q=%0d d.q bits=%0d", c.inv(), $bits(c.q), $bits(d.q));
    $finish;
  end
endmodule
