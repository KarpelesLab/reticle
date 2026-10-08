# `spi_display_rx` — receiving a display's four-wire SPI

This is the one block in this library written from **nobody's
specification**. Everything it assumes about the link on the other side
of its pins was observed by a user with a logic analyser on their own
hardware, and the observation changed three times while the block was
being written. **And the screen is not on this machine** — it will be
connected on a different computer, by a different person, on different
hardware — so this page is not a prelude to a bring-up. It *is* the
bring-up: §4 and §6 are written for someone we will never talk to, and
every number in §7 is a ratio they can check against their own clock
rather than a frequency that happens to suit ours.

The project's confidence levels, which this page uses throughout:

- **OBSERVED** — the user watched it on an analyser. The strongest claim
  here about the far side, and the only kind there is.
- **MEASURED** — a number this repository produced by running something:
  a byte out of `sim::Simulator`, a cell count out of
  `fpga::synthesize_for`. Reproducible from a clean checkout by the test
  named beside it.
- **READING** — an inference that explains the rest, with no way to check
  it from here.
- **CHECKED** — observed on a real part, the way
  `ip/usb/usb_cdc_acm/README.md` means it. **There is no CHECKED claim on
  this page at all**, and §10 is about that, because
  `ip/crypto/sha256/README.md` §8 is this project's standing lesson that
  six kinds of simulation evidence and a wrong answer on silicon are
  perfectly compatible.

## 1. What was observed, in three passes

**First pass.** OBSERVED:

> "This SPI is used to drive a screen, I confirmed SCLK, commands sent on
> a wire, actual pixels sent on another wire, and the 4th wire is raised
> only between every 8 SCLK give or take (basically between bytes)."

Read at the time as: two *separate data lanes*, one for commands and one
for pixels, with no D/C pin — which would have been genuinely unusual,
since a display link muxes both onto one wire and uses D/C to say which.
A two-lane receiver was designed and built on that reading.

**Second pass.** OBSERVED: the fourth wire is not approximate.

> "I looked closely and confirm that 4th wire. SCLK sends bursts of 8
> bits, and in between each burst byte-strobe is powered."

This is the observation the block's framing rests on, and it is the most
load-bearing fact on this page: **bursts of exactly eight edges**, with
the fourth wire high in the gap. Not "give or take". That turned the
bit-count check from a tolerance into a fault indicator — see §4.

**Third pass.** OBSERVED, and it retired the two-lane reading:

> "nevermind I misread that, this is standard SPI with command and pixels
> on the same wire, and a wire that says if data or command (DC I guess),
> just because DC was also going on and off at each bit I mistook it for
> data."

and then, on `dc` itself:

> "you're right, DC shifts only once a byte, and is mostly the reverse of
> CS (I guess)."

So the link is conventional four-wire SPI:

| Wire | What it does | How sure |
|------|--------------|----------|
| `sclk` | the master's clock, in **bursts of exactly eight edges** with an idle gap between them. Not continuous. | OBSERVED, twice |
| `mosi` | the one data wire, carrying commands **and** pixels | OBSERVED (third pass; the first had this as two wires) |
| `dc` | says whether the byte is data or a command, and **holds for the whole byte** | OBSERVED (third pass; the first had it toggling per bit) |
| `cs_n` | high between bursts, low while a burst is in progress | OBSERVED |

Receive only. There is no `miso` in the capture and the block has no
transmit path.

**The two-lane reading was ruled out by the user, not overlooked.** It is
recorded here because the block kept two things from it: the byte
counters are per-*category* (`cmd_byte_count` and `data_byte_count`)
rather than one total, which is what the two-lane design needed and what
turns out to answer a better question; and `dc_change_count` exists
precisely because `dc` was once believed to carry data.

## 2. Why the fourth wire is called `cs_n`

READING: the fourth wire is the **chip select**. A master that deasserts
`cs_n` between byte transfers produces exactly the reported observation
— "high between bursts of eight" is an idle active-low select — and that
makes this textbook four-wire SPI (`sclk`, `mosi`, `cs_n`, `dc`) rather
than anything exotic. The block is named and documented for that reading.

**Nothing in the block depends on it.** What the logic needs from that
wire is a *frame*: an edge that says a byte ended and an edge that says
the next began. A per-byte strobe pulse would serve identically, which is
what `CS_PULSE = 1` is for. So the reading is a documentation decision,
and what would distinguish the two is a question about the user's master
rather than about this receiver: a chip select is usually deasserted once
per *transaction* — one select held across a command and its parameters
— and a select that drops between every byte is a master that chose to.
If the user's master ever holds the select across several bytes,
`FRAME_MODE = 1` is the setting for it and `bit_error_count` is what will
have said so.

