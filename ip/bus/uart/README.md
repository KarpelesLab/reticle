# A UART whose framing a host can choose, and four errors instead of one

`uart` was 8N1 with one `rx_error`. It is now 5 to 8 data bits, five parity
modes, one or two stop bits, and four separate receive errors — and the
configuration arrives on **ports**, because the thing that chooses it is a
USB host sending SET_LINE_CODING at run time and not a parameter somebody
typed.

That last sentence is the whole design problem. `ip/usb/usb_cdc_acm` has
been decoding `dwDTERate`, `bCharFormat`, `bParityType` and `bDataBits` for
some time, and `testdata/fpga/cynthion/usb_cdc_uart.v` has been wiring all
four up and honouring exactly one of them. `stty -F /dev/ttyACM1 9600 parenb
cs7` was stored, reported back correctly by GET_LINE_CODING, and **ignored
on the wire**. Closing that is what this round is.

- §1 says what each kind of claim here rests on.
- §2 is **eight modules in two layers**, which is a shape Verilog forced and
  area then justified.
- §3 is the character format and the CDC mapping, in full, including every
  value that is not implemented and what it does instead.
- §4 is **area, measured**: what 8N1 costs now, what full configurability
  costs, and the one decision that was made by measuring rather than
  arguing.
- §5 is the four receive errors — how each is raised, and how the receiver
  recovers from each.
- §6 is the baud divisor, which did not change.
- §7 is **what each test would and would not catch**, including three
  mutations that were run to find out.
- §8 is what this block still does not do.
- §9 is what has and has not been measured on hardware.

---

## 1. Confidence, and what it is based on

Three kinds of claim appear below and they are not worth the same.

- **Measured.** Every area and depth figure is from
  `docs/ip-library.md`'s footprint table, which `tests/ip_library.rs`
  regenerates by running the real flow for LUT4, LUT6, an iCE40 HX1K and an
  ECP5 45F. Every behavioural claim is from a simulation in that same file,
  and §7 says which test is which.
- **Quoted.** The CDC encodings are read off the PSTN subclass
  specification (USB Communications Class Subclass Specification for PSTN
  Devices, revision 1.2, §6.3.11, Table 17) and are a reading until
  something measured them. The 16550's overrun and break behaviour is
  likewise quoted from its datasheet's description and used as a precedent,
  not as a requirement.
- **Argued.** Why 1.5 stop bits are sent as 2, why the receiver has no
  stop-bit setting, why nine data bits are not offered: these are
  judgements, each with its reasoning written out so a later reader can
  disagree with the reasoning rather than guess at it.

Nothing here has been measured against a second vendor's UART. §9 says what
that means.

---

## 2. Eight modules in two layers

```
uart              8N1 by default; uart_tx + uart_rx
  uart_tx         one half, framing from parameters
  uart_rx         the other half, framing from parameters

uart_frame        the configurable pair; uart_frame_tx + uart_frame_rx
  uart_frame_tx   the transmitter. Framing on ports.
  uart_frame_rx   the receiver. Framing on ports, four errors, a handshake.

uart_baud_div     a bit rate -> clocks per bit, by long division
uart_line_coding  a host's three bytes -> the three format ports
```

**There is one transmitter and one receiver.** `uart_tx` instantiates
`uart_frame_tx` with its format ports tied to constants built from its
parameters, and `uart_rx` does the same with `uart_frame_rx`. Nothing is
implemented twice, so nothing can drift.

### Why a parameter and a port, rather than one or the other

A **port**, because the host's line coding arrives at run time. SET_LINE_CODING
is a control transfer; the data bits and the parity it names are known only
once a program has run `stty`. A format compiled into a parameter cannot
follow that, and a serial port that cannot follow it is the defect this
round exists to fix.

