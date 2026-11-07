// 65536 words of 1 (sp_wf).
module sp_wf_65536x1 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [18:0] sr;
  always @(posedge clk) sr <= {sr[17:0], si};
  reg [0:0] mem [0:65535];
  reg [0:0] q0;
  always @(posedge clk) begin
    if (sr[17]) begin
      mem[sr[15:0]] <= sr[16];
      q0 <= sr[16];
    end else q0 <= mem[sr[15:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[18];
endmodule
