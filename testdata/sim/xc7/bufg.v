`timescale 1ns / 1ps
// The global clock buffers: two clocks of unrelated periods, a select and
// enables that change at awkward moments, and every output printed each
// nanosecond.
module tb;
  reg i0 = 1'b0, i1 = 1'b0;
  always #5 i0 = ~i0;
  always #7 i1 = ~i1;
  reg sel = 1'b0, ce = 1'b1, ce0 = 1'b1, ce1 = 1'b1, s0 = 1'b1, s1 = 1'b0, ig0 = 1'b0, ig1 = 1'b0;
  wire [10:0] o;
  BUFG u0 (.O(o[0]), .I(i0));
  BUFGCE u1 (.O(o[1]), .CE(ce), .I(i0));
  BUFGCE_1 u2 (.O(o[2]), .CE(ce), .I(i0));
  BUFGMUX u3 (.O(o[3]), .I0(i0), .I1(i1), .S(sel));
  BUFGMUX_1 u4 (.O(o[4]), .I0(i0), .I1(i1), .S(sel));
  BUFGMUX_CTRL u5 (.O(o[5]), .I0(i0), .I1(i1), .S(sel));
  BUFGCTRL u6 (.O(o[6]), .I0(i0), .I1(i1), .CE0(ce0), .CE1(ce1), .S0(s0), .S1(s1),
    .IGNORE0(ig0), .IGNORE1(ig1));
  BUFGCTRL #(.INIT_OUT(1), .PRESELECT_I1("TRUE"), .IS_CE0_INVERTED(1'b1))
    u7 (.O(o[7]), .I0(i0), .I1(i1), .CE0(~ce0), .CE1(ce1), .S0(s0), .S1(s1),
    .IGNORE0(ig0), .IGNORE1(ig1));
  BUFGP u8 (.O(o[8]), .I(i1));
  BUFGCE #(.CE_TYPE("ASYNC")) u9 (.O(o[9]), .CE(ce), .I(i0));
  BUFGCTRL #(.PRESELECT_I0("TRUE"), .IS_I0_INVERTED(1'b1), .IS_S1_INVERTED(1'b1))
    u10 (.O(o[10]), .I0(i0), .I1(i1), .CE0(ce0), .CE1(ce1), .S0(s0), .S1(~s1),
    .IGNORE0(ig0), .IGNORE1(ig1));
  integer i;
  initial begin
    for (i = 0; i < 700; i = i + 1) begin
      #0.5 $display("%0d %b", i, o);
      #0.5;
      case (i)
        130: ce = 0;
        163: ce = 1;
        171: sel = 1;
        212: sel = 0;
        250: begin s0 = 0; s1 = 1; end
        262: ce = 0;
        264: ce = 1;
        290: ce1 = 0;
        320: ce1 = 1;
        345: begin s0 = 1; s1 = 0; end
        380: ig0 = 1;
        381: begin s0 = 0; s1 = 1; end
        400: ig1 = 1;
        403: begin s0 = 1; s1 = 0; end
        430: begin ig0 = 0; ig1 = 0; end
        455: ce0 = 0;
        480: begin s0 = 0; s1 = 1; end
        510: ce0 = 1;
        511: sel = 1;
        540: begin s0 = 1; s1 = 1; end
        560: begin s0 = 0; s1 = 0; end
        600: begin s0 = 1; s1 = 0; ce0 = 1; end
        650: ce = 0;
        652: ce = 1;
      endcase
    end
    $finish;
  end
endmodule
