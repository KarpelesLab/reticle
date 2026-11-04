// A RAM128X1D by name, reading both.
module ram128x1d (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [15:0] sr;
  always @(posedge clk) sr <= {sr[14:0], si};
  wire spo, dpo;
  RAM128X1D #(.INIT(128'hc985261ac4329a58e0a8dd3ddcfffa49)) dut (.SPO(spo), .DPO(dpo), .A(sr[6:0]), .DPRA(sr[13:7]),
      .D(sr[14]), .WE(sr[15]), .WCLK(clk));
  reg a, b;
  always @(posedge clk) begin
    a <= spo;
    b <= dpo;
    so <= a ^ b;
  end
endmodule
