`timescale 1ns / 1ps
// ISERDESE2 coming out of reset: an SDR 2, a DDR 4 and an SDR 3 for each of
// eight moments RST falls, the input high, so that Q1 shows when each
// starts.
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
  wire d = 1'b1;
  wire [7:0] q0;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u0 (.Q1(q0[0]), .Q2(q0[1]), .Q3(q0[2]), .Q4(q0[3]), .Q5(q0[4]), .Q6(q0[5]), .Q7(q0[6]), .Q8(q0[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst0), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q1;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u1 (.Q1(q1[0]), .Q2(q1[1]), .Q3(q1[2]), .Q4(q1[3]), .Q5(q1[4]), .Q6(q1[5]), .Q7(q1[6]), .Q8(q1[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst0), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q2;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u2 (.Q1(q2[0]), .Q2(q2[1]), .Q3(q2[2]), .Q4(q2[3]), .Q5(q2[4]), .Q6(q2[5]), .Q7(q2[6]), .Q8(q2[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst0), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q3;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u3 (.Q1(q3[0]), .Q2(q3[1]), .Q3(q3[2]), .Q4(q3[3]), .Q5(q3[4]), .Q6(q3[5]), .Q7(q3[6]), .Q8(q3[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst1), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q4;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u4 (.Q1(q4[0]), .Q2(q4[1]), .Q3(q4[2]), .Q4(q4[3]), .Q5(q4[4]), .Q6(q4[5]), .Q7(q4[6]), .Q8(q4[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst1), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q5;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u5 (.Q1(q5[0]), .Q2(q5[1]), .Q3(q5[2]), .Q4(q5[3]), .Q5(q5[4]), .Q6(q5[5]), .Q7(q5[6]), .Q8(q5[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst1), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q6;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u6 (.Q1(q6[0]), .Q2(q6[1]), .Q3(q6[2]), .Q4(q6[3]), .Q5(q6[4]), .Q6(q6[5]), .Q7(q6[6]), .Q8(q6[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst2), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q7;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u7 (.Q1(q7[0]), .Q2(q7[1]), .Q3(q7[2]), .Q4(q7[3]), .Q5(q7[4]), .Q6(q7[5]), .Q7(q7[6]), .Q8(q7[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst2), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q8;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u8 (.Q1(q8[0]), .Q2(q8[1]), .Q3(q8[2]), .Q4(q8[3]), .Q5(q8[4]), .Q6(q8[5]), .Q7(q8[6]), .Q8(q8[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst2), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q9;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u9 (.Q1(q9[0]), .Q2(q9[1]), .Q3(q9[2]), .Q4(q9[3]), .Q5(q9[4]), .Q6(q9[5]), .Q7(q9[6]), .Q8(q9[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst3), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q10;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u10 (.Q1(q10[0]), .Q2(q10[1]), .Q3(q10[2]), .Q4(q10[3]), .Q5(q10[4]), .Q6(q10[5]), .Q7(q10[6]), .Q8(q10[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst3), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q11;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u11 (.Q1(q11[0]), .Q2(q11[1]), .Q3(q11[2]), .Q4(q11[3]), .Q5(q11[4]), .Q6(q11[5]), .Q7(q11[6]), .Q8(q11[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst3), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q12;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u12 (.Q1(q12[0]), .Q2(q12[1]), .Q3(q12[2]), .Q4(q12[3]), .Q5(q12[4]), .Q6(q12[5]), .Q7(q12[6]), .Q8(q12[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst4), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q13;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u13 (.Q1(q13[0]), .Q2(q13[1]), .Q3(q13[2]), .Q4(q13[3]), .Q5(q13[4]), .Q6(q13[5]), .Q7(q13[6]), .Q8(q13[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst4), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q14;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u14 (.Q1(q14[0]), .Q2(q14[1]), .Q3(q14[2]), .Q4(q14[3]), .Q5(q14[4]), .Q6(q14[5]), .Q7(q14[6]), .Q8(q14[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst4), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q15;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u15 (.Q1(q15[0]), .Q2(q15[1]), .Q3(q15[2]), .Q4(q15[3]), .Q5(q15[4]), .Q6(q15[5]), .Q7(q15[6]), .Q8(q15[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst5), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q16;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u16 (.Q1(q16[0]), .Q2(q16[1]), .Q3(q16[2]), .Q4(q16[3]), .Q5(q16[4]), .Q6(q16[5]), .Q7(q16[6]), .Q8(q16[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst5), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q17;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u17 (.Q1(q17[0]), .Q2(q17[1]), .Q3(q17[2]), .Q4(q17[3]), .Q5(q17[4]), .Q6(q17[5]), .Q7(q17[6]), .Q8(q17[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst5), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q18;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u18 (.Q1(q18[0]), .Q2(q18[1]), .Q3(q18[2]), .Q4(q18[3]), .Q5(q18[4]), .Q6(q18[5]), .Q7(q18[6]), .Q8(q18[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst6), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q19;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u19 (.Q1(q19[0]), .Q2(q19[1]), .Q3(q19[2]), .Q4(q19[3]), .Q5(q19[4]), .Q6(q19[5]), .Q7(q19[6]), .Q8(q19[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst6), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q20;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u20 (.Q1(q20[0]), .Q2(q20[1]), .Q3(q20[2]), .Q4(q20[3]), .Q5(q20[4]), .Q6(q20[5]), .Q7(q20[6]), .Q8(q20[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst6), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q21;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(2), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u21 (.Q1(q21[0]), .Q2(q21[1]), .Q3(q21[2]), .Q4(q21[3]), .Q5(q21[4]), .Q6(q21[5]), .Q7(q21[6]), .Q8(q21[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst7), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q22;
  ISERDESE2 #(.DATA_RATE("DDR"), .DATA_WIDTH(4), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u22 (.Q1(q22[0]), .Q2(q22[1]), .Q3(q22[2]), .Q4(q22[3]), .Q5(q22[4]), .Q6(q22[5]), .Q7(q22[6]), .Q8(q22[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div2), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst7), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  wire [7:0] q23;
  ISERDESE2 #(.DATA_RATE("SDR"), .DATA_WIDTH(3), .INTERFACE_TYPE("NETWORKING"), .IOBDELAY("NONE"), .NUM_CE(1)) u23 (.Q1(q23[0]), .Q2(q23[1]), .Q3(q23[2]), .Q4(q23[3]), .Q5(q23[4]), .Q6(q23[5]), .Q7(q23[6]), .Q8(q23[7]), .O(), .SHIFTOUT1(), .SHIFTOUT2(), .BITSLIP(1'b0), .CE1(1'b1), .CE2(1'b1), .CLKDIVP(1'b0), .CLK(clk), .CLKB(~clk), .CLKDIV(div3), .OCLK(1'b0), .OCLKB(1'b1), .D(d), .DDLY(1'b0), .DYNCLKDIVSEL(1'b0), .DYNCLKSEL(1'b0), .OFB(1'b0), .RST(rst7), .SHIFTIN1(1'b0), .SHIFTIN2(1'b0));
  integer i;
  initial begin
    #0.5;
    for (i = 0; i < 160; i = i + 1) begin
      #2 $display("%0d %b", i, {q23[0], q22[0], q21[0], q20[0], q19[0], q18[0], q17[0], q16[0], q15[0], q14[0], q13[0], q12[0], q11[0], q10[0], q9[0], q8[0], q7[0], q6[0], q5[0], q4[0], q3[0], q2[0], q1[0], q0[0]});
    end
    $finish;
  end
endmodule