module inv (
  input wire a,
  output wire y
);
  assign y = ~a;
endmodule

module gates (
  input wire clk,
  input wire rst,
  input wire en,
  input wire [1:0] a,
  input wire [1:0] b,
  input wire s,
  input wire [2:0] sel,
  output wire [1:0] n,
  output wire [1:0] o,
  output wire [1:0] x,
  output wire [1:0] m,
  output wire [1:0] p,
  output wire all,
  output wire any,
  output wire par,
  output wire f,
  output reg [1:0] q,
  (* init = 2'h1 *) output reg [1:0] r,
  output reg [1:0] l,
  output wire [1:0] copy,
  output wire inv_out,
  output wire bb_out
);
  function [1:0] reticle_pmux_2_3;
    input [1:0] a;
    input [5:0] b;
    input [2:0] s;
    integer i;
    begin
      reticle_pmux_2_3 = a;
      for (i = 0; i < 3; i = i + 1)
        if (s[i]) reticle_pmux_2_3 = b[i * 2 +: 2];
    end
  endfunction
  assign copy = {a[0], 1'b1};
  assign n = ~a;
  assign o = a | b;
  assign x = a ^ 2'h3;
  assign m = s ? b : a;
  assign p = reticle_pmux_2_3(a, {b, a, n}, sel);
  assign all = &a;
  assign any = |a;
  assign par = ^{a, b};
  localparam [7:0] lut0_INIT = 8'h96;
  assign f = lut0_INIT[{s, a}];
  always @(posedge clk) begin
    q <= a;
  end
  always @(negedge clk) begin
    if (!rst) r <= 2'h2;
    else if (en) r <= b;
  end
  always @* begin
    if (en) l = m;
  end
  SB_LUT4 #(.LUT_INIT(16'h8000)) bb0 (
    .I0(s),
    .I1(all),
    .I2(1'b0),
    .I3(1'bx),
    .O(bb_out)
  );
  inv u_inv (
    .a(s),
    .y(inv_out)
  );
endmodule
