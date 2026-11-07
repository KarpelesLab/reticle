`timescale 1ns/1ps
module tb;
  typedef struct packed { logic v; logic [6:0] val; } beat_t;
  beat_t st [2];
  int i;
  initial begin
    st[0] = 8'h85;
    st[1] = 8'h03;
    i = 1;
    $display("M1 st[0].v=%b st[0].val=%0d st[1].val=%0d st[i].val=%0d st[i-1].v=%b", st[0].v, st[0].val, st[1].val, st[i].val, st[i-1].v);
    $finish;
  end
endmodule
