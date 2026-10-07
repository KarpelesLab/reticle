// uart_line_coding — a USB host's SET_LINE_CODING turned into a UART's
// character format, and a wire that says when it could not be done.
//
// What it does
//   `ip/usb/usb_cdc_acm` decodes SET_LINE_CODING and brings out the four
//   fields of PSTN 1.2 §6.3.11's `dwDTERate`, `bCharFormat`,
//   `bParityType` and `bDataBits`. `uart_baud_div` turns the first into
//   `div`; this turns the other three into `uart_frame`'s `cfg_data_bits`,
//   `cfg_parity` and `cfg_stop`, so that a design wiring a USB serial
//   port to a UART has no table of its own to get wrong.
//
//   It is combinational and has no clock. The host's three bytes are
//   already registers inside `usb_cdc_acm`, and a decode of a register is
//   not worth a register of its own.
//
// The mapping, in full
//   `bDataBits` — §6.3.11 Table 17 allows 5, 6, 7, 8 and 16.
//
//       asked   sent and expected   ok
//       -----   -----------------   --
//           5   5 data bits          1
//           6   6                    1
//           7   7                    1
//           8   8                    1
//          16   8                    0
//       other   8                    0
//
//     Sixteen data bits are not implementable behind an eight-bit
//     `tx_data` and `rx_data`, and widening those would widen every FIFO
//     and every bridge in front of them for a format no terminal program
//     offers. **Nine** is not in the table at all and is not implemented
//     either: nine-bit multidrop framing is usually done by abusing the
//     parity bit as an address mark, which `cfg_parity`'s mark and space
//     modes will do on the wire — but nothing here treats such a bit as
//     an address, so what this block offers is the signalling and not the
//     protocol.
//
//   `bParityType` — 0 none, 1 odd, 2 even, 3 mark, 4 space.
//
//       asked   sent and expected   ok
//       -----   -----------------   --
//           0   none                 1
//           1   odd                  1
//           2   even                 1
//           3   mark                 1
//           4   space                1
//       other   none                 0
//
//     All five are implemented. A parity bit is one bit and choosing
//     between `^data`, `~^data`, 1 and 0 is a four-way multiplexer on it,
//     so mark and space were not worth refusing once odd and even
//     existed. 5 and above are not values the specification defines.
//
//   `bCharFormat` — 0 one stop bit, 1 one and a half, 2 two.
//
//       asked   sent                            ok
//       -----   -----------------------------   --
//           0   1 stop bit                       1
//           1   2 stop bits (1.5 not built)      0
//           2   2 stop bits                      1
//       other   1 stop bit                       0
//
//     `uart_frame_tx`'s header argues the 1.5 case at length: a receiver
//     samples the first stop bit and nothing after it, so two stop bits
//     where one and a half were asked for is more mark time than the far
//     end needs and never less. `ok` goes low anyway, because a design
//     that wants to know it is not getting exactly what the host asked
//     for is entitled to know.
//
//   `ok` is **low for anything inexact**, in all three fields at once. It
//   does not say which field, and it does not distinguish the benign
//   substitution (1.5 stop bits became 2) from the lossy ones (16 data
//   bits became 8, an undefined parity became none). A design that needs
//   that distinction compares its own `char_format` against 1; the tables
//   above are the whole of the behaviour and there are only three cases.
//
// What it does not do
//   Nothing is reported back to the host. CDC has no way for a device to
//   refuse a line coding: SET_LINE_CODING is a control write that either
//   succeeds or stalls, and `usb_cdc_acm` accepts it and reports it back
//   verbatim in GET_LINE_CODING, which is what the specification asks
//   for. Stalling a request because a format is not implementable would
//   make `stty` fail rather than make the wire right, and a host that is
//   told a setting was accepted and then measures the line is the only
//   observer that can tell — so `ok` is for an LED, a status register or
//   a test, and not for the bus.
//
//   It does not touch `dwDTERate`. `uart_baud_div` is that, and it has
//   its own `ok` for the rates it cannot keep time with.
module uart_line_coding (
    // Straight off `usb_cdc_acm`'s ports of the same names.
    input  wire [7:0] char_format,
    input  wire [7:0] parity,
    input  wire [7:0] data_bits,

    // For `uart_frame`'s ports of the same names.
    output wire [3:0] cfg_data_bits,
    output wire [2:0] cfg_parity,
    output wire [1:0] cfg_stop,

    // The three fields were all implementable exactly as asked.
    output wire       ok
);
    // Five to eight fit in four bits exactly, which is why the port is
    // four bits wide and not eight.
    wire bits_ok = (data_bits >= 8'd5) && (data_bits <= 8'd8);
    assign cfg_data_bits = bits_ok ? data_bits[3:0] : 4'd8;

    // Zero to four fit in three bits, and `uart_frame_tx` uses
    // `bParityType`'s own numbering, so this is a range check and a
    // narrowing and no table at all.
    wire par_ok = (parity <= 8'd4);
    assign cfg_parity = par_ok ? parity[2:0] : 3'd0;

    // One stop bit for 0 and for anything undefined; two for 1 and 2.
    wire fmt_ok = (char_format == 8'd0) || (char_format == 8'd2);
    assign cfg_stop =
        ((char_format == 8'd1) || (char_format == 8'd2)) ? 2'd2 : 2'd0;

    assign ok = bits_ok & par_ok & fmt_ok;
endmodule
