# `iso7816_uart` — the character layer of an ISO/IEC 7816-3 terminal

One wire, half duplex, open drain, eight data bits with even parity and
two stop bits — "8E2" — at a rate that changes between characters. This
is the terminal's half of a smart card contact, and only the *character*
layer of it: the card clock, the activation and reset sequence, the ATR
decode, PPS and the T=0 and T=1 transport layers are all somebody else's
block.

What it does provide for that sequencer is `active`: a way to be held
**inert**, with the wire released, so the sequencer can own the contact
while VCC is switched. §4 says why releasing the line is the safe state
and why coming back out of it cannot invent a character.

The project's confidence levels, which this page uses throughout:

- **QUOTED** — taken from ISO/IEC 7816-3 as the standard was described to
  this project. The strongest claim on this page about the protocol, and
  it is a *reading* until a card measures it.
- **MEASURED** — a number this repository produced by running something:
  a byte out of `sim::Simulator`, a cell count out of
  `fpga::synthesize_for`. Reproducible from a clean checkout by the test
  named beside it.
- **DECIDED** — a question the standard as described does not answer, and
  the answer this block gives. §2 is the list, and every entry says what
  would change it.
- **CHECKED** — observed on a real part, the way
  `ip/usb/usb_cdc_acm/README.md` means it. **There is no CHECKED claim on
  this page at all**, and §6 is about that.

## 1. Nothing here has been near a card or a part

Said first, because it is the most important thing on the page.

- **No card.** Every statement about the far side is QUOTED from a
  specification. Nothing in this repository has sent a character to a
  smart card or received one from one, and the far side in every test is
  a model written from the same quotations the block was — so the tests
  prove the block does what this page says, not that this page is right.
- **No part.** No bitstream, no pin assignment, no top level under
  `testdata/fpga/`, and no constraints file. The numbers in §7 are a
  mapped cell count and a combinational depth, which is the shape of a
  critical path and not a closed clock.

`ip/crypto/sha256/README.md` §8 is this project's standing reason to say
that loudly: a block that passed published vectors, a second independent
implementation, mapped-netlist equivalence at two lookup-table widths, a
constant-time measurement and every simulation this repository can run,
and then computed **wrong digests** on an ECP5 because the data path had
no carry cell under it. Simulation has no unrouted wires; an unrouted
slice input on an ECP5 reads as a one, and `CLAUDE.md` records eight
rounds lost to a three-bit register that read 5.

What would close it is one session against a real card with §3's five
counters read out. Two zeros — `parity_error_count` and `timeout_count` —
after an ATR and a PPS would be worth more than everything below.

## 2. The protocol, and the six questions it did not answer

### QUOTED

| Thing | What the standard says |
|-------|------------------------|
| etu | The bit period: `F/D` cycles of the card clock. Default `F = 372`, `D = 1`, so 21505 baud at 8 MHz. After a successful PPS exchange `F/D` shrinks and the rate rises. |
| Character | One start bit (low, 1 etu), eight data bits, one parity bit **making the count of ones in those nine bits even**, then a guard time of at least 2 etu with the line high. |
| Direct convention | First ATR byte `TS = 0x3B`. A logic one is **high**; the data bits arrive **least significant first**. |
| Inverse convention | `TS = 0x3F`. A logic one is **low**; the data bits arrive **most significant first**. Both the polarity and the bit order invert. |
| The wire | One wire, half duplex, **open drain**. It idles high through a pull-up; a transmitter pulls it low for a zero and *releases* it for a one. |
| T=0 parity error | A receiver that sees bad parity pulls the line low for 1 to 2 etu starting 10.5 etu after the start bit's falling edge. The transmitter samples at about 11.5 etu and repeats the character if it finds the line low. |
| Guard time | Beyond the mandatory 2 etu it is `TC1` from the ATR: 0 to 254 extra etu. The obligation is on the interval between the **leading edges of two consecutive characters**. |
| Waiting time | A card that never answers must become a reported timeout. |

### DECIDED — and this is the honest list

Six things the account above does not settle. Each is a decision, not a
reading, and each says what would change it.

1. **How many times a character may be repeated.** Nothing fixes a
   limit, and a block that repeated for ever is the hang a waiting time
   exists to prevent. `MAX_REPEAT` is a parameter, default 3, and giving
   up raises `tx_abort` and completes the handshake. **What would change
   it:** a terminal specification that names a number, or a card that
   turns out to signal errors far more often than three characters in a
   row.

