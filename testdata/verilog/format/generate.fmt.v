// Generate constructs: if / else if / case / for, labelled and unlabelled,
// nested, with and without generate / endgenerate, and genvar forms.
module gen #(
  parameter N = 4,
  parameter MODE = 1,
  parameter USE_REG = 0
) (
  input            clk,
  input  [N - 1:0] a,
  output [N - 1:0] y
);
  genvar i, j;

  generate
    for (i = 0; i < N; i = i + 1) begin : g_loop
      wire t = a[i];
      assign y[i] = ~t;
    end
  endgenerate

  for (genvar k = 0; k < N; k++) begin : g_sv
    wire [1:0] pair = {a[k], a[k]};
  end

  generate
    if (MODE == 0) begin : g_mode0
      assign y = a;
    end else begin
      if (MODE == 1) begin : g_mode1
        assign y = ~a;
      end else begin
        assign y = {N{1'b0}};
      end
    end
  endgenerate

  if (USE_REG) begin
    reg [N - 1:0] r;
  end else begin
    wire [N - 1:0] r = a;
  end

  generate
    case (MODE)
      0:       begin : g_c0
        assign y = a;
      end
      1, 2:    begin : g_c12
        assign y = a + 1;
      end
      default: begin
        assign y = 'x;
      end
    endcase
  endgenerate

  generate
    for (i = 0; i < 2; i = i + 1) begin : g_outer
      for (j = 0; j < 2; j = j + 1) begin : g_inner
        wire cell_w = a[i] & a[j];
        if (i == j) begin : g_diag
          wire d = cell_w;
        end
      end
    end
  endgenerate

  generate
    begin : g_bare
      wire bare = 1'b1;
    end
  endgenerate

  if (N > 2) begin
    wire big = 1'b1;
  end
  if (N > 8) ; else begin : g_small
    wire sm = 1'b1;
  end
  for (i = 0; i < N; i = i + 1) begin
    assign y[i] = a[i];
  end
  for (genvar m = N - 1; m >= 0; m--) begin
    always @(posedge clk) $display("%0d", m);
  end
endmodule
