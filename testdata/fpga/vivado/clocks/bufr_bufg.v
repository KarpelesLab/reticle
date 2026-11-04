// BUFR on a BUFG.
module bufr_bufg (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  wire g;
  BUFG up (.O(g), .I(clk2));
  BUFR dut (.O(o), .CE(1'b1), .CLR(1'b0), .I(g));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
