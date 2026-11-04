// BUFR on a pad with no clock capability.
module bufr_plainpad (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire plain
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFR dut (.O(o), .CE(1'b1), .CLR(1'b0), .I(plain));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
