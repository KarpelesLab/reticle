// A MUXF7_D, with both of its outputs read.
module f7_d (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [13:0] sr;
  always @(posedge clk) sr <= {sr[12:0], si};
  wire a, b, o, lo;
  LUT6 #(.INIT(64'h6996966996696996)) la (.O(a), .I0(sr[0]), .I1(sr[1]), .I2(sr[2]), .I3(sr[3]), .I4(sr[4]), .I5(sr[5]));
  LUT6 #(.INIT(64'hFE01FE01FE01FE00)) lb (.O(b), .I0(sr[6]), .I1(sr[7]), .I2(sr[8]), .I3(sr[9]), .I4(sr[10]), .I5(sr[11]));
  MUXF7_D dut (.O(o), .LO(lo), .I0(a), .I1(b), .S(sr[12]));
  reg q;
  always @(posedge clk) q <= lo;
  always @(posedge clk) so <= q ^ (o & sr[13]);
endmodule
