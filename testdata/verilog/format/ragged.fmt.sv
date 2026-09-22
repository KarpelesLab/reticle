module ragged #(
  parameter int W = 4,
  parameter int D = 2
) (
  input  logic           clk,
  input  logic [W - 1:0] a,
  output logic [W - 1:0] y
);
  logic [W - 1:0] pipe[D];
  always_ff @(posedge clk) begin
    pipe[0] <= a;
    for (int i = 1; i < D; i++) pipe[i] <= pipe[i - 1];
  end
  assign y = pipe[D - 1];
endmodule
