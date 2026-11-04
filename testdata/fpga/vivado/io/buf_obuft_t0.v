module buf_obuft_t0(input s0, input s1, input s2, output r, output o_pad);
  OBUFT dut(.O(o_pad), .I(s0), .T(1'b0));
  assign r = 1'b0;
endmodule
