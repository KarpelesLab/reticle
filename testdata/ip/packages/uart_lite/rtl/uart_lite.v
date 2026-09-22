// An 8N1 transmitter: one start bit, eight data bits, one stop bit,
// shifted out least significant bit first, with a one-entry FIFO in
// front so the producer need not wait for the whole frame.
module uart_lite(
  input        clk,
  input        rst_n,
  input  [7:0] tx_data,
  input        tx_valid,
  output       tx_ready,
  output       tx
);
  wire [7:0] fifo_rdata;
  wire       fifo_full;
  wire       fifo_empty;
  reg  [3:0] bit_index;
  reg  [9:0] shifter;
  reg        busy;

  fifo_sync u_fifo (
    .clk   (clk),
    .rst_n (rst_n),
    .wdata (tx_data),
    .push  (tx_valid && !fifo_full),
    .pop   (!busy && !fifo_empty),
    .rdata (fifo_rdata),
    .full  (fifo_full),
    .empty (fifo_empty)
  );

  assign tx_ready = !fifo_full;
  assign tx       = shifter[0];

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      bit_index <= 4'd0;
      shifter   <= 10'h3ff;
      busy      <= 1'b0;
    end else if (!busy) begin
      if (!fifo_empty) begin
        shifter   <= {1'b1, fifo_rdata, 1'b0};
        bit_index <= 4'd0;
        busy      <= 1'b1;
      end
    end else begin
      shifter   <= {1'b1, shifter[9:1]};
      bit_index <= bit_index + 4'd1;
      if (bit_index == 4'd9) busy <= 1'b0;
    end
  end
endmodule
