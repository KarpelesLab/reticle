`timescale 1ns / 1ps
// The IO buffers over legal levels: data, the differential pair, tristate and
// disable pins at 0 and 1, and a pad driven from outside only while its
// buffer is off (a single-ended one may also be left undriven; a differential
// pair is driven complementary or, with `extb`, equal). Each combination is
// visited twice in different orders, so that a differential input's equal
// levels show what they hold. One line per combination.
module tb;
  reg i = 0, ib = 0, t = 0, dis = 0, ext = 0, extb = 0, x = 0;
  wire pad, padb, pad2, pad2b, pad3, pad3b;
  wire pad1, pad4;
  assign pad1 = ext && t ? x : 1'bz;
  assign pad4 = ext && t ? x : 1'bz;
  assign pad = t ? x : 1'bz;
  assign padb = t ? (extb ? x : ~x) : 1'bz;
  assign pad2 = t ? x : 1'bz;
  assign pad2b = t ? (extb ? x : ~x) : 1'bz;
  assign pad3 = t ? x : 1'bz;
  assign pad3b = dis ? (extb ? x : ~x) : 1'bz;
  wire [39:0] o;
  IBUF u0 (.O(o[0]), .I(i));
  IBUFG u1 (.O(o[1]), .I(i));
  OBUF u2 (.O(o[2]), .I(i));
  OBUFT u3 (.O(o[3]), .I(i), .T(t));
  IBUFDS u4 (.O(o[4]), .I(i), .IB(ib));
  IBUFGDS u5 (.O(o[5]), .I(i), .IB(ib));
  OBUFDS u6 (.O(o[6]), .OB(o[7]), .I(i));
  OBUFTDS u7 (.O(o[8]), .OB(o[9]), .I(i), .T(t));
  IBUFDS_DIFF_OUT u8 (.O(o[10]), .OB(o[11]), .I(i), .IB(ib));
  IBUF_IBUFDISABLE u9 (.O(o[12]), .I(i), .IBUFDISABLE(dis));
  IBUF_INTERMDISABLE u10 (.O(o[13]), .I(i), .IBUFDISABLE(dis), .INTERMDISABLE(t));
  IBUFDS_IBUFDISABLE u11 (.O(o[14]), .I(i), .IB(ib), .IBUFDISABLE(dis));
  IBUFDS_INTERMDISABLE u12 (.O(o[15]), .I(i), .IB(ib), .IBUFDISABLE(dis), .INTERMDISABLE(t));
  IBUFDS_DIFF_OUT_IBUFDISABLE u13 (.O(o[16]), .OB(o[17]), .I(i), .IB(ib), .IBUFDISABLE(dis));
  IBUFDS_DIFF_OUT_INTERMDISABLE u14 (.O(o[18]), .OB(o[19]), .I(i), .IB(ib),
    .IBUFDISABLE(dis), .INTERMDISABLE(t));
  IBUF_IBUFDISABLE #(.USE_IBUFDISABLE("FALSE")) u15 (.O(o[20]), .I(i), .IBUFDISABLE(dis));
  IBUFDS_IBUFDISABLE #(.USE_IBUFDISABLE("FALSE")) u16 (.O(o[21]), .I(i), .IB(ib),
    .IBUFDISABLE(dis));
  IOBUF u17 (.O(o[22]), .IO(pad1), .I(i), .T(t));
  IOBUFDS u18 (.O(o[23]), .IO(pad), .IOB(padb), .I(i), .T(t));
  IOBUF_INTERMDISABLE u19 (.O(o[24]), .IO(pad4), .I(i), .T(t), .IBUFDISABLE(dis),
    .INTERMDISABLE(1'b0));
  IOBUFDS_INTERMDISABLE u20 (.O(o[25]), .IO(pad2), .IOB(pad2b), .I(i), .T(t),
    .IBUFDISABLE(dis), .INTERMDISABLE(1'b0));
  IOBUFDS_DIFF_OUT u21 (.O(o[26]), .OB(o[27]), .IO(pad3), .IOB(pad3b), .I(i), .TM(t),
    .TS(dis));
  IOBUF_INTERMDISABLE #(.USE_IBUFDISABLE("FALSE")) u22 (.O(o[28]), .IO(pad4), .I(i), .T(t),
    .IBUFDISABLE(dis), .INTERMDISABLE(1'b0));
  wire pu, pd;
  PULLUP u23 (.O(pu));
  PULLDOWN u24 (.O(pd));
  assign o[29] = pu;
  assign o[30] = pd;
  assign o[31] = pad1;
  assign o[32] = padb;
  assign o[33] = pad4;
  assign o[34] = pad2b;
  assign o[35] = pad3;
  assign o[36] = pad3b;
  wire o37, o38, pad5, pad5b;
  assign pad5 = t ? x : 1'bz;
  assign pad5b = t ? (extb ? x : ~x) : 1'bz;
  IOBUFDS_DIFF_OUT_INTERMDISABLE u25 (.O(o37), .OB(o38), .IO(pad5), .IOB(pad5b), .I(ib),
    .TM(t), .TS(t), .IBUFDISABLE(dis), .INTERMDISABLE(1'b0));
  assign o[37] = o37;
  assign o[38] = o38;
  assign o[39] = 1'b0;
  integer n, m, k;
  initial begin
    for (m = 0; m < 2; m = m + 1)
      for (n = 0; n < 128; n = n + 1) begin
        k = m ? n * 37 % 128 : n;
        {extb, ext, x, dis, t, ib, i} = k[6:0];
        #1 $display("%b %b", k[6:0], o);
      end
    $finish;
  end
endmodule
