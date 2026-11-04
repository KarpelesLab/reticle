// BUFR with an enable and a clear, dividing.
module bufr_both (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFR #(.BUFR_DIVIDE("4")) dut (.O(o), .CE(sr[1]), .CLR(sr[2]), .I(clk2));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
