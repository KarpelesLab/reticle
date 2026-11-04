module buf_keeper_input(input s0, input s1, input s2, output r, input o_pad);
  KEEPER p(.O(o_pad));
  assign r = 1'b0;
endmodule
