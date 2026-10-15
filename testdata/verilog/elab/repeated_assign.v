// The same assignment made in every iteration of a generate loop that does
// not use the loop variable: one connection, not six drivers.
module repeated_assign(input [5:0] d, output [5:0] q);
  wire [5:0] delayed;
  genvar k;
  generate
    for (k = 0; k < 6; k = k + 1) begin : g
      assign delayed = d;
      assign q[k] = delayed[k];
    end
  endgenerate
endmodule
