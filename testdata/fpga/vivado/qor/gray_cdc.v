// A Gray-coded pointer crossing between two clocks, with exceptions to honour.
module gray_cdc (input wire clk, input wire clk2, input wire si, output reg so);
  reg [11:0] wp, wg, s1, s2, rg;
  always @(posedge clk) begin
    if (si) wp <= wp + 12'd1;
    wg <= wp ^ (wp >> 1);
  end
  always @(posedge clk2) begin
    s1 <= wg;
    s2 <= s1;
    rg <= s2;
    so <= ^rg;
  end
endmodule
