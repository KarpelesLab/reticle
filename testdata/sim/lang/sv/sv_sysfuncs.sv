`timescale 1ns/1ps
module tb;
  logic [7:0] v;
  int i;
  initial begin
    $display("SF1 clog2 0=%0d 1=%0d 2=%0d 3=%0d 8=%0d 9=%0d 1024=%0d", $clog2(0), $clog2(1), $clog2(2), $clog2(3), $clog2(8), $clog2(9), $clog2(1024));
    v = 8'b1011_0010;
    $display("SF2 countones=%0d onehot=%b onehot0=%b", $countones(v), $onehot(v), $onehot0(v));
    v = 8'b0001_0000;
    $display("SF3 countones=%0d onehot=%b onehot0=%b", $countones(v), $onehot(v), $onehot0(v));
    v = 8'b0;
    $display("SF4 onehot=%b onehot0=%b", $onehot(v), $onehot0(v));
    v = 8'b01x0_z001;
    $display("SF5 isunknown=%b countones=%0d", $isunknown(v), $countones(v));
    $display("SF6 countbits 1=%0d 0=%0d x=%0d z=%0d xz=%0d", $countbits(v, '1), $countbits(v, '0), $countbits(v, 'x), $countbits(v, 'z), $countbits(v, 'x, 'z));
    v = 8'hF0;
    $display("SF7 isunknown=%b", $isunknown(v));
    $display("SF8 signed=%0d unsigned=%0d", $signed(v), $unsigned(-8'sd1));
    i = -20;
    $display("SF9 unsigned(i)=%0d", $unsigned(i));
    $display("SF10 bits=%0d clog2(expr)=%0d", $bits(v[5:2]), $clog2(i + 30));
    $display("SF11 high(v)=%0d low(v)=%0d", $high(v), $low(v));
    $display("SF12 typename int: %s", $typename(i));
    $finish;
  end
endmodule
