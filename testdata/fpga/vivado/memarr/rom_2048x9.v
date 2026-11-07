// 2048 words of 9, initial contents, no write.
module rom_2048x9 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [11:0] sr;
  always @(posedge clk) sr <= {sr[10:0], si};
  reg [8:0] mem [0:2047];
  integer k;
  reg [127:0] h;
  initial for (k = 0; k < 2048; k = k + 1) begin
    h = {k * 64'h6e0fe2188528cb73, k * 64'ha33987bebdec1acf};
    mem[k] = h[127:119];
  end
  reg [8:0] q0;
  always @(posedge clk) q0 <= mem[sr[10:0]];
  always @(posedge clk) so <= ^(q0) ^ sr[11];
endmodule
