// uart — a UART, 8N1 by default: `uart_tx` and `uart_rx` sharing one
// baud divisor and one framing.
//
// What it does
//   Wires the transmitter and the receiver together behind one clock, one
//   reset, one divisor and one set of framing parameters, which is what a
//   design almost always wants: the two halves of a serial port run at
//   the same rate and with the same framing. Everything the two
//   sub-blocks do and do not do is in `uart_tx.v` and `uart_rx.v`; this
//   file adds nothing but the wiring.
//
//   The divisor is a port as well as a parameter. `div` is clock cycles
//   per bit at run time and CLK_DIV is what the block falls back to when
//   `div` is zero or too small to work with, so a design with a fixed
//   rate ties `div` to zero and behaves exactly as it did before the port
//   existed. `uart_baud_div` in this package turns a bit rate into a
//   `div`.
//
//   The **framing is parameters only** — `DATA_BITS`, `PARITY`,
//   `STOP_BITS`, defaulting to 8, none and 1 — so an instantiation
//   written before they existed means exactly what it always did. A
//   design whose framing a host chooses at run time instantiates
//   `uart_frame`, which is this wiring around the configurable halves and
//   takes the format on three ports. `ip/bus/uart/README.md` §2 says why
//   the split exists and what each side of it costs.
//
// What it does not do
//   There is no register interface — this is the raw ready/valid form. It
//   has no FIFO on either side, no flow control (`rts`/`cts`), no
//   auto-baud, no loopback mode and no interrupt output. A UART with
//   registers behind AXI4-Lite would be this block, two `fifo_sync`
//   instances and a register file; the pieces are all in this library.
//
//   It reports no overrun, because `rx_valid` here is a strobe with no
//   `rx_ready` to be not-ready: `uart_rx`'s header says what that means
//   and `uart_frame` is the version that has the handshake.
//
//   Changing `div` does not resynchronise a character already in flight:
//   both halves read it as each bit begins, so a change mid-character
//   costs that character. Change the rate while the line is idle.
module uart #(
    // Clock cycles per bit when `div` does not give one, shared by both
    // halves. At least four.
    parameter CLK_DIV = 16,
    // Data bits in a character: 5, 6, 7 or 8. Anything else is 8.
    parameter DATA_BITS = 8,
    // `bParityType` of PSTN 1.2: 0 none, 1 odd, 2 even, 3 mark, 4 space.
    // Anything else is none.
    parameter PARITY = 0,
    // Stop bits sent: 1 or 2. Anything else is 2. The receiver samples
    // the first stop bit and nothing after it, so this reaches the
    // transmitter only.
    parameter STOP_BITS = 1
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
    // Any of the three below it; see `uart_rx.v`.
    output wire       rx_error,
    output wire       rx_frame_error,
    output wire       rx_parity_error,
    output wire       rx_break
);
    uart_tx #(
        .CLK_DIV   (CLK_DIV),
        .DATA_BITS (DATA_BITS),
        .PARITY    (PARITY),
        .STOP_BITS (STOP_BITS)
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
        .CLK_DIV   (CLK_DIV),
        .DATA_BITS (DATA_BITS),
        .PARITY    (PARITY)
    ) u_rx (
        .clk             (clk),
        .rst_n           (rst_n),
        .div             (div),
        .rx              (rx),
        .rx_data         (rx_data),
        .rx_valid        (rx_valid),
        .rx_error        (rx_error),
        .rx_frame_error  (rx_frame_error),
        .rx_parity_error (rx_parity_error),
        .rx_break        (rx_break)
    );
endmodule
