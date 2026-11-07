`timescale 1ns/1ps
module tb;
  typedef struct packed { logic [7:0] q; logic [7:0] r; } divres_t;
  typedef logic [3:0] nib4_t [4];
  typedef logic [1:0][3:0] pnib_t;
  function automatic divres_t divmod(logic [7:0] a, logic [7:0] b);
    divres_t d;
    d.q = a / b;
    d.r = a % b;
    return d;
  endfunction
  function automatic pnib_t swapn(pnib_t x);
    return {x[0], x[1]};
  endfunction
  divres_t dr;
  nib4_t n;
  initial begin
    dr = divmod(8'd47, 8'd5);
    $display("FR1 q=%0d r=%0d all=%h", dr.q, dr.r, dr);
    $display("FR2 member of call q=%0d", divmod(8'd100, 8'd7).q);
    $display("FR3 swapn=%h", swapn(8'hA5));
    $finish;
  end
endmodule
