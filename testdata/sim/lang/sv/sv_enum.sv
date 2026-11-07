`timescale 1ns/1ps
module tb;
  typedef enum {A, B, C, D} e1_t;
  typedef enum logic [2:0] {X0 = 3'd1, X1 = 3'd3, X2 = 3'd6} e2_t;
  typedef enum bit [1:0] {P, Q = 2, R} e3_t;
  typedef enum logic [3:0] {S_IDLE, S_RUN = 4'h5, S_DONE} e4_t;
  e1_t e1;
  e2_t e2;
  e3_t e3;
  e4_t e4;
  int k;
  initial begin
    $display("E1 init e1=%0d e2=%b e3=%0d", e1, e2, e3);
    e1 = C;
    $display("E2 e1=%0d name=%s first=%s last=%s num=%0d", e1, e1.name(), e1.first().name(), e1.last().name(), e1.num());
    e1 = e1.next();
    $display("E3 next=%s", e1.name());
    e1 = e1.next();
    $display("E4 wrap next=%s", e1.name());
    e1 = e1.prev();
    $display("E5 prev=%s", e1.name());
    e1 = e1.next(2);
    $display("E6 next(2)=%s", e1.name());
    e2 = X1;
    $display("E7 e2=%0d %b name=%s next=%s prev=%s", e2, e2, e2.name(), e2.next().name(), e2.prev().name());
    $display("E8 e2 first=%0d last=%0d num=%0d", e2.first(), e2.last(), e2.num());
    e3 = R;
    $display("E9 R=%0d P=%0d Q=%0d", e3, P, Q);
    e4 = S_DONE;
    $display("E10 S_DONE=%h bits=%0d", e4, $bits(e4));
    e1 = e1_t'(2);
    $display("E11 cast 2 -> %s", e1.name());
    e2 = e2_t'(3'd2);
    $display("E12 cast invalid -> %0d name='%s'", e2, e2.name());
    k = B + 1;
    $display("E13 B+1=%0d", k);
    for (e1 = e1.first(); e1 != e1.last(); e1 = e1.next())
      $display("E14 iter %s=%0d", e1.name(), e1);
    e1 = D;
    case (e1)
      A: $display("E15 case A");
      B, C: $display("E15 case BC");
      default: $display("E15 case default %s", e1.name());
    endcase
    $display("E16 compare C<D=%b A==0=%b", C < D, A == 0);
    e3 = e3.first();
    repeat (4) begin
      $display("E17 e3=%s %0d", e3.name(), e3);
      e3 = e3.next();
    end
    $display("E18 %%s e4=%s", e4.name());
    $finish;
  end
endmodule
