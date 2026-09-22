module counter_synth (
  input wire clk,
  input wire rst,
  input wire en,
  output reg [3:0] q,
  input wire [2:0] waddr,
  input wire [7:0] wdata,
  input wire we,
  output wire [7:0] mem_out
);
  wire [3:0] q_next;
  wire [3:0] inc;
  wire carry;
  wire sel;
  wire lut_out;
  reg [7:0] \buf  [0:7];
  assign inc = q + 4'h1;
  assign q_next = en ? inc : q;
  always @(posedge clk) begin
    if (rst) q <= 4'h0;
    else q <= q_next;
  end
  assign carry = |q;
  assign sel = carry & en;
  localparam [3:0] lut0_INIT = 4'h6;
  assign lut_out = lut0_INIT[{sel, carry}];
  assign mem_out = \buf [q[2:0]];
  always @(posedge clk) begin
    if (we) \buf [waddr] <= wdata;
  end
  SB_GB bb0 (
    .USER_SIGNAL_TO_GLOBAL_BUFFER(lut_out)
  );
endmodule
