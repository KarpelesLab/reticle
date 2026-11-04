// BUFGCTRL, IGNORE0 tied to 1.
module gctrl_ignore0_1 (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFGCTRL dut (.O(o), .I0(clk), .I1(clk2), .CE0(sr[0]), .CE1(sr[1]), .S0(sr[2]), .S1(sr[3]), .IGNORE0(1'b1), .IGNORE1(sr[5]));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
