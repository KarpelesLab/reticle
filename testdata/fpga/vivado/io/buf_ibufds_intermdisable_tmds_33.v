module buf_ibufds_intermdisable_tmds_33(input s0, input s1, input s2, output r, input i_pad, input ib_pad);
  wire x_o;
  IBUFDS_INTERMDISABLE dut(.I(i_pad), .IB(ib_pad), .IBUFDISABLE(s0), .INTERMDISABLE(s1), .O(x_o));
  assign r = x_o;
endmodule
