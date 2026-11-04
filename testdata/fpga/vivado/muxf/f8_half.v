// A MUXF8 by name with one MUXF7 and one flip-flop for its data.
module f8_half (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [14:0] sr;
  always @(posedge clk) sr <= {sr[13:0], si};
  wire a, b, ab, o;
  LUT6 #(.INIT(64'h6996966996696996)) la (.O(a), .I0(sr[0]), .I1(sr[1]), .I2(sr[2]), .I3(sr[3]), .I4(sr[4]), .I5(sr[5]));
  LUT6 #(.INIT(64'hFE01FE01FE01FE00)) lb (.O(b), .I0(sr[6]), .I1(sr[7]), .I2(sr[8]), .I3(sr[9]), .I4(sr[10]), .I5(sr[11]));
  MUXF7 m0 (.O(ab), .I0(a), .I1(b), .S(sr[12]));
  MUXF8 dut (.O(o), .I0(ab), .I1(sr[13]), .S(sr[14]));
  always @(posedge clk) so <= o;
endmodule