A **parameter as well**, because Verilog-2005 has no default for a port.
An input left unconnected at an instantiation reads as `x` — the
elaborator says so, with `V0004`, `an unconnected input reads as x` — so
adding three input ports to `uart`, `uart_tx` and `uart_rx` would have
required every existing instantiation to connect them. There are five:
`examples/soc`, `examples/mos6502_computer`, `examples/mos6502_monitor`,
`examples/apple2`, and the Cynthion serial designs under
`testdata/fpga/cynthion/`. **None of them changed a line**, and that was a
requirement rather than a nicety: a library block whose users have to be
edited to keep working has not been extended, it has been replaced.

So the split is not a preference. It is the only arrangement in which
`uart #(.CLK_DIV(104)) u_uart (...)` still means 8N1 and
`uart_frame` can take a format off a wire.

### What is a parameter, what is a port, module by module

| Module | `CLK_DIV` | `div` | data bits | parity | stop bits | `rx_ready` |
|---|---|---|---|---|---|---|
| `uart` | parameter | port | parameter | parameter | parameter | — |
| `uart_tx` | parameter | port | parameter | parameter | parameter | — |
| `uart_rx` | parameter | port | parameter | parameter | n/a | — |
| `uart_frame` | parameter | port | **port** | **port** | **port** | **port** |
| `uart_frame_tx` | parameter | port | **port** | **port** | **port** | — |
| `uart_frame_rx` | parameter | port | **port** | **port** | n/a | **port** |

`CLK_DIV` stays a parameter and `div` stays a port for the reasons
`uart_tx.v`'s header already gives: `div` of zero means "use `CLK_DIV`", so
a fixed-rate design ties it low and nothing changes.

**Outputs were free to add.** An unconnected *output* is ordinary, so
`uart` and `uart_rx` gained `rx_frame_error`, `rx_parity_error` and
`rx_break` without touching a single user. `rx_error` is now the
disjunction of those three, which is what it always was plus the break it
used to deliver as a framing error on a zero byte.

### Why `rx_overrun` is not on `uart_rx`

Because that module cannot know. `rx_valid` there is a one-cycle strobe
with no `rx_ready`: a character nobody takes in that cycle is lost, and
nothing in the block can tell that from a character nobody wanted.
`uart_frame_rx` has the handshake, so it can, and it has the wire.
`uart_rx` says so in its header instead of offering a signal that is always
zero.

---

## 3. The character format, and the CDC mapping

### Data bits

5, 6, 7 and 8 are implemented. `cfg_data_bits` is a count, and **anything
else is 8**.

The transmitter masks the byte to the width and places the parity bit after
the last data bit, as a four-way choice made once per character. The
receiver shifts the line into the top of an eight-bit register and
right-aligns the result by 8 − N at the end — two multiplexer stages, which
disappear entirely when the width is a constant eight.

**Nine data bits are not implemented**, and 9 is not a CDC value either.
Nine-bit framing is a multidrop convention in which the ninth bit marks an
address, and it is normally done on real hardware by abusing the parity bit
— which this block will do on the wire, since `cfg_parity` has mark and
space. What it will not do is treat such a bit as an address: there is no
address-match register here and no "received an address" signal. Doing it
properly would also widen `tx_data` and `rx_data` to nine bits and with
them every FIFO and bridge in front of them, for a format no terminal
program offers.

**Sixteen data bits** are in CDC's table and are not implemented, for the
same datapath reason. `uart_line_coding` catches a host asking for them.

### Parity

All five of `bParityType` are implemented: none, odd, even, mark and space.

A parity bit is one bit, and choosing between `^data`, `~^data`, `1` and
`0` is a four-way multiplexer on it, so mark and space came almost free
once odd and even existed. The receiver computes the same four and compares.
`cfg_parity` values 5 to 7 are not defined by the specification and are
treated as **none**, which is the block's default; `uart_line_coding` never
produces one.

Even parity leaves the number of ones in the whole character even; odd
leaves it odd. The model in `tests/ip_library.rs` computes that by
**counting ones**, not by an exclusive or, so that the test and the
hardware cannot be wrong in the same direction.

### Stop bits

