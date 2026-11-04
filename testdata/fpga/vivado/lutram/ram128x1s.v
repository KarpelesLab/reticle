// A RAM128X1S by name, read into a flip-flop.
module ram128x1s (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire o;
  RAM128X1S #(.INIT(128'hdb83930d36a680e7daf2699447893324)) dut (.O(o), .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .A4(sr[4]), .A5(sr[5]), .A6(sr[6]),
      .D(sr[7]), .WE(sr[8]), .WCLK(clk));
  always @(posedge clk) so <= o ^ sr[9];
endmodule
