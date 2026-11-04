// A MUXF8 by name fed by flip-flops and not by MUXF7s.
module f8_from_ff (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [2:0] sr;
  always @(posedge clk) sr <= {sr[1:0], si};
  wire o;
  MUXF8 dut (.O(o), .I0(sr[0]), .I1(sr[1]), .S(sr[2]));
  always @(posedge clk) so <= o;
endmodule