## 3. `dc` looking like the inverse of `cs_n` is what a pixel run looks like

Worth writing down so that nobody re-derives it. The reading that `dc` is
"mostly the reverse of `cs_n`" is **the expected appearance of a
pixel-only capture**, not a property of the link:

- during bulk pixel traffic `dc` sits at one level for *thousands* of
  consecutive bytes while `cs_n` toggles once per byte;
- zoomed out, a constant `dc` beside a toggling `cs_n` reads as
  anti-correlation, when in fact `dc` is carrying no visible information
  at all and `cs_n` is doing all the moving;
- command bytes appear at initialisation and around address-window
  writes, so a capture taken mid-frame can contain **none**.

`cmd_byte_count` against `data_byte_count` settles it without anyone
looking at a waveform again:

- **commands ≈ 0 over a long capture** → it was a pixel run, `dc` was
  constant, nothing is odd. `spi_display_rx_receives_a_plausible_display_session`
  simulates exactly this and asserts `cmd_byte_count == 0` for it;
- **commands appear and `dc` still mirrors `cs_n` byte for byte** →
  something genuinely unusual, and worth stopping to understand before
  trusting any decode.

This is the same shape of note as the `VbusState` reading in
[`ip/usb/usb_host_ulpi/README.md`](../../usb/usb_host_ulpi/README.md) §5:
a reading that looked wrong, turned out to be the board and not the
decode, and cost rounds before someone traced the net and wrote it down.

## 3a. `frame_count` counted one frame too many, and the reset value that did it

**Fixed, 8 October 2026.** The account below keeps every measurement that
got there, including the two attempts that were wrong, because the way the
two faults hid each other is the useful part and is a shape worth
recognising again: *a defect and the thing that masked it were the same
flip-flop's reset value.*

### What was measured

**MEASURED, 8 October 2026, by `examples/basys3/spi_console_tb.v`**, which
drives this block's four pins and reads its counters back over a serial
port. Two readings:

- `frame_count` was **1 before anything had been driven at all**;
- it was **9 after eight frames**.

Every other counter was right, and the bytes were right, because the
phantom frame carried no bits. The only symptom was this one counter,
permanently one too high.

### The first fault: a reset value of the pin where the signal is normalised

`gap` is polarity-normalised — "high between bytes, whatever polarity the
pin uses" — so at idle it is 1 in *both* polarities, while `gap_q` reset
to `(CS_ACTIVE_LOW != 0) ? 1'b0 : 1'b1`, the idle level of the **pin**.
For an active-low select those differ, so a `gap_rise` was manufactured on
the first clock, and `frame_close` is `gap_rise` with no guard on it.

A part that comes out of reset with an **idle line** is between bytes, and
the reset value said it was inside one.

### The second fault: the phantom was also what armed the block

`seen_gap` gates `frame_open`, and it was latched **only inside
`if (frame_close)`**. So on an idle line the phantom edge was the only
thing that ever armed the block, and the two faults cancelled: the block
worked and miscounted.

Two measurements of that coupling, both run rather than reasoned:

- **`gap_q` corrected alone** (to `1'b1`): the phantom frame goes away and
  the **first byte of every session is dropped**, because nothing arms the
  block any more. `spi_console_tb` then reports 2 command bytes where 3
  were sent.
