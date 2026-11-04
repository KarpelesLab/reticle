// BUFMRCE, enable inverted.
module bufmrce_inv (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire mr, o;
  BUFMRCE #(.IS_CE_INVERTED(1'b1)) dut (.O(mr), .CE(sr[1]), .I(clk2));
  BUFR u_r (.O(o), .CE(1'b1), .CLR(1'b0), .I(mr));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