2. **When the terminal stops expecting a character.** The waiting time is
   QUOTED as the maximum interval between two consecutive characters'
   leading edges, so the timer is armed at the end of **any** character
   and fires once per arming. That means a link which has simply finished
   its exchange produces one `rx_timeout` — because this block has no
   idea how long the card's answer was supposed to be. Knowing that is
   the transport layer's job, and the transport layer is not here. **What
   would change it:** an `expect` input driven by a T=0 state machine
   that knows the expected length, which is the shape the next block up
   should have. Until then a host that knows the answer is complete sets
   `wt_etu` to zero, or ignores the strobe.

3. **`TC1 = 255`.** ISO 7816-3 gives 255 a special meaning for T=1 rather
   than "255 extra etu". This block treats `guard_etu` uniformly as extra
   etu, so 255 asks for 257 etu between leading edges. **What would
   change it:** implementing T=1, at which point the special case belongs
   in whatever decodes the ATR, not here.

4. **Who wins when both ends start at once.** On a half-duplex wire the
   terminal might accept a byte in the same cycle a start edge arrives.
   This block gives the **receive** path priority: the card is already
   talking, and the byte waits. `tx_ready` is low for the whole of an
   incoming character and for the guard interval after it, so a producer
   observing the handshake never sees its byte taken into a collision.
   **What would change it:** nothing obvious; the opposite choice loses a
   character off the wire rather than delaying one into it.

5. **What happens to a character in flight when the contact is
   deactivated.** It is abandoned, `tx_abort` reports it, and
   `tx_char_count` does not count it — that counter is characters whose
   *transmission completed*. The alternative, finishing the frame before
   going inert, cannot be offered: the hazard in §4 is that the wire must
   be released before VCC drops, and a frame is up to 256 etu long.
   **What would change it:** nothing. A sequencer that wants the frame
   finished waits for `tx_ready` before deasserting `active`, which it
   can, and this block cannot make that decision for it.

6. **Whether a character that failed its parity check should be
   delivered.** It is, with `rx_parity_error` beside it, and the repeat
   arrives afterwards as a second character. That follows
   `ip/bus/uart`'s `uart_frame_rx` — a wrong byte at a consumer is more
   informative than silence — and it keeps `rx_char_count` a count of
   characters *on the wire*, which is what makes it comparable against a
   driven total. **What would change it:** a consumer for which a bad
   byte is worse than a missing one, which would want a parameter rather
   than a different default.

## 3. The interface

Everything a session changes is a **run-time input**, and every rate is
in **system clock cycles** rather than card clocks, which is
`ip/bus/uart`'s `div` convention and keeps every divisor a whole number.

### The board's numbers

The system clock is **112 MHz**, chosen because it divides by an even
number to every card clock the owner wants — so the card clock's duty
cycle is exactly half at every rate, and `etu_div`, which is
`(F/D) × divisor`, is an exact whole number at every rate too:

| Card clock | Divisor from 112 MHz | `etu_div` at `F/D = 372` | baud | `etu_div` at `F/D = 4` | baud |
|-----------|---------------------|--------------------------|------|------------------------|------|
| 8 MHz | 14 | **5208** | 21505 | **56** | 2 000 000 |
| 7 MHz | 16 | 5952 | 18817 | 64 | 1 750 000 |
| 5.6 MHz | 20 | 7440 | 15054 | 80 | 1 400 000 |
| 4 MHz | 28 | 10416 | 10753 | 112 | 1 000 000 |
| 3.5 MHz | 32 | 11904 | 9409 | 128 | 875 000 |
| 2 MHz | 56 | 20832 | 5376 | 224 | 500 000 |
| 1.75 MHz | 64 | 23808 | 4704 | 256 | 437 500 |
| 1 MHz | 112 | 41664 | 2688 | 448 | 250 000 |

`DIV_MIN` is 8, so the fastest of those is seven times the floor. Nothing
in the block knows any of this: it is told clocks per etu and nothing
else, which is why the table is here and not in the HDL.

