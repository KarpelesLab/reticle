`timescale 1ns/1ps
module tb;
  typedef struct packed { logic [7:0] q; logic [7:0] r; } dr_t;
  function automatic dr_t divmod(logic [7:0] a, logic [7:0] b);
    divmod.q = a / b;
    divmod.r = a % b;
  endfunction
  initial begin
    $display("M1 divmod(100,7).q=%0d .r=%0d whole=%h", divmod(8'd100, 8'd7).q, divmod(8'd100, 8'd7).r, divmod(8'd100, 8'd7));
    $finish;
  end
endmodule
