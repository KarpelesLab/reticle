// A RAM32X1S by name, read into a flip-flop.
module ram32x1s (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [7:0] sr;
  always @(posedge clk) sr <= {sr[6:0], si};
  wire o;
  RAM32X1S #(.INIT(32'h5e009686)) dut (.O(o), .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .A4(sr[4]),
      .D(sr[5]), .WE(sr[6]), .WCLK(clk));
  always @(posedge clk) so <= o ^ sr[7];
endmodule