| Port | Width | What it is |
|------|-------|------------|
| `active` | 1 | Low holds the block **inert**: wire released, transmitter idle and refusing bytes, anything in flight abandoned with `tx_abort`, receiver held off. Synchronous to `clk` and registered, so it takes effect on the next clock. |
| `etu_div` | 16 | System clocks per etu. 5208 is 21505 baud on the intended board; 56 is 2 Mbaud. Read **once per character**. Below `DIV_MIN` it means "use `ETU_DIV`" — see §4. |
| `guard_etu` | 8 | Extra guard etu beyond the mandatory two: `TC1`. Read when a character is accepted. |
| `convention` | 1 | 0 direct, 1 inverse. A **port and not a parameter**, because the terminal only learns it by decoding `TS` at run time. |
| `wt_etu` | 24 | Waiting time in etu. Zero disables the timer. |
| `tx_data`, `tx_valid`, `tx_ready` | 8, 1, 1 | Ready/valid. A transfer happens on the edge where both are high. `tx_ready` is low while a character is in flight **in either direction** and for the guard interval after one. |
| `tx_abort` | 1 | One cycle: the character was repeated `MAX_REPEAT` times and the far end was still signalling an error. The handshake completed and the character is gone. |
| `rx_data`, `rx_valid`, `rx_ready` | 8, 1, 1 | `rx_valid` is a **handshake**, high until `rx_ready` is high in the same cycle. Tie `rx_ready` to one for strobe behaviour. |
| `rx_parity_error` | 1 | The delivered character's parity was wrong. Held with `rx_valid`. |
| `rx_overrun` | 1 | One cycle: a character was assembled over one nobody had taken. The new one is kept. Structurally impossible with `rx_ready` tied high — which is why the handshake exists at all. |
| `rx_timeout` | 1 | One cycle: the waiting time expired. One per arming. |
| `io_i`, `io_oe`, `io_o` | 1, 1, 1 | The pad. `io_o` is a **constant**, whose level is `OUT_INVERT`; see §4. The polarity of all three is a parameter, because the pad is not always what drives the contact. |

### The counters

Each is `COUNT_WIDTH` bits and **saturates** rather than wrapping: an
instrument that wraps reports a small number for a large fault.

| Counter | Expected | What it means |
|---------|----------|---------------|
| `tx_char_count` | — | Characters whose transmit handshake completed, repeats and aborts included. Equal to the bytes a producer drove, which is what makes it checkable. |
| `rx_char_count` | — | Characters delivered, good parity or bad. A count of characters *on the wire*. |
| `parity_error_count` | **0** | Of those, the ones whose parity was wrong. |
| `repeat_count` | **0** | Retransmissions. A character sent twice is one `tx_char_count` and one of these. |
| `timeout_count` | **0** | Waiting-time expiries. Read §2's second DECIDED entry before reading a non-zero value as a fault. |

Three of the five have an expected reading of **exactly zero** against a
working card, which is worth far more than a counter whose expected
reading is "small".

## 4. Six things that are arithmetic rather than opinion

### `io_o` is a constant, and that is the contention argument

The pad interface is `io_oe` and `assign io_o = 1'b0;`. Nothing in this
block can drive the line high, and that is **structural** rather than a
promise: there is no path from anything to `io_o`.
`a_streams_ready_is_a_function_of_registers` walks the timing graph
backwards from it and from `io_oe` and reaches no input port, which is the
mechanised form of the same statement — and `io_oe` depending on `io_i`
would be a combinational loop the moment the pad is wired to the pin,
since `io_i` is the wire `io_oe` drives.

The testbench's model of the contact is a wired-and with a pull-up rather
than two tri-state drivers, for the same reason: "nothing drives this
high" should be a property of the model as well as of the block.

### `active`, the VCC hazard, and why a released line is safe

The board switches the card's **VCC** through an external high-side
switch so that the terminal can do a proper cold reset, and that makes
the ordering mandatory: VCC up, then CLK, then RST released, and in
reverse on the way down — RST low, CLK stopped low, **IO released**, then
VCC off. Driving a pin into an unpowered device forward-biases its ESD
diodes and can partially power it or damage it, so "IO released before
VCC drops" is a **hazard requirement**.

