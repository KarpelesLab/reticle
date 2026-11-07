`timescale 1ns/1ps
// sv_struct_packed with assignment patterns replaced by concatenations,
// the nested member write removed, and %p removed.
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
  pair_t tmp;
  int v;
  initial begin
    pr = {4'hA, 4'h5};
    $display("S1 pr=%h hi=%h lo=%h bits=%0d", pr, pr.hi, pr.lo, $bits(pr));
    pr = 8'hC3;
    $display("S3 pr.hi=%h pr.lo=%h", pr.hi, pr.lo);
    pk = {1'b1, 4'hF, 4'h0, 3'd5};
    $display("S4 pk=%h %b bits=%0d", pk, pk, $bits(pk));
    $display("S5 pk.p.hi=%h pk.tag=%0d pk[11]=%b", pk.p.hi, pk.tag, pk[11]);
    tmp = pk.p;
    tmp.lo = 4'h9;
    pk.p = tmp;
    $display("S6 pk=%h", pk);
    flat = pk;
    $display("S7 flat=%h", flat);
    pk = 12'h000;
    pk.tag = 3'b111;
    $display("S8 pk=%h", pk);
    pk.valid = 1'b1;
    $display("S8b pk=%h valid=%b", pk, pk.valid);
    sp = 8'hF0;
    v = sp;
    $display("S11 signed struct sp=%0d v=%0d", sp, v);
    sp.a = 4'h7;
    $display("S12 sp=%0d sp.a=%0d", sp, sp.a);
    $display("S13 pr==8'hC3 %b", pr == 8'hC3);
    pr[7:4] = 4'h8;
    $display("S17 pr=%h", pr);
    pr.hi[0] = 1'b1;
    $display("S18 member bit write pr=%h", pr);
    pr.lo[3:2] = 2'b01;
    $display("S19 member part write pr=%h", pr);
    $display("S20 sp.a+sp.b=%0d sp.b signed? %0d", sp.a + sp.b, sp.b);
    $finish;
  end
endmodule
