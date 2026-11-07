// 128 words of 4 (sp_wf).
module sp_wf_128x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [12:0] sr;
  always @(posedge clk) sr <= {sr[11:0], si};
  reg [3:0] mem [0:127];
  reg [3:0] q0;
  always @(posedge clk) begin
    if (sr[11]) begin
      mem[sr[6:0]] <= sr[10:7];
      q0 <= sr[10:7];
    end else q0 <= mem[sr[6:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[12];
endmodule
