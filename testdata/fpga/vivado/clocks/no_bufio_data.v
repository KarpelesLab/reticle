// BUFIO's output read as data.
module no_bufio_data (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire clk2
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire o;
  BUFIO dut (.O(o), .I(clk2));
  reg t;
  always @(posedge clk) t <= o;
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
