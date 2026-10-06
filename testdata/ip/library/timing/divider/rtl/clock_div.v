// A divide-by-sixteen tick, with the edge detector it needs coming from
// another package of the same library.
module clock_div(input clk, input rst_n, output tick);
  reg [3:0] count;
  always @(posedge clk) begin
    if (!rst_n)
      count <= 4'd0;
    else
      count <= count + 4'd1;
  end
  pulse_edge u_edge(.clk(clk), .level(count[3]), .pulse(tick));
endmodule
