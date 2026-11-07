// 256 words of 16, ram_style "distributed".
module style_distributed_256x16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [33:0] sr;
  always @(posedge clk) sr <= {sr[32:0], si};
  (* ram_style = "distributed" *) reg [15:0] mem [0:255];
  reg [15:0] q0;
  always @(posedge clk) if (sr[24]) mem[sr[7:0]] <= sr[23:8];
  always @(posedge clk) q0 <= mem[sr[32:25]];
  always @(posedge clk) so <= ^(q0) ^ sr[33];
endmodule