The sequencer that owns that ordering is a separate block. `active` is
what lets it own the line: low releases the wire, idles the transmitter,
drops `tx_ready`, abandons anything in flight with `tx_abort`, and holds
the receiver from starting a character. It is **registered**, so it takes
effect on the next clock — 9 ns at 112 MHz, against a VCC switch whose
slew is microseconds — and that is also what keeps `io_oe` and `tx_ready`
functions of registers, which `a_streams_ready_is_a_function_of_registers`
checks. An output enable fed combinationally from an input port is a
glitch path on a shared bus and a combinational loop once the pad is
wired, since `io_i` is the wire `io_oe` drives.

**Releasing the line is the safe state**, and that is why one bit is
enough: an open-drain bus idles high through its pull-up, so a released
line is indistinguishable from an idle one to the far end. There is no
bus turnaround to get wrong and no level being driven at a device that is
about to lose its supply.

**Coming back out of it must not invent a character**, and two separate
things are needed for that. The input synchroniser **keeps running**
while the block is inert, so it is never showing a reset value when
`active` rises. And `settle_sr` is **held cleared** while inert, so no
edge is taken for three clocks after `active` rises — by which time both
halves of the comparison were sampled while the contact was live.

Neither is sufficient alone, and §5's mutation table proves it with two
separate failures: a chain that stopped would present a reset value, and
a settle window that was only counted out once at reset would take an
edge that happened on an unpowered contact. The testbench drives both —
the contact held low *before* activation, and the contact falling in the
**same instant** `active` rises.

### Three polarity parameters, because the pad is not always the driver

`OE_INVERT`, `OUT_INVERT` and `IN_INVERT` are one exclusive or each and
they are independent, because the three paths are independent:

| Parameter | 1 means | For |
|-----------|---------|-----|
| `OE_INVERT` | `io_oe` is driven **low** to enable the driver | a pad or buffer with an active-low enable |
| `OUT_INVERT` | `io_o` is a constant **one** | an external transistor or inverting buffer that pulls the contact down when driven high |
| `IN_INVERT` | `io_i` arrives inverted | a receive path through an inverting buffer |

They are independent because an external pull-down FET changes the
output's sense while the input still taps the contact directly, so the
three combinations that are not "all or nothing" are the realistic ones.
One parameter beats a second block.

`io_o` stays a **constant** in every combination, which is what keeps the
contention argument above: the pad pulls the contact to one level or lets
go of it, and can never drive both.

MEASURED by §8c of the testbench, which instantiates **a second copy of
the block with all three set** on the same contact through an inverting
external stage, with the first copy held inert so the two never collide —
which is itself a use of `active` — and exchanges a character in each
direction through all three inversions. An untested parameter is a
liability, and these are the kind nobody notices is wrong until a board
is built.

### `DIV_MIN` is 8, and five is the floor

The error pulse and the sample point are measured by **different ends**,
each from its own detection of the same edge, and each is late by its own
synchroniser. A receiver's pulse starts at 10.5 etu by its own counting
and is therefore up to three clocks late; a transmitter samples at 11.5
etu by its own counting and reads the line through two more flops, so it
is effectively sampling 11.5 etu less two clocks. For a pulse of 1 etu to
cover that sample:

```
11.5*d - 2  >=  10.5*d + 3      →   d >= 5
```

This block drives a **2 etu** pulse, the long end of the permitted range,
which doubles the far end's margin. `DIV_MIN = 8` is the floor with half
an etu of margin at each end and a round number to state. 50 — 2 Mbaud at
112 MHz with an 8 MHz card clock — is seven times it.

Below `DIV_MIN`, `etu_div` means "use the `ETU_DIV` parameter", which is
exactly how `uart_rx` treats a `div` it cannot use: a rate that cannot be
expressed has to leave the port working rather than stop the block.
MEASURED by §8 of `rtl/iso7816_uart_tb.v`, which sets `etu_div` to 1 and
keeps talking.

### A repeat waits two etu longer than the guard

Measured from the original start bit, the far end's pulse can run to 12.5
etu and a minimum guard ends at 12.0. A repeat that began there would put
its **start bit inside the far end's own pulse**, where nothing could see
it, and the character would be lost in a way that looks like the card
having gone away. So `guard_limit` is two etu longer once an error has
been seen — the length of the longest permitted pulse — and the repeat's
leading edge lands at 14 etu at the earliest.

That is one of the twenty-seven mutations in §5, and the one most worth
having: it is a fault that only shows up in the error path, which is the
path nothing exercises in normal use.

### Both conventions are one exclusive or

