`timescale 1ns/1ps
module tb;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  typedef struct packed { logic v; pair_t p; logic [2:0] tag; } pkt_t;
  pkt_t pk;
  initial begin
    pk = 12'hF85;
    $display("M1 read pk.p=%h pk.p.hi=%h pk.p.lo=%h", pk.p, pk.p.hi, pk.p.lo);
    $finish;
  end
endmodule
