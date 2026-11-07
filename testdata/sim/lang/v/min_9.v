`timescale 1ns/1ps
module tb;
  integer k;
  initial begin
    begin : body
      for (k = 0; k < 10; k = k + 1)
        if (k == 3) disable body;
    end
    $display("k %0d", k);
    $finish;
  end
endmodule
