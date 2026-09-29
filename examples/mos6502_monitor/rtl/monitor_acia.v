// monitor_acia — a 65C51-style ACIA whose wire is a USB pipe.
//
// What it does
//   Four registers at `$5000`, the layout a W65C51N data sheet gives
//   them, with a byte stream on the other side instead of a serial line.
//   A 6502 that knows how to talk to a 65C51 talks to this, and what it
//   is talking to is `ip/usb_cdc_acm`'s `out_*` and `in_*`.
//
//     RS  name     read                        write
//     00  DATA     the byte received, taken    a byte to send
//                  by the read
//     01  STATUS   see below                   a programmed reset
//     10  COMMAND  the last byte written       DTR, interrupts, parity
//     11  CONTROL  the last byte written       stop bits, word length,
//                                              receiver clock, baud
//
//   STATUS, bit by bit, and which of them are real here:
//
//     7  IRQ                 **always 0**. Nothing in this machine
//                            raises an interrupt and the COMMAND
//                            register's two enables are stored and not
//                            acted on, so there is never a request to
//                            report.
//     6  DSR (active low)    **always 0**, meaning data set ready. The
//                            other end of this pipe is a host that has
//                            enumerated the device; there is no modem.
//     5  DCD (active low)    **always 0**, meaning carrier present, for
//                            the same reason.
//     4  TDRE                **real**: high when the transmit holding
//                            register is free.
//     3  RDRF                **real**: high when a byte has arrived and
//                            has not been read yet.
//     2  Overrun             **always 0**. It cannot happen: `rx_ready`
//                            is low while RDRF is high, so the block
//                            upstream holds the byte rather than
//                            overwriting one.
//     1  Framing error       **always 0**. There are no frames; a USB
//                            bulk endpoint delivers bytes or does not.
//     0  Parity error        **always 0**, and the COMMAND register's
//                            parity bits are stored and not acted on.
//
//   **TDRE is the correct behaviour and not the erratum.** Real W65C51N
//   silicon leaves TDRE permanently set, so a polling loop never waits
//   and characters are lost; that is a defect of a part, not a feature
//   of the interface, and a model of it here would only teach software
//   to spin on a timer instead. TDRE here means what the data sheet says
//   it means.
//
// The baud rate, which is the interesting part
//   CONTROL's low four bits select a bit rate from a fixed table — the
//   one in the data sheet, sixteen codes from 50 baud to 19200, with
//   code 0 meaning *"the receiver and transmitter are clocked from
//   outside this part"* rather than naming a rate at all.
//
//   That table cannot express 115200. It is four bits wide and it stops
//   at 19200, and no amount of wanting changes it. So this block does
//   two things with a host's `dwDTERate`, and they are different things:
//
//   1. **It writes the clock source into CONTROL when the host changes
//      rate**, which is bits 3..0 *and* bit 4. Those two fields are one
//      fact between them — bit 4 is where the receiver's clock comes
//      from and bits 3..0 are the generator's rate — so a rate the table
//      names becomes {generator, code} and a rate it cannot name becomes
//      {external, 0000}. Code 0 is not a fudge: the data sheet's own
//      meaning for it is the 16x external clock, and a USB serial bridge
//      is exactly a part whose bit clock arrives from outside.
//
//      **`$10` was written here once and it means nothing**: the
//      generator selected with a rate field of `0000`, which is to say
//      "use the rate the generator makes" and "the generator makes no
//      rate" at the same time. A register a program can read must not
//      hold a state a data sheet cannot name, which is why bit 4 moves
//      with the rate field and not only the four bits below it.
//
//      **Those five bits are the host's and not the processor's.** A
//      program writes CONTROL and reads back its own bits 7..5 -- the
//      stop bit and the word length -- and the host's bits 4..0. That is
//      a departure from the part, where a program owns the whole
//      register, and the alternative was tried and measured: with a
//      last-writer-wins rule the last writer was always the processor,
//      because the machine is held in reset until the host has configured
//      the port and so the monitor's `sta ACIAX` always came second.
//      `5003` answered 19200 on a port opened at 115200. A register whose
//      answer depends on which of two things spoke last is worth less
//      than one that always answers the question it exists to answer.
//
//   2. **It reports the rate the part is programmed to on `rate`**, for
//      whatever wants to put a real waveform on a pin. Code 0 reports
//      the host's own `dwDTERate`, because that is what "clocked from
//      outside" resolves to on this board.
//
//   Codes 3 and 4 are 109.92 and 134.58 baud on the data sheet — the
//   teleprinter rates — and a host asks for those as 110 and 134, which
//   are the numbers compared against here and reported back.
//
// What was read, and what was inferred
//   Every register layout above is **quoted** from a W65C51N data sheet
//   and not measured; there is no 65C51 here to compare against and there
//   could not be, since this one's other side is a USB pipe. Three things
//   are **inferred**, and are marked here so that a reader with the data
//   sheet open knows which sentences to check:
//
//   * That code 0 is the right answer for a rate the table cannot name.
//     The data sheet says code 0 selects an external receiver clock at
//     sixteen times the bit rate; calling a USB host "external" is this
//     design's reading of that and not the data sheet's sentence.
//   * That codes 3 and 4 answer to 110 and 134. The data sheet's numbers
//     are 109.92 and 134.58 — what a 1.8432 MHz crystal divides to — and
//     hosts ask for the rounded ones. No host was observed asking.
//   * That a programmed reset leaves CONTROL alone. Whether the control
//     register is among what a write to the status register clears is
//     read here as "no", because a rate a host set surviving a program
//     resetting the part is the behaviour that matters on this board. If
//     that is wrong about the part, it is deliberately wrong.
//
// What it does not do
//   No interrupts, and therefore no IRQ pin: COMMAND's receiver and
//   transmitter interrupt enables are stored so a program can read back
//   what it wrote, and nothing reads them. No RTS or DTR pin, for the
//   same reason and because there is nothing on this board to wire them
//   to. No parity generation or checking, no word lengths other than
//   eight, no second stop bit, no receiver clock input: every one of
//   those is a bit of COMMAND or CONTROL that is stored and not acted
//   on, which is the honest shape for a register a program may read back
//   but whose effect has nowhere to land.
//
//   No transmit or receive shift register either. A byte written to DATA
//   is handed to the block upstream whole; there is no moment at which
//   half of it has gone.
module monitor_acia (
    input  wire        clk,
    input  wire        rst_n,

    // The processor's side. `sel` is "this cycle addresses the ACIA" and
    // `access` is the one clock in which the cycle completes, so a read
    // of DATA takes the byte exactly once however many clocks the
    // processor is held for.
    input  wire        sel,
    input  wire [1:0]  rs,
    input  wire        we,
    input  wire        access,
    input  wire [7:0]  din,
    output reg  [7:0]  dout,

    // The byte stream. `rx_*` is what arrived from the host, `tx_*` what
    // goes back; both are one byte with a strobe, because that is what
    // a register interface produces.
    input  wire [7:0]  rx_data,
    input  wire        rx_valid,
    output wire        rx_ready,
    output wire [7:0]  tx_data,
    output wire        tx_valid,
    input  wire        tx_ready,

    // What the host asked the line to be, straight off
    // `usb_cdc_acm`'s `baud`.
    input  wire [31:0] host_rate,

    // What this ACIA is programmed to, for something that drives a pin.
    output reg  [31:0] rate,
    // And the two registers, for a design that wants to light an LED
    // with one of their bits.
    output wire [7:0]  command,
    output wire [7:0]  control
);
    localparam [1:0] R_DATA    = 2'd0;
    localparam [1:0] R_STATUS  = 2'd1;
    localparam [1:0] R_COMMAND = 2'd2;
    localparam [1:0] R_CONTROL = 2'd3;

    // The data sheet's reset values: COMMAND 02h is "DTR not ready, both
    // interrupts disabled, no parity", CONTROL 00h is "eight data bits,
    // one stop bit, externally clocked".
    localparam [7:0] COMMAND_RESET = 8'h02;
    localparam [7:0] CONTROL_RESET = 8'h00;

    reg [7:0] command_q;
    reg [7:0] control_q;
    assign command = command_q;
    assign control = control_q;

    // -----------------------------------------------------------------
    // Receive: one byte, and RDRF.
    // -----------------------------------------------------------------
    // The byte, and whether there is one. **The byte has no reset and the
    // flag does**, which is the same division `usb_bulk_ep`'s buffers are
    // written with and for the same reason: `rx_q` is read only while
    // `rdrf` says there is something in it, and `rdrf` resets to zero, so
    // a reset on the byte would be eight flip-flops' worth of clearing
    // something nothing can look at. On this part that is not only waste:
    // a flip-flop with a reset needs its tile's set/reset wire, and a
    // distributed RAM needs the same wire for its write enable, so every
    // reset this design does not ask for is one less thing for the router
    // to fit into a tile that already has a RAM in it.
    reg [7:0] rx_q;
    reg       rdrf;

    // Room only while nothing is waiting. That is what makes the overrun
    // bit permanently zero rather than permanently wrong.
    assign rx_ready = ~rdrf;

    wire read_data  = sel & access & ~we & (rs == R_DATA);
    wire write_data = sel & access &  we & (rs == R_DATA);
    wire reset_cmd  = sel & access &  we & (rs == R_STATUS);

    // -----------------------------------------------------------------
    // Transmit: one byte, and TDRE.
    // -----------------------------------------------------------------
    // The same division on the way out: `tx_q` is handed over only while
    // `tx_pending` says there is a byte, and that resets to zero.
    reg [7:0] tx_q;
    reg       tx_pending;

    assign tx_data  = tx_q;
    assign tx_valid = tx_pending;
    // Free only while nothing is waiting, so a poll immediately after a
    // write cannot see a stale 1 and overwrite the byte it just sent.
    wire tdre = ~tx_pending;

    // -----------------------------------------------------------------
    // The baud table, both ways round.
    // -----------------------------------------------------------------
    // Code to rate. Codes 3 and 4 are the data sheet's 109.92 and 134.58
    // baud, which a host names 110 and 134.
    reg [31:0] code_rate;
    always @(*) begin
        case (control_q[3:0])
            4'h1:    code_rate = 32'd50;
            4'h2:    code_rate = 32'd75;
            4'h3:    code_rate = 32'd110;
            4'h4:    code_rate = 32'd134;
            4'h5:    code_rate = 32'd150;
            4'h6:    code_rate = 32'd300;
            4'h7:    code_rate = 32'd600;
            4'h8:    code_rate = 32'd1200;
            4'h9:    code_rate = 32'd1800;
            4'hA:    code_rate = 32'd2400;
            4'hB:    code_rate = 32'd3600;
            4'hC:    code_rate = 32'd4800;
            4'hD:    code_rate = 32'd7200;
            4'hE:    code_rate = 32'd9600;
            4'hF:    code_rate = 32'd19200;
            // Code 0: clocked from outside the part, which on this board
            // is the host. A `case` with no `default` is a latch.
            default: code_rate = host_rate;
        endcase
    end

    // Rate to code. Every value in the table is under 65536, so the top
    // half of `host_rate` only has to be zero; that halves the width of
    // sixteen comparators.
    wire        in_table = (host_rate[31:16] == 16'd0);
    wire [15:0] low   = host_rate[15:0];
    reg  [3:0]  host_code;
    // Whether the table named the rate at all, which is what decides the
    // receiver clock source as well as the rate field.
    wire        host_named = (host_code != 4'h0);
    always @(*) begin
        if (!in_table)                    host_code = 4'h0;
        else case (low)
            16'd50:    host_code = 4'h1;
            16'd75:    host_code = 4'h2;
            16'd110:   host_code = 4'h3;
            16'd134:   host_code = 4'h4;
            16'd150:   host_code = 4'h5;
            16'd300:   host_code = 4'h6;
            16'd600:   host_code = 4'h7;
            16'd1200:  host_code = 4'h8;
            16'd1800:  host_code = 4'h9;
            16'd2400:  host_code = 4'hA;
            16'd3600:  host_code = 4'hB;
            16'd4800:  host_code = 4'hC;
            16'd7200:  host_code = 4'hD;
            16'd9600:  host_code = 4'hE;
            16'd19200: host_code = 4'hF;
            default:   host_code = 4'h0;
        endcase
    end

    // -----------------------------------------------------------------
    // The registers
    // -----------------------------------------------------------------
    // The two bytes themselves, in a process of their own with no reset
    // in it at all, which is what makes them cost no set/reset wire.
    always @(posedge clk) begin
        if (rx_valid && rx_ready) rx_q <= rx_data;
        if (write_data)           tx_q <= din;
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            rdrf        <= 1'b0;
            tx_pending  <= 1'b0;
            command_q   <= COMMAND_RESET;
            control_q   <= CONTROL_RESET;
            rate        <= 32'd0;
        end else begin
            // A byte from the host always wins over a read that empties
            // the register in the same clock, because `rx_ready` was low
            // while `rdrf` was high and the block upstream cannot have
            // offered one.
            if (rx_valid && rx_ready) begin
                rdrf <= 1'b1;
            end else if (read_data) begin
                rdrf <= 1'b0;
            end

            if (write_data) begin
                tx_pending <= 1'b1;
            end else if (tx_pending && tx_ready) begin
                tx_pending <= 1'b0;
            end

            if (reset_cmd) begin
                // A programmed reset. The data sheet says it clears the
                // command register's low bits and the receiver, and
                // leaves the control register alone — so the rate a host
                // set survives a program resetting the part, which is
                // the behaviour that matters here.
                command_q <= COMMAND_RESET;
                rdrf      <= 1'b0;
            end else if (sel & access & we & (rs == R_COMMAND)) begin
                command_q <= din;
            end

            // CONTROL is owned by two things and the split is by field.
            // Bits 7..5 are framing -- stop bits and word length -- and
            // they are the processor's: it writes them and reads back
            // what it wrote. Bits 4..0 are where the bit clock comes
            // from, and on this board that is not the processor's to
            // decide, so they are **a level and not an edge**: whatever
            // the host last asked for, continuously.
            //
            // That is a departure from the part, where a program owns the
            // whole register, and it is deliberate. The edge was tried
            // first and the ordering is what killed it: the machine is
            // held in reset until the host has configured the port, so
            // the host's rate arrives *before* the monitor's `sta ACIAX`,
            // and a rule where the last writer wins made the last writer
            // always the processor. `5003` then answered 19200 on a port
            // the host had opened at 115200 -- measured on the board, in
            // the build before this one. A register whose answer depends
            // on which of two things spoke last is worth less than one
            // that always answers the question it exists to answer.
            if (sel & access & we & (rs == R_CONTROL)) begin
                control_q[7:5] <= din[7:5];
            end
            control_q[4]   <= host_named;
            control_q[3:0] <= host_code;

            rate <= code_rate;
        end
    end

    // -----------------------------------------------------------------
    // Reading one
    // -----------------------------------------------------------------
    always @(*) begin
        case (rs)
            R_DATA:    dout = rx_q;
            // {IRQ, DSR#, DCD#, TDRE, RDRF, overrun, framing, parity}
            R_STATUS:  dout = {3'b000, tdre, rdrf, 3'b000};
            R_COMMAND: dout = command_q;
            default:   dout = control_q;
        endcase
    end
endmodule
