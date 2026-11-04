module buf_ibufds_diff_out_ob_blvds_25(input s0, input s1, input s2, output r, input i_pad, input ib_pad);
  wire x_ob;
  IBUFDS_DIFF_OUT dut(.I(i_pad), .IB(ib_pad), .OB(x_ob));
  assign r = x_ob;
endmodule
