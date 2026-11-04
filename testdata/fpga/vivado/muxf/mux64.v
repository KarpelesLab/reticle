// An inferred 64-to-1 multiplexer into a flip-flop.
module mux64 (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [69:0] sr;
  always @(posedge clk) sr <= {sr[68:0], si};
  always @(posedge clk) so <= sr[sr[69:64]];
endmodule
