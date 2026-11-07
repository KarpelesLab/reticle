// 128 words of 8, ram_style "registers".
module style_registers_128x8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [23:0] sr;
  always @(posedge clk) sr <= {sr[22:0], si};
  (* ram_style = "registers" *) reg [7:0] mem [0:127];
  reg [7:0] q0;
  always @(posedge clk) if (sr[15]) mem[sr[6:0]] <= sr[14:7];
  always @(posedge clk) q0 <= mem[sr[22:16]];
  always @(posedge clk) so <= ^(q0) ^ sr[23];
endmodule