- **arming from the gap's *level* as well** (`if (gap) seen_gap <= 1'b1;`
  in place of the latch inside `frame_close`): satisfies
  `spi_console_tb` completely, and with the `cs_n` synchroniser's `INIT`
  also set to the inactive level it fails **fifteen of the sixteen**
  testbenches in §9 — every one that simulates — with `bit_error_count`
  reading 1 where a correct master disagrees with nothing, in every frame
  mode. With `INIT` left at 0 it fails exactly one of them,
  `spi_display_rx_takes_a_chip_select_of_either_polarity`.

That last line was recorded as "for reasons not yet understood" and the
change reverted. The reason is the **third** reset value, and finding it is
what made the fix work.

### The reason: a synchroniser's `INIT` is not an observation of a pin

For the first `SYNC_STAGES` clocks after reset each `cdc_sync` chain still
presents its own `INIT`, and on the clock after that the `*_q` an edge
detector compares against is still the `INIT`-derived one. So **any edge
this block detects in the first `SYNC_STAGES + 1` clocks is manufactured
by reset values and says nothing about the wire.**

That is why arming from the level failed. With `INIT` presenting "idle",
`gap` reads 1 during that blind window, the level arms the block, and then
the real level propagating in arrives as a `gap_fall` — a **frame open**,
on a line that was asserted all along. The block is framed in the middle
of the byte the reset landed in, which is precisely the failure
`spi_display_rx_cannot_be_mis_framed_by_a_late_start` exists to forbid, and
the first real frame close reports the one bit error. One spurious arm, one
spurious frame open, one spurious bit error, in every mode.

### The fix

A `SYNC_STAGES + 1`-bit shift register, `settle_sr`, counts the blind
window out, and **every** edge in the block is gated on it:

```verilog
wire settled   = settle_sr[SYNC_STAGES];
wire sclk_rise = settled && sclk_s && !sclk_q;
wire gap_rise  = settled && gap && !gap_q;
```

and `seen_gap` is armed from the gap's **level**, once that level is an
observation:

```verilog
if (settled && gap) seen_gap <= 1'b1;
```

Three things fall out of it:

- **No edge is ever manufactured**, so the reset values of `sclk_q` and
  `gap_q` stop mattering; `gap_q` resets to `1'b1`, the normalised "between
  bytes", and nothing compares against it until it has been loaded from a
  pin.
- **A part that comes up on an idle line arms without an edge**, which is
  the thing a frame-close latch structurally could not do. It is also the
  honest reading of "armed": an idle select *is* the gap, and seeing it is
  seeing it.
- **A part that comes up mid-byte still discards that byte**, because at
  the moment the window closes `gap` reads 0 and the block is not armed —
  so it waits for the real deassertion, exactly as before.

It costs **three flip-flops and four LUT4 on an ECP5** (§9), and one
transition: a pin that moves within `SYNC_STAGES + 1` clocks of reset
release is seen as a level and not as an edge. Nothing is armed or framed
that early, so there is nothing for it to lose.

### What `INIT` should be, which was the other open question

The question was whether the `cs_n` synchroniser's `INIT(0)` is wrong,
since with `CS_ACTIVE_LOW = 1` it presents "asserted" until the real level
arrives. The answer turned out to be that **nothing should depend on it**,
and now nothing does: `INIT` only ever colours the blind window, and no
edge is taken from inside the window. All four chains are left at `INIT(0)`
rather than given a polarity-dependent value, because a block whose
behaviour depends on a synchroniser's reset value is the defect, and
picking a better value would have hidden the next instance of it.

That is a claim, so it is measured two ways. The committed one is in
`spi_display_rx_counts_exactly_the_frames_it_was_driven`, which holds all
four pins **high** — the opposite level to every chain's `INIT` — with
nothing driven, so every chain transitions once as the real level
propagates, and asserts that no counter and no bit counter moves. Driving a
pin opposite to its `INIT` is the same experiment as flipping the `INIT`.
The uncommitted cross-check: flipping the `sclk` and `cs_n` chains' `INIT`
by hand leaves all sixteen testbenches passing, **MEASURED, 8 October
2026**.

### A second instance, found by asking the same question of the other pins

`frame_count` was the counter that was wrong, but the fault was a reset
value, so the same question was put to the other three pins — and `sclk`
had it too. With `sclk` resting **high** (CPOL = 1, an ordinary way for an
SPI master to idle) and `FRAME_MODE = 2`, which counts from reset with
`framed` already set, the edge of `sclk` propagating through its own
synchroniser **was taken as a bit**: `bit_count` read 1 before the master
sent anything, and every byte of the stream after it was one bit out.
**MEASURED, 8 October 2026** at `bit_count=1 frame_count=1` on the
unfixed block with all four pins idle high. The same `settled` gate fixes
it, and the zero-frame cases of the new testbench pin it.

No corresponding fault exists on `mosi` or `dc`: neither is edge-detected,
only sampled, and nothing samples them until the block is armed.

### Why the sixteenth testbench had to be written

This is the part worth keeping. The fifteen testbenches in §9 *do* compare
`frame_count` against the number of frames the model drove — `far.frames`,
asserted in most of them. They could not catch this because **every one of
them starts with the select asserted**, a part coming up inside a byte, and
in that state the two reset values agreed and no phantom was manufactured.
The one state that exposed it was the commonest one on a real board: the
line idle, and **nothing driven yet**.

So the new test's first assertion is the zero case — no frames driven, so
no frames counted — and it is the assertion that fails on the unfixed
block. Testing what a block *does* and testing what it *counts* are
different jobs, and a counter is only tested against a **total driven since
reset**, including zero. `frame_count` is now an absolute and not only a
difference.

