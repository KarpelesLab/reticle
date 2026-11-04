module buf_iobufds_intermdisable_lvds_25(input s0, input s1, input s2, output r, inout io_pad, inout iob_pad);
  wire x_o;
  IOBUFDS_INTERMDISABLE dut(.IO(io_pad), .IOB(iob_pad), .I(s0), .T(s1), .IBUFDISABLE(s2), .INTERMDISABLE(s2), .O(x_o));
  assign r = x_o;
endmodule
