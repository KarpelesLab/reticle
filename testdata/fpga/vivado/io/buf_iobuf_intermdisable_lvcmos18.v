module buf_iobuf_intermdisable_lvcmos18(input s0, input s1, input s2, output r, inout io_pad);
  wire x_o;
  IOBUF_INTERMDISABLE dut(.IO(io_pad), .I(s0), .T(s1), .IBUFDISABLE(s2), .INTERMDISABLE(s2), .O(x_o));
  assign r = x_o;
endmodule
