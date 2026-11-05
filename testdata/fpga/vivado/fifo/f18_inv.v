// A FIFO18E1 on one clock with every inverter it has set.
module f18_inv (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [38:0] sr;
  always @(posedge clk) sr <= {sr[37:0], si};
  // Five cycles of reset from power-up, then ten with nothing enabled,
  // then writes and reads at random.
  reg [3:0] boot = 4'd0;
  always @(posedge clk) if (boot != 4'd15) boot <= boot + 4'd1;
  wire rst = boot < 4'd5;
  wire live = boot == 4'd15;
  wire wren = live & sr[37];
  wire rden = live & sr[36];
  wire [31:0] o_do;
  wire [3:0] o_dop;
  wire [0:0] o_almostempty;
  wire [0:0] o_almostfull;
  wire [0:0] o_empty;
  wire [0:0] o_full;
  wire [11:0] o_rdcount;
  wire [11:0] o_wrcount;
  wire [0:0] o_rderr;
  wire [0:0] o_wrerr;
  reg wren_n = 1'b1;
  always @(posedge clk) wren_n <= ~(wren);
  reg rden_n = 1'b1;
  always @(posedge clk) rden_n <= ~(rden);
  reg rst_n = 1'b1;
  always @(posedge clk) rst_n <= ~(rst);
  reg rstreg_n = 1'b1;
  always @(posedge clk) rstreg_n <= ~(sr[3]);
  FIFO18E1 #(
    .DATA_WIDTH(18),
    .FIFO_MODE("FIFO18"),
    .EN_SYN("TRUE"),
    .DO_REG(1),
    .IS_RDCLK_INVERTED(1'b1),
    .IS_WRCLK_INVERTED(1'b1),
    .IS_RDEN_INVERTED(1'b1),
    .IS_WREN_INVERTED(1'b1),
    .IS_RST_INVERTED(1'b1),
    .IS_RSTREG_INVERTED(1'b1)) dut (
    .DI(sr[31:0]), .DIP(sr[35:32]), .WREN(wren_n), .RDEN(rden_n), .RST(rst_n), .RSTREG(rstreg_n), .REGCE(1'b1), .WRCLK(clk), .RDCLK(clk), .DO(o_do), .DOP(o_dop), .ALMOSTEMPTY(o_almostempty), .ALMOSTFULL(o_almostfull), .EMPTY(o_empty), .FULL(o_full), .RDCOUNT(o_rdcount), .WRCOUNT(o_wrcount), .RDERR(o_rderr), .WRERR(o_wrerr));
  always @(posedge clk) so <= ^o_do[15:0] ^ ^o_dop[1:0] ^ ^o_almostempty ^ ^o_almostfull ^ ^o_empty ^ ^o_full ^ ^o_rdcount ^ ^o_wrcount ^ ^o_rderr ^ ^o_wrerr ^ sr[38];
endmodule
