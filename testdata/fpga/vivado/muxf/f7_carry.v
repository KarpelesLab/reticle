// A table that is both a carry chain's propagate and a MUXF7's data.
module f7_carry (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [20:0] sr;
  always @(posedge clk) sr <= {sr[19:0], si};
  wire [7:0] x = sr[7:0];
  wire [7:0] y = sr[15:8];
  reg [7:0] sum;
  wire b, o;
  LUT6 #(.INIT(64'hFE01FE01FE01FE00)) lb (.O(b), .I0(sr[14]), .I1(sr[15]), .I2(sr[16]), .I3(sr[17]), .I4(sr[18]), .I5(sr[19]));
  always @(posedge clk) sum <= x + y;
  MUXF7 dut (.O(o), .I0(x[0] ^ y[0]), .I1(b), .S(sr[20]));
  always @(posedge clk) so <= o ^ (^sum);
endmodule
