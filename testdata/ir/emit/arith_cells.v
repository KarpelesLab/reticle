module datapath (
  input wire clk,
  input wire rst_n,
  input wire en,
  input wire signed [7:0] a,
  input wire signed [7:0] b,
  input wire [7:0] u,
  input wire [7:0] v,
  input wire [2:0] n,
  input wire [1:0] sel,
  output wire signed [7:0] sum,
  output wire [7:0] diff,
  output wire signed [7:0] prod,
  output wire signed [7:0] quot,
  output wire [7:0] rem,
  output wire signed [7:0] shl,
  output wire [7:0] shr,
  output wire signed [7:0] sshr,
  output wire eq,
  output wire ne,
  output wire lt,
  output wire le,
  output wire gt,
  output wire ge,
  output wire [7:0] pm,
  output reg signed [7:0] q,
  output reg [7:0] q2,
  output reg [7:0] rd,
  input wire [1:0] raddr,
  input wire [1:0] waddr
);
  reg [7:0] \buf  [0:3];
  function [7:0] reticle_pmux_8_2;
    input [7:0] a;
    input [15:0] b;
    input [1:0] s;
    integer i;
    begin
      reticle_pmux_8_2 = a;
      for (i = 0; i < 2; i = i + 1)
        if (s[i]) reticle_pmux_8_2 = b[i * 8 +: 8];
    end
  endfunction
  assign sum = a + b;
  assign diff = u - v;
  assign prod = a * b;
  assign quot = a / b;
  assign rem = u % v;
  assign shl = a << n;
  assign shr = u >> n;
  assign sshr = a >>> n;
  assign eq = a == b;
  assign ne = u != v;
  assign lt = a < b;
  assign le = a <= b;
  assign gt = u > v;
  assign ge = u >= v;
  assign pm = reticle_pmux_8_2(u, {v, diff}, sel);
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) q <= 8'sh00;
    else if (en) q <= sum;
  end
  always @(negedge clk or posedge en) begin
    if (en) q2 <= 8'hff;
    else q2 <= diff;
  end
  always @(posedge clk) begin
    if (en) rd <= \buf [raddr];
  end
  always @* begin
    if (en) \buf [waddr] = u;
  end
endmodule
