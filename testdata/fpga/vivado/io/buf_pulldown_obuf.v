module buf_pulldown_obuf(input s0, input s1, input s2, output r, output o_pad);
  OBUF dut(.O(o_pad), .I(s0));
  PULLDOWN p(.O(o_pad));
  assign r = 1'b0;
endmodule
