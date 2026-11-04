module buf_iobuf_t1(input s0, input s1, input s2, output r, inout io_pad);
  wire x_o;
  IOBUF dut(.IO(io_pad), .I(s0), .T(1'b1), .O(x_o));
  assign r = x_o;
endmodule
