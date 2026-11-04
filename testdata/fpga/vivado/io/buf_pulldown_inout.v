module buf_pulldown_inout(input s0, input s1, input s2, output r, inout o_pad);
  PULLDOWN p(.O(o_pad));
  assign r = 1'b0;
endmodule
