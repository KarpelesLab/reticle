// 1024 words of 40 with initial contents, written and read.
module init_1024x40 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [61:0] sr;
  always @(posedge clk) sr <= {sr[60:0], si};
  reg [39:0] mem [0:1023];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 1024; k = k + 1) begin
    h = {k * 64'h3057558033a3f769, k * 64'hf0268907baf92eb3};
    mem[k] = h[127:88];
  end
  reg [39:0] q0;
  always @(posedge clk) if (sr[50]) mem[sr[9:0]] <= sr[49:10];
  always @(posedge clk) q0 <= mem[sr[60:51]];
  always @(posedge clk) so <= ^(q0) ^ sr[61];
endmodule
