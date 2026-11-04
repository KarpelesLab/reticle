module buf_ibufgds_diff_out_blvds_25(input s0, input s1, input s2, output r, input i_pad, input ib_pad);
  wire x_o, x_ob;
  IBUFGDS_DIFF_OUT dut(.I(i_pad), .IB(ib_pad), .O(x_o), .OB(x_ob));
  assign r = x_o ^ x_ob;
endmodule
