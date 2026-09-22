// A pipelined multiply-accumulate with always_ff / always_comb /
// always_latch, non-blocking assignments and a struct-typed register.
module mac #(
  parameter int W = 16
) (
  input  logic                  clk,
  input  logic                  rst_n,
  input  logic                  clear,
  input  logic signed [W - 1:0] a,
                                b,
  output logic signed [2 * W:0] acc,
  output logic                  overflow
);
  typedef struct packed {
    logic signed [2 * W - 1:0] prod;
    logic                      valid;
  } stage_t;

  stage_t s1;
  logic signed [2 * W:0] sum;
  logic latch_q;

  always_ff @(posedge clk or negedge rst_n) begin : stage1
    if (!rst_n) begin
      s1 <= '{prod: '0, valid: 1'b0};
    end else begin
      s1.prod  <= a * b;
      s1.valid <= 1'b1;
    end
  end : stage1

  always_comb sum = acc + s1.prod;

  always_ff @(posedge clk) begin
    if (clear) acc <= '0;
    else if (s1.valid) acc <= sum;
  end

  always_ff @(posedge clk iff !clear) overflow <= sum[2 * W] ^ sum[2 * W - 1];

  always_latch if (clear) latch_q = 1'b0;
endmodule
