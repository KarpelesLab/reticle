// A counter with a terminal compare: the carry chain and its entry.
module counter64 (input wire clk, input wire si, output reg so);
  reg [63:0] limit, count;
  always @(posedge clk) begin
    limit <= {limit[62:0], si};
    count <= count == limit ? 64'd0 : count + 64'd1;
    so <= count == limit;
  end
endmodule
