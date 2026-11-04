module buf_ibufgds_tmds_33(input s0, input s1, input s2, output r, input i_pad, input ib_pad);
  wire x_o;
  IBUFGDS dut(.I(i_pad), .IB(ib_pad), .O(x_o));
  assign r = x_o;
endmodule
