// r_a1m1
module r_a1m1 (input wire clk, input wire si, output reg so);
  reg [39:0] in;
  wire signed [17:0] a0 = in[17:0];
  wire signed [17:0] b0 = in[35:18];
  wire rr = in[38];
  reg signed [17:0] a1;
  reg signed [17:0] b1;
  wire signed [35:0] m0 = a1 * b1;
  reg signed [35:0] m1;
  always @(posedge clk) begin in <= {in[38:0], si}; so <= ^m1; end
  always @(posedge clk) begin
     a1 <= a0;
     b1 <= b0;
     m1 <= m0;
  end
endmodule
