`timescale 1ns / 1ps
// ISERDESE2 in NETWORKING mode at every SDR width from 2 to 8 and DDR 4, 6
// and 8: one fast clock, a divided clock rising with it for each ratio, a
// serial input changing between edges, and Q1 to Q8 printed half a
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
  wire d = lfsr[0];
  reg slip = 1'b0;
  wire [7:0] q0;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u0 (.Q1(q0[0]), .Q2(q0[1]), .Q3(q0[2]), .Q4(q0[3]), .Q5(q0[4]), .Q6(q0[5]), .Q7(q0[6]), .Q8(q0[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q1;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u1 (.Q1(q1[0]), .Q2(q1[1]), .Q3(q1[2]), .Q4(q1[3]), .Q5(q1[4]), .Q6(q1[5]), .Q7(q1[6]), .Q8(q1[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q2;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u2 (.Q1(q2[0]), .Q2(q2[1]), .Q3(q2[2]), .Q4(q2[3]), .Q5(q2[4]), .Q6(q2[5]), .Q7(q2[6]), .Q8(q2[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div4), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q3;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(5), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u3 (.Q1(q3[0]), .Q2(q3[1]), .Q3(q3[2]), .Q4(q3[3]), .Q5(q3[4]), .Q6(q3[5]), .Q7(q3[6]), .Q8(q3[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div5), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q4;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(6), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u4 (.Q1(q4[0]), .Q2(q4[1]), .Q3(q4[2]), .Q4(q4[3]), .Q5(q4[4]), .Q6(q4[5]), .Q7(q4[6]), .Q8(q4[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div6), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q5;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(7), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u5 (.Q1(q5[0]), .Q2(q5[1]), .Q3(q5[2]), .Q4(q5[3]), .Q5(q5[4]), .Q6(q5[5]), .Q7(q5[6]), .Q8(q5[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div7), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q6;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(8), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u6 (.Q1(q6[0]), .Q2(q6[1]), .Q3(q6[2]), .Q4(q6[3]), .Q5(q6[4]), .Q6(q6[5]), .Q7(q6[6]), .Q8(q6[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div8), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q7;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u7 (.Q1(q7[0]), .Q2(q7[1]), .Q3(q7[2]), .Q4(q7[3]), .Q5(q7[4]), .Q6(q7[5]), .Q7(q7[6]), .Q8(q7[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q8;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(6), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u8 (.Q1(q8[0]), .Q2(q8[1]), .Q3(q8[2]), .Q4(q8[3]), .Q5(q8[4]), .Q6(q8[5]), .Q7(q8[6]), .Q8(q8[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q9;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(8), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u9 (.Q1(q9[0]), .Q2(q9[1]), .Q3(q9[2]), .Q4(q9[3]), .Q5(q9[4]), .Q6(q9[5]), .Q7(q9[6]), .Q8(q9[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(slip), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div4), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  integer i;
  initial begin
    #0.5;
    for (i = 0; i < 400; i = i + 1) begin
      #2 $display("%0d %b %b %b %b %b %b %b %b %b %b %b", i, d, q0, q1, q2, q3, q4, q5, q6, q7, q8, q9);
    end
    $finish;
  end
endmodule
