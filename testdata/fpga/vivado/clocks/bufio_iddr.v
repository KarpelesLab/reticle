// BUFIO on a clock pad, clocking an IDDR on a pad of its bank.
module bufio_iddr (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2,
    input wire d_pad
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o, q1;
  BUFIO dut (.O(o), .I(clk2));
  IDDR u_load (.C(o), .CE(1'b1), .D(d_pad), .S(1'b0), .R(1'b0), .Q1(q1), .Q2());
  assign y = q1;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
