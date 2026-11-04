// Two BUFHs on one BUFG, each with its own flip-flops.
module bufh_two (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire g, o, p;
  BUFG up (.O(g), .I(clk2));
  BUFH a (.O(o), .I(g));
  BUFH b (.O(p), .I(g));
  reg t, u;
  always @(posedge o) t <= sr[0];
  always @(posedge p) u <= sr[1];
  assign y = t ^ u;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