One and two are implemented. `cfg_stop` follows `bCharFormat` — 0 is one
stop bit, 2 is two — and **1, one and a half, sends two**.

That is a decision, and the case for it is:

- a receiver samples the **first** stop bit and nothing after it, so 1.5
  and 2 are indistinguishable to anything listening;
- a far end expecting 1.5 bit periods of mark gets 2, which is more idle
  than it asked for and never less, so no frame is lost;
- the cost of doing it properly is a second limit for the bit timer — a
  sixteen-bit multiplexer choosing `div_last` or half of it — plus a state
  to spend it in, in a block whose whole per-bit loop is one comparison;
- the only machines that ever asked for 1.5 are five-bit Baudot
  teleprinters, which is also why `bDataBits` 5 exists at all;
- and the substitution is **reported**: `uart_line_coding` drops `ok` for
  it.

So: half a bit period of throughput per character, nothing on the wire that
a receiver can object to, and a wire that says it was not exact.

**The receiver has no stop-bit setting at all**, and that is the same
observation from the other side. A frame is over once the line has been
mark at the first stop bit's sample; the second stop bit of a two-stop
format is indistinguishable from idle. The 16550 does the same. So
`uart_frame_rx` takes two format ports and not three, and the state machine
returns to idle at the *middle* of the first stop bit, which leaves half a
bit period of mark before the earliest legal next start edge whatever the
transmitter was set to.

The consequence for testing is sharp and §7 has it: **a loopback cannot see
a wrong stop-bit count.**

### The mapping, in full

`uart_line_coding` is combinational — the host's three bytes are already
registers inside `usb_cdc_acm`, and a decode of somebody else's register
does not need one of its own — and this is all of it.

`bDataBits`:

| asked | sent and expected | `ok` |
|---|---|---|
| 5 | 5 data bits | 1 |
| 6 | 6 | 1 |
| 7 | 7 | 1 |
| 8 | 8 | 1 |
| 16 | 8 | 0 |
| anything else (including 9) | 8 | 0 |

`bParityType`:

| asked | sent and expected | `ok` |
|---|---|---|
| 0 | none | 1 |
| 1 | odd | 1 |
| 2 | even | 1 |
| 3 | mark | 1 |
| 4 | space | 1 |
| anything else | none | 0 |

`bCharFormat`:

| asked | sent | `ok` |
|---|---|---|
| 0 | 1 stop bit | 1 |
| 1 (1.5 stop bits) | 2 stop bits | 0 |
| 2 | 2 stop bits | 1 |
| anything else | 1 stop bit | 0 |

`ok` is low for anything inexact in any of the three fields at once. It
does not say which field and it does not distinguish the benign
substitution (1.5 became 2) from the lossy ones (16 became 8, an undefined
parity became none). There are only three cases and they are all in these
tables; a design that needs the distinction compares its own `char_format`
against 1.

**Nothing is reported back to the host, and that is not an omission.** CDC
gives a device no way to refuse a line coding: SET_LINE_CODING either
succeeds or stalls, and `usb_cdc_acm` accepts it and reports it back
verbatim in GET_LINE_CODING, which is what the specification asks for.
Stalling because a format is not implementable would make `stty` fail
rather than make the wire right. So `ok` is for an LED, a status register
or a test — `testdata/fpga/cynthion/usb_cdc_uart.v` puts it on LED 0,
with the baud divider's `ok`.

**Nothing hangs for any value.** Every out-of-range value has a defined
substitution above, and the receiver's state machine returns to idle from
every state including the two of its three bits that nothing reaches.

---

## 4. Area, measured

All from `docs/ip-library.md`'s footprint table, at `CLK_DIV=104`.

| | LUT4 | LUT4 depth | ECP5 45F LUT4 | ECP5 45F FF |
|---|---|---|---|---|
| `uart`, 8N1, before this round | 213 | 8 | 215 | 120 |
| `uart`, 8N1, now | 237 | 8 | 239 | 126 |
| `uart_frame`, fully configurable | 303 | 8 | 305 | 131 |