The start bit is a *level* and not a logic value: the line is pulled low
for 1 etu in both conventions. So the two differ only over the nine bits
after it, and two facts fall out:

- electrically, the inverse frame is the direct frame's data bits
  **reversed and then complemented**, parity bit included;
- even parity over nine logic ones, complemented an odd number of times,
  is **odd** parity over the nine bits on the wire. So the check is
  `(^nine_bits) == convention` — one exclusive or and one comparison, not
  two separate checks.

A block that inverted the polarity and forgot the bit order, or the other
way round, passes a test whose bytes happen to be bit-symmetric or
self-complementary. §3 of the testbench captures the nine bits the
transmitter put on the wire and decodes them **all four ways** — as they
are, polarity only, order only, and both — and asserts that only the last
matches. For `0x3B` the four readings are `0x23`, `0xDC`, `0xC4` and
`0x3B`: four different bytes, so no two can be confused.

## 5. What the tests would and would not catch

`rtl/iso7816_uart_tb.v` is **HDL and not Rust**, which is a departure
from most of this library and is on purpose: a model of a timed
open-drain wire wants `#` delays in absolute simulation time rather than a
loop over clock edges. `ip/video/ssd1306_slave/rtl/ssd1306_slave_tb.v` is
written the same way for the same reason.
`iso7816_uart_talks_to_a_model_card` in `tests/ip_library.rs` runs it, so
CI executes it: a `_tb.v` nothing executes is a file and not a test. That
test also fails on **any** simulator message and on a testbench that
stops short of `$finish`, which is how its own watchdog reports a hang.

The model card lands every event it causes on a `$time` ending in 1 or 6,
so no pin it drives ever changes in the same instant as the clock edge
that samples it — a race an event-driven simulator may resolve either way
and a real circuit may lose. That is the discipline `ip/bus/spi_display_rx`'s
`FarSide` established here, and it is why every `etu_div` in the
testbench is even.

**The accounting is checked before the bytes.** All five counters are
compared against totals the testbench keeps itself, at **eighteen
checkpoints including before anything is driven at all**. That order is
not boilerplate: `ip/bus/spi_display_rx` shipped with two defects that
masked each other and left one counter permanently one too high, and
fifteen of its own testbenches missed it, because every one of them
started from the state in which the two faults agreed. Its README §3a is
the account, and the lesson taken here is that a counter is tested
against a total driven since reset, **including zero, and from every
start state**.

### MEASURED: twenty-seven mutations, twenty-seven caught

A test suite that has not been run against a broken implementation is a
test suite nobody has measured. Each line below is one textual change to
`rtl/iso7816_uart.v`, and the message is what the testbench said.

| Mutation | Caught by |
|----------|-----------|
| inverse convention inverts the polarity but not the bit order | the four-way decode: `the terminal sent dc, wanted 3b` |
| ...the bit order but not the polarity | the parity of the captured frame |
| the receive path ignores `convention` | `byte 6 is 23, wanted 3b` |
| the receive parity check ignores `convention` | `byte 6 has rx_parity_error 1, wanted 0` |
| the receiver never re-reads `etu_div` | the card-first character at a rate the terminal never transmitted at |
| the transmitter never re-reads `etu_div` | `no start bit` after the rate change |
| the error pulse is never driven on receive | `the terminal did not drive an error pulse` |
| the error pulse is never sampled on transmit | the watchdog: the card waits for a repeat that never comes |
| a repeat begins inside the far end's pulse | the repeat's parity, because its start bit was swallowed |
| one counter resets to one | `tx_char_count 1, drove 0`, before anything was driven |
| `repeat_count` never moves | `repeat_count 0, the card drove 1 error pulses` |
| the waiting time never expires | `0 timeout strobes after the waiting time` |
| the waiting time fires every etu rather than once per arming | `4 timeout strobes` |
| `guard_etu` is ignored and the guard is always two etu | `tx_ready at 15.5 etu with guard_etu 4` |
| the line is driven high for a one | `io_o is 1; nothing may drive this line high` |
| `rx_overrun` is raised for a character nobody missed | `14 overruns for two characters nobody took` |
| the start bit is watched as a level rather than an edge | a line held low for 60 etu makes a phantom character every eleven etu |
| the start edge is taken before the synchroniser has settled | a line held low **across reset** makes one character |
| the wire is not released on the clock `active` falls | `io_oe is 1 on the clock after active fell` |
| the state machine keeps running while inert | `0 aborts after a transmission was abandoned` |
| `tx_ready` stays high while inert | `tx_ready is high while the block is inert` |
| an abandoned transmission is not reported | the same `0 aborts` |
| the settle window is not reopened when `active` comes back | an edge from before `active` rose made a character |
| `OE_INVERT` is ignored | `after reset io_oe=0 io_o=0 line=0`, from §0 |
| `OUT_INVERT` is ignored | the continuous assertion that `inv_o` is a constant one |
| `IN_INVERT` is ignored | `the inverted instance read ... da, wanted 4b` |
| `etu_div` below `DIV_MIN` is used as given | `no start bit`: an etu of one clock is unsamplable |