## 4. The numbers, and how to read them

**This is the bring-up procedure.** Whoever connects the screen gets six
counters and nothing else — no oscilloscope trace from us, no reference
decode, no part we can compare against. So each one is written out with
what it should read and what a wrong value means.

An instrument whose expected reading is **exactly zero** is far more
useful than one whose expected reading is "small", and two of these are
in that category because of the second and third observations in §1.

| Counter | Width | Expected | What a non-zero value means |
|---------|-------|----------|------------------------------|
| `cmd_byte_count` | `COUNT_WIDTH` | — | Bytes delivered with `dc` saying *command*. Compare with the next row; that comparison is §3. |
| `data_byte_count` | `COUNT_WIDTH` | — | Bytes delivered with `dc` saying *data*. The two together are the delivered byte count, and the gap between their sum and `frame_count` is how many frames were rejected. |
| `frame_count` | `COUNT_WIDTH` | — | Frames the fourth wire closed, counted **raw**: before the block is framed, and whatever the bit count was. On a part that comes up with the select **idle** this is exactly the bytes in a clean run; on one that comes up **inside** a byte it is one more, because the deassertion that closes that byte is a real edge on the wire and this counter is what says the wire moved at all. It counts nothing that the wire did not do — §3a is the round that made that true, and `spi_display_rx_counts_exactly_the_frames_it_was_driven` is what holds it. |
| `bit_error_count` | `COUNT_WIDTH` | **0** | A frame did not carry eight bits. The bursts were OBSERVED to be exactly eight, so this is a fault and not a tolerance. `last_bit_count` says *which* count it arrived at — seven and nine are identical in this counter and completely different in the diagnosis. |
| `dc_change_count` | `COUNT_WIDTH` | **0** | `dc` moved inside a byte. Per-byte holding was OBSERVED, so a wire that disagrees with itself is either not `dc` — a mislabelled analyser channel, which is what the first pass in §1 actually was — or a display that qualifies per bit. At the default `DC_SAMPLE = 0` those bytes are **withheld**, because a wrongly tagged command byte is worse than a missing one. |
| `overrun_count` | `COUNT_WIDTH` | **0** | A sampled input held its level for fewer than `PHASE_MARGIN` system clocks, so its next transition could have been missed. This is the *early* warning: it rises before any bit is lost. See §7. |
| `last_bit_count` | 4 | **8** | The bit count at the most recent frame close. **A live value, not a log**: a good frame after a bad one overwrites it, so `bit_error_count` is the record and this is the detail. Saturates at fifteen, so sixteen bits cannot read as eight and look correct. |
| `bit_count` | 4 | — | Bits in the frame in progress, live. Useful on a probe, not in a log. |
| `framed` | 1 | **1** | The block has seen a frame close and then a frame open, so it is accumulating bits. **Read this first when nothing comes out at all.** |
| `framing_error`, `dc_error`, `overrun` | 1 | **0** | The three error counters as single wires, for a design that wants three lights rather than a bus. |

Every counter **saturates** rather than wrapping. An instrument that
wraps reports a small number for a large fault.

**The one output that is not a counter**: `rx_valid` is high for exactly
one system clock per delivered byte, with the byte on `rx_byte` and its
tag on `rx_is_data`. There is no buffer, so a consumer that cannot take a
byte a frame misses it — put `ip/memory/fifo_sync` behind it if that
matters.

**If `framed` is low**, no byte will ever be delivered in `FRAME_MODE` 0
or 1, and `frame_count` then says whether the fourth wire moved at all:
zero means the wire is absent, stuck, or on a different pin than you
think. That is a deliberate choice over guessing the byte phase, because
a guessed phase is wrong seven times in eight. `FRAME_MODE = 2` is the
escape hatch — it counts eight bits from reset and needs no frame — and
it still counts frames, so running in mode 2 *measures* whether mode 0
would have worked.

## 5. What a measurement still has to choose

Two parameters cannot be chosen by anything inside this block, and the
tests say so explicitly rather than leaving it implied.

**`SAMPLE_EDGE`** — which `sclk` edge samples `mosi`. A wrong choice
samples the wire as it changes and delivers plausible-looking rubbish
with **every counter still reading zero**. MEASURED, by
`spi_display_rx_samples_the_edge_and_the_bit_order_it_is_told_to`: it
builds a falling-edge master, reads it on the rising edge, and asserts
that three frames of eight bits arrive with no mismatch, no overrun and a
perfectly held `dc` — and that the bytes are wrong. `dc` cannot give it
away either, because a master drives the tag as it opens the frame and
both edges are long after that.