**8N1 costs 24 more LUT4 — about 11% — and no extra logic depth.** The
depth matters more than the count on this backend, and it did not move: the
per-bit loop is still one sixteen-bit equality against a latched register,
and everything the format decides is decided once per character, outside
the loop.

Where the 24 go, with the format constant and almost everything folding
away: a twelve-bit frame shifter instead of ten (the longest frame is one
start, eight data, one parity and two stop); a four-bit frame-length
register and its equality, which must be latched (below); the eight-input
zero test that break detection needs; and a three-bit state register with
six states where there were four in two bits.

**Full configurability costs 66 LUT4 on top of that**, still at depth 8:
the two multiplexer stages that right-align 5, 6 or 7 data bits, the
four-way parity selectors at both ends, the four-way frame word, and the
range checks.

### The one decision that was measured rather than argued

An earlier version latched the receive format at the start edge, the way
`div` is latched, and `uart` at 8N1 came out at **275 LUT4** instead of
237. The 38 lookup tables were `cfg_data_bits`'s aligner and
`cfg_parity`'s selector *failing to fold*, because a constant does not
propagate through a flip-flop in this synthesiser: the values were
constant, but they had been through a register first. The configurable pair
barely noticed — 311 against 303 — so the whole 38 were being spent on
behalf of designs that had not asked for a configurable format at all.

So the receiver reads its format **continuously** and the transmitter
latches its whole frame, and the asymmetry has a reason beyond the
measurement. A transmitter *owns* the character it started: it must finish
the frame with the length it began with, or the line is left mid-character
with the wrong number of stop bits. A receiver owns nothing — a host that
changed the line coding changed its own transmitter too, so the character
crossing that change was already lost at the sender, whatever this end
latched. And the receiver recovers from every mid-character change, because
its state machine returns to idle from every state.

`div` is still latched per character at both ends, and that is the
expensive one to get wrong: `uart_tx.v`'s header has the measurement —
comparing the bit counter against a run-time value *every bit* cost
**eleven extra levels** of logic.

### `uart_line_coding`'s own footprint

**18 LUT4 at depth 3**, no flip-flops, no clock. It is the only block in
the library with no clock at all.

---

## 5. The four receive errors

One wire cannot report four events, so `uart_frame_rx` has four. Framing
and parity belong to a character; break and overrun belong to neither.

| Signal | Shape | Raised when | Recovery |
|---|---|---|---|
| `rx_frame_error` | held with `rx_valid` | the stop bit sampled low | automatic; idle at the middle of the stop bit |
| `rx_parity_error` | held with `rx_valid` | the parity bit disagreed with the data | automatic, as above |
| `rx_break` | one cycle | the whole frame was low, stop bit included | waits in a state of its own for mark |
| `rx_overrun` | one cycle | a character arrived while the last was unread | automatic; the newer character is the one offered |

### Framing

The stop bit was not mark. The byte the broken frame produced is
**delivered anyway**, with `rx_valid` — a wrong byte at a consumer says
more than silence does, and that is what `uart_rx` has always done. The
flag is a property of the character, so it is held alongside `rx_valid` and
cleared when the character is taken.

### Parity

The parity bit was not what the data and `cfg_parity` say. Also a property
of the character, also held, and the byte is delivered for the same reason.
Never raised when the format has no parity bit, because there is nothing to
disagree with.

### Break

A break is the line held low for longer than a frame. It is detected as
**every bit of the frame low** — each data bit, the parity bit if there is
one, and the stop bit — which is the 16550's rule (a framing error on an
all-zero character).

It raises `rx_break` for one cycle, produces **no character**, and raises
no framing error. A bridge that echoed a break as a `0x00` would be
reporting a byte nobody sent.

It fires **once** per break, however long the break is. The receiver then
sits in a state whose only job is to wait for the line to return to mark,
and only then looks for a start edge again. Without that state a held-low
line produces a break every frame period for ever, which is exactly what
the test catches when the state is removed: three breaks instead of one.

