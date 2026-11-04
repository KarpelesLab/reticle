module buf_pulldown_input(input s0, input s1, input s2, output r, input o_pad);
  PULLDOWN p(.O(o_pad));
  assign r = 1'b0;
endmodule
