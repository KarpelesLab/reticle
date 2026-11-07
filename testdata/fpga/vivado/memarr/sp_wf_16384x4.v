// 16384 words of 4: one address, write first.
module sp_wf_16384x4 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [19:0] sr;
  always @(posedge clk) sr <= {sr[18:0], si};
  reg [3:0] mem [0:16383];
  reg [3:0] q0;
  always @(posedge clk) begin
    if (sr[18]) begin
      mem[sr[13:0]] <= sr[17:14];
      q0 <= sr[17:14];
    end else q0 <= mem[sr[13:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[19];
endmodule
