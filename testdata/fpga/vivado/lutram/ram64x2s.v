// A RAM64X2S by name, both outputs read.
module ram64x2s (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [8:0] sr;
  always @(posedge clk) sr <= {sr[7:0], si};
  wire o0, o1;
  RAM64X2S #(.INIT_00(64'h562ca7c63b620483), .INIT_01(64'hd22298668e91fed9)) dut (.O0(o0), .O1(o1),
      .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .A4(sr[4]), .A5(sr[5]), .D0(sr[6]), .D1(sr[7]), .WE(sr[8]), .WCLK(clk));
  always @(posedge clk) so <= o0 ^ o1;
endmodule