Two of those are worth their own note, because the first version of each
was vacuous and the measurement is what said so:

- **the release on `active` falling** was first checked four clocks
  later, and the mutation passed. Forcing `state` back to idle releases
  the wire a clock later anyway, so only the **first** clock tells the
  gate apart from the state machine's own recovery — and the hazard is
  about the first clock. The committed test also asserts that the
  transmitter really is driving at the instant it deasserts `active`, so
  that there is something to release.
- **the settle window being reopened** was first checked with the
  contact held low for two etu *before* activation, and that mutation
  passed too: with the chain still running, both halves of the
  comparison are already the low level and there is no edge to take.
  What catches it is the contact falling in the **same instant**
  `active` rises, so the edge is still inside the window when the block
  wakes.

The two before them are the ones this block owes to
`ip/bus/spi_display_rx/README.md` §3a, which is a fault **found on a
part**: a synchroniser's reset value compared against as if it were an
observation of a pin. This block has one edge detector, and it had the
same fault. For the first two clocks after reset `io_sync_q` and
`io_prev_q` are still showing what reset put there, so a contact held low
— by a dead card, or by the other end of the wire — arrives as a start
bit and decodes as a character of nine zeros, **whose parity is
legitimately even**, so the only trace of it is a counter. A three-bit
`settle_sr` counts the blind window out and gates the detector. §0 of the
testbench holds the line low across reset at the level opposite to the
synchroniser's own reset value, which is the same experiment as flipping
that reset value — and §8b runs it again across an `active` cycle, which
is the same fault on a block that comes back to life rather than one that
comes out of reset.

Two readings of the first version of that test are worth keeping,
because the first one was wrong:

- holding the line low for **eight clocks** after reset and then
  releasing it: the fault passes. The manufactured start bit is rejected
  as a glitch at the middle of the start bit, half an etu later, because
  the line has come back by then. Releasing the wire too early is what
  makes the test vacuous.
- holding it low for **twelve etu**: the fault produces one character and
  the test fails. That is the committed version.

### What the tests would not catch

- **Anything about a real card.** The model is written from the same
  quotations as the block. If §2's reading of the standard is wrong, the
  model is wrong in the same direction and both agree.
- **Anything about a part, a pad or IO timing.** See §1.
- **Metastability.** The simulator has no metastable resolution, so the
  two-flop synchroniser is tested for latency and not for what it is for.
- **Clock error against a real card.** Every test runs the terminal and
  the card off the same notion of an etu, so the half-bit-per-frame
  budget a receiver really has is never spent. A card whose `F/D` and
  clock give an etu a few per cent from the terminal's is not simulated
  at all, and the sampling is one sample at the nominal centre with no
  majority vote and no mid-character resynchronisation — so the budget is
  the usual half a bit over a frame and nothing here measures it.
- **A glitch.** There is no deglitching beyond the synchroniser and the
  check at the middle of the start bit, so a low lasting more than half
  an etu is a character as far as this block is concerned.
- **The transmitter's own start-bit edge being *detected*.** For the
  terminal-to-card direction the model card anchors on the handshake
  rather than reacting to the edge, because one sequential process
  models both ends. It does assert that the line is low immediately
  after the accept, and it samples the nine bits at their nominal
  centres, so a wrong bit period or a wrong frame length is caught — but
  the reactive path is only exercised in the card-to-terminal direction
  and on a repeat, where `card_watch_react` waits for the edge rather
  than for a time this file chose.
