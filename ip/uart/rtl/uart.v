// uart — an 8N1 UART: `uart_tx` and `uart_rx` sharing one baud divisor.
//
// What it does
//   Wires the transmitter and the receiver together behind one clock, one
//   reset and one divisor, which is what a design almost always wants:
//   the two halves of a serial port run at the same rate. Everything the
//   two sub-blocks do and do not do is in `uart_tx.v` and `uart_rx.v`;
//   this file adds nothing but the wiring.
//
//   The divisor is a port as well as a parameter. `div` is clock cycles
//   per bit at run time and CLK_DIV is what the block falls back to when
//   `div` is zero or too small to work with, so a design with a fixed
//   rate ties `div` to zero and behaves exactly as it did before the port
//   existed. `uart_baud_div` in this package turns a bit rate into a
//   `div`.
//
// What it does not do
//   There is no register interface — this is the raw ready/valid form. It
//   has no FIFO on either side, no flow control (`rts`/`cts`), no
//   auto-baud, no loopback mode and no interrupt output. A UART with
//   registers behind AXI4-Lite would be this block, two `fifo_sync`
//   instances and a register file; the pieces are all in this library.
//
//   Changing `div` does not resynchronise a character already in flight:
//   both halves read it as each bit begins, so a change mid-character
//   costs that character. Change the rate while the line is idle.
module uart #(
    // Clock cycles per bit when `div` does not give one, shared by both
    // halves. At least four.
    parameter CLK_DIV = 16
) (
    input  wire       clk,
    input  wire       rst_n,

    // Clock cycles per bit, shared by both halves. Zero means CLK_DIV.
    input  wire [15:0] div,

    input  wire [7:0] tx_data,
    input  wire       tx_valid,
    output wire       tx_ready,
    output wire       tx,

    input  wire       rx,
    output wire [7:0] rx_data,
    output wire       rx_valid,
    output wire       rx_error
);
    uart_tx #(
        .CLK_DIV (CLK_DIV)
    ) u_tx (
        .clk      (clk),
        .rst_n    (rst_n),
        .div      (div),
        .tx_data  (tx_data),
        .tx_valid (tx_valid),
        .tx_ready (tx_ready),
        .tx       (tx)
    );

    uart_rx #(
        .CLK_DIV (CLK_DIV)
    ) u_rx (
        .clk      (clk),
        .rst_n    (rst_n),
        .div      (div),
        .rx       (rx),
        .rx_data  (rx_data),
        .rx_valid (rx_valid),
        .rx_error (rx_error)
    );
endmodule
