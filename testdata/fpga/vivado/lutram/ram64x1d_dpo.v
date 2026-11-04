// A RAM64X1D dpo by name, reading dpo.
module ram64x1d_dpo (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [13:0] sr;
  always @(posedge clk) sr <= {sr[12:0], si};
  wire spo, dpo;
  RAM64X1D #(.INIT(64'h7918ef1458c650f4)) dut (.SPO(), .DPO(dpo), .A0(sr[0]), .A1(sr[1]), .A2(sr[2]), .A3(sr[3]), .A4(sr[4]), .A5(sr[5]), .DPRA0(sr[6]), .DPRA1(sr[7]), .DPRA2(sr[8]), .DPRA3(sr[9]), .DPRA4(sr[10]), .DPRA5(sr[11]),
      .D(sr[12]), .WE(sr[13]), .WCLK(clk));
  reg a, b;
  always @(posedge clk) begin
    a <= spo;
    b <= dpo;
    so <= b;
  end
endmodule
