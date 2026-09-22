// Two flip-flops in the destination domain, which is the whole of a
// single-bit CDC synchroniser.
module cdc_sync(
  input  clk,
  input  rst_n,
  input  d,
  output q
);
  reg [1:0] sync_q;

  assign q = sync_q[1];

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) sync_q <= 2'b00;
    else        sync_q <= {sync_q[0], d};
  end
endmodule
