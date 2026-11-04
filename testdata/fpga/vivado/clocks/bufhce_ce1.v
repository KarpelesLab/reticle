// BUFHCE, enable tied high.
module bufhce_ce1 (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire g, o;
  BUFG up (.O(g), .I(clk2));
  BUFHCE dut (.O(o), .CE(1'b1), .I(g));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
