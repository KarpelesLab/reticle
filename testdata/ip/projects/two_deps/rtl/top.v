// The only HDL the user of this project writes: a top level that wires
// the board's pins to the IP.
module top(
  input       clk,
  input       rst_n,
  input [7:0] data,
  input       valid,
  output      ready,
  output      tx
);
  uart_lite u_uart (
    .clk      (clk),
    .rst_n    (rst_n),
    .tx_data  (data),
    .tx_valid (valid),
    .tx_ready (ready),
    .tx       (tx)
  );
endmodule
