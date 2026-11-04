// An inferred 16-to-1 multiplexer into a flip-flop.
module mux16 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [19:0] sr;
  always @(posedge clk) sr <= {sr[18:0], si};
  always @(posedge clk) so <= sr[sr[19:16]];
endmodule
