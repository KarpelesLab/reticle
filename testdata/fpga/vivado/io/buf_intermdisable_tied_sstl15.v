module buf_intermdisable_tied_sstl15(input s0, input s1, input s2, output r, input i_pad);
  wire x_o;
  IBUF_INTERMDISABLE dut(.I(i_pad), .IBUFDISABLE(s0), .INTERMDISABLE(1'b0), .O(x_o));
  assign r = x_o;
endmodule
