# `spi_display_rx` — receiving a display's four-wire SPI

This is the one block in this library written from **nobody's
specification**. Everything it assumes about the link on the other side
of its pins was observed by the user with a logic analyser on their own
hardware, and the observation changed three times while the block was
being written. So this page is organised the way `docs/fpga-trellis.md`
is: what was observed, what was *concluded*, and which conclusions were
wrong — because the two wrong readings are the reason two of the
parameters exist.

The project's rule applies throughout: a claim that was **measured** is
marked as such, and a claim that is only a **reading** says so.

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
| `mosi` | the one data wire, carrying commands **and** pixels | OBSERVED (third pass; the second pass had this as two wires) |
| `dc` | says whether the byte is data or a command, and **holds for the whole byte** | OBSERVED (third pass; the second pass had it toggling per bit) |
| `cs_n` | high between bursts, low while a burst is in progress | OBSERVED |

Receive only. There is no `miso` in the capture and the block has no
transmit path.

**The two-lane reading was ruled out by the user, not overlooked.** It is
recorded here because the block kept two things from it: the counters are
per-*category* (`cmd_byte_count` and `data_byte_count`) rather than one
total, which is what the two-lane design needed and what turns out to
answer a better question; and `dc_change_count` exists precisely because
`dc` was once believed to carry data.

## 2. Why the fourth wire is called `cs_n`

READING, not measured: the fourth wire is the **chip select**. A master
that deasserts `cs_n` between byte transfers produces exactly the
reported observation — "high between bursts of eight" is an idle
active-low select — and that makes this textbook four-wire SPI
(`sclk`, `mosi`, `cs_n`, `dc`) rather than anything exotic. The block is
named and documented for that reading.

**Nothing in the block depends on it.** What the logic needs from that
wire is a *frame*: an edge that says a byte ended and an edge that says
the next began. A per-byte strobe pulse would serve identically, which is
what `CS_PULSE = 1` is for. So the reading is a documentation decision,
and the thing that would distinguish the two is a question for the
user's master rather than for this receiver: a chip select is deasserted
once per *transaction* on most displays — one select held across a
command and its parameters — and a select that drops between every byte
is a master that chose to. If the user's master ever holds the select
across several bytes, `FRAME_MODE = 1` is the setting for it and
`bit_error_count` is what will have said so.

## 3. `dc` looking like the inverse of `cs_n` is what a pixel run looks like

Worth writing down so that nobody re-derives it. The user's reading that
`dc` is "mostly the reverse of `cs_n`" is **the expected appearance of a
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

## 4. The numbers that should be zero

An instrument whose expected reading is **exactly zero** is far more
useful than one whose expected reading is "small", and two of these are
now in that category because of the second and third observations above.

| Counter | Expected | What a non-zero value means |
|---------|----------|------------------------------|
| `bit_error_count` | **0** | A frame did not carry eight bits. The bursts were confirmed to be exactly eight, so this is a fault: the sampling edge is wrong, the oversampling is too slow, or the far side is not what we think. `last_bit_count` says *which* count it arrived at — seven and nine look identical in the counter and completely different in the diagnosis. |
| `dc_change_count` | **0** | `dc` moved inside a byte. `dc` holding for a whole byte is confirmed, so a wire that disagrees with itself is either not `dc` — a mislabelled analyser channel, which is what the second pass above actually was — or a display that qualifies per bit. In the default `DC_SAMPLE = 0` those bytes are **withheld**, because a wrongly tagged command byte is worse than a missing one. |
| `overrun_count` | **0** | A sampled input held a level for only one system clock, so its next transition could have been missed. See §6. |
| `cmd_byte_count`, `data_byte_count` | — | Not expected to be anything. They are the measurement of §3. |
| `frame_count` | — | Frames `cs_n` closed, counted raw: before the block is framed, and whatever the bit count was. Compare it with the byte counts to see how many frames were rejected. |

`framed` is the fifth instrument and the one to read first when nothing
comes out at all: in `FRAME_MODE` 0 and 1 a `framed` that stays low means
**no byte will ever be delivered**, and `frame_count` then says whether
the wire moved at all. That is the diagnostic for a select that is
absent, stuck, or on the wrong pin — and it is a deliberate choice over
guessing a phase, because a guessed phase is wrong seven times in eight.

Every counter saturates rather than wrapping. An instrument that wraps
reports a small number for a large fault.

## 5. What a measurement still has to choose

Two parameters cannot be chosen by anything inside this block, and the
tests say so explicitly rather than leaving it implied.

**`SAMPLE_EDGE`** — which `sclk` edge samples `mosi`. A wrong choice
samples the wire as it changes and delivers plausible-looking rubbish
with **every counter still reading zero**:
`spi_display_rx_samples_the_edge_and_the_bit_order_it_is_told_to` builds
a falling-edge master, reads it on the rising edge, and asserts that
three frames of eight bits arrive with no mismatch, no overrun and a
perfectly held `dc` — and that the bytes are wrong. `dc` cannot give it
away either, because a master drives the tag as it opens the frame and
both edges are long after that.

