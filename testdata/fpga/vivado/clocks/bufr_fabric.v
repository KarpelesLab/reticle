// BUFR on a flip-flop's output.
module bufr_fabric (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  reg d;
  always @(posedge clk) d <= sr[1];
  BUFR dut (.O(o), .CE(1'b1), .CLR(1'b0), .I(d));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
