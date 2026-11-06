`timescale 1ns / 1ps
// OSERDESE2 at every SDR width from 2 to 8 and DDR 4, 6 and 8: one fast clock,
// a divided clock rising with it for each ratio, a reset released between
// edges, pseudo-random data and tristate words, and OQ and TQ printed half a
// nanosecond after every fast edge.
module tb;
  reg clk = 1'b0;
  integer half = 0;
  reg div2 = 1'b0;
  reg div3 = 1'b0;
  reg div4 = 1'b0;
  reg div5 = 1'b0;
  reg div6 = 1'b0;
  reg div7 = 1'b0;
  reg div8 = 1'b0;
  always begin
    #2 half = half + 1;
    clk = half % 2 == 1;
    div2 = (half + 3) % 4 < 2;
    div3 = (half + 5) % 6 < 3;
    div4 = (half + 7) % 8 < 4;
    div5 = (half + 9) % 10 < 5;
    div6 = (half + 11) % 12 < 6;
    div7 = (half + 13) % 14 < 7;
    div8 = (half + 15) % 16 < 8;
  end
  reg rst = 1'b1;
  initial #211 rst = 1'b0;
  reg [31:0] lfsr = 32'h2468_ace1;
  initial begin
    #1;
    forever #2 lfsr = {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
  end
  wire [7:0] d = lfsr[7:0];
  wire [3:0] t = lfsr[11:8];
  wire oce = 1'b1, tce = 1'b1;
  wire [9:0] oq, tq;
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u0 (.OQ(oq[0]), .TQ(tq[0]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(3), .TRISTATE_WIDTH(1)) u1 (.OQ(oq[1]), .TQ(tq[1]), .CLK(clk), .CLKDIV(div3), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(1)) u2 (.OQ(oq[2]), .TQ(tq[2]), .CLK(clk), .CLKDIV(div4), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(5), .TRISTATE_WIDTH(1)) u3 (.OQ(oq[3]), .TQ(tq[3]), .CLK(clk), .CLKDIV(div5), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(6), .TRISTATE_WIDTH(1)) u4 (.OQ(oq[4]), .TQ(tq[4]), .CLK(clk), .CLKDIV(div6), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(7), .TRISTATE_WIDTH(1)) u5 (.OQ(oq[5]), .TQ(tq[5]), .CLK(clk), .CLKDIV(div7), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(8), .TRISTATE_WIDTH(1)) u6 (.OQ(oq[6]), .TQ(tq[6]), .CLK(clk), .CLKDIV(div8), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u7 (.OQ(oq[7]), .TQ(tq[7]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(6), .TRISTATE_WIDTH(1)) u8 (.OQ(oq[8]), .TQ(tq[8]), .CLK(clk), .CLKDIV(div3), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(8), .TRISTATE_WIDTH(1)) u9 (.OQ(oq[9]), .TQ(tq[9]), .CLK(clk), .CLKDIV(div4), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  integer i;
  initial begin
    #0.5;
    for (i = 0; i < 400; i = i + 1) begin
      #2 $display("%0d %b %b %b", i, oq, tq, {oce, tce});
    end
    $finish;
  end
endmodule
