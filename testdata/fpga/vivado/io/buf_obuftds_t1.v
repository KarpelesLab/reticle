module buf_obuftds_t1(input s0, input s1, input s2, output r, output o_pad, output ob_pad);
  OBUFTDS dut(.O(o_pad), .OB(ob_pad), .I(s0), .T(1'b1));
  assign r = 1'b0;
endmodule
