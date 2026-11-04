// One table read by two MUXF7: its output cannot be both slices' I0.
module f7_shared (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [19:0] sr;
  always @(posedge clk) sr <= {sr[18:0], si};
  wire a, b, c, x, y;
  LUT6 #(.INIT(64'h6996966996696996)) la (.O(a), .I0(sr[0]), .I1(sr[1]), .I2(sr[2]), .I3(sr[3]), .I4(sr[4]), .I5(sr[5]));
  LUT6 #(.INIT(64'hFE01FE01FE01FE00)) lb (.O(b), .I0(sr[6]), .I1(sr[7]), .I2(sr[8]), .I3(sr[9]), .I4(sr[10]), .I5(sr[11]));
  LUT6 #(.INIT(64'h8001800180018000)) lc (.O(c), .I0(sr[12]), .I1(sr[13]), .I2(sr[14]), .I3(sr[15]), .I4(sr[16]), .I5(sr[17]));
  MUXF7 m0 (.O(x), .I0(a), .I1(b), .S(sr[18]));
  MUXF7 m1 (.O(y), .I0(a), .I1(c), .S(sr[19]));
  reg q;
  always @(posedge clk) q <= x;
  always @(posedge clk) so <= q ^ y;
endmodule
