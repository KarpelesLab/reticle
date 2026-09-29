// uart_baud_div — clock cycles per bit from a bit rate, by long division.
//
// What it does
//   Turns a bit rate in bits per second into the divisor `uart_tx` and
//   `uart_rx` want on their `div` ports, and says whether the answer is
//   one a UART can actually keep time with.
//
//   It exists because `ip/usb_cdc_acm` receives SET_LINE_CODING and
//   reports `dwDTERate` — a 32-bit number a host picks — and a divisor
//   from it is a division by a run-time value. `usb_cdc_req`'s header
//   says so in as many words: *"a serial port whose divisor followed
//   dwDTERate would need a divide by a run-time value, which is a
//   design's business and not this block's"*. This is that business,
//   done once, here, so that two designs do not each do it differently.
//
//   The sum is
//
//       div = round(CLK_HZ / rate) = (CLK_HZ + rate/2) / rate
//
//   computed as a restoring long division, one quotient bit per clock,
//   32 clocks for an answer — 0.53 us at 60 MHz, against a host that
//   changes the rate at most a few times a second. Adding half the
//   divisor to the numerator first is what makes it round to nearest
//   rather than down, and `rate/2` is a shift, so the only division in
//   the block is the one being done.
//
// The error, and why this much of it is allowed
//   A bit period is a whole number of clocks, so a rate the clock cannot
//   divide exactly is approximated, and rounding to nearest leaves at
//   most half a clock of error in a bit — a relative error of
//   `0.5 / div`. An 8N1 receiver samples the stop bit 9.5 bit periods
//   after the start edge and has half a bit of margin there, so the
//   textbook budget is about 5% and the practical one about 2%
//   (`uart_rx.v` says the same). Half a clock in `div` clocks is under 2%
//   as soon as `div` is 25 or more.
//
//   So DIV_MIN is **30** by default and not 4: a divisor of 30 is 1.67%
//   of error at worst, and every rate this block calls usable is
//   therefore inside the budget rather than merely expressible. At 60 MHz
//   that makes 2 Mbaud the fastest rate it will accept. The alternative —
//   accepting a divisor of 4 and its 12.5% — is a port that reports
//   success and drops every character, which is worse than saying no.
//
//   Worked, for the rates a host asks for at 60 MHz:
//
//       rate     div   actual rate   error
//       ------   ---   -----------   ------
//         9600  6250    9600.00      0.000%
//        19200  3125   19200.00      0.000%
//        38400  1563   38387.72     -0.032%
//        57600  1042   57581.57     -0.032%
//       115200   521  115163.15     -0.032%
//       230400   260  230769.23     +0.160%
//       460800   130  461538.46     +0.160%
//       921600    65  923076.92     +0.160%
//      1000000    60 1000000.00      0.000%
//
// What it does not do
//   Nothing is buffered or acknowledged: `rate` is watched, and a change
//   starts a division that replaces `div` when it finishes. `busy` is
//   high while one is running and `div` keeps its old value throughout,
//   so a UART never sees a half-computed divisor — but it does see the
//   old one, which is the right answer for a rate nobody has finished
//   asking for yet.
//
//   There is no fractional divisor and no oversampling: a 16x sampling
//   receiver with a fractional accumulator would carry less error at high
//   rates, and `uart_rx` is a mid-bit sampler, so this block's job ends
//   at the nearest whole number.
//
//   `ok` is a statement about arithmetic, not about the wire. It says the
//   rate was expressible inside the error budget; it does not say a cable
//   is attached or that the other end agrees.
module uart_baud_div #(
    // The clock this divisor counts. 60 MHz is a ULPI board's oscillator,
    // which is where this block is first used.
    parameter [31:0] CLK_HZ = 32'd60_000_000,
    // What `div` holds before a rate has been worked out, and whenever
    // one cannot be: 521 is 115200 baud at 60 MHz, the rate a serial
    // console is opened at more often than any other.
    parameter [15:0] DIV_RESET = 16'd521,
    // The smallest divisor called usable. See "The error" above; 30 is
    // 1.67% and the 8N1 budget is about 2%.
    parameter [15:0] DIV_MIN = 16'd30
) (
    input  wire        clk,
    input  wire        rst_n,

    // The rate the host asked for, straight off `usb_cdc_acm`'s `baud`.
    // Sampled continuously; a change starts a division.
    input  wire [31:0] rate,

    // Clock cycles per bit, for `uart`'s `div` port.
    output reg  [15:0] div,
    // The last rate divided to something inside the error budget. Low
    // after reset until the first usable rate arrives, and low again for
    // a rate that is zero, too slow to express in sixteen bits, or fast
    // enough that the rounding would break framing.
    output reg         ok,
    // A division is running. `div` holds its previous value meanwhile.
    output reg         busy
);
    // The widest the quotient can be is the numerator's width: a rate of
    // one divides CLK_HZ by one. Sixteen bits is all `uart` can take, so
    // the top sixteen are kept only to notice that they are not zero.
    // Thirty-two steps, so the step counter runs 0..31 and is five bits —
    // exactly the values it holds, which on an ECP5 is not a style point
    // (see CLAUDE.md, "Fabric hazards worth knowing").
    localparam [4:0] LAST_STEP = 5'd31;

    // The numerator, shifted left one bit per step; the remainder, which
    // takes the bit that falls out of it; and the quotient, which takes
    // one bit per step from the bottom.
    reg [31:0] num;
    reg [32:0] rem;
    reg [31:0] quo;
    reg [31:0] den;
    reg [4:0]  step;
    reg [31:0] seen;

    // One step of restoring division: shift the next numerator bit into
    // the remainder, and subtract the denominator if it fits.
    wire [32:0] shifted = {rem[31:0], num[31]};
    wire        fits    = (shifted >= {1'b0, den});
    wire [32:0] next    = fits ? (shifted - {1'b0, den}) : shifted;

    // A rate whose quotient does not fit sixteen bits, or falls below the
    // error budget, is one this block declines. `den == 0` is caught by
    // the same test: nothing is ever started for it, so `quo` stays zero
    // and zero is below DIV_MIN.
    wire fits_16   = (quo[31:16] == 16'd0);
    wire big_enough = (quo[15:0] >= DIV_MIN);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            div  <= DIV_RESET;
            ok   <= 1'b0;
            busy <= 1'b0;
            num  <= 32'd0;
            rem  <= 33'd0;
            quo  <= 32'd0;
            den  <= 32'd0;
            step <= 5'd0;
            // Not the reset rate: whatever `rate` is at reset has to be
            // *divided*, not assumed, and zero is not a rate a host can
            // have asked for, so the first comparison always fires.
            seen <= 32'd0;
        end else if (busy) begin
            rem  <= next;
            num  <= {num[30:0], 1'b0};
            quo  <= {quo[30:0], fits};
            step <= step + 5'd1;
            if (step == LAST_STEP) busy <= 1'b0;
        end else if (seen != rate) begin
            // A new rate. Start the division, unless there is nothing to
            // divide by: a rate of zero is not a rate, so it takes the
            // same answer as a rate that will not fit — DIV_RESET, with
            // `ok` low to say the number came from the parameter and not
            // from the host.
            seen <= rate;
            if (rate == 32'd0) begin
                div <= DIV_RESET;
                ok  <= 1'b0;
            end else begin
                // CLK_HZ + rate/2, so the quotient rounds to nearest.
                num  <= CLK_HZ + {1'b0, rate[31:1]};
                den  <= rate;
                rem  <= 33'd0;
                quo  <= 32'd0;
                step <= 5'd0;
                busy <= 1'b1;
            end
        end else begin
            // A division has just finished (or nothing is happening and
            // this repeats harmlessly): take the quotient if it is one a
            // UART can keep time with.
            if (fits_16 && big_enough) begin
                div <= quo[15:0];
                ok  <= 1'b1;
            end else begin
                div <= DIV_RESET;
                ok  <= 1'b0;
            end
        end
    end
endmodule
