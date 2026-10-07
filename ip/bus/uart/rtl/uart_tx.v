// uart_tx — a UART transmitter with a ready/valid input, 8N1 by default.
//
// What it does
//   Shifts out one start bit (low), the data bits least significant
//   first, an optional parity bit and one or two stop bits (high), each
//   held for one baud divisor's worth of `clk`. The divisor is clock
//   cycles per bit: for a 12 MHz clock and 115200 baud it is
//   12_000_000 / 115_200 = 104.
//
//   The bit period is `div` clocks when the caller drives a usable one,
//   and CLK_DIV when it drives zero — see `div`'s own comment. A design
//   with a fixed rate ties `div` to zero and nothing changes.
//
//   The input is a ready/valid handshake. `tx_ready` is high while the
//   shifter is idle; a transfer happens on the rising edge where
//   `tx_valid` and `tx_ready` are both high, and `tx_data` is captured
//   then. `tx` idles high, so a line left alone reads as idle.
//
//   **The framing is three parameters and defaults to 8N1**, so an
//   instantiation written before they existed means exactly what it
//   always did. The transmitter itself is `uart_frame_tx`, whose three
//   format ports this module ties to constants; everything about how a
//   character is assembled, which values are implemented and what an
//   unimplemented one does is in that file's header.
//
//   A design whose framing is **chosen at run time** — by a USB host's
//   SET_LINE_CODING, which is the case this package was extended for —
//   cannot use parameters and instantiates `uart_frame_tx` or
//   `uart_frame` instead. Verilog has no default for a port, so the
//   format could not be made a port here without every existing
//   instantiation having to connect it; the split is that constraint and
//   not a preference.
//
// What it does not do
//   No break generation, no flow control and no FIFO — put `fifo_sync` in
//   front of it if the producer cannot wait. The bit period is an exact
//   whole number of clock cycles, so a baud rate that does not divide the
//   clock is approximated by the divisor and the error is the caller's to
//   check (under about 2% for a frame to survive) — `uart_baud_div`
//   computes one from a bit rate and says when the answer is usable.
//
//   `div` and the format are read **once per character**, in the cycle
//   the byte is accepted, and latched. A character in flight therefore
//   keeps the rate it started at however `div` moves, and a change takes
//   effect from the next character — which is the only definition under
//   which a rate change never corrupts a byte.
//
//   Reading it once per character is also what keeps it cheap, and the
//   numbers are measured rather than argued. Comparing the bit counter
//   against a run-time value *every bit* needs a sixteen-bit magnitude
//   comparison inside the loop: `uart` came out at 229 `LUT4` and a logic
//   depth of **19** that way, against 120 and **6** before the port
//   existed at all. Latching the limit makes the loop a sixteen-bit
//   *equality* against a register, and `uart` is 215 `LUT4` at depth
//   **8** — so the whole run-time divisor costs about 95 lookup tables
//   and two levels, and a magnitude comparison would have cost eleven
//   more levels for nothing. The same rule is why the frame length and
//   the twelve-bit frame word are latched too rather than recomputed per
//   bit. `docs/ip-library.md`'s footprint table is where those numbers
//   are, and `ip/bus/uart/README.md` §4 has what the format cost on top.
module uart_tx #(
    // Clock cycles per bit when `div` does not give one. At least four.
    parameter CLK_DIV = 16,
    // Data bits in a character: 5, 6, 7 or 8. Anything else is 8.
    parameter DATA_BITS = 8,
    // `bParityType` of PSTN 1.2: 0 none, 1 odd, 2 even, 3 mark, 4 space.
    // Anything else is none.
    parameter PARITY = 0,
    // Stop bits: 1 or 2. Anything else is 2. One and a half is not
    // offered here at all, because a parameter is a choice somebody made
    // on purpose and `uart_frame_tx`'s header says 1.5 is always worse
    // than 2 on this wire; `uart_line_coding` is where a host asking for
    // it is handled.
    parameter STOP_BITS = 1
) (
    input  wire       clk,
    input  wire       rst_n,

    // Clock cycles per bit, taken in the cycle a byte is accepted. Zero,
    // or anything below DIV_MIN, means "use CLK_DIV" — a rate has to mean
    // something even when it cannot be expressed, and the divisor the
    // design was built with is the only answer that leaves the port
    // working. Tie it to zero for a fixed rate.
    input  wire [15:0] div,

    input  wire [7:0] tx_data,
    input  wire       tx_valid,
    output wire       tx_ready,

    output wire       tx
);
    // The parameters in the widths `uart_frame_tx` takes them in.
    localparam [3:0] CFG_DATA_BITS = DATA_BITS;
    localparam [2:0] CFG_PARITY    = PARITY;
    localparam [1:0] CFG_STOP      = (STOP_BITS == 1) ? 2'd0 : 2'd2;

    uart_frame_tx #(
        .CLK_DIV (CLK_DIV)
    ) u_tx (
        .clk           (clk),
        .rst_n         (rst_n),
        .div           (div),
        .cfg_data_bits (CFG_DATA_BITS),
        .cfg_parity    (CFG_PARITY),
        .cfg_stop      (CFG_STOP),
        .tx_data       (tx_data),
        .tx_valid      (tx_valid),
        .tx_ready      (tx_ready),
        .tx            (tx)
    );
endmodule
