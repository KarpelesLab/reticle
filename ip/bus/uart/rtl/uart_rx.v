// uart_rx — a UART receiver with a valid strobe, 8N1 by default.
//
// What it does
//   Waits for the falling edge that starts a character, waits half a bit
//   period and checks the line is still low (so a glitch does not start a
//   character), then samples the data bits, the parity bit if the framing
//   has one, and the stop bit at the middle of each bit period.
//   `rx_valid` is high for one cycle when a character has been assembled,
//   with the byte on `rx_data`.
//
//   `rx` is asynchronous to `clk` by definition, so it goes through a
//   two-flop synchroniser before anything looks at it. That costs two
//   cycles of latency and is not optional.
//
//   **The framing is two parameters and defaults to 8N1**, so an
//   instantiation written before they existed means exactly what it
//   always did. The receiver itself is `uart_frame_rx`, whose two format
//   ports this module ties to constants; there is no stop-bit parameter
//   because a receiver samples the first stop bit and nothing after it,
//   and that file's header says why at length.
//
//   A design whose framing is **chosen at run time** instantiates
//   `uart_frame_rx` or `uart_frame` instead. Verilog has no default for a
//   port, so the format could not be made a port here without every
//   existing instantiation having to connect it.
//
// The errors, and what this module can and cannot tell you
//   `rx_error` is what it always was — *something* was wrong with the
//   character — and it is now the disjunction of the three signals beside
//   it, each of which says which:
//
//   * `rx_frame_error` — the stop bit was not high. The byte the broken
//     frame produced is still delivered, with `rx_valid`.
//   * `rx_parity_error` — the parity bit disagreed with the data. Never
//     high with the default framing, which has no parity bit. The byte is
//     still delivered.
//   * `rx_break` — the line was low for a whole frame, which is a break
//     and **not a character**: `rx_valid` stays low, one `rx_break` cycle
//     is produced however long the break is, and the receiver then waits
//     for the line to return to mark before looking for another start
//     edge. That last part is the recovery, and it is why a held-low line
//     does not produce a break every frame period for ever.
//
//     This is a **change** from the behaviour before the errors were
//     separated: a frame of all zeros with a low stop bit used to arrive
//     as 0x00 with `rx_error`. It is now a break and arrives as no
//     character at all. Every consumer in this repository left `rx_error`
//     unconnected and read `rx_valid` as a keystroke, so the change can
//     only remove a byte nobody sent.
//
//   **An overrun cannot be reported here**, and the reason is this
//   module's own interface: `rx_valid` is a strobe with no `rx_ready`, so
//   a character nobody takes in that one cycle is lost and nothing in the
//   block can tell that from a character nobody wanted. `uart_frame_rx`
//   has the handshake and the `rx_overrun` that goes with it; a design
//   that needs to know instantiates that instead. Saying so is better
//   than offering a wire that is always zero.
//
// What it does not do
//   One sample per bit at the nominal centre: no majority vote over three
//   samples, no oversampling clock recovery and no FIFO — a character not
//   consumed in the cycle `rx_valid` is high is lost, so put `fifo_sync`
//   behind it if the consumer cannot keep up. It does not resynchronise
//   mid-character, so the clock error budget is the usual half a bit over
//   a frame, about 5% in theory and under 2% in practice.
//
//   `div` is read **once per character**, in the cycle the start edge is
//   found, and latched with the half-bit period derived from it. A
//   character being received therefore keeps the rate it started at
//   however `div` moves, and a change takes effect from the next start
//   edge. `uart_tx` does the same and its header says what that costs and
//   saves. The framing is parameters here, so there is nothing to latch;
//   `uart_frame_rx`'s header says why it does not latch its ports either
//   and what that was measured to save in this module.
module uart_rx #(
    // Clock cycles per bit when `div` does not give one. At least four,
    // so half a bit is countable.
    parameter CLK_DIV = 16,
    // Data bits in a character: 5, 6, 7 or 8. Anything else is 8.
    parameter DATA_BITS = 8,
    // `bParityType` of PSTN 1.2: 0 none, 1 odd, 2 even, 3 mark, 4 space.
    // Anything else is none.
    parameter PARITY = 0
) (
    input  wire       clk,
    input  wire       rst_n,

    // Clock cycles per bit, read when a start edge is found. Zero, or
    // anything below DIV_MIN, means "use CLK_DIV": a rate that cannot be
    // expressed has to leave the port working rather than stop it. Tie
    // it to zero for a fixed rate.
    input  wire [15:0] div,

    input  wire       rx,

    output wire [7:0] rx_data,
    output wire       rx_valid,
    // Any of the three below. What it has always been, widened only by
    // the break that used to arrive as a framing error on a zero byte.
    output wire       rx_error,
    output wire       rx_frame_error,
    output wire       rx_parity_error,
    output wire       rx_break
);
    // The parameters in the widths `uart_frame_rx` takes them in.
    localparam [3:0] CFG_DATA_BITS = DATA_BITS;
    localparam [2:0] CFG_PARITY    = PARITY;

    assign rx_error = rx_frame_error | rx_parity_error | rx_break;

    uart_frame_rx #(
        .CLK_DIV (CLK_DIV)
    ) u_rx (
        .clk             (clk),
        .rst_n           (rst_n),
        .div             (div),
        .cfg_data_bits   (CFG_DATA_BITS),
        .cfg_parity      (CFG_PARITY),
        .rx              (rx),
        // Always ready, which is what makes `rx_valid` the one-cycle
        // strobe this module has always had — and what makes an overrun
        // unobservable, as the header says.
        .rx_ready        (1'b1),
        .rx_data         (rx_data),
        .rx_valid        (rx_valid),
        .rx_frame_error  (rx_frame_error),
        .rx_parity_error (rx_parity_error),
        .rx_break        (rx_break),
        // Structurally impossible with `rx_ready` tied high.
        .rx_overrun      ()
    );
endmodule
