// IBUFDS into a BUFG.
module ibufds_bufg (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2,
    input wire clk2b
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire n, o;
  IBUFDS pad (.O(n), .I(clk2), .IB(clk2b));
  BUFG dut (.O(o), .I(n));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
