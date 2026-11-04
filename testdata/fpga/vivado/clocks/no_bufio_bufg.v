// BUFIO on a BUFG.
module no_bufio_bufg (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2,
    output wire q_pad
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire g, o, q;
  BUFG up (.O(g), .I(clk2));
  BUFIO dut (.O(o), .I(g));
  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));
  OBUF u_q (.O(q_pad), .I(q));
  assign y = sr[1];
  always @(posedge clk) so <= y ^ sr[0];
endmodule
