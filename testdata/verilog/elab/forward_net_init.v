// A net initialiser reading a variable declared further down: a
// continuous assignment, lowered once the whole scope is declared.
module forward_net_init(input clk, input [7:0] d, output on, output [3:0] mode);
  wire on_w = cfg[7];
  wire [3:0] mode_w = cfg[3:0];
  reg [7:0] cfg;
  always @(posedge clk) cfg <= d;
  assign on = on_w;
  assign mode = mode_w;
endmodule
