module buf_ibufdisable_tied0_sstl15(input s0, input s1, input s2, output r, input i_pad);
  wire x_o;
  IBUF_IBUFDISABLE dut(.I(i_pad), .IBUFDISABLE(1'b0), .O(x_o));
  assign r = x_o;
endmodule
