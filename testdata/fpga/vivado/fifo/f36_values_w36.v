// A FIFO36E1 36 wide with values of its own.
module f36_values_w36 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [74:0] sr;
  always @(posedge clk) sr <= {sr[73:0], si};
  // Five cycles of reset from power-up, then ten with nothing enabled,
  // then writes and reads at random.
  reg [3:0] boot = 4'd0;
  always @(posedge clk) if (boot != 4'd15) boot <= boot + 4'd1;
  wire rst = boot < 4'd5;
  wire live = boot == 4'd15;
  wire wren = live & sr[73];
  wire rden = live & sr[72];
  wire [63:0] o_do;
  wire [7:0] o_dop;
  wire [0:0] o_almostempty;
  wire [0:0] o_almostfull;
  wire [0:0] o_empty;
  wire [0:0] o_full;
  wire [12:0] o_rdcount;
  wire [12:0] o_wrcount;
  wire [0:0] o_rderr;
  wire [0:0] o_wrerr;
  FIFO36E1 #(
    .DATA_WIDTH(36),
    .FIFO_MODE("FIFO36"),
    .EN_SYN("TRUE"),
    .DO_REG(1),
    .INIT(72'h39115d2443e83f09d1),
    .SRVAL(72'h021cd7a7c5fabe4920)) dut (
    .DI(sr[63:0]), .DIP(sr[71:64]), .WREN(wren), .RDEN(rden), .RST(rst), .RSTREG(1'b0), .REGCE(1'b1), .WRCLK(clk), .RDCLK(clk), .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0), .DO(o_do), .DOP(o_dop), .ALMOSTEMPTY(o_almostempty), .ALMOSTFULL(o_almostfull), .EMPTY(o_empty), .FULL(o_full), .RDCOUNT(o_rdcount), .WRCOUNT(o_wrcount), .RDERR(o_rderr), .WRERR(o_wrerr));
  always @(posedge clk) so <= ^o_do[31:0] ^ ^o_dop[3:0] ^ ^o_almostempty ^ ^o_almostfull ^ ^o_empty ^ ^o_full ^ ^o_rdcount ^ ^o_wrcount ^ ^o_rderr ^ ^o_wrerr ^ sr[74];
endmodule
