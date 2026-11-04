// Two narrow data tables, and a third that shares inputs with one and must leave the slice.
module f7_pair (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [12:0] sr;
  always @(posedge clk) sr <= {sr[11:0], si};
  wire a = sr[0] & sr[1] & sr[2];
  wire b = sr[3] | sr[4] | sr[5];
  wire c = sr[0] ^ sr[1] ^ sr[10] ^ sr[11];
  wire o;
  MUXF7 dut (.O(o), .I0(a), .I1(b), .S(sr[6]));
  reg q, r;
  always @(posedge clk) q <= c & sr[7] & sr[8] & sr[9];
  always @(posedge clk) r <= c | sr[7] | sr[8] | sr[9];
  always @(posedge clk) so <= q ^ r ^ (o & sr[12]);
endmodule
