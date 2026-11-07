// 4096 words of 18: one address, write first.
module sp_wf_4096x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [31:0] sr;
  always @(posedge clk) sr <= {sr[30:0], si};
  reg [17:0] mem [0:4095];
  reg [17:0] q0;
  always @(posedge clk) begin
    if (sr[30]) begin
      mem[sr[11:0]] <= sr[29:12];
      q0 <= sr[29:12];
    end else q0 <= mem[sr[11:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[31];
endmodule
