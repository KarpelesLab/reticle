// BUFGMUX_CTRL, select tied low.
module gmuxctrl_s0 (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFGMUX_CTRL dut (.O(o), .I0(clk), .I1(clk2), .S(1'b0));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
