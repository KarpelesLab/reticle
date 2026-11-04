module buf_iobuf_params(input s0, input s1, input s2, output r, inout io_pad);
  wire x_o;
  IOBUF #(.IOSTANDARD("LVCMOS18"),.DRIVE(4),.SLEW("FAST")) dut(.IO(io_pad), .I(s0), .T(s1), .O(x_o));
  assign r = x_o;
endmodule
