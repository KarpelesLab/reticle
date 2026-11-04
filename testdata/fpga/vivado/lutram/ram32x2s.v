// A RAM32X2S by name, both outputs read.
module ram32x2s (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [7:0] sr;
  always @(posedge clk) sr <= {sr[6:0], si};
  wire o0, o1;
  RAM32X2S #(.INIT_00(32'hc89ee39f), .INIT_01(32'h98741b7d)) dut (.O0(o0), .O1(o1),
      .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .A4(sr[4]), .D0(sr[5]), .D1(sr[6]), .WE(sr[7]), .WCLK(clk));
  always @(posedge clk) so <= o0 ^ o1;
endmodule
