`timescale 1ns / 1ps
// FIFO18E1 and FIFO36E1 on one clock (EN_SYN TRUE): a reset held for six
// cycles, then writes and reads at random, a phase of mostly writes that
// fills each and one of mostly reads that empties it, and every output
// printed a nanosecond after each rising edge.
module tb;
  reg clk = 1'b0;
  always #5 clk = ~clk;
  reg rst = 1'b1;
  reg [31:0] lfsr = 32'h1234_abcd;
  reg [31:0] data = 32'h0;
  integer cycle = 0;
  reg wr = 1'b0, rd = 1'b0;
  always @(negedge clk) begin
    lfsr <= {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
    data <= {lfsr[7:0], lfsr[31:8]};
    cycle <= cycle + 1;
    rst <= cycle < 14;
    // Writes and reads once out of reset and settled; mostly writes from
    // cycle 40, mostly reads from 1200, both at random from 2300.
    wr <= cycle > 20 && (cycle < 1200 ? lfsr[0] | lfsr[1] : cycle < 2300 ? lfsr[0] & lfsr[1] & lfsr[2] : lfsr[3]);
    rd <= cycle > 20 && (cycle < 40 ? 1'b0 : cycle < 1200 ? lfsr[4] & lfsr[5] & lfsr[6] : cycle < 2300 ? lfsr[4] | lfsr[5] : lfsr[7]);
  end
  wire [31:0] do0, do1, do2;
  wire [63:0] do3;
  wire [3:0] dop0, dop1, dop2;
  wire [7:0] dop3;
  wire [11:0] rc0, wc0, rc1, wc1, rc2, wc2;
  wire [12:0] rc3, wc3;
  wire [5:0] f0, f1, f2, f3;
  FIFO18E1 #(.EN_SYN("TRUE"), .DATA_WIDTH(18), .DO_REG(0), .FIFO_MODE("FIFO18"),
    .ALMOST_EMPTY_OFFSET(13'h0006), .ALMOST_FULL_OFFSET(13'h0009))
    u0 (.DO(do0), .DOP(dop0), .ALMOSTEMPTY(f0[0]), .ALMOSTFULL(f0[1]), .EMPTY(f0[2]),
    .FULL(f0[3]), .RDERR(f0[4]), .WRERR(f0[5]), .RDCOUNT(rc0), .WRCOUNT(wc0), .DI(data),
    .DIP(lfsr[11:8]), .RDCLK(clk), .RDEN(rd), .REGCE(1'b1), .RST(rst), .RSTREG(1'b0),
    .WRCLK(clk), .WREN(wr));
  FIFO18E1 #(.EN_SYN("TRUE"), .DATA_WIDTH(9), .DO_REG(1), .FIFO_MODE("FIFO18"),
    .ALMOST_EMPTY_OFFSET(13'h0010), .ALMOST_FULL_OFFSET(13'h0020))
    u1 (.DO(do1), .DOP(dop1), .ALMOSTEMPTY(f1[0]), .ALMOSTFULL(f1[1]), .EMPTY(f1[2]),
    .FULL(f1[3]), .RDERR(f1[4]), .WRERR(f1[5]), .RDCOUNT(rc1), .WRCOUNT(wc1), .DI(data),
    .DIP(lfsr[11:8]), .RDCLK(clk), .RDEN(rd), .REGCE(1'b1), .RST(rst), .RSTREG(1'b0),
    .WRCLK(clk), .WREN(wr));
  FIFO18E1 #(.EN_SYN("TRUE"), .DATA_WIDTH(36), .DO_REG(0), .FIFO_MODE("FIFO18_36"),
    .ALMOST_EMPTY_OFFSET(13'h0005), .ALMOST_FULL_OFFSET(13'h0007))
    u2 (.DO(do2), .DOP(dop2), .ALMOSTEMPTY(f2[0]), .ALMOSTFULL(f2[1]), .EMPTY(f2[2]),
    .FULL(f2[3]), .RDERR(f2[4]), .WRERR(f2[5]), .RDCOUNT(rc2), .WRCOUNT(wc2), .DI(data),
    .DIP(lfsr[11:8]), .RDCLK(clk), .RDEN(rd), .REGCE(1'b1), .RST(rst), .RSTREG(1'b0),
    .WRCLK(clk), .WREN(wr));
  FIFO36E1 #(.EN_SYN("TRUE"), .DATA_WIDTH(4), .DO_REG(0), .FIFO_MODE("FIFO36"),
    .ALMOST_EMPTY_OFFSET(13'h0080), .ALMOST_FULL_OFFSET(13'h0080))
    u3 (.DO(do3), .DOP(dop3), .ALMOSTEMPTY(f3[0]), .ALMOSTFULL(f3[1]), .EMPTY(f3[2]),
    .FULL(f3[3]), .RDERR(f3[4]), .WRERR(f3[5]), .RDCOUNT(rc3), .WRCOUNT(wc3), .DI({32'h0, data}),
    .DIP({4'h0, lfsr[11:8]}), .RDCLK(clk), .RDEN(rd), .REGCE(1'b1), .RST(rst),
    .RSTREG(1'b0), .WRCLK(clk), .WREN(wr), .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0));
  always @(posedge clk) begin
    #1 $display("%0d %b%b %h %h %h %h %b %h %h %h %h %b %h %h %h %h %b %h %h %h %h %b %h %h",
      cycle, wr, rd, do0, dop0, rc0, wc0, f0, do1, dop1, rc1, wc1, f1, do2, dop2, rc2, wc2, f2,
      do3[31:0], dop3[3:0], rc3, wc3, f3, 1'b0, 1'b0);
    if (cycle == 3000) $finish;
  end
endmodule
