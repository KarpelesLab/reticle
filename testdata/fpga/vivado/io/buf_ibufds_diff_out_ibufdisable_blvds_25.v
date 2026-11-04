module buf_ibufds_diff_out_ibufdisable_blvds_25(input s0, input s1, input s2, output r, input i_pad, input ib_pad);
  wire x_o, x_ob;
  IBUFDS_DIFF_OUT_IBUFDISABLE dut(.I(i_pad), .IB(ib_pad), .IBUFDISABLE(s0), .O(x_o), .OB(x_ob));
  assign r = x_o ^ x_ob;
endmodule
