module crlf (input wire clk, input wire d, output reg q);
  // a comment with a carriage return before the newline
  always @(posedge clk) begin
    q <= d;
  end
endmodule
