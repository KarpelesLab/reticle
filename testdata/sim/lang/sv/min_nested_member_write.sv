`timescale 1ns/1ps
module tb;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  typedef struct packed { logic v; pair_t p; } pkt_t;
  pkt_t pk;
  initial begin
    pk = 9'h0;
    pk.p.lo = 4'h9;
    $display("M1 pk=%h", pk);
    $finish;
  end
endmodule