**`MSB_FIRST`** — the same kind of unknown. SPI is almost always most
significant bit first, which is the default, and a wrong choice is
bit-reversed bytes.

**The cheapest experiment that settles both** needs no extra hardware:
build the design twice, once at each `SAMPLE_EDGE`, and compare the
**command** byte stream against the screen's own initialisation
sequence. Display controllers open with a recognisable run — a software
reset, a sleep-out, a pixel-format byte — and exactly one of the four
(edge, bit order) combinations produces bytes that look like a controller
being initialised. A bit-reversed stream is obvious by eye; a one-bit
shift is obvious the moment two candidate decodes are put side by side.
That is why these are parameters: the measurement is a twenty-minute
experiment and the argument is unresolvable.

## 6. What to look at first when the numbers are wrong

In this order. It is cheap to write down now and expensive to work out
later.

1. **`framed` is low, or `frame_count` is 0** — the fourth wire is not
   reaching the block, or is on a different pin. Nothing else can be
   diagnosed until this is fixed. If the wire is definitely connected and
   definitely moving, try `CS_ACTIVE_LOW = 0`: at the wrong polarity the
   block frames the gaps instead of the bytes, which is loud rather than
   subtle (`bit_error_count` climbs and nothing is delivered).
2. **Bytes arrive and they are garbage, with every counter at zero** —
   the **sampling edge**. This is the first thing to suspect because it
   is the failure that is completely silent, and §5 says how to settle
   it. Try `SAMPLE_EDGE = 1`. If the bytes become plausible but
   *reversed*, that was the bit order instead: `MSB_FIRST = 0`.
3. **`bit_error_count` is climbing with `last_bit_count` below 8** — the
   **oversampling ratio** against the real `sclk`. Edges are being lost.
   Measure or look up the master's `sclk` and check it against §7's
   ratio; if it is too fast for the system clock, the answer is a faster
   system clock (or `PHASE_MARGIN = 1`, which buys a factor of two and
   gives up the early warning), never a change to this block.
   `overrun_count` will almost always be non-zero too, and in that order:
   the margin goes before the bits do.
4. **`overrun_count` is climbing but `bit_error_count` is 0** — the
   margin is gone and nothing is broken *yet*. Same remedy as 3, with
   less urgency. Check whether the idle gap is the culprit rather than
   the burst: §7 gives the gap its own minimum, and a short gap is the
   one that bites while the byte rate looks comfortable.
5. **`last_bit_count` reads 9 or more** — not a rate problem. 9 means
   there were more clock edges inside one frame than there should be.
   **15 is the saturation value**: the in-frame bit counter stops there,
   so 15 means at least fifteen edges arrived before the frame closed,
   and the usual cause is two frames merged because a close was missed —
   a gap below §7's minimum. It saturates rather than wrapping precisely
   so that sixteen bits cannot read as eight and look correct. Remember
   that this one is a live value: if `bit_error_count` is non-zero and
   `last_bit_count` reads 8, a good frame has overwritten the evidence
   and the stream is intermittent rather than uniformly broken.
6. **`dc_change_count` is non-zero** — the fourth thing to suspect, and
   the one that says the *wiring* is not what the labels claim. Either
   the wire called `dc` is really `mosi` on another channel — which is
   exactly the mistake the first pass in §1 made — or this display
   qualifies per bit. `DC_SAMPLE = 1` delivers the bytes anyway and tags
   each with its own first bit, which makes the mislabelling obvious: the
   tag will track the data.
7. **Everything reads zero and nothing happens at all** — suspect
   `sclk`. The block is entirely passive until an `sclk` edge arrives,
   and no counter distinguishes "no clock" from "no traffic".

So, condensed: **sampling edge, then the oversampling ratio against the
real `sclk`, then the fourth wire's polarity, then whether the fourth
wire is `cs_n` at all.**

## 7. The clock domain, and the rate as a ratio of clocks

`sclk` is an external pin, asynchronous to the design's clock, and **it
is not used as a clock here**. Two reasons:

1. an external pin driving a clock network needs a clock buffer the pad
   can actually reach, which is a constraint this backend only learned
   recently (`place::confine_to_reachable`); and
2. a block clocked on `sclk` has to cross a domain on the way out
   anyway, so the crossing is not avoided, only moved somewhere less
   convenient.

