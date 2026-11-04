module buf_obuft_params(input s0, input s1, input s2, output r, output o_pad);
  OBUFT #(.IOSTANDARD("LVCMOS25"),.DRIVE(16)) dut(.O(o_pad), .I(s0), .T(s1));
  assign r = 1'b0;
endmodule
