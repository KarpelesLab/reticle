// r_a2
module r_a2 (input wire clk, input wire si, output reg so);
  reg [39:0] in;
  wire signed [17:0] a0 = in[17:0];
  wire signed [17:0] b0 = in[35:18];
  wire rr = in[38];
  reg signed [17:0] a1;
  reg signed [17:0] a2;
  reg signed [17:0] b1;
  reg signed [17:0] b2;
  wire signed [35:0] m0 = a2 * b2;
  always @(posedge clk) begin in <= {in[38:0], si}; so <= ^m0; end
  always @(posedge clk) begin
     a1 <= a0;
     a2 <= a1;
     b1 <= b0;
     b2 <= b1;
  end
endmodule
