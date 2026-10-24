// top: wrap
// Explicit ports, `.name(net)`, whose outside name differs from the net
// inside: Reticle's own netlist writer names a top-level port this way
// (`.clk(clk$pad)`). The port must be `clk` outside and drive `clk_pad`
// inside. Before the fix, the inside name was looked up by the outside
// name and every such port was reported as undeclared.
module m (.a(a_pad), .b(b_pad), c);
  input a_pad;
  input [1:0] b_pad;
  output c;
  assign c = a_pad ^ b_pad[1];
endmodule

module wrap (input x, input [1:0] y, output z);
  m u (.a(x), .b(y), .c(z));
endmodule