**Where the boundary is not pinned**, said plainly: a line low for *about*
one frame. A receiver with no second timer cannot distinguish "low for
exactly one frame" from "a `0x00` whose stop bit failed", and this block
chooses to call it a break. That is a definition, not a measurement, and no
test asserts anything either side of it.

### Overrun

A character was assembled while the previous one had not been taken.
`rx_valid` is a **handshake** in this module — it stays high until a cycle
in which `rx_ready` is high too — and that is what makes an overrun
detectable at all. The **newer** character is kept and the older one is
lost, which is what a 16550 does and what its datasheet means by the
previous character being destroyed; the alternative keeps a stale byte and
loses the fresher evidence of whatever is going wrong.

With `rx_ready` tied to one — which is `uart_rx`, and every existing user —
the handshake degenerates to exactly the one-cycle strobe it always was,
and `rx_overrun` can never fire, because nothing is ever not taken.

### Nothing detected leaves the receiver stuck

That is the property worth stating on its own, because an error that is
reported and then wedges the block is worse than one that is not reported.
The state machine has six states in three bits; five of them advance on a
bit-period count and the sixth waits on a level; the two encodings nothing
reaches go to idle. There is no input sequence that keeps it out of idle
for more than one frame plus the time the line is held low.

---

## 6. The baud divisor, which did not change

`uart_baud_div` turns a bit rate into clocks per bit by restoring long
division — one quotient bit a clock, 32 clocks for an answer — and says
whether the answer is inside the error budget. Its own header has the
worked table, the 2% budget and why `DIV_MIN` is 30 rather than 4. Nothing
in this round touched it.

What is worth repeating here is the shape it shares with
`uart_line_coding`: both turn a host's number into something a UART can
use, both have an `ok` that means "exactly what was asked for", and
neither tells the host anything. A design wires them side by side; both
Cynthion serial designs and `examples/mos6502_monitor` do.

---

## 7. What each test would and would not catch

All in `tests/ip_library.rs`.

### The sweeps

- **`uart_frame_tx_sends_the_frame_a_hand_decoder_reads`** — all forty
  formats (four widths x five parities x two stop-bit counts), two bytes
  each, with the line **decoded in Rust by hand**. For each frame: the data
  bits masked to the width, the parity bit against counted ones, every
  sample of the stop region against being mark, and the distance to the
  next start edge against the frame length the format implies.
  *Would not catch:* anything about the receiver; a wrong bit period, which
  is `uart_takes_its_divisor_from_a_port`; any error condition. It also
  cannot catch a transmitter correct at eight clocks a bit and wrong at
  6250 — the frame is counted in bit periods, and the bit period is one
  latched comparison that `a_hosts_rate_becomes_a_bit_period` measures
  separately.
- **`uart_frame_rx_reads_the_frame_a_hand_encoder_writes`** — the same
  forty formats driven onto `rx` from levels an **encoder written here from
  the specification** produces, four bytes each including `0x00` and
  `0xFF`. *Would not catch:* an error condition; anything about the
  transmitter.
- **`uart_frame_halves_agree_at_every_character_format`** — the loopback,
  all forty formats. *Would not catch:* a shared misunderstanding, by
  construction. §7's mutation list says how literally that is true.

### Why a hand decoder and not `uart_rx`

Because two halves of one misunderstanding agree with each other, and in
this block that is not a worry but a demonstrated fact. Three mutations
were run against the finished RTL:

| Mutation | Hand decoder | Hand encoder | Loopback |
|---|---|---|---|
| odd and even parity swapped in the transmitter | **caught** | passed | **caught** |
| `two_stop` forced low, so every format sends one stop bit | **caught** | passed | **passed** |
| break-recovery state removed | n/a | n/a | n/a (its own test caught it) |

