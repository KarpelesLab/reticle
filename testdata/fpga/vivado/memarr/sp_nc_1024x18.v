// 1024 words of 18: one address, no change.
module sp_nc_1024x18 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [29:0] sr;
  always @(posedge clk) sr <= {sr[28:0], si};
  reg [17:0] mem [0:1023];
  reg [17:0] q0;
  always @(posedge clk) begin
    if (sr[28]) mem[sr[9:0]] <= sr[27:10];
    else q0 <= mem[sr[9:0]];
  end
  always @(posedge clk) so <= ^(q0) ^ sr[29];
endmodule