- **What the bytes mean.** There is no ATR decode, no PPS and no T=0 or
  T=1 state machine here, so a session that is bit-perfect and asks the
  card for the wrong thing passes everything on this page.
- **The activation ordering itself.** §4's hazard is about VCC, CLK and
  RST, and none of the three is in this block. What is tested is that
  `active` releases the wire within a clock and that coming back cannot
  invent a character; whether the sequencer deasserts `active` *before*
  it drops VCC is that block's test to write.
- **A pad that cannot be released.** The three polarity parameters are
  tested against a model of an external inverting stage, which is
  arithmetic in a testbench and not a transistor. Nothing here says a
  real pad's output enable has the timing or the leakage an open-drain
  contact needs.

## 6. Reusing `ip/bus/uart`, and why it was not reused

`uart_frame_tx` and `uart_frame_rx` already do eight data bits, even
parity computed over the character, two stop bits and a run-time divisor
on a port. The reuse path exists and is worth writing down, because it is
not obviously wrong:

- 8E2 direct is `cfg_data_bits = 8`, `cfg_parity = 2` (even),
  `cfg_stop = 2`;
- 8E2 **inverse** is the same transmitter fed `~reverse(byte)` with
  `cfg_parity = 1` (odd), by the arithmetic in §4 — the complement of
  nine bits turns even into odd;
- open drain is `io_oe = ~tx` with a constant-zero `io_o`;
- `uart_frame_rx` already has the receive handshake and the `rx_overrun`
  that goes with it.

What does not fit is everything that makes this a *contact* rather than
two wires:

1. **The receiver has to drive.** The T=0 error pulse is a receiver
   pulling the line low 10.5 etu after the start bit — *inside* the frame
   it has just received, where a UART receiver has no output at all and
   no timebase left to measure from, since `uart_frame_rx` returns to
   idle at the middle of the first stop bit.
2. **The transmitter has to sample.** At 11.5 etu, half an etu past the
   end of a two-stop frame, which is after `uart_frame_tx` has finished
   and dropped its bit timer.
3. **Both are on one wire**, so the two timebases have to be the same
   one. Two blocks with independent `div` latches cannot agree about
   where 10.5 and 11.5 etu are, and that agreement is the whole of §4's
   arithmetic.
4. **The guard time is up to 256 etu** and the obligation is on the
   interval between characters in **either** direction, which no stop-bit
   setting expresses — `uart_frame_tx` offers one or two.
5. **A repeat has to re-send a character the producer has let go of**,
   which means holding the frame, not the byte.

So a dedicated implementation: one state machine, one etu timebase, eight
states, and the conventions as one exclusive or. It keeps `ip/bus/uart`'s
rules rather than its code — `div` latched once per character and the
limit compared for equality rather than magnitude, which is the
measurement in `uart_tx`'s header; `DIV_MIN` meaning "use the parameter";
two separately named synchroniser flops; and `rx_valid` a handshake so
that `rx_overrun` means something. One thing it does *not* copy:
`uart_frame_tx` computes even parity as `^dat`, an exclusive or, not by
counting ones — the two are the same function and this block uses the
exclusive or too.

## 7. Area, and what it costs to instrument

MEASURED by `footprints_match_the_documentation` and tabulated in
[`docs/ip-library.md`](../../../docs/ip-library.md). On an ECP5 45F at
the defaults: **85 `CCU2C` and 486 `LUT4` with 271 flip-flops**, LUT
depth 8. On an iCE40 HX1K, 140 `SB_CARRY` and 576 `SB_LUT4` at the same
depth.

Three readings of that table:

- **`PARITY_RETRY = 0` saves four `LUT4` and nothing else** — 482 against
  486, with the same 85 carry cells and the same 271 flip-flops. The
  error pulse, the 11.5 etu sample, the repeat and `tx_abort` together
  are four lookup tables, because almost everything they need (the etu
  timebase, the guard counter, the frame register, the two extra states)
  is there for the normal path anyway. The parameter is for a link that
  does not want the behaviour, not for one that cannot afford it.
- **Eighty of the 271 flip-flops are the five counters** at
  `COUNT_WIDTH = 16`, and a large part of the 85 carry cells is their
  adders. `COUNT_WIDTH = 8` roughly halves that and `COUNT_WIDTH = 1`
  turns every counter into a sticky flag. A bring-up build wants all five
  at 16 bits; a deployed one may want none of them.
