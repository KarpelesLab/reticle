module top(input clk, input rst_n, output tick);
  clock_div u_div(.clk(clk), .rst_n(rst_n), .tick(tick));
endmodule
