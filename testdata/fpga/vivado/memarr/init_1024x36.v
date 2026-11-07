// 1024 words of 36 with initial contents, written and read.
module init_1024x36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [57:0] sr;
  always @(posedge clk) sr <= {sr[56:0], si};
  reg [35:0] mem [0:1023];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 1024; k = k + 1) begin
    h = {k * 64'hfe6dc43af3a3afa5, k * 64'h47b43154d47747d9};
    mem[k] = h[127:92];
  end
  reg [35:0] q0;
  always @(posedge clk) if (sr[46]) mem[sr[9:0]] <= sr[45:10];
  always @(posedge clk) q0 <= mem[sr[56:47]];
  always @(posedge clk) so <= ^(q0) ^ sr[57];
endmodule
