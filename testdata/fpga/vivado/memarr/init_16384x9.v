// 16384 words of 9 with initial contents, written and read.
module init_16384x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [38:0] sr;
  always @(posedge clk) sr <= {sr[37:0], si};
  reg [8:0] mem [0:16383];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 16384; k = k + 1) begin
    h = {k * 64'hf6ae1c76b03df8db, k * 64'hf810b5bb4594c6e3};
    mem[k] = h[127:119];
  end
  reg [8:0] q0;
  always @(posedge clk) if (sr[23]) mem[sr[13:0]] <= sr[22:14];
  always @(posedge clk) q0 <= mem[sr[37:24]];
  always @(posedge clk) so <= ^(q0) ^ sr[38];
endmodule
