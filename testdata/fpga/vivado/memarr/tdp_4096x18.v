// 4096 words of 18: two addresses each read and written.
module tdp_4096x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [62:0] sr;
  always @(posedge clk) sr <= {sr[61:0], si};
  reg [17:0] mem [0:4095];
  reg [17:0] q0;
  reg [17:0] p0;
  always @(posedge clk) begin
    if (sr[30]) mem[sr[11:0]] <= sr[29:12];
    q0 <= mem[sr[11:0]];
  end
  always @(posedge clk) begin
    if (sr[61]) mem[sr[42:31]] <= sr[60:43];
    p0 <= mem[sr[42:31]];
  end
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[62];
endmodule
