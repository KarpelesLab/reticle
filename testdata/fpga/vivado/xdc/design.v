// The design every XDC fixture here is read against. It is written to have one
// of each thing a constraint can name: a port of each range shape, registers
// scalar and wide, a level of hierarchy, a tristate pad, a register used as a
// clock and a clock manager's output.
module sub(input clk, input d, output reg q);
  reg s1;
  always @(posedge clk) begin
    s1 <= d;
    q <= s1;
  end
endmodule

module design_top(
  input clk,
  input clk2,
  input rst,
  input [3:0] a,
  input [7:4] hi,
  input [0:3] be,
  input single,
  output reg [3:0] q,
  output so,
  output y0,
  output y1,
  output y2,
  output y3,
  inout io1
);
  reg [3:0] r;
  reg [7:4] t;
  reg [1:0] div;
  reg m;
  reg clk_div;
  reg on_div;
  reg on_mmcm;
  wire w;
  wire clk_mmcm;
  wire feedback;

  sub u_sub(.clk(clk2), .d(single), .q(so));

  MMCME2_BASE #(
    .CLKFBOUT_MULT_F(10.0),
    .CLKIN1_PERIOD(10.0),
    .CLKOUT0_DIVIDE_F(5.0)
  ) u_mmcm(
    .CLKIN1(clk),
    .CLKFBIN(feedback),
    .CLKFBOUT(feedback),
    .CLKOUT0(clk_mmcm),
    .RST(rst),
    .PWRDWN(1'b0)
  );

  always @(posedge clk) begin
    if (rst) r <= 0; else r <= r + a + hi + be;
    t <= r;
    q <= t;
    div <= div + 1;
    m <= r[0] ^ single;
    clk_div <= ~clk_div;
  end
  always @(posedge clk_div) on_div <= m;
  always @(posedge clk_mmcm) on_mmcm <= m;

  assign w = m & r[1];
  assign y0 = w;
  assign y1 = div[1];
  assign y2 = on_div;
  assign y3 = on_mmcm;
  assign io1 = m ? 1'bz : 1'b0;
endmodule
