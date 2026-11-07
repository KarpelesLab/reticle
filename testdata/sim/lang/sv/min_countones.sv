`timescale 1ns/1ps
module tb;
  logic [7:0] v = 8'b1011_0010;
  logic [7:0] u = 8'b01x0_z001;
  initial begin
    $display("M1 countones=%0d onehot=%b onehot0=%b", $countones(v), $onehot(v), $onehot0(v));
    $display("M2 isunknown=%b countbits1=%0d", $isunknown(u), $countbits(u, '1));
    $finish;
  end
endmodule
