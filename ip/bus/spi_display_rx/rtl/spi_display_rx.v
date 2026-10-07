// spi_display_rx — the receiving end of a display's four-wire SPI link,
// oversampled in the system clock domain.
//
// What is on the other side, and how we know
//   There is no datasheet for this link. Everything below is what the
//   user observed on their own hardware with an analyser, in three
//   passes that corrected each other; README.md §1 records all three,
//   including the two readings that turned out to be wrong, because the
//   path to the answer is the useful part.
//
//     `sclk`  the clock the display's master generates, in **bursts of
//             exactly eight edges** with an idle gap between bursts. It
//             is not continuous.
//     `mosi`  the one data wire. It carries commands **and** pixels.
//     `dc`    says whether the byte on `mosi` is data or a command. It
//             holds for a whole byte, which is conventional.
//     `cs_n`  the fourth wire, **high between bursts** and low while a
//             burst is in progress.
//
//   Receive only: there is no `miso` and no transmit path.
//
//   This is textbook four-wire SPI, and the block is named for that
//   rather than for anything exotic. The fourth wire is read as a chip
//   select, because a master that deasserts `cs_n` between bytes
//   produces exactly the observation reported — but nothing here depends
//   on that reading: what the block needs from the wire is a **frame**,
//   and a per-byte strobe would serve identically. README.md §2 says why
//   the naming went that way and what would distinguish the two.
//
// What it does
//   All four pins are synchronised into `clk` and oversampled there. No
//   edge of `sclk` clocks anything. Every detected sampling edge of the
//   synchronised `sclk` shifts one bit of `mosi` into a shift register;
//   `dc` is sampled on the same edges.
//
//   A byte is complete when the frame says the burst ended. `rx_valid`
//   is then high for one `clk` cycle with the byte on `rx_byte` and its
//   tag on `rx_is_data` — high for a data byte, low for a command.
//
//   `cs_n`'s **falling** edge (the frame opening) resets the bit counter
//   and its rising edge (the frame closing) completes the byte. That is
//   alignment for free: the block cannot be permanently mis-framed by
//   starting in the middle of a stream, which is the one failure a
//   continuous-clock receiver cannot avoid. The frame a reset lands
//   inside is *discarded* rather than decoded wrong — `framed` stays low
//   until a frame close **and then** a frame open have both been seen,
//   so the frame the block trusts is one whose beginning it watched
//   rather than one inferred from the synchronisers' reset value.
//
// What is a parameter, and why
//   Each of these is a fact about the far side that nobody has measured.
//   They are parameters so that a measurement can choose them, and the
//   defaults are what the observations above imply:
//
//     SAMPLE_EDGE    which `sclk` edge samples `mosi` and `dc`: 0
//                    rising, 1 falling. Nothing in this block can tell
//                    which is right — a wrong choice samples the wires
//                    as they change and produces plausible-looking
//                    rubbish with every counter still reading zero.
//                    README.md §5 says what experiment chooses it, and
//                    `spi_display_rx_samples_the_edge_and_the_bit_order_it_is_told_to`
//                    is the test that makes that silence explicit.
//     MSB_FIRST      1 takes the first bit of a burst as bit 7, 0 as
//                    bit 0. SPI is almost always the former.
//     FRAME_MODE     0 authority — `cs_n` frames the byte, and a frame
//                      that was not eight bits is reported and the byte
//                      dropped. **The default, and for this link it is
//                      simply correct**: the bursts were confirmed to be
//                      exactly eight edges.
//                    1 alignment only — the first frame sets the phase,
//                      after which the block counts eight and ignores
//                      `cs_n`. For a master that holds the select across
//                      several bytes.
//                    2 ignored — count eight from reset. For a source
//                      with a continuous clock and no usable frame.
//                    Modes 1 and 2 are contingencies. They cost one mux
//                    on the counter's clear, and mode 2 **keeps the
//                    frame counters live**: running in mode 2 measures
//                    whether mode 0 would have worked without depending
//                    on it.
//     CS_ACTIVE_LOW  1 if a **high** `cs_n` means "between bytes", as
//                    observed. 0 inverts it.
//     CS_PULSE       0 `cs_n` is a level framing the gap, as observed.
//                    1 it is a pulse at the boundary instead — the
//                      reading the user's first description allowed, and
//                      not disproved, only made unlikely. One assertion
//                      then both ends a byte and opens the next.
//     DC_SAMPLE      which bit's `dc` tags the byte.
//                    0 require agreement — tag from the first bit, and
//                      **drop** a byte whose `dc` moved inside it. The
//                      default: `dc` holding for a whole byte is
//                      confirmed, so a signal that disagrees with itself
//                      is a fault, and a wrongly tagged command byte is
//                      worse than a missing one.
//                    1 first bit, delivered whatever `dc` did.
//                    2 last bit, delivered whatever `dc` did.
//     DC_DATA_LEVEL  the `dc` level that means a data byte: 1 by the
//                    display convention, which nothing here has
//                    checked.
//
// The instrumentation, and which numbers should be zero
//   This block exists as much to measure the link as to receive it, so
//   everything below is counted in every mode.
//
//     cmd_byte_count   bytes delivered with `dc` saying command, and
//     data_byte_count  bytes delivered with `dc` saying data. **The
//                      pair that settles what the capture was**: the
//                      user saw `dc` looking like the inverse of `cs_n`,
//                      which is what a pixel-only capture looks like —
//                      `dc` constant for thousands of bytes while
//                      `cs_n` does all the moving. Commands near zero
//                      over a long run confirms that reading. Commands
//                      appearing *and* `dc` still tracking `cs_n` byte
//                      for byte would be genuinely odd and worth
//                      stopping for.
//     frame_count      frames seen, counted raw — before `framed`, and
//                      whatever the bit count was.
//     bit_error_count  frames whose bit count was not a byte boundary:
//                      in mode 0 a count other than eight, in modes 1
//                      and 2 a frame closing mid-byte. **This should
//                      read exactly zero.** It is not a tolerance: the
//                      bursts are eight edges, so a non-zero value says
//                      the sampling edge is wrong, the oversampling is
//                      too slow, or the far side is not what we think.
//     dc_change_count  bytes during which `dc` changed. **Also exactly
//                      zero**, now that per-byte holding is confirmed —
//                      which is what makes DC_SAMPLE = 0 clearly right,
//                      since a wire that holds always agrees with
//                      itself. A non-zero value means the analyser's
//                      channels were not what they were labelled, or
//                      that this display really does qualify per bit;
//                      the number says which, and in mode 0 the bytes
//                      are withheld until it is understood.
//     overrun_count    clocks in which a sampled input had held its
//                      level for only one `clk` — the margin for
//                      detecting its next transition is gone, so the
//                      next one may be missed entirely. **Also expected
//                      to be zero**; see the rate limit below.
//     bit_count        bits in the frame in progress, live.
//     last_bit_count   the bit count at the most recent frame close,
//                      which turns one `bit_error_count` into "it was
//                      seven" or "it was nine". Saturates at fifteen.
//     framed           a frame open has been seen, so bits are being
//                      accumulated. **In modes 0 and 1 a `framed` that
//                      stays low means no byte will ever be
//                      delivered**, and `frame_count` says whether the
//                      wire moved at all. That is the diagnostic for a
//                      select that is absent, stuck, or on the wrong
//                      pin.
//     framing_error    the three counters above as single wires, for a
//     dc_error         design that wants a light and not a bus.
//     overrun
//
//   Every counter **saturates** rather than wrapping: an instrument that
//   wraps reports a small number for a large fault. COUNT_WIDTH is what
//   they cost — they are most of this block's flip-flops, and eight bits
//   each is plenty for a bench run.
//
// The clock domain, and the rate this imposes
//   `sclk` is an external pin and asynchronous to `clk`, and it is
//   **not used as a clock here**. Clocking on it would need a clock
//   buffer the pad can actually reach — `place::confine_to_reachable`
//   is a constraint this backend only learned recently — and would make
//   every output of this block cross a domain anyway. So the whole block
//   is in the `clk` domain, which `spi_display_rx_is_one_clock_domain`
//   proves by asking `timing::analyze_cdc` and getting one domain and
//   no crossing.
//
//   Oversampling costs a rate limit and here it is. A level presented to
//   a free-running sampler for T clock periods is seen by at least
//   floor(T) sampling edges, so **each `sclk` phase must last at least
//   two `clk` periods** for its transition to be certain of being seen
//   with a clock of margin in hand: a four-`clk` `sclk` period at a 50%
//   duty cycle, which at the Cynthion's 60 MHz is **15 MHz** — a minimum
//   high time and a minimum low time of 33.3 ns each. A display link of
//   a few megahertz is an order of magnitude inside that.
//
//   Above it, nothing is silently dropped. Two independent counters
//   report it: a phase seen only once raises `overrun_count` before any
//   bit is actually lost, and a phase seen *no* times loses an edge,
//   which in mode 0 arrives at the frame close as a bit count below
//   eight and raises `bit_error_count`, with `last_bit_count` saying how
//   many bits did arrive. A too-fast `sclk` is therefore reported twice
//   and corrupts nothing quietly.
//
//   The **gap gives the recovery time**, so the limit is on the in-burst
//   `sclk` period and not on a sustained rate: delivering a byte takes
//   one `clk` and there is no buffer to drain, so a frame may follow its
//   predecessor as closely as `cs_n` allows. But a **short gap can
//   starve the block even when the byte rate is comfortable**, because
//   `cs_n` is synchronised and edge-detected exactly like `sclk`:
//   **`cs_n` must be deasserted for at least two `clk` periods**, 33.3
//   ns at 60 MHz. A shorter gap is reported — one `clk` of deassertion
//   raises `overrun_count`, and a deassertion missed altogether shows up
//   as sixteen bits at the next frame close.
//
// Why `mosi` and `dc` cannot skew against each other
//   They are sampled together, and three structural reasons rather than
//   timing arguments say they agree:
//
//   1. Each of the four inputs goes through **its own `cdc_sync`
//      instance with the same SYNC_STAGES**, so `mosi` and `dc` are
//      delayed by exactly the same number of `clk` periods. Nothing in
//      this block can give one wire a longer path than the other.
//   2. Both are sampled **in the same `clk` cycle by the same enable** —
//      the one detected `sclk` edge — reading the two synchroniser
//      outputs at one instant. There is no per-wire sampling decision to
//      disagree about.
//   3. The sample instant is **half an `sclk` period away from either
//      wire's transitions**, which the rate limit above makes at least
//      two `clk` periods. A wire's first synchroniser flop may resolve a
//      metastable input either way and so may see a transition a clock
//      early or late, independently per wire — but that uncertainty is
//      one `clk`, spent two `clk` away from the instant that matters, so
//      the two cannot be presenting different bits of the frame. Driver
//      skew between them is absorbed by the same margin.
//
//   `spi_display_rx_samples_both_wires_whatever_their_skew` drives the
//   two with deliberately unequal skew after the non-sampling edge and
//   checks every byte and every tag.
//
// What it does not do
//   No transmit, no `miso`, no multi-slave select decode. No elastic
//   buffer — `rx_valid` is a one-cycle pulse, and a consumer that cannot
//   take a byte a frame misses it, which is what `ip/memory/fifo_sync`
//   is for. No pixel format, no address window, no command decoding and
//   no display model: what the bytes mean is the screen's business and
//   this block does not guess. No measurement of the `sclk` *frequency*
//   — it reports whether it could sample, not what the rate was. And no
//   deglitching beyond the synchroniser: a runt on `sclk` that lasts two
//   `clk` periods is a bit as far as this block is concerned.
module spi_display_rx #(
    // Which `sclk` edge samples `mosi` and `dc`: 0 rising, 1 falling.
    parameter SAMPLE_EDGE   = 0,
    // 1 takes the first bit of a burst as bit 7, 0 as bit 0.
    parameter MSB_FIRST     = 1,
    // 0 `cs_n` frames the byte, 1 it only aligns, 2 it is ignored.
    parameter FRAME_MODE    = 0,
    // 1 if a high `cs_n` means "between bytes".
    parameter CS_ACTIVE_LOW = 1,
    // 0 `cs_n` is a level over the gap, 1 a pulse at the boundary.
    parameter CS_PULSE      = 0,
    // 0 require `dc` to agree across the byte, 1 take the first bit's,
    // 2 take the last bit's.
    parameter DC_SAMPLE     = 0,
    // The `dc` level that means a data byte rather than a command.
    parameter DC_DATA_LEVEL = 1,
    // Synchroniser depth for all four inputs, 2 to 4.
    parameter SYNC_STAGES   = 2,
    // Width of every instrumentation counter.
    parameter COUNT_WIDTH   = 16
) (
    input  wire                   clk,
    input  wire                   rst_n,

    // The link's four pins, all asynchronous to `clk`.
    input  wire                   sclk,
    input  wire                   mosi,
    input  wire                   dc,
    input  wire                   cs_n,

    // One byte a frame, with the tag `dc` gave it.
    output reg  [7:0]             rx_byte,
    output reg                    rx_is_data,
    output reg                    rx_valid,

    // Instrumentation.
    output wire [COUNT_WIDTH-1:0] cmd_byte_count,
    output wire [COUNT_WIDTH-1:0] data_byte_count,
    output wire [COUNT_WIDTH-1:0] frame_count,
    output wire [COUNT_WIDTH-1:0] bit_error_count,
    output wire [COUNT_WIDTH-1:0] dc_change_count,
    output wire [COUNT_WIDTH-1:0] overrun_count,
    output wire [3:0]             bit_count,
    output wire [3:0]             last_bit_count,
    output wire                   framed,
    output wire                   framing_error,
    output wire                   dc_error,
    output wire                   overrun
);
    // `cs_n` is in charge of the byte boundary.
    localparam AUTHORITY = (FRAME_MODE == 0) ? 1'b1 : 1'b0;
    // Bits are accumulated from reset, with no alignment to wait for.
    localparam FREE_RUN  = (FRAME_MODE == 2) ? 1'b1 : 1'b0;
    // A byte whose `dc` moved inside it is not to be trusted.
    localparam DC_AGREE  = (DC_SAMPLE == 0) ? 1'b1 : 1'b0;
    // The tag comes from the last bit rather than the first.
    localparam DC_LAST   = (DC_SAMPLE == 2) ? 1'b1 : 1'b0;

    localparam [COUNT_WIDTH-1:0] CNT_ZERO = {COUNT_WIDTH{1'b0}};
    localparam [COUNT_WIDTH-1:0] CNT_MAX  = {COUNT_WIDTH{1'b1}};
    localparam [COUNT_WIDTH-1:0] CNT_ONE  = {{(COUNT_WIDTH-1){1'b0}}, 1'b1};

    // -------------------------------------------------------------------
    // The crossing: one chain per pin, all of the same depth
    // -------------------------------------------------------------------
    wire sclk_s;
    wire mosi_s;
    wire dc_s;
    wire cs_s;

    cdc_sync #(
        .WIDTH  (1),
        .STAGES (SYNC_STAGES),
        .INIT   (0)
    ) sync_sclk (
        .clk   (clk),
        .rst_n (rst_n),
        .d     (sclk),
        .q     (sclk_s)
    );

    cdc_sync #(
        .WIDTH  (1),
        .STAGES (SYNC_STAGES),
        .INIT   (0)
    ) sync_mosi (
        .clk   (clk),
        .rst_n (rst_n),
        .d     (mosi),
        .q     (mosi_s)
    );

    cdc_sync #(
        .WIDTH  (1),
        .STAGES (SYNC_STAGES),
        .INIT   (0)
    ) sync_dc (
        .clk   (clk),
        .rst_n (rst_n),
        .d     (dc),
        .q     (dc_s)
    );

    cdc_sync #(
        .WIDTH  (1),
        .STAGES (SYNC_STAGES),
        .INIT   (0)
    ) sync_cs (
        .clk   (clk),
        .rst_n (rst_n),
        .d     (cs_n),
        .q     (cs_s)
    );

    // -------------------------------------------------------------------
    // Edge detection on what came back, and the margin it had
    // -------------------------------------------------------------------
    reg sclk_q;
    reg gap_q;
    // The current level of this input has been observed at least twice,
    // so its next transition cannot be missed. Out of reset both inputs
    // count as settled, or the first real edge would report an overrun
    // that never happened.
    reg sclk_held;
    reg gap_held;
    // A frame close has been seen, so the frame open that follows it is
    // one whose beginning we watched. Without this the block would trust
    // a frame open inferred from the synchronisers' own reset value,
    // which is a guess about the pin and not an observation of it — and
    // the guess is wrong for one of the two CS_ACTIVE_LOW settings. One
    // flip-flop makes the behaviour out of reset independent of both the
    // polarity and the synchroniser's INIT.
    reg seen_gap;

    // High between bytes, whatever polarity the pin uses.
    wire gap = (CS_ACTIVE_LOW != 0) ? cs_s : !cs_s;

    wire sclk_rise = sclk_s && !sclk_q;
    wire sclk_fall = !sclk_s && sclk_q;
    wire sclk_edge = sclk_rise || sclk_fall;
    wire sample    = (SAMPLE_EDGE != 0) ? sclk_fall : sclk_rise;

    wire gap_rise = gap && !gap_q;
    wire gap_fall = !gap && gap_q;
    wire gap_edge = gap_rise || gap_fall;

    // A frame closed; a frame opened. With a pulse rather than a level
    // the one assertion does both, the ending byte first — so the pulse
    // that arms the block is the one before the first byte it delivers.
    wire frame_close = gap_rise;
    wire frame_open  = ((CS_PULSE != 0) ? gap_rise : gap_fall) && seen_gap;

    // A level that lasted one `clk` is a transition we were one clock
    // from losing, on either input.
    wire margin_lost = (sclk_edge && !sclk_held) || (gap_edge && !gap_held);

    // -------------------------------------------------------------------
    // The shift register, the bit counter and the `dc` tag
    // -------------------------------------------------------------------
    reg [7:0] data_sh;
    reg [3:0] bit_cnt;
    reg [3:0] last_cnt;
    reg       framed_q;
    // `dc` as it was at the first bit of this frame, as it was at the
    // most recent bit, and whether those two ever disagreed.
    reg       dc_first;
    reg       dc_hold;
    reg       dc_moved;

    // A bit is taken this cycle.
    wire take = sample && framed_q;

    // The state a frame close in this same cycle must judge: the sample
    // is ordered before the close, because that is the order the far
    // side produced them in. The counter saturates, so a frame that
    // never closes cannot wrap it back onto eight.
    wire [3:0] cnt_now = take ? ((bit_cnt == 4'd15) ? 4'd15 : bit_cnt + 4'd1)
                              : bit_cnt;
    wire [7:0] data_now = take ? {data_sh[6:0], mosi_s} : data_sh;

    // `dc` at the first bit, `dc` at the latest bit, and whether it has
    // moved inside this frame.
    wire dc_first_now = (take && bit_cnt == 4'd0) ? dc_s : dc_first;
    wire dc_last_now  = take ? dc_s : dc_hold;
    wire dc_moved_now = dc_moved
                     || (take && (bit_cnt != 4'd0) && (dc_s != dc_hold));

    // The byte is complete: at the frame close when `cs_n` is the
    // authority, on the eighth bit otherwise.
    wire complete = AUTHORITY ? (frame_close && framed_q && (cnt_now == 4'd8))
                              : (take && (cnt_now == 4'd8));
    // And it is delivered unless `dc` disagreed with itself and this
    // build insists that it must not.
    wire dc_reject = DC_AGREE && dc_moved_now;
    wire deliver   = complete && !dc_reject;

    // What a frame close should have found. Under `cs_n`'s authority
    // that is eight bits; counting for ourselves it is a byte boundary,
    // which a byte completing in this very cycle also is.
    wire close_aligned = AUTHORITY ? (cnt_now == 4'd8)
                                   : (complete || (cnt_now == 4'd0));
    wire mismatch = frame_close && framed_q && !close_aligned;

    // Where the bit counter goes back to zero: every frame close when
    // `cs_n` frames the byte, the first frame open when it only aligns,
    // and every completed byte when we count for ourselves. A byte
    // dropped over `dc` still clears it, or one bad tag would desync
    // every byte after it.
    wire counter_clear = AUTHORITY ? (frame_close || frame_open)
                                   : ((frame_open && !framed_q) || complete);

    // The first bit of the frame sits at bit 7 of the shift register
    // when it arrived first, and at bit 0 when it arrived last.
    wire [7:0] byte_out = (MSB_FIRST != 0) ? data_now
                        : {data_now[0], data_now[1], data_now[2], data_now[3],
                           data_now[4], data_now[5], data_now[6], data_now[7]};
    // The tag, at the pin level the parameter calls data.
    wire dc_tag  = DC_LAST ? dc_last_now : dc_first_now;
    wire is_data = (DC_DATA_LEVEL != 0) ? dc_tag : !dc_tag;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            sclk_q     <= 1'b0;
            gap_q      <= (CS_ACTIVE_LOW != 0) ? 1'b0 : 1'b1;
            sclk_held  <= 1'b1;
            gap_held   <= 1'b1;
            seen_gap   <= 1'b0;
            data_sh    <= 8'd0;
            bit_cnt    <= 4'd0;
            last_cnt   <= 4'd0;
            framed_q   <= FREE_RUN;
            dc_first   <= 1'b0;
            dc_hold    <= 1'b0;
            dc_moved   <= 1'b0;
            rx_byte    <= 8'd0;
            rx_is_data <= 1'b0;
            rx_valid   <= 1'b0;
        end else begin
            sclk_q    <= sclk_s;
            gap_q     <= gap;
            sclk_held <= !sclk_edge;
            gap_held  <= !gap_edge;

            rx_valid <= deliver;
            if (deliver) begin
                rx_byte    <= byte_out;
                rx_is_data <= is_data;
            end

            data_sh <= data_now;

            if (frame_close) begin
                last_cnt <= cnt_now;
                seen_gap <= 1'b1;
            end

            if (frame_open) begin
                framed_q <= 1'b1;
            end

            if (counter_clear) begin
                bit_cnt  <= 4'd0;
                dc_moved <= 1'b0;
            end else begin
                bit_cnt  <= cnt_now;
                dc_moved <= dc_moved_now;
            end
            dc_first <= dc_first_now;
            dc_hold  <= dc_last_now;
        end
    end

    // -------------------------------------------------------------------
    // The counters, each saturating
    // -------------------------------------------------------------------
    reg [COUNT_WIDTH-1:0] cmd_bytes_q;
    reg [COUNT_WIDTH-1:0] data_bytes_q;
    reg [COUNT_WIDTH-1:0] frames_q;
    reg [COUNT_WIDTH-1:0] bit_errors_q;
    reg [COUNT_WIDTH-1:0] dc_changes_q;
    reg [COUNT_WIDTH-1:0] overruns_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            cmd_bytes_q  <= CNT_ZERO;
            data_bytes_q <= CNT_ZERO;
            frames_q     <= CNT_ZERO;
            bit_errors_q <= CNT_ZERO;
            dc_changes_q <= CNT_ZERO;
            overruns_q   <= CNT_ZERO;
        end else begin
            if (deliver && !is_data && cmd_bytes_q != CNT_MAX) begin
                cmd_bytes_q <= cmd_bytes_q + CNT_ONE;
            end
            if (deliver && is_data && data_bytes_q != CNT_MAX) begin
                data_bytes_q <= data_bytes_q + CNT_ONE;
            end
            if (frame_close && frames_q != CNT_MAX) begin
                frames_q <= frames_q + CNT_ONE;
            end
            if (mismatch && bit_errors_q != CNT_MAX) begin
                bit_errors_q <= bit_errors_q + CNT_ONE;
            end
            // Counted when the byte completes, whether or not the byte
            // itself was delivered: the point of the number is that it
            // should be zero, and dropping the byte must not hide it.
            if (complete && dc_moved_now && dc_changes_q != CNT_MAX) begin
                dc_changes_q <= dc_changes_q + CNT_ONE;
            end
            if (margin_lost && overruns_q != CNT_MAX) begin
                overruns_q <= overruns_q + CNT_ONE;
            end
        end
    end

    assign cmd_byte_count  = cmd_bytes_q;
    assign data_byte_count = data_bytes_q;
    assign frame_count     = frames_q;
    assign bit_error_count = bit_errors_q;
    assign dc_change_count = dc_changes_q;
    assign overrun_count   = overruns_q;
    assign bit_count       = bit_cnt;
    assign last_bit_count  = last_cnt;
    assign framed          = framed_q;
    assign framing_error   = bit_errors_q != CNT_ZERO;
    assign dc_error        = dc_changes_q != CNT_ZERO;
    assign overrun         = overruns_q != CNT_ZERO;
endmodule
