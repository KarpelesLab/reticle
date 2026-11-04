// A MUXF7 by name with one table and one flip-flop for its data.
module f7_half (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [7:0] sr;
  always @(posedge clk) sr <= {sr[6:0], si};
  wire a, o;
  LUT6 #(.INIT(64'h6996966996696996)) la (.O(a), .I0(sr[0]), .I1(sr[1]), .I2(sr[2]), .I3(sr[3]), .I4(sr[4]), .I5(sr[5]));
  MUXF7 dut (.O(o), .I0(a), .I1(sr[6]), .S(sr[7]));
  always @(posedge clk) so <= o;
endmodule
