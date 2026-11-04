// BUFIO on a flip-flop's output.
module no_bufio_fabric (
    input  wire clk,
    input  wire si,
    output reg  so,
    output wire q_pad
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o, q;
  reg d;
  always @(posedge clk) d <= sr[1];
  BUFIO dut (.O(o), .I(d));
  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));
  OBUF u_q (.O(q_pad), .I(q));
  assign y = sr[1];
  always @(posedge clk) so <= y ^ sr[0];
endmodule
