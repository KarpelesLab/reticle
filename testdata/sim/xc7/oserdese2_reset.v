`timescale 1ns / 1ps
// OSERDESE2 coming out of reset: an SDR 2 and a DDR 4 for each of eight
// moments RST falls, every input high, so that the first bit each puts out
// shows when its output starts.
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
  reg rst0 = 1'b1;
  initial #211 rst0 = 1'b0;
  reg rst1 = 1'b1;
  initial #213.5 rst1 = 1'b0;
  reg rst2 = 1'b1;
  initial #215 rst2 = 1'b0;
  reg rst3 = 1'b1;
  initial #219 rst3 = 1'b0;
  reg rst4 = 1'b1;
  initial #223 rst4 = 1'b0;
  reg rst5 = 1'b1;
  initial #225.5 rst5 = 1'b0;
  reg rst6 = 1'b1;
  initial #229 rst6 = 1'b0;
  reg rst7 = 1'b1;
  initial #233 rst7 = 1'b0;
  reg [31:0] lfsr = 32'h2468_ace1;
  initial begin
    #1;
    forever #2 lfsr = {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
  end
  wire [7:0] d = 8'hff;
  wire [3:0] t = 4'hf;
  wire oce = 1'b1, tce = 1'b1;
  wire [15:0] oq, tq;
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u0 (.OQ(oq[0]), .TQ(tq[0]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst0), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u1 (.OQ(oq[1]), .TQ(tq[1]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst0), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u2 (.OQ(oq[2]), .TQ(tq[2]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst1), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u3 (.OQ(oq[3]), .TQ(tq[3]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst1), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u4 (.OQ(oq[4]), .TQ(tq[4]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst2), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u5 (.OQ(oq[5]), .TQ(tq[5]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst2), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u6 (.OQ(oq[6]), .TQ(tq[6]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst3), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u7 (.OQ(oq[7]), .TQ(tq[7]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst3), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u8 (.OQ(oq[8]), .TQ(tq[8]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst4), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u9 (.OQ(oq[9]), .TQ(tq[9]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst4), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u10 (.OQ(oq[10]), .TQ(tq[10]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst5), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u11 (.OQ(oq[11]), .TQ(tq[11]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst5), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u12 (.OQ(oq[12]), .TQ(tq[12]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst6), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u13 (.OQ(oq[13]), .TQ(tq[13]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst6), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("SDR"), .DATA_RATE_TQ("SDR"), .DATA_WIDTH(2), .TRISTATE_WIDTH(1)) u14 (.OQ(oq[14]), .TQ(tq[14]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst7), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  OSERDESE2 #(.DATA_RATE_OQ("DDR"), .DATA_RATE_TQ("DDR"), .DATA_WIDTH(4), .TRISTATE_WIDTH(4)) u15 (.OQ(oq[15]), .TQ(tq[15]), .CLK(clk), .CLKDIV(div2), .D1(d[0]), .D2(d[1]), .D3(d[2]), .D4(d[3]), .D5(d[4]), .D6(d[5]), .D7(d[6]), .D8(d[7]), .OCE(oce), .RST(rst7), .T1(t[0]), .T2(t[1]), .T3(t[2]), .T4(t[3]), .TCE(tce), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0), .TBYTEIN(1'b0));
  integer i;
  initial begin
    #0.5;
    for (i = 0; i < 400; i = i + 1) begin
      #2 $display("%0d %b %b %b", i, oq, tq, 2'b11);
    end
    $finish;
  end
endmodule
