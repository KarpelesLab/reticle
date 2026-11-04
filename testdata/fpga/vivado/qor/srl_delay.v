// A long delay line with no reset: shift registers, not flip-flops.
module srl_delay (input wire clk, input wire si, output reg so);
  reg [31:0] in;
  reg [31:0] d [0:63];
  integer i;
  always @(posedge clk) begin
    in <= {in[30:0], si};
    d[0] <= in;
    for (i = 1; i < 64; i = i + 1) d[i] <= d[i - 1];
    so <= ^d[63];
  end
endmodule
