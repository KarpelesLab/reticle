// A counter and a top level that instantiates it, for the language
// server's Verilog session.
module counter (
  input  wire      clk,
  input  wire      rst,
  output reg [7:0] q
);
  wire [7:0] next;

  assign next = q + 8'd1;

  always @(posedge clk) begin
    if (rst) q <= 8'd0;
    else q <= next;
  end
endmodule

module top (
  input  wire      clk,
  input  wire      rst,
  output wire [7:0] value
);
  counter u_counter (
    .clk(clk),
    .rst(rst),
    .q(value)
  );
endmodule
