// A one-entry synchronous FIFO. Small on purpose: it is here to be
// resolved, elaborated and instantiated, not to be fast.
//
// The reset is brought through `cdc_sync`, which is why this package has
// a dependency at all.
module fifo_sync(
  input        clk,
  input        rst_n,
  input  [7:0] wdata,
  input        push,
  input        pop,
  output [7:0] rdata,
  output       full,
  output       empty
);
  reg [7:0] data_q;
  reg       valid_q;
  wire      rst_sync_n;

  cdc_sync u_rst_sync (
    .clk   (clk),
    .rst_n (rst_n),
    .d     (1'b1),
    .q     (rst_sync_n)
  );

  assign rdata = data_q;
  assign full  = valid_q;
  assign empty = !valid_q;

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      data_q  <= 8'd0;
      valid_q <= 1'b0;
    end else if (!rst_sync_n) begin
      valid_q <= 1'b0;
    end else if (push && !valid_q) begin
      data_q  <= wdata;
      valid_q <= 1'b1;
    end else if (pop && valid_q) begin
      valid_q <= 1'b0;
    end
  end
endmodule
