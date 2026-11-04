// A BUFMR on the right of the die driving BUFR_X1Y8.
module bufmr_right_r8 (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire rpad
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire mr, o;
  BUFMR dut (.O(mr), .I(rpad));
  BUFR u_r (.O(o), .CE(1'b1), .CLR(1'b0), .I(mr));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