**The loopback passes a transmitter that forgot the second stop bit**, and
it passes it for a principled reason rather than by luck: the receiver
samples the first stop bit and nothing after it, so there is no receiver
anywhere that could catch this. Only a decoder that counts bit periods can,
and only because it is told the format. That row is the justification for
the whole hand-written-decoder approach, and
`examples/mos6502_computer/tb/computer_tb.v` and
`examples/soc/tb/soc_tb.v` decode their lines by hand for the same reason
in Verilog.

### The errors, each deliberately

- **`uart_frame_rx_reports_a_framing_error_and_takes_the_next_character`** —
  stop bit held low on a non-zero byte, then a good character.
  *Would not catch:* a receiver that flagged everything, which the forty
  clean formats rule out.
- **`uart_frame_rx_reports_a_parity_error_and_takes_the_next_character`** —
  a **data** bit inverted under odd and even parity, so the receiver has to
  compute the parity of what it received rather than compare the parity bit
  against itself; the parity bit inverted under mark and space, where no
  data change could disagree with a constant. Then a good character.
- **`uart_frame_rx_reports_a_break_once_and_waits_for_the_line`** — the
  line held low for three frame periods. Exactly one `rx_break`, no
  character, and a good character after the line is let go. Catches both a
  receiver that reports a break per frame and one that never comes back.
  *Would not catch:* the boundary case in §5.
- **`uart_frame_rx_reports_an_overrun_and_keeps_the_newer_character`** —
  `rx_ready` held low across two characters. One `rx_overrun`, the newer
  byte is the one offered, the held byte is taken when `rx_ready` returns,
  and a third character arrives clean.
  *Would not catch:* an overrun in `uart` or `uart_rx`, which cannot
  happen; a character delivered in the same cycle the previous one is
  taken, which the RTL orders explicitly and nothing exercises.

### The mapping and the loop

- **`uart_line_coding_maps_the_formats_a_host_asks_for`** — every value the
  specification defines in each field, plus an undefined one in each,
  against §3's tables.
  *Would not catch:* whether `uart_frame` then uses the three numbers.
- **`a_hosts_line_coding_becomes_the_frame_on_the_line`** — **the loop,
  closed by a variable.** The host's three bytes go into
  `uart_line_coding`, its three answers come out as numbers, those numbers
  are driven onto `uart_frame`'s ports, and the frame on `tx` is decoded by
  hand against what the host asked for. Nothing is spelled twice: the
  format the decoder expects is derived from what the hardware produced.
  `a_hosts_rate_becomes_a_bit_period` is the same shape for the rate, and
  the two together are the whole claim that a host's line coding changes a
  waveform.
  *Would not catch:* the wiring in a design. That
  `testdata/fpga/cynthion/usb_cdc_uart.v` connects `usb_cdc_acm`'s three
  fields to this decoder and its outputs to the UART is structural, and
  nothing short of a host setting a format on the part measures it.

### Compatibility

- **`uart_rx_error_is_the_disjunction_of_the_three_it_now_has`** — a frame
  with a low stop bit through the 8N1 `uart`, asserting that `rx_error` is
  still the wire it was and that `rx_frame_error` is the one of the three
  that is set.
- **`uart_reports_a_framing_error_when_the_stop_bit_is_missing`**,
  **`uart_receives_the_byte_its_own_transmitter_sends`**,
  **`uart_takes_its_divisor_from_a_port`**,
  **`uart_falls_back_to_its_parameter_for_a_divisor_it_cannot_use`**,
  **`uart_receives_a_byte_at_a_divisor_from_its_port`** — all five predate
  this round and **none was edited**. That they still pass is the sharpest
  statement that 8N1 means what it meant.
- **`packages_resolve_and_elaborate`** and the five example test suites
  elaborate designs nobody touched, which is the structural half of the
  same claim.

### One test-harness defect worth recording

