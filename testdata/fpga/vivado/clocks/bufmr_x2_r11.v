// BUFMRCE_X0Y2 driving BUFR_X0Y11.
module bufmr_x2_r11 (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire mr, o;
  BUFMR dut (.O(mr), .I(clk2));
  BUFR u_r (.O(o), .CE(1'b1), .CLR(1'b0), .I(mr));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
