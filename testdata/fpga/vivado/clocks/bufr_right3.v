// BUFR on site 3 of a row on the right.
module bufr_right3 (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire rpad
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFR dut (.O(o), .CE(1'b1), .CLR(1'b0), .I(rpad));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
