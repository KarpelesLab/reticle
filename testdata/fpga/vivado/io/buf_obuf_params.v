module buf_obuf_params(input s0, input s1, input s2, output r, output o_pad);
  OBUF #(.IOSTANDARD("LVCMOS33"),.DRIVE(8),.SLEW("FAST")) dut(.O(o_pad), .I(s0));
  assign r = 1'b0;
endmodule
