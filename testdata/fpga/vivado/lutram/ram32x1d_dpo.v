// A RAM32X1D dpo by name, reading dpo.
module ram32x1d_dpo (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [11:0] sr;
  always @(posedge clk) sr <= {sr[10:0], si};
  wire spo, dpo;
  RAM32X1D #(.INIT(32'hfdad55d4)) dut (.SPO(), .DPO(dpo), .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .A4(sr[4]), .DPRA0(sr[5]), .DPRA1(sr[6]), .DPRA2(sr[7]), .DPRA3(sr[8]), .DPRA4(sr[9]),
      .D(sr[10]), .WE(sr[11]), .WCLK(clk));
  reg a, b;
  always @(posedge clk) begin
    a <= spo;
    b <= dpo;
    so <= b;
  end
endmodule
