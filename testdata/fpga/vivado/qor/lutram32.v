// A small memory read asynchronously: distributed RAM.
module lutram32 (input wire clk, input wire si, output reg so);
  reg [36:0] in;
  reg [31:0] mem [0:31];
  reg [31:0] q;
  always @(posedge clk) begin
    in <= {in[35:0], si};
    if (in[36]) mem[in[4:0]] <= in[35:5];
    q <= mem[in[9:5]];
    so <= ^q;
  end
endmodule
