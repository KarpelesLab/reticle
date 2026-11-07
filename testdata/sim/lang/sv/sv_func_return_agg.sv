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
  function automatic nib4_t ramp(logic [3:0] base);
    nib4_t t;
    for (int i = 0; i < 4; i++) t[i] = base + i;
    return t;
  endfunction
  divres_t dr;
  nib4_t n;
  initial begin
    dr = divmod(8'd47, 8'd5);
    $display("FR1 q=%0d r=%0d all=%h", dr.q, dr.r, dr);
    $display("FR2 member of call q=%0d", divmod(8'd100, 8'd7).q);
    $display("FR3 swapn=%h", swapn(8'hA5));
    n = ramp(4'd13);
    $display("FR4 ramp=%0d %0d %0d %0d", n[0], n[1], n[2], n[3]);
    $finish;
  end
endmodule
