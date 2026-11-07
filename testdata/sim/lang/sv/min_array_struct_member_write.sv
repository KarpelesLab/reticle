`timescale 1ns/1ps
module tb;
  typedef struct packed { logic v; logic [6:0] val; } beat_t;
  beat_t st [2];
  initial begin
    st[0] = 8'h00;
    st[0].val = 7'd5;
    $display("M1 st[0]=%h", st[0]);
    $finish;
  end
endmodule
