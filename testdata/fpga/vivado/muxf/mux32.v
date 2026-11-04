// An inferred 32-to-1 multiplexer into a flip-flop.
module mux32 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [36:0] sr;
  always @(posedge clk) sr <= {sr[35:0], si};
  always @(posedge clk) so <= sr[sr[36:32]];
endmodule
