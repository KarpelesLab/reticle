// The RTL `simple.xml` describes. Small on purpose: it is here so that
// an imported IP-XACT component can be resolved and elaborated like any
// other package, not to count quickly.
module counter(
  input        clk,
  input        rst_n,
  input        clear,
  output [7:0] count
);
  reg [7:0] count_q;

  assign count = count_q;

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      count_q <= 8'd0;
    end else if (clear) begin
      count_q <= 8'd0;
    end else begin
      count_q <= count_q + 8'd1;
    end
  end
endmodule