So all four pins go through their own `cdc_sync` and the whole block runs
on the system clock. MEASURED, by `spi_display_rx_is_one_clock_domain`:
`timing::analyze_cdc` finds one domain and no crossing in the
synthesised, flattened netlist.

**The rate limit is a ratio, not a frequency**, because nothing in the
block knows what the system clock runs at. A level presented to a
free-running sampler for `T` clock periods is seen by at least `floor(T)`
sampling edges, so for a transition to be certain of being seen with
margin in hand:

> **each `sclk` phase must last at least `PHASE_MARGIN` system clock
> periods**, so at a 50% duty cycle **one `sclk` period must last at
> least `2 * PHASE_MARGIN` system clocks** — four at the default, which
> makes the fastest `sclk` a **quarter of the system clock**.

That is the number to check, and it needs no knowledge of this project.
Worked examples at `PHASE_MARGIN = 2`:

| System clock | Minimum clocks per `sclk` period | Fastest `sclk` | Minimum `sclk` high and low time | Minimum `cs_n` deassertion |
|--------------|----------------------------------|----------------|----------------------------------|----------------------------|
| 12 MHz | 4 | 3 MHz | 167 ns | 167 ns |
| 25 MHz | 4 | 6.25 MHz | 80 ns | 80 ns |
| 50 MHz | 4 | 12.5 MHz | 40 ns | 40 ns |
| 60 MHz | 4 | 15 MHz | 33.3 ns | 33.3 ns |
| 100 MHz | 4 | 25 MHz | 20 ns | 20 ns |

A display link of a few megahertz is comfortably inside all of those. If
yours is not, raise the system clock — this block is cheap enough (§9)
that it is not what limits a design's clock — or set `PHASE_MARGIN = 1`,
which halves the requirement to two clocks per `sclk` period and gives up
the early warning in exchange, leaving `bit_error_count` as the only
instrument. MEASURED, by
`spi_display_rx_states_its_rate_limit_as_a_ratio_of_clocks`: the same
waveform at four clocks per `sclk` period passes at `PHASE_MARGIN` 1 and
2 and reports an overrun at 3 and 4, and at six clocks a period it passes
at 3 — the ratio is the rule and the parameter moves it.

**Above the limit, nothing is dropped quietly.** Two independent
instruments report it, in this order:

- at a *shade* over it — three system clocks a period at the default
  margin — a phase is sometimes seen only once. No edge is lost yet,
  every byte is still right, and `overrun_count` rises. **The warning
  arrives before the corruption**, which is the whole point of counting
  phases rather than bits;
- well past it — an `sclk` period and a half of the system clock — whole
  phases fall between sampling edges and edges are genuinely lost. The
  frame then closes at fewer than eight bits, `bit_error_count` rises,
  `last_bit_count` says how many bits did arrive, and in the default mode
  **nothing is delivered**: a corrupt byte is never shifted out.

Both are asserted in `spi_display_rx_reports_an_sclk_it_cannot_oversample`.

**The gap gives the recovery time**, so the constraint is on the in-burst
`sclk` period and not on a sustained byte rate: delivering a byte takes
one system clock and there is no buffer to drain, so a frame may follow
its predecessor as closely as the fourth wire allows. But **a short gap
can starve the block even when the byte rate is comfortable**, because
that wire is synchronised and edge-detected exactly like `sclk`:

> **`cs_n` must be deasserted for at least `PHASE_MARGIN` system clock
> periods** — two by default, which is the last column of the table
> above.

`spi_display_rx_reports_a_gap_too_short_to_see` pins both sides of it: at
exactly two clocks the bytes are right and `overrun_count` is 0; at one
clock the bytes still survive but `overrun_count` rises, because the
deassertion was seen once and the assertion after it could have been
missed. A gap missed altogether merges two bytes, and the next frame
close reports `last_bit_count` as **15** — sixteen bits arrived and the
counter saturates there rather than wrapping onto eight and looking
correct. `spi_display_rx_reports_two_frames_merged_by_a_gap_it_never_saw`
drives a gap of half a system clock, which falls **entirely between two
sampling edges**: `overrun_count` cannot see that one at all, because
there is no sampled edge whose margin it could measure, which is exactly
why the bit count is a second instrument and not a redundant one.

## 8. Why `mosi` and `dc` cannot skew against each other

Three structural reasons rather than timing arguments:

1. each input goes through **its own `cdc_sync` instance with the same
   `SYNC_STAGES`**, so `mosi` and `dc` are delayed by exactly the same
   number of system clocks. Nothing in the block can give one wire a
   longer path than the other;
