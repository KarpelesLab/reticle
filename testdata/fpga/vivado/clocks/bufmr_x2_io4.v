// BUFMRCE_X0Y2 driving BUFIO_X0Y4.
module bufmr_x2_io4 (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2,
    output wire q_pad
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire mr, o, q;
  BUFMR dut (.O(mr), .I(clk2));
  BUFIO u_i (.O(o), .I(mr));
  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));
  OBUF u_q (.O(q_pad), .I(q));
  assign y = sr[1];
  always @(posedge clk) so <= y ^ sr[0];
endmodule
