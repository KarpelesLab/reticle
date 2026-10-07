// uart_frame — `uart_frame_tx` and `uart_frame_rx` behind one divisor and
// one character format.
//
// What it does
//   Wires the configurable transmitter and the configurable receiver
//   together behind one clock, one reset, one divisor and one format,
//   which is what a serial port is: the two halves of it run at the same
//   rate and with the same framing. Everything the two sub-blocks do and
//   do not do is in `uart_frame_tx.v` and `uart_frame_rx.v`; this file
//   adds nothing but the wiring.
//
//   `uart` is the same wiring around `uart_tx` and `uart_rx`, which is
//   this pair with the format compiled in. The difference between them is
//   the whole point of this package's shape: a design with a fixed
//   framing instantiates `uart` and pays for no multiplexers, and a
//   design whose framing a host chooses instantiates this one.
//
//   `cfg_stop` reaches the transmitter only. `uart_frame_rx`'s header has
//   the reason — a receiver checks the first stop bit and nothing after
//   it, so one, one and a half and two stop bits are the same thing to it
//   — and it is wired here rather than left to a reader to notice.
//
// What it does not do
//   There is no register interface — this is the raw ready/valid form. No
//   FIFO on either side, no flow control (`rts`/`cts`), no auto-baud, no
//   loopback mode and no interrupt output. A UART with registers behind
//   AXI4-Lite would be this block, two `fifo_sync` instances and a
//   register file; the pieces are all in this library.
//
//   Changing `div` or the format does not resynchronise a character
//   already in flight, so a change mid-character costs that character.
//   Change them while the line is idle — which is when a host changes
//   them, since a line coding arrives on a control transfer. The two
//   halves latch different things and `uart_frame_rx`'s header has the
//   measurement that decided it: the transmitter latches the whole frame
//   because it owns the character it started, and the receiver latches
//   only `div`, because a format that had been through a flip-flop would
//   stop folding to nothing in `uart_rx` and cost 38 lookup tables there
//   for a guarantee a receiver cannot use.
module uart_frame #(
    // Clock cycles per bit when `div` does not give one, shared by both
    // halves. At least four.
    parameter CLK_DIV = 16
) (
    input  wire        clk,
    input  wire        rst_n,

    // Clock cycles per bit, shared by both halves. Zero means CLK_DIV.
    input  wire [15:0] div,

    // The character format, shared by both halves.
    // `uart_frame_tx`'s header is the table of values; `uart_line_coding`
    // turns a USB host's SET_LINE_CODING into these three.
    input  wire [3:0]  cfg_data_bits,
    input  wire [2:0]  cfg_parity,
    input  wire [1:0]  cfg_stop,

    input  wire [7:0]  tx_data,
    input  wire        tx_valid,
    output wire        tx_ready,
    output wire        tx,

    input  wire        rx,
    input  wire        rx_ready,
    output wire [7:0]  rx_data,
    output wire        rx_valid,
    output wire        rx_frame_error,
    output wire        rx_parity_error,
    output wire        rx_break,
    output wire        rx_overrun
);
    uart_frame_tx #(
        .CLK_DIV (CLK_DIV)
    ) u_tx (
        .clk           (clk),
        .rst_n         (rst_n),
        .div           (div),
        .cfg_data_bits (cfg_data_bits),
        .cfg_parity    (cfg_parity),
        .cfg_stop      (cfg_stop),
        .tx_data       (tx_data),
        .tx_valid      (tx_valid),
        .tx_ready      (tx_ready),
        .tx            (tx)
    );

    uart_frame_rx #(
        .CLK_DIV (CLK_DIV)
    ) u_rx (
        .clk             (clk),
        .rst_n           (rst_n),
        .div             (div),
        .cfg_data_bits   (cfg_data_bits),
        .cfg_parity      (cfg_parity),
        .rx              (rx),
        .rx_ready        (rx_ready),
        .rx_data         (rx_data),
        .rx_valid        (rx_valid),
        .rx_frame_error  (rx_frame_error),
        .rx_parity_error (rx_parity_error),
        .rx_break        (rx_break),
        .rx_overrun      (rx_overrun)
    );
endmodule
