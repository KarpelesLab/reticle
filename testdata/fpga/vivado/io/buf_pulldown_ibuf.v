module buf_pulldown_ibuf(input s0, input s1, input s2, output r, input i_pad);
  wire x_o;
  IBUF dut(.I(i_pad), .O(x_o));
  PULLDOWN p(.O(i_pad));
  assign r = x_o;
endmodule
