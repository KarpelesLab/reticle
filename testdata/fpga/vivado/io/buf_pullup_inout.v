module buf_pullup_inout(input s0, input s1, input s2, output r, inout o_pad);
  PULLUP p(.O(o_pad));
  assign r = 1'b0;
endmodule
