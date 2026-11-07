`timescale 1ns/1ps
interface reg_if;
  logic [7:0] q;
  assign q = 8'h12;
  function automatic logic [7:0] inv();
    return ~q;
  endfunction
endinterface
module user (reg_if r, output logic [7:0] o);
  assign o = r.inv();
endmodule
module tb;
  logic [7:0] o;
  reg_if c ();
  user u (.r(c), .o(o));
  initial begin
    #1 $display("M1 o=%h", o);
    $finish;
  end
endmodule
