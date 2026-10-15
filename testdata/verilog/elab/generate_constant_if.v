// A generate loop whose iterations each write one register under a
// condition on the loop variable: only the iteration the condition
// selects is a driver, so this elaborates with one driver per register.
// Before, every iteration counted, and NewAE's Husky SAD reported each
// such register driven from 255 places.
module generate_constant_if(input clk, input [3:0] d, output reg [3:0] q, output reg first);
  genvar i;
  generate
    for (i = 0; i < 4; i = i + 1) begin : g
      always @(posedge clk) begin
        q[i] <= d[i];
        if (i == 0) first <= d[0];
      end
    end
  endgenerate
endmodule
