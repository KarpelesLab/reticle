`timescale 1ns / 1ps
// The regional and horizontal clock buffers: BUFR in every divide, with its
// enable and clear changing at random away from the input's edges; BUFH,
// BUFHCE, BUFIO, BUFMR and BUFMRCE beside it. Each output change is
// printed with the quarter nanosecond it fell in.
module tb;
  reg i = 1'b0;
  always #5 i = ~i;
  reg [31:0] lfsr = 32'h1357_2468;
  wire ce = lfsr[0] | lfsr[1] | lfsr[2];
  wire clr = lfsr[3] & lfsr[4] & lfsr[5] & lfsr[6];
  wire [17:0] o;
  BUFR #(.BUFR_DIVIDE("BYPASS")) u0 (.O(o[0]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("1")) u1 (.O(o[1]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("2")) u2 (.O(o[2]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("3")) u3 (.O(o[3]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("4")) u4 (.O(o[4]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("5")) u5 (.O(o[5]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("6")) u6 (.O(o[6]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("7")) u7 (.O(o[7]), .CE(ce), .CLR(clr), .I(i));
  BUFR #(.BUFR_DIVIDE("8")) u8 (.O(o[8]), .CE(ce), .CLR(clr), .I(i));
  BUFH u9 (.O(o[9]), .I(i));
  BUFHCE u10 (.O(o[10]), .CE(ce), .I(i));
  BUFHCE #(.INIT_OUT(1)) u11 (.O(o[11]), .CE(ce), .I(i));
  BUFHCE #(.CE_TYPE("ASYNC")) u12 (.O(o[12]), .CE(ce), .I(i));
  BUFIO u13 (.O(o[13]), .I(i));
  BUFMR u14 (.O(o[14]), .I(i));
  BUFMRCE u15 (.O(o[15]), .CE(ce), .I(i));
  BUFMRCE #(.INIT_OUT(1), .IS_CE_INVERTED(1'b1)) u16 (.O(o[16]), .CE(~ce), .I(i));
  BUFHCE #(.IS_CE_INVERTED(1'b1)) u17 (.O(o[17]), .CE(~ce), .I(i));
  integer tick = 0;
  reg [17:0] last = 18'bx;
  initial begin
    #0.125;
    forever begin
      if (o !== last) $display("%0d %b", tick, o);
      last = o;
      #0.25 tick = tick + 1;
    end
  end
  integer k;
  initial begin
    #2.75;
    for (k = 0; k < 120; k = k + 1) begin
      #(k % 3 == 0 ? 11.5 : 16) lfsr = {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
    end
    #50 $finish;
  end
endmodule
