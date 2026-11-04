// BUFIO on a pad of one bank clocking an ODDR on a pad of another.
module no_bufio_far (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2,
    output wire far_d
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o, q;
  BUFIO dut (.O(o), .I(clk2));
  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));
  OBUF u_q (.O(far_d), .I(q));
  assign y = sr[1];
  always @(posedge clk) so <= y ^ sr[0];
endmodule
