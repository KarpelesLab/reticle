// s ? c - d : a + b, eight bits: the shared adders.
module shared8 (input wire clk, input wire si, output reg so);
  reg [32:0] sr;
  reg [7:0] y;
  always @(posedge clk) begin
    sr <= {sr[31:0], si};
    y <= sr[32] ? sr[23:16] - sr[31:24] : sr[7:0] + sr[15:8];
    so <= ^y;
  end
endmodule
