module buf_keeper_iobuf(input s0, input s1, input s2, output r, inout io_pad);
  wire x_o;
  IOBUF dut(.IO(io_pad), .I(s0), .T(s1), .O(x_o));
  KEEPER p(.O(io_pad));
  assign r = x_o;
endmodule
