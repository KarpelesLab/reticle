// A RAM16X1D by name, reading both.
module ram16x1d (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire spo, dpo;
  RAM16X1D #(.INIT(16'hbea3)) dut (.SPO(spo), .DPO(dpo), .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .DPRA0(sr[4]), .DPRA1(sr[5]), .DPRA2(sr[6]), .DPRA3(sr[7]),
      .D(sr[8]), .WE(sr[9]), .WCLK(clk));
  reg a, b;
  always @(posedge clk) begin
    a <= spo;
    b <= dpo;
    so <= a ^ b;
  end
endmodule
