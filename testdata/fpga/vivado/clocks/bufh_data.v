// BUFH's output read as data, as the primitive's probe does.
module bufh_data (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFH dut (.O(o), .I(clk));
  reg q;
  always @(posedge clk) q <= o;
  assign y = q;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
