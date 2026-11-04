module vref_ball_refused(input en, input dn, output r, input a0, input a1);
  assign r = a0 ^ a1;
endmodule