**`MSB_FIRST`** — the same kind of unknown. SPI is almost always most
significant bit first, which is the default, and a wrong choice is
bit-reversed bytes.

**The cheapest experiment that settles both** needs no new hardware and
no board: build the design twice, once at each `SAMPLE_EDGE`, and compare
the **command** byte stream against the screen's own initialisation
sequence. Display controllers open with a recognisable run — a software
reset, a sleep-out, a pixel-format byte — and exactly one of the four
(edge, bit order) combinations produces bytes that look like a controller
being initialised. A bit-reversed stream is obvious by eye; a one-bit
shift is obvious the moment two candidate decodes are put side by side.
That is why the parameters are parameters: the measurement is a
twenty-minute experiment and the argument is unresolvable.

## 6. The clock domain, and the rate

`sclk` is an external pin, asynchronous to the design's clock, and **it
is not used as a clock here**. Two reasons:

1. an external pin driving a clock network needs a clock buffer the pad
   can actually reach, which is a constraint this backend only learned
   recently (`place::confine_to_reachable`); and
2. a block clocked on `sclk` has to cross a domain on the way out
   anyway, so the crossing is not avoided, only moved somewhere less
   convenient.

So all four pins go through their own `cdc_sync` and the whole block runs
on the system clock. **MEASURED**, by
`spi_display_rx_is_one_clock_domain`: `timing::analyze_cdc` finds one
domain and no crossing in the synthesised, flattened netlist.

**The rate limit.** A level presented to a free-running sampler for `T`
clock periods is seen by at least `floor(T)` sampling edges, so each
`sclk` phase must last **at least two system clock periods** for its
transition to be certain of being seen with a clock of margin in hand.
That is an `sclk` period of four system clocks at a 50% duty cycle:

| System clock | Fastest `sclk` | Minimum high and low time |
|--------------|----------------|---------------------------|
| 60 MHz (Cynthion) | **15 MHz** | 33.3 ns each |
| 100 MHz | 25 MHz | 20 ns each |
| 12 MHz | 3 MHz | 167 ns each |

A display link of a few megahertz is an order of magnitude inside the
60 MHz figure. `spi_display_rx_reports_an_sclk_it_cannot_oversample`
pins the limit itself: at exactly four system clocks a period every byte
is right and `overrun_count` is **0**.

**Above it, nothing is dropped quietly.** Two independent instruments
report it, in this order:

- at a *shade* over the limit — three system clocks a period — a phase
  is sometimes seen only once. No edge is lost yet, every byte is still
  right, and `overrun_count` rises. **The warning arrives before the
  corruption**, which is the whole point of counting phases rather than
  bits.
- well past it — an `sclk` period and a half of the system clock — whole
  phases fall between sampling edges and edges are genuinely lost. The
  frame then closes at fewer than eight bits, `bit_error_count` rises,
  `last_bit_count` says how many bits did arrive, and in the default
  mode **nothing is delivered**: a corrupt byte is never shifted out.

Both of those are asserted in that test.

**The gap gives the recovery time**, so the constraint is on the
in-burst `sclk` period and not on a sustained byte rate: delivering a
byte takes one system clock and there is no buffer to drain, so a frame
may follow its predecessor as closely as `cs_n` allows. But **a short
gap can starve the block even when the byte rate is comfortable**,
because `cs_n` is synchronised and edge-detected exactly like `sclk`:

> **`cs_n` must be deasserted for at least two system clock periods —
> 33.3 ns at 60 MHz.**

That is a fact the user can check against their capture.
`spi_display_rx_reports_a_gap_too_short_to_see` pins both sides of it: at
exactly two clocks the bytes are right and `overrun_count` is 0; at one
clock the bytes still survive but `overrun_count` rises, because the
deassertion was seen once and the assertion after it could have been
missed. A gap missed altogether merges two bytes and shows up as sixteen
bits at the next frame close.

## 7. Why `mosi` and `dc` cannot skew against each other

Three structural reasons rather than timing arguments:

1. each input goes through **its own `cdc_sync` instance with the same
   `SYNC_STAGES`**, so `mosi` and `dc` are delayed by exactly the same
   number of system clocks. Nothing in the block can give one wire a
   longer path than the other;
2. both are sampled **in the same clock cycle by the same enable** — the
   one detected `sclk` edge — reading the two synchroniser outputs at one
   instant. There is no per-wire sampling decision to disagree about;
3. the sample instant is **half an `sclk` period away from either wire's
   transitions**, which §6's rate limit makes at least two system clocks.
   A wire's first synchroniser flop may resolve a metastable input either
   way, and so may see a transition a clock early or late, independently
   per wire — but that uncertainty is one clock, spent two clocks away
   from the instant that matters. Driver skew between the two is absorbed
   by the same margin.