2. both are sampled **in the same clock cycle by the same enable** — the
   one detected `sclk` edge — reading the two synchroniser outputs at one
   instant. There is no per-wire sampling decision to disagree about;
3. the sample instant is **half an `sclk` period away from either wire's
   transitions**, which §7's rate limit makes at least `PHASE_MARGIN`
   system clocks. A wire's first synchroniser flop may resolve a
   metastable input either way, and so may see a transition a clock early
   or late, independently per wire — but that uncertainty is one clock,
   spent `PHASE_MARGIN` clocks away from the instant that matters. Driver
   skew between the two is absorbed by the same margin.

`spi_display_rx_samples_both_wires_whatever_their_skew` drives `mosi`
immediately after the non-sampling edge and `dc` three system clocks
later, right up against the setup the first sampling edge leaves, and
checks every byte and every tag.

## 9. What the tests would and would not catch

Sixteen testbenches drive a model of the master — `FarSide` in
`tests/ip_library.rs` — in **absolute simulation time**, with every
far-side event on an odd tick so that no pin ever changes in the same
instant as the system clock edge that samples it. That race is one an
event-driven simulator is entitled to resolve either way and a real
circuit is entitled to lose, and the model asserts the property rather
than trusting the arithmetic.

Area, MEASURED by `footprints_match_the_documentation` and tabulated in
[`docs/ip-library.md`](../../../docs/ip-library.md): on an ECP5 45F,
**57 `CCU2C` and 101 LUT4** with 144 flip-flops at the defaults, of which
**96 flip-flops are the six counters** at `COUNT_WIDTH = 16`. The
receiver proper is under fifty flip-flops, so `COUNT_WIDTH = 8`
roughly halves the block and `COUNT_WIDTH = 1` turns every counter into a
sticky flag. LUT depth 4, and `FRAME_MODE = 2` is 99 LUT4 with two fewer
flip-flops.

Three of those flip-flops and four of those LUT4 are §3a's `settle_sr`,
which is `SYNC_STAGES + 1` bits wide and gates every edge detector in the
block. It is the price of not taking a synchroniser's reset value for an
observation of a pin, and at two per cent of the block's storage it is
not a price worth optimising.

This block was first measured at 209 LUT4 and depth 5, before this
backend inferred a carry cell, and the correction is instructive: **more
than half of its logic was the six counters' ripple adders.** That is the
cost of instrumenting a block, and it is the reason `COUNT_WIDTH` is a
parameter — a bring-up build wants all six counters at 16 bits, and a
deployed one may want none of them. `FRAME_MODE = 2` used to be 11 LUT4
smaller than mode 0 and is now two LUT4 smaller, because what it removed
was addition that a carry chain no longer spends lookup tables on.

Beyond those sixteen, two shared tests name this block:
`spi_display_rx_is_one_clock_domain` asks `timing::analyze_cdc` for its
domains, and `a_streams_ready_is_a_function_of_registers` walks the
timing graph backwards from every output a design reads — `rx_valid`,
`rx_byte`, `rx_is_data`, all six counters and the three error wires — and
asserts that **no input port is reachable**. That is the mechanised form
of the whole argument for oversampling: `rx_valid` written as a
combinational `assign` would still pass the one-domain test and would put
an asynchronous pin straight into the consumer's logic, and this one
fails it.

The sixteenth is
`spi_display_rx_counts_exactly_the_frames_it_was_driven`, and it is the
only one that is about the **accounting** rather than the behaviour: it
compares all six counters against totals a model chose, from a reset, for
0, 1, 2, 8 and 70 frames, in all three framing modes, with the select both
idle and asserted when the reset is released. §3a is why it exists. Its
zero cases are also the committed form of "no counter moves from a
synchroniser's reset value": all four pins are held at the opposite level
to every chain's `INIT` with nothing driven, and every counter and
`bit_count` must read zero.

**What the tests would catch.** A wrong shift direction, a lost or
doubled bit, a byte delivered twice (`rx_valid` is checked to be one
cycle per counted byte, and every byte to land in exactly one of the two
category counters), a frame of seven or nine bits treated as eight, a
byte delivered when the frame said it should not be, a mis-framed start
after a reset landing mid-byte, a counter that disagrees with its sticky
wire, a `dc` sampled at the wrong point or carrying the data, an `sclk`
too fast to oversample, a `PHASE_MARGIN` that does not mean what §7 says
it means, a gap too short to see, and a fourth wire of the wrong
polarity, and a gap of half a system clock that is missed entirely —
which merges two bytes and is the one case `overrun_count` cannot see,
since there is no sampled edge whose margin it could measure. Each of the
off-by-one cases is run in **all three** framing modes, because the modes
differ precisely in what they do about them.

