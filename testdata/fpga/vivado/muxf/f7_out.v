// A MUXF7 whose output goes on to logic, so that it must leave the slice.
module f7_out (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [12:0] sr;
  always @(posedge clk) sr <= {sr[11:0], si};
  wire a, b, o;
  LUT6 #(.INIT(64'h6996966996696996)) la (.O(a), .I0(sr[0]), .I1(sr[1]), .I2(sr[2]), .I3(sr[3]), .I4(sr[4]), .I5(sr[5]));
  LUT6 #(.INIT(64'hFE01FE01FE01FE00)) lb (.O(b), .I0(sr[6]), .I1(sr[7]), .I2(sr[8]), .I3(sr[9]), .I4(sr[10]), .I5(sr[11]));
  MUXF7 dut (.O(o), .I0(a), .I1(b), .S(sr[12]));
  reg q;
  always @(posedge clk) q <= sr[0] & sr[12];
  always @(posedge clk) so <= q ^ (o & sr[3]);
endmodule
