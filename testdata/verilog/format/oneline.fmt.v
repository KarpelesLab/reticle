module oneline (
  input  wire       clk,
  input  wire       rst,
  input  wire [7:0] d,
  output reg  [7:0] q,
  output wire       parity
);
  reg [7:0] shadow;
  assign parity = ^q;
  always @(posedge clk or posedge rst) begin
    if (rst) begin
      q      <= 8'h00;
      shadow <= 8'hff;
    end else begin
      q      <= d;
      shadow <= q;
    end
  end
endmodule
