module ops;
  // Arithmetic and bitwise.
  a = b + c - d * e / f % g ** h;
  a = ~b & c | d ^ e ~^ f ^~ g;
  a = &b | ~&c | ~|d | ~^e;
  a = !b && c || d;
  // Comparison.
  a = b < c; a = b > c; a = b <= c; a = b >= c;
  a = b == c; a = b != c; a = b === c; a = b !== c;
  a = b ==? c; a = b !=? c;
  // Shifts.
  a = b << c; a = b >> c; a = b <<< c; a = b >>> c;
  // Assignment operators.
  a += 1; a -= 1; a *= 2; a /= 2; a %= 2;
  a &= b; a |= b; a ^= b;
  a <<= 1; a >>= 1; a <<<= 1; a >>>= 1;
  a++; a--; ++a; --a;
  // Selects and concatenation.
  a = b[3:0]; a = b[7-:4]; a = b[0+:4]; a = {b, c}; a = {4{b}};
  // Ternary, scope, events, implication, casts, patterns.
  a = b ? c : d;
  a = pkg::VALUE; a = $unit::x;
  -> ev; ->> ev; @(posedge clk); wait(a);
  assert property (@(posedge clk) a |-> b);
  assert property (@(posedge clk) a |=> ##1 b ##[1:$] c);
  a = b <-> c;
  a = int'(b); a = '{1, 2}; a = '{default: 0};
  x = a.b.c; inst .* ;
  a = b inside {[1:5], 7};
  a = b -> c;
  (a => b) = 1; (a *> b) = 1;
  a = b &&& c;
  #5 a = 1; #(1:2:3) b = 2;
  a = $;
endmodule
