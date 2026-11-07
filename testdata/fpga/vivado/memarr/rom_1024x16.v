// 1024 words of 16, initial contents, no write.
module rom_1024x16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [10:0] sr;
  always @(posedge clk) sr <= {sr[9:0], si};
  reg [15:0] mem [0:1023];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 1024; k = k + 1) begin
    h = {k * 64'h48ea078693e41687, k * 64'h5d5310068b6ee2af};
    mem[k] = h[127:112];
  end
  reg [15:0] q0;
  always @(posedge clk) q0 <= mem[sr[9:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[10];
endmodule
