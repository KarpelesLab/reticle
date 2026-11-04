// A RAM16X1S by name, read into a flip-flop.
module ram16x1s (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [6:0] sr;
  always @(posedge clk) sr <= {sr[5:0], si};
  wire o;
  RAM16X1S #(.INIT(16'hf28b)) dut (.O(o), .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]),
      .D(sr[4]), .WE(sr[5]), .WCLK(clk));
  always @(posedge clk) so <= o ^ sr[6];
endmodule
