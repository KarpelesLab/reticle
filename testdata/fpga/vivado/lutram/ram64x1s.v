// A RAM64X1S by name, read into a flip-flop.
module ram64x1s (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [8:0] sr;
  always @(posedge clk) sr <= {sr[7:0], si};
  wire o;
  RAM64X1S #(.INIT(64'h52a8b4ae90d8c779)) dut (.O(o), .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .A4(sr[4]), .A5(sr[5]),
      .D(sr[6]), .WE(sr[7]), .WCLK(clk));
  always @(posedge clk) so <= o ^ sr[8];
endmodule
