// Module instantiations: positional and named parameters and ports,
// empty and implicit connections, wildcard, instance arrays, and the
// declaration-versus-instantiation lookahead.
module leaf #(parameter W = 4, parameter D = 1) (input [W - 1:0] i, output [W - 1:0] o, input clk);
  assign o = i;
endmodule

module top (input clk, input [3:0] a, output [3:0] y, output [7:0] wide);
  wire [3:0] t, u;

  leaf l0 (a, t, clk);
  leaf #(8) l1 (.i({a, a}), .o(wide), .clk(clk));
  leaf #(.W(4), .D(2)) l2 (.i(t), .o(u), .clk);
  leaf #(.W(4)) l3 (.i(u), .o(), .clk(clk));
  leaf #(4, 1) l4 (.*);
  leaf l5 (.i(a), .o(y), .clk(clk)), l6 (.i(a), .o(), .clk(clk));
  leaf #(.W(1)) bank [3:0] (.i(a), .o(t), .clk(clk));
  leaf #(.W(1)) bank2 [0:3] (a, u, clk);
  leaf l7 (a, , clk);
  leaf l8 ();
  leaf #(.D()) l9 (.i(a), .o(y), .clk(clk));
  leaf l10 (.i(a), .o(y), .clk(clk));
  leaf (a, t, clk);
  leaf l11 [1:0][1:0] (.i(a), .o(t), .clk(clk));
  leaf #(.W(4)) l12 (.i(a[3:0]), .o(y[3:0]), .clk(top.clk));
endmodule
