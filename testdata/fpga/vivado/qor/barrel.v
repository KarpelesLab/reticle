// A 32-bit rotate by a variable amount: many levels of small multiplexers.
module barrel (input wire clk, input wire si, output reg so);
  reg [36:0] in;
  reg [31:0] q;
  wire [63:0] twice = {in[31:0], in[31:0]};
  always @(posedge clk) begin
    in <= {in[35:0], si};
    q <= twice >> in[36:32];
    so <= ^q;
  end
endmodule
