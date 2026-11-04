module left_hstl18(input en, input dn, output r, output a0, inout a1);
  assign a0 = dn;
  wire x1; IOBUF u1(.IO(a1), .I(dn), .T(en), .O(x1));
  assign r = x1;
endmodule
