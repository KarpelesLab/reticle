// BUFHCE, INIT_OUT 1.
module bufhce_init1 (
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
  BUFHCE #(.INIT_OUT(1)) dut (.O(o), .CE(sr[1]), .I(g));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
