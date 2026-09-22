// A placeholder body: this package exists for its manifest's version
// requirement, which nothing in the tree can satisfy.
module uart_strict(
  input  clk,
  input  rst_n,
  output tx
);
  reg tx_q;
  assign tx = tx_q;
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) tx_q <= 1'b1;
    else        tx_q <= ~tx_q;
  end
endmodule
