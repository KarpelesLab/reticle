module buf_iobufds_diff_out_lvds_25(input s0, input s1, input s2, output r, inout io_pad, inout iob_pad);
  wire x_o, x_ob;
  IOBUFDS_DIFF_OUT dut(.IO(io_pad), .IOB(iob_pad), .I(s0), .TM(s1), .TS(s2), .O(x_o), .OB(x_ob));
  assign r = x_o ^ x_ob;
endmodule
