`timescale 1ns/1ps
interface reg_if;
  logic [7:0] q;
  function automatic logic [7:0] inv();
    return ~q;
  endfunction
endinterface
module tb;
  reg_if c ();
  initial begin
    c.q = 8'h12;
    $display("M1 c.inv=%h", c.inv());
    $finish;
  end
endmodule
