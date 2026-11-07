// 1024 words of 16, initial contents, read twice.
module rom_r2_1024x16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [47:0] sr;
  always @(posedge clk) sr <= {sr[46:0], si};
  reg [15:0] mem [0:1023];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 1024; k = k + 1) begin
    h = {k * 64'he97cee1ac486495d, k * 64'h2ed4e7df9e61be1d};
    mem[k] = h[127:112];
  end
  reg [15:0] q0;
  always @(posedge clk) if (sr[26]) mem[sr[9:0]] <= sr[25:10];
  always @(posedge clk) q0 <= mem[sr[36:27]];
  reg [15:0] p0;
  always @(posedge clk) p0 <= mem[sr[46:37]];
  always @(posedge clk) so <= ^(q0 ^ p0) ^ sr[47];
endmodule
