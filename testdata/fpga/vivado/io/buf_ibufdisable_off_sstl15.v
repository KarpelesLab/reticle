module buf_ibufdisable_off_sstl15(input s0, input s1, input s2, output r, input i_pad);
  wire x_o;
  IBUF_IBUFDISABLE #(.USE_IBUFDISABLE("FALSE")) dut(.I(i_pad), .IBUFDISABLE(s0), .O(x_o));
  assign r = x_o;
endmodule
