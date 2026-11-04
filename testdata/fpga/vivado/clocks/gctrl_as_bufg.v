// BUFGCTRL with its pins where a BUFG's are.
module gctrl_as_bufg (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFGCTRL dut (.O(o), .I0(clk), .I1(clk2), .CE0(1'b1), .CE1(1'b0), .S0(1'b1), .S1(1'b0), .IGNORE0(1'b0), .IGNORE1(1'b1));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
