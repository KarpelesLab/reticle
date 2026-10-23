// An output port connected to a constant is not a target at all, and says
// so where the connection is: it used to be reported as the IR validator's
// internal error.
module eoc_inv(input a, output y);
  assign y = ~a;
endmodule

module error_output_to_constant(input a);
  eoc_inv u0 (.a(a), .y(1'b0));
endmodule
