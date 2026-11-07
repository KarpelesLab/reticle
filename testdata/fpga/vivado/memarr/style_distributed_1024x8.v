// 1024 words of 8, ram_style "distributed".
module style_distributed_1024x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [29:0] sr;
  always @(posedge clk) sr <= {sr[28:0], si};
  (* ram_style = "distributed" *) reg [7:0] mem [0:1023];
  reg [7:0] q0;
  always @(posedge clk) if (sr[18]) mem[sr[9:0]] <= sr[17:10];
  always @(posedge clk) q0 <= mem[sr[28:19]];
  always @(posedge clk) so <= ^(q0) ^ sr[29];
endmodule