**And, since §3a, a counter that is off by a fixed amount from reset** —
any of the six, in either start state, including with nothing driven at
all. That is the one item this list gained by being wrong about it.

**What they would not catch**, and this list is the honest part:

- **the sampling edge or the bit order.** §5 is explicit: the tests
  *prove* the block cannot tell, which is a useful thing to have proved
  and is not the same as choosing;
- **metastability.** The simulator has no metastable resolution, so the
  synchronisers are tested for latency and not for what they are for.
  §8's argument is a reading of how the sampling is structured, not a
  measurement;
- **anything about pins, pads or IO timing.** See §10;
- **the real `sclk` frequency.** §7 is arithmetic about a ratio. Nobody
  here has measured the master's clock, and if it turns out to be above
  the ratio, the remedy is in §6 step 3;
- **what the bytes mean.** There is no display model here. A decode that
  is bit-perfect and addresses the wrong window would pass every test on
  this page;
- **a counter that is wrong only once it has saturated.** Nothing drives
  65 536 frames, so the saturation arithmetic is exercised by the
  `COUNT_WIDTH` the testbenches use and not at the top of the range;
- **a glitch.** There is no deglitching beyond the synchroniser, so a
  runt on `sclk` lasting `PHASE_MARGIN` system clocks is a bit as far as
  this block is concerned, and no test says otherwise;
- **`CS_PULSE = 1` as a *better* reading.**
  `spi_display_rx_takes_a_boundary_pulse_instead_of_a_level` drives a
  pulse and decodes it both ways, and the level reading wins: a pulse's
  own trailing edge serves as the frame open, so reading it as a level
  loses nothing, while the pulse setting spends the first byte arming.
  The parameter is tested and **not vindicated**; it is there for a
  strobe whose release cannot be trusted or does not exist.

## 10. Nothing here has been near a part

**There is no CHECKED claim on this page.** No bitstream, no board, no
pin assignment, no top level — and that is by instruction rather than by
omission: the screen is not on this machine and will be connected
elsewhere, so a board design made here would be a guess about hardware
nobody here can see. There is deliberately no
`testdata/fpga/*` top level and no constraints file for this block.

This project has a specific reason to say that loudly.
`ip/crypto/sha256/README.md` §8 records a block that passed published
vectors, a second independent implementation, mapped-netlist equivalence
at two lookup-table widths, a constant-time measurement and every
simulation this repository can run — and then computed **wrong digests**
on an ECP5, differently between identical runs, because the data path had
no carry cell under it. Simulation has no unrouted wires and no
metastability, and that is not a small gap: an unrouted slice input on an
ECP5 reads as a one, and `CLAUDE.md` records eight rounds lost to a
three-bit register that read 5.

That particular cause is now fixed — this backend infers a `CCU2C`, which
is where this block's 57 of them come from — and the round that fixed it
proved the point a second time rather than retiring it. A carry chain's
constant operand pins were left unrouted, so `cnt + 1` computed `cnt - 1`
on the part while placing, routing, decoding, passing an exhaustive check
*and* passing a SAT equivalence proof, because in a netlist a constant pin
evaluates to the constant. A board caught it; nothing else could.

So, concretely, what remains unknown about this block:

- **that the four signals arrive.** Nothing here says these pins can be
  assigned on any particular part, share a bank, meet an IO standard, or
  reach the fabric with the setup a real pad needs;
- **that the timing closes.** The LUT depth in §9 is the mapped
  combinational depth, which is the shape of the critical path and not a
  closed clock. The counters are carry chains now rather than ripples of
  lookup tables, which is why the depth is 4 and not 5, but a depth is
  still not a slack figure and there is no vendor timing model here;
- **that the far side is what §1 says.** Every fact about the link is one
  person's reading of an analyser display. The two counters in §4 that
  should read zero are the test of that reading, and they can only be run
  by whoever has the screen;
- **that `dc` and the data wire are not swapped**, which is §6 step 6 and
  the one wiring error this block can detect by itself.

What would close all of it is one run on the part the screen is attached
to, with the §4 counters read out. Two zeros — `bit_error_count` and
`dc_change_count` — would be worth more than every assertion on this page
put together, because they are the only evidence that could exist that
the far side is what the capture suggested.
