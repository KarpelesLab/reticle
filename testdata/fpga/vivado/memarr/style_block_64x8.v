// 64 words of 8, ram_style "block".
module style_block_64x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [21:0] sr;
  always @(posedge clk) sr <= {sr[20:0], si};
  (* ram_style = "block" *) reg [7:0] mem [0:63];
  reg [7:0] q0;
  always @(posedge clk) if (sr[14]) mem[sr[5:0]] <= sr[13:6];
  always @(posedge clk) q0 <= mem[sr[20:15]];
  always @(posedge clk) so <= ^(q0) ^ sr[21];
endmodule
