// top: top_only
// With the top named, a module nothing under it instantiates is not
// elaborated, so its error is not reported: the multiply-driven register
// in `unused` would be one.
module unused(input c, input d);
  reg q;
  always @(posedge c) q <= 1'b0;
  always @(posedge d) q <= 1'b1;
endmodule

module top_only(input a, output y);
  assign y = ~a;
endmodule
