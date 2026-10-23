// Instance outputs connected to elements of a net array, as
// `.rdata(d[g])` connects each of a generate loop's FIFOs: an element is
// a target an `assign` can write but not an expression an instance can
// drive, so each port gets a net of its own, assigned into the element.
// This used to stop at an internal error from the IR validator.
module ona_inv(input [3:0] a, output [3:0] y);
  assign y = ~a;
endmodule

module output_to_net_array(input [3:0] a, output [3:0] y0, output [3:0] y1);
  wire [3:0] d [0:1];
  ona_inv u0 (.a(a), .y(d[0]));
  ona_inv u1 (.a(~a), .y(d[1]));
  assign y0 = d[0];
  assign y1 = d[1];
endmodule
