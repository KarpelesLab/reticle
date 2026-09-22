// Verilog-2005 expressions: the precedence table, associativity,
// reductions, concatenation and replication, selects, hierarchical names,
// literals, min:typ:max and system function calls.
module exprs;
  wire [7:0] a, b, c, d;
  wire [31:0] w;
  wire x, y, z;
  wire [7:0] m[0:3];

  // Binary precedence, low to high.
  assign x = a || b && c;
  assign x = a | b ^ c & d;
  assign x = a == b != c;
  assign x = a < b <= c > d >= a;
  assign x = a << 1 >> 2 <<< 3 >>> 4;
  assign x = a + b - c;
  assign x = a * b / c % d;
  assign x = (a ** b) ** c;
  assign x = a + b * c ** d - a / b;
  assign x = a === b !== c;
  assign x = a ~^ b ~^ c;
  assign x = a -> b;
  assign x = a -> b -> c;
  assign x = a <-> b;
  assign x = a ? b : c ? d : a;
  assign x = a ? b ? c : d : a;
  assign x = a || b ? c : d;
  assign x = a -> b ? c : d;
  assign x = (a + b) * c;
  assign x = a + b * c;

  // Unary operators and reductions.
  assign x = -a + +b;
  assign x = !a && ~b;
  assign x = &a | ~&b ^ |c | ~|d;
  assign x = ^a ~^ ~^b;
  assign x = ~^a;
  assign x = - -a;
  assign x = ~&a & &b;
  assign x = -a ** 2;

  // Concatenation, replication, selects.
  assign w = {a, b, c, d};
  assign w = {4{a}};
  assign w = {2{a, b}};
  assign w = {{2{a}}, {2{b}}};
  assign x = a[3];
  assign x = a[3:0];
  assign x = a[i+:4];
  assign x = a[7-:4];
  assign x = m[2][3:1];
  assign x = m[i][j];
  assign x = w[a[1:0]];
  assign x = top.sub.inst.sig;
  assign x = top.arr[2].sig[3:0];
  assign x = $root.top.sig;

  // Literals.
  assign w = 32'hdead_beef;
  assign w = 8'b1010_zx01;
  assign w = 'h1f;
  assign w = 'sd5;
  assign w = 16'so17;
  assign w = 4'bx;
  assign w = 123;
  assign w = 1_000;
  assign w = 3.14;
  assign w = 1e10;
  assign w = 2.5e-3;
  assign w = "str";
  assign w = 4'd3 + 3'd4;

  // Calls and system functions.
  assign x = f(a, b);
  assign x = $clog2(256);
  assign x = $signed(a) < $signed(b);
  assign x = $unsigned(a) + $bits(w);
  assign x = $random % 8;
  assign x = $time > 100;
  assign x = f();
  assign x = g(.a(1), .b());

  // Min:typ:max and parenthesised forms.
  wire #(1:2:3) delayed = a;
  assign x = a + b - c;
  assign x = a;
  assign x = (a ? b : c) + d;
  assign x = a[b + c - 1];
endmodule
