module buf_ibuf_params(input s0, input s1, input s2, output r, input i_pad);
  wire x_o;
  IBUF #(.IOSTANDARD("SSTL15"),.IBUF_LOW_PWR("FALSE")) dut(.I(i_pad), .O(x_o));
  assign r = x_o;
endmodule
