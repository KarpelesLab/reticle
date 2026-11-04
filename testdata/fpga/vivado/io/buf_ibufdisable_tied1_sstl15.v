module buf_ibufdisable_tied1_sstl15(input s0, input s1, input s2, output r, input i_pad);
  wire x_o;
  IBUF_IBUFDISABLE dut(.I(i_pad), .IBUFDISABLE(1'b1), .O(x_o));
  assign r = x_o;
endmodule
