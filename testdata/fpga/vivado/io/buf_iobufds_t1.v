module buf_iobufds_t1(input s0, input s1, input s2, output r, inout io_pad, inout iob_pad);
  wire x_o;
  IOBUFDS dut(.IO(io_pad), .IOB(iob_pad), .I(s0), .T(1'b1), .O(x_o));
  assign r = x_o;
endmodule
