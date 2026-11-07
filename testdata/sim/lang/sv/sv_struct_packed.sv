`timescale 1ns/1ps
module tb;
  typedef struct packed {
    logic [3:0] hi;
    logic [3:0] lo;
  } pair_t;
  typedef struct packed {
    logic valid;
    pair_t p;
    logic [2:0] tag;
  } pkt_t;
  typedef struct packed signed {
    logic [3:0] a;
    logic [3:0] b;
  } spair_t;
  pair_t pr;
  pkt_t pk;
  spair_t sp;
  logic [11:0] flat;
  int v;
  initial begin
    pr = '{hi: 4'hA, lo: 4'h5};
    $display("S1 pr=%h hi=%h lo=%h bits=%0d", pr, pr.hi, pr.lo, $bits(pr));
    pr = '{4'h1, 4'h2};
    $display("S2 pr=%h", pr);
    pr = 8'hC3;
    $display("S3 pr.hi=%h pr.lo=%h", pr.hi, pr.lo);
    pk = '{valid: 1'b1, p: '{hi: 4'hF, lo: 4'h0}, tag: 3'd5};
    $display("S4 pk=%h %b bits=%0d", pk, pk, $bits(pk));
    $display("S5 pk.p.hi=%h pk.tag=%0d pk[11]=%b", pk.p.hi, pk.tag, pk[11]);
    pk.p.lo = 4'h9;
    $display("S6 pk=%h", pk);
    flat = pk;
    $display("S7 flat=%h", flat);
    pk = 12'h000;
    pk.tag = 3'b111;
    $display("S8 pk=%h", pk);
    pk = '{default: 0};
    $display("S9 default0 pk=%h", pk);
    pk = '{default: '1};
    $display("S10 default1 pk=%h", pk);
    sp = 8'hF0;
    v = sp;
    $display("S11 signed struct sp=%0d v=%0d", sp, v);
    sp.a = 4'h7;
    $display("S12 sp=%0d sp.a=%0d", sp, sp.a);
    $display("S13 pr==8'hC3 %b", pr == 8'hC3);
    $display("S14 %%p pr=%p", pr);
    $display("S15 %%p pk=%p", pk);
    pr = '{hi: 4'h3, default: 4'hE};
    $display("S16 pr=%h", pr);
    pr[7:4] = 4'h8;
    $display("S17 pr=%h", pr);
    $finish;
  end
endmodule