The hand decoder's first draft looked for the next **falling edge** to find
the next frame, and decoded half of one character and half of the next: a
one-to-zero transition between two data bits is a falling edge too. No rule
about how much mark must precede an edge fixes it, because a data bit one
period long is preceded by exactly as much mark as a single stop bit. What
fixes it is the one thing asynchronous framing actually requires a receiver
to know — the frame is as many bit periods long as the format says — so the
decoder resumes at the end of the frame it has already read. Writing that
down is the point: it is the same fact that makes a receiver ignore the
stop-bit count.

---

## 8. What this block still does not do

- **No register interface.** This is the raw ready/valid form. A UART
  behind AXI4-Lite would be `uart_frame`, two `fifo_sync` instances and a
  register file; every piece is in this library.
- **No FIFO.** `uart_frame_rx` holds one character, which is why it can
  report an overrun; `ip/memory/fifo_sync` is the block to put behind it.
- **No flow control.** No `rts`, no `cts`, no XON/XOFF.
- **No break generation.** A design that wants to send one holds `tx` low
  itself; there is no port for it. Receiving a break is implemented; §5.
- **No 9-bit framing and no address matching.** §3.
- **No 1.5 stop bits.** §3, with the argument.
- **No auto-baud, no loopback mode, no interrupt output.**
- **No oversampling and no majority vote.** One sample per bit at the
  nominal centre, so the clock error budget is the usual half a bit over a
  frame — and a longer frame is the tighter case, because the last sample
  is further from the start edge. Eight data bits with parity and two stop
  bits is the worst of the forty.
- **No fractional divisor.** `uart_baud_div`'s header says what that costs
  at high rates.
- **The package version is still 1.0.0.** It should be 1.1.0: this round is
  new, backwards-compatible modules and ports. It was left alone because
  `tests/mos6502_computer.rs` asserts the exact line
  `package uart 1.0.0 library ../../ip/bus/uart` in a generated lock file,
  and that file was outside this round's scope. Every `depends uart ^1.0.0`
  in the tree accepts either, so nothing resolves differently — but the
  bump is owed.

---

## 9. What has and has not been measured on hardware

**Nothing in this round has been on a part.** The board was holding the USB
proxy with a GreatFET behind it and another round was using it, so the
hardware step was left undone rather than fought over. What that leaves
unmeasured is specific:

- that a host's `stty -F /dev/ttyACM1 9600 parenb cs7` produces a 7E1 frame
  at 9600 baud on ball C11 — which is the demonstration this round is for;
- that `testdata/fpga/cynthion/usb_cdc_uart.v` is wired the way §7 says it
  cannot test;
- that the extra 24 lookup tables at 8N1 place and route in the designs
  that use them. The ECP5 flow was run on `usb_cdc_uart.v` and
  `docs/ip-library.md`'s table runs it on `uart` and `uart_frame`, so this
  is measured for those; the five example designs are not re-measured here.

What *is* measured without a board: every area figure in §4, every
behavioural claim in §5 and §7, and the three mutations.

The procedure for when the board is free is the one
`testdata/fpga/cynthion/usb_cdc_uart.v`'s header describes: load it, watch
LED 0 for whether the whole line coding was honoured exactly, and put an
instrument on LED 5, which carries the real waveform out of ball C11. The
thing to read off it is **the number of bit periods between two start
edges**, because that is what the framing changes and what no loopback can
see.

---

## 10. Reading list

- USB Communications Class Subclass Specification for PSTN Devices,
  revision 1.2, §6.3.11 and Table 17 — `bCharFormat`, `bParityType`,
  `bDataBits`.
- The same document, §6.5.4 Table 31 — `wSerialState`, whose break,
  framing, parity and overrun bits are what §5's four signals would drive
  in a design that chose to.
- `ip/usb/usb_cdc_acm/README.md` — what the host actually sends and what
  that block does with it, including the "nothing acts on the line coding"
  section this round makes obsolete.
- `docs/ip-library.md` — the footprint table §4 quotes, and the catalogue.
- `uart_frame_tx.v`, `uart_frame_rx.v` and `uart_line_coding.v` headers —
  the long form of §3, §4 and §5 at the point of use.
