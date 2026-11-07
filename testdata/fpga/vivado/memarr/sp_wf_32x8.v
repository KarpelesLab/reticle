// 32 words of 8 (sp_wf).
module sp_wf_32x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [14:0] sr;
  always @(posedge clk) sr <= {sr[13:0], si};
  reg [7:0] mem [0:31];
  reg [7:0] q0;
  always @(posedge clk) begin
    if (sr[13]) begin
      mem[sr[4:0]] <= sr[12:5];
      q0 <= sr[12:5];
    end else q0 <= mem[sr[4:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[14];
endmodule
