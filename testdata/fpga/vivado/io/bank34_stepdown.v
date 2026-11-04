module bank34_stepdown(input en, input dn, output r, output a0);
  assign a0 = dn;
  assign r = 1'b0;
endmodule
