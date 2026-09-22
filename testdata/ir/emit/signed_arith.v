module alu (
  input wire signed [7:0] a,
  input wire signed [7:0] b,
  input wire [7:0] u,
  input wire [2:0] n,
  output wire signed [7:0] sum,
  output wire signed [7:0] diff,
  output wire signed [7:0] prod,
  output wire signed [7:0] quot,
  output wire signed [7:0] rem,
  output wire signed [7:0] pw,
  output wire signed [7:0] asr,
  output wire [7:0] usr,
  output wire signed [7:0] lsr,
  output wire signed [7:0] shl,
  output wire lt,
  output wire ge,
  output wire eq,
  output wire [7:0] mixed,
  output wire signed [15:0] ext,
  output wire [15:0] zext,
  output wire [3:0] trunc,
  output wire signed [15:0] sext_sum,
  output wire [3:0] trunc_sum,
  output wire signed [7:0] neg,
  output wire signed [7:0] sel,
  output wire [15:0] cat,
  output wire [15:0] rep,
  output wire \bit ,
  output wire [3:0] part,
  output wire [3:0] part_dn,
  output wire par,
  output wire wild,
  output wire exact,
  output wire lnot,
  output wire land,
  output wire signed [7:0] nested,
  input wire clk,
  output reg [7:0] uq
);
  function [3:0] reticle_mslice_8_8_4;
    input [7:0] x;
    input [7:0] i;
    reticle_mslice_8_8_4 = x[i -: 4];
  endfunction
  function signed [15:0] reticle_sext_8_16;
    input [7:0] x;
    reticle_sext_8_16 = {{8{x[7]}}, x};
  endfunction
  function [3:0] reticle_trunc_8_4;
    input [7:0] x;
    reticle_trunc_8_4 = x[3:0];
  endfunction
  assign sum = a + b;
  assign diff = a - b;
  assign prod = a * b;
  assign quot = a / b;
  assign rem = a % b;
  assign pw = a ** 8'sh02;
  assign asr = a >>> n;
  assign usr = {$signed(u) >>> n};
  assign lsr = a >> n;
  assign shl = a << 8'h01;
  assign lt = a < b;
  assign ge = a >= 8'sh00;
  assign eq = u == $unsigned(a);
  assign mixed = u + $unsigned(a);
  assign ext = $signed({{8{a[7]}}, a});
  assign zext = {{8{1'b0}}, a};
  assign trunc = a[3:0];
  assign sext_sum = reticle_sext_8_16(a + b);
  assign trunc_sum = reticle_trunc_8_4(u + u);
  assign neg = -a;
  assign sel = lt ? a : b;
  assign cat = {a, u};
  assign rep = {2{u}};
  assign \bit  = a[n];
  assign part = u[n +: 4];
  assign part_dn = reticle_mslice_8_8_4(a + b, 8'h07);
  assign par = ^u;
  assign wild = (u & 8'h89) == 8'h81;
  assign exact = u === 8'hxx;
  assign lnot = !u;
  assign land = lt && ge;
  assign nested = a - (b + (a * 8'sh03));
  always @(posedge (~clk)) begin
    uq <= u;
  end
endmodule
