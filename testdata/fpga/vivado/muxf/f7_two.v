// Two MUXF7 with different selects, one registered and one read by logic.
module f7_two (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [25:0] sr;
  always @(posedge clk) sr <= {sr[24:0], si};
  wire a, b, c, d, ab, cd;
  LUT6 #(.INIT(64'h6996966996696996)) la (.O(a), .I0(sr[0]), .I1(sr[1]), .I2(sr[2]), .I3(sr[3]), .I4(sr[4]), .I5(sr[5]));
  LUT6 #(.INIT(64'hFE01FE01FE01FE00)) lb (.O(b), .I0(sr[6]), .I1(sr[7]), .I2(sr[8]), .I3(sr[9]), .I4(sr[10]), .I5(sr[11]));
  LUT6 #(.INIT(64'h8001800180018000)) lc (.O(c), .I0(sr[12]), .I1(sr[13]), .I2(sr[14]), .I3(sr[15]), .I4(sr[16]), .I5(sr[17]));
  LUT6 #(.INIT(64'hA5A5C3C3F00FF00E)) ld (.O(d), .I0(sr[18]), .I1(sr[19]), .I2(sr[20]), .I3(sr[21]), .I4(sr[22]), .I5(sr[23]));
  MUXF7 m0 (.O(ab), .I0(a), .I1(b), .S(sr[24]));
  MUXF7 m1 (.O(cd), .I0(c), .I1(d), .S(sr[25]));
  reg q;
  always @(posedge clk) q <= ab;
  always @(posedge clk) so <= q ^ cd;
endmodule