`spi_display_rx_samples_both_wires_whatever_their_skew` drives `mosi`
immediately after the non-sampling edge and `dc` three system clocks
later, right up against the setup the first sampling edge leaves, and
checks every byte and every tag.

## 8. What the tests would and would not catch

Twelve testbenches drive a model of the master — `FarSide` in
`tests/ip_library.rs` — in **absolute simulation time**, with every
far-side event on an odd tick so that no pin ever changes in the same
instant as the system clock edge that samples it. That race is one an
event-driven simulator is entitled to resolve either way and a real
circuit is entitled to lose, and the model asserts the property rather
than trusting the arithmetic.

**What they would catch.** A wrong shift direction, a lost or doubled
bit, a byte delivered twice (`rx_valid` is checked to be one cycle per
counted byte, and every byte to land in exactly one of the two category
counters), a frame of seven or nine bits treated as eight, a byte
delivered when the frame said it should not be, a mis-framed start after
a reset landing mid-byte, a counter that disagrees with its sticky wire,
a `dc` sampled at the wrong point, an `sclk` too fast to oversample, a
gap too short to see, and a chip select of the wrong polarity. Each of
the off-by-one cases is run in **all three** framing modes, because the
modes differ precisely in what they do about them.

**What they would not catch**, and this list is the honest part:

- **the sampling edge or the bit order.** §5 is explicit: the tests
  *prove* the block cannot tell, which is a useful thing to have proved
  and is not the same as choosing;
- **metastability.** The simulator has no metastable resolution, so the
  synchronisers are tested for latency and not for what they are for.
  The argument in §7 is a reading of how the sampling is structured, not
  a measurement;
- **anything about the pins.** No pin assignment exists yet (§9), so
  nothing here says these four signals can be routed to one bank, meet
  their IO standard, or reach the fabric with the setup a real part
  needs;
- **the real `sclk` frequency.** The rate analysis is arithmetic about
  the system clock, and the user's actual `sclk` has not been measured.
  If it turns out to be above 15 MHz at 60 MHz, the answer is a faster
  system clock, not a change to this block;
- **what the bytes mean.** There is no display model here. A decode that
  is bit-perfect and addresses the wrong window would pass every test on
  this page;
- **a glitch.** There is no deglitching beyond the synchroniser, so a
  runt on `sclk` lasting two system clocks is a bit as far as this block
  is concerned, and no test says otherwise.

## 9. What a board would add, and what the user has to tell us first

**No board was touched.** Two other rounds may have had the Cynthion, and
more to the point **the pin assignment is not known**, which makes a
bitstream impossible rather than merely unwise.

What a board would add that simulation cannot:

- that the four signals *arrive* — the right wires on the right balls,
  at an IO standard the bank can carry;
- the two numbers in §4 read off real traffic. `bit_error_count` and
  `dc_change_count` are expected to be zero, and a zero from the part is
  worth more than every assertion on this page put together, because it
  is the only evidence that the far side is what the capture suggested;
- whether `cmd_byte_count` stays at zero over a long run, which settles
  §3;
- the real `sclk` rate, by implication: an `overrun_count` of zero after
  minutes of traffic is a measurement that the link is inside the §6
  limit.

**What the user has to tell us to get there: which four pins the signals
arrive on.** That is the whole blocker. And the Cynthion makes it
awkward, for a reason worth repeating from
`testdata/fpga/cynthion/bidir_loopback.v`: the only free user IO on the
part this project describes is the **two PMOD headers** — PMOD A is
`C9 B9 D11 C12 C8 D8 D9 C10` and PMOD B is `B4 B5 B6 B7 C5 A5 A6 A7`,
all `dir="io"` on the top edge — and this project has deliberately
avoided them until now, because *nothing this machine can read says
whether anything is plugged into one*. Every other pin on the two edges
the backend describes is claimed by the ULPI transceivers, the HyperRAM,
the Type-C controllers or the LEDs, and **driving a pin something else
on the board also drives can damage hardware**.

So the cheapest experiment, in order:

1. the user says which four PMOD pins they will wire the screen's
   `sclk`, `mosi`, `dc` and `cs_n` to, and confirms nothing else is on
   that header. Four inputs is the easy direction — the block drives
   none of them, so the worst case of a wrong guess is that it reads
   rubbish rather than that it fights another driver;
2. a top level that is this block plus `ip/bus/uart`'s transmitter, with
   the five counters of §4 and `last_bit_count` clocked out as a line of
   text once a second. No new HDL beyond the top level, and the answer
   arrives on a serial terminal;
3. if the serial port is inconvenient, the degenerate version is three
   LEDs: `framing_error`, `dc_error` and `overrun`. Three dark LEDs after
   a minute of traffic is the headline result, and the LEDs are a pin
   assignment this project has already used and watched
   (`testdata/fpga/cynthion/leds.v`).

Step 2 is the one worth doing, because the counters are the block's
reason for existing and a light that stays dark does not say what the
traffic *was*.