- **The three polarity parameters cost nothing**, because each is one
  exclusive or against a constant and folds away at elaboration. They do
  not appear in the table at all, which is the point of them being
  parameters rather than ports.

The rest is the run-time inputs: a 16-bit comparison and two 16-bit
registers for the rate, a 24-bit waiting-time counter with its latched
limit, a 9-bit guard counter, and the 10-bit frame held twice so a repeat
needs no second read of `tx_data`. A design with a fixed rate would tie
`etu_div` to zero and pay for `ETU_DIV` only — but it cannot, because the
whole point of this block is that a PPS exchange changes the rate after
the design has been built.

LUT depth 8 is the mapped combinational depth, which is the shape of a
critical path and not a slack figure. There is no vendor timing model
here, and at 112 MHz the period is 8.93 ns — so whether eight levels
close is a question for a real place and route on a real part, which §1
says has not happened.

## 8. Using it

```verilog
iso7816_uart #(
    .ETU_DIV      (5208),   // 21505 baud: 112 MHz, an 8 MHz card clock, F/D 372
    .PARITY_RETRY (1),
    .MAX_REPEAT   (3),
    .COUNT_WIDTH  (16),
    // The board's polarities. All three default to zero, which is a pad
    // driving the contact directly with an active-high enable.
    .OE_INVERT    (0),
    .OUT_INVERT   (0),
    .IN_INVERT    (0)
) u_card (
    .clk (clk), .rst_n (rst_n),
    // From the activation sequencer: low releases the wire and holds the
    // block inert. Deassert it **before** VCC drops; see §4.
    .active (card_active),
    // From the ATR and the PPS, which a layer above decodes.
    .etu_div (etu_div), .guard_etu (guard_etu),
    .convention (convention), .wt_etu (wt_etu),
    .tx_data (tx_data), .tx_valid (tx_valid), .tx_ready (tx_ready),
    .tx_abort (tx_abort),
    .rx_data (rx_data), .rx_ready (1'b1), .rx_valid (rx_valid),
    .rx_parity_error (rx_parity_error), .rx_overrun (),
    .rx_timeout (rx_timeout),
    // The pad. `io_o` is a constant zero; drive the pin from `io_oe`.
    .io_i (io_i), .io_oe (io_oe), .io_o (io_o),
    .tx_char_count (tx_chars), .rx_char_count (rx_chars),
    .parity_error_count (par_errs), .repeat_count (repeats),
    .timeout_count (timeouts)
);
```

**The pin needs a pull-up and a pad that can be let go of.** On a part
whose IO buffer takes an output enable, `io_oe` is that enable and `io_o`
is tied low; with a bidirectional pin declared in the constraints,
`io_i` reads the wire back. Nothing on this page has been through a pad,
so that paragraph is a DECIDED reading of how the block should be wired
and not a CHECKED one.

**Bringing a session up**, which is the procedure for whoever has the
card:

1. The activation block drives VCC, the card clock and RST, in that
   order, and raises `active` once the contact is live. That block is not
   here. On the way down it lowers `active` **first**, waits a clock, and
   only then stops the clock and drops VCC.
2. Set `etu_div` to the default rate — `(F/D) × divisor`, which is
   `372 × 14 = 5208` for an 8 MHz card clock off 112 MHz; §3's table has
   every rate — `guard_etu` to 0, `convention` to 0, and `wt_etu` to the
   default waiting time in etu.
3. Read the first character. **`TS` decides the convention**: `0x3B`
   direct, `0x3F` inverse. If it arrives as neither, try the other
   `convention` — a terminal cannot know which until it has a byte, and
   an inverse `0x3F` read as direct is `0x03`.
4. Decode the rest of the ATR for `TC1`, drive it onto `guard_etu`, and
   for `F/D` if a PPS is wanted.
5. After a successful PPS, change `etu_div` **while the line is idle**.
   The block reads it at the next character; a character in flight keeps
   the rate it started at. For 2 Mbaud off the same 8 MHz card clock that
   is `F/D = 4`, so `etu_div = 56`.
6. Read `parity_error_count` and `timeout_count`. Both should be zero.
   `repeat_count` non-zero with `parity_error_count` zero means the
   *card* is complaining about what the terminal sent, which is a rate or
   a convention problem and not a noise problem.
