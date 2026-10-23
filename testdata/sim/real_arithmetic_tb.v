// Real arithmetic in a testbench, as NewAE's Husky testbench writes it:
// a `.v` file declaring `int`, read with `--sv` as `iverilog -g2005-sv`
// reads it; reals assigned, divided, compared with integers and passed to
// `$ceil`, `$floor` and `$atan2`; and a real assigned to an integer,
// rounded to the nearest with a tie away from zero (IEEE 1364-2005 4.8.2).
module real_arithmetic_tb;
  real r, q, s = 1.25;
  integer k, m, n, p, z;
  int samples;
  initial begin
    r = 7.5;
    k = r;                         // 8: a tie rounds away from zero
    m = -r;                        // -8
    q = r / 3;                     // 2.5
    n = q;                         // 3
    samples = $ceil(r * 2 / 4) * 4;  // ceil(3.75) = 4, times 4 = 16
    p = $floor(q);                 // 2
    s += 0.25;                     // 1.5
    z = (q > 2) ? 1 : 0;           // a real compared with an integer
    if (k == 8 && m == -8 && n == 3 && samples == 16 && p == 2 && s == 1.5 && z == 1
        && $atan2(1.0, 1.0) > 0.78 && $atan2(1.0, 1.0) < 0.79)
      $display("PASS: reals round, compare, accumulate and call");
    else
      $display("FAIL: k %0d m %0d n %0d samples %0d p %0d s %f z %0d", k, m, n, samples, p, s, z);
  end
endmodule
