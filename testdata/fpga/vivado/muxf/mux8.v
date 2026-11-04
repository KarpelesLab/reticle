// An inferred 8-to-1 multiplexer into a flip-flop.
module mux8 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [10:0] sr;
  always @(posedge clk) sr <= {sr[9:0], si};
  always @(posedge clk) so <= sr[sr[10:8]];
endmodule
