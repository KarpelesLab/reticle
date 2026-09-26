# ULPI, and a USB device behind one

`usb_device_ulpi` is a USB 2.0 full-speed device for a board whose USB
lines do not reach the FPGA. On such a board the lines belong to a
transceiver chip, and what reaches the FPGA is **ULPI** — the UTMI+ Low
Pin Interface — a byte-parallel bus with a 60 MHz clock and three control
signals. The transceiver does the line work: NRZI, bit stuffing, the SYNC
field, the end of packet, the serialiser, the analogue front end. The
FPGA does the bytes.

This document describes that bus well enough to write a link layer
against it, and it is what `rtl/usb_ulpi_link.v` was written from. It is
laid out the way [`docs/apollo-protocol.md`](../../docs/apollo-protocol.md)
is, because the two kinds of fact in it are very different: some are read
out of a published specification, and some are a reading of it that no
device has confirmed — **and nothing here has been confirmed by a
device**, because nothing here has run on a board. §11 says exactly what
simulation established and what it did not.

Nothing was transcribed from anybody's implementation. The specification
is a published document and the encodings below are read out of it; the
state machines that use them are this repository's own, and where the
specification leaves a choice, §10 says which was taken and why.

## 1. Confidence, and what it is based on

- **HIGH** — stated in the *UTMI+ Low Pin Interface Specification,
  Revision 1.1*, 20 October 2004. Each such statement names the section
  it is in, so it can be checked without reading any code.
- **MEDIUM** — a reading of that specification which it does not state in
  so many words, or a property of a particular transceiver rather than of
  ULPI. No device has contradicted it and no device has confirmed it.
- **LOW** — inference that explains the rest, with no way to check it
  here.

Everything about the **board** is separately marked, because its
provenance is different again: it comes from a platform description
quoted by this project's owner and not read off any hardware here.

---

## 2. The pins

ULPI is twelve signals in its 8-bit form. `clock`, `data`, `dir`, `nxt`
and `stp` are the whole of the interface; everything else a transceiver
has — D+, D-, VBUS, ID, the crystal, the reset pin — is on the other side
of it or outside it.

| Signal | Direction | What it does |
|--------|-----------|--------------|
| `clock` | either | The interface clock, 60 MHz. A transceiver **must** be able to source it; taking it from the Link is **optional** (§3.7.1.2). Everything on the bus is timed on its rising edge. |
| `data[7:0]` | bidirectional | The bus. The Link drives it to 00h when idle. |
| `dir` | transceiver → Link | Who owns `data`. Low: the Link may drive. High: the transceiver drives. |
| `nxt` | transceiver → Link | Throttles every kind of data except register read data and receive commands. During a transmit it means "this byte is taken"; during a receive it means "this byte is valid". |
| `stp` | Link → transceiver | Ends a USB transmit or a register write, and may stop a receive. |

> *Provenance*: Table 1 and Table 2, §2.2 and §3.2. HIGH.

Two facts about those signals are easy to miss and both matter:

- **`nxt` is not only a throttle.** The transceiver asserts `dir` and
  `nxt` *together* to say a USB receive is starting, when `dir` was
  previously low. So `dir` rising with `nxt` high is a packet, and `dir`
  rising with `nxt` low is anything else. HIGH (Table 2, §3.8.2.4).
- **The transceiver pulls `dir` high whenever it cannot accept data**,
  for instance while its internal PLL is not stable, whichever end is
  the clock source. So `dir` high is not always a receive and not always
  an abort; sometimes it only means "not now". HIGH (Table 2).

**There is no reset signal in ULPI.** The reset pin a transceiver has is
that transceiver's own, not part of this interface — the interface's own
reset is a register bit (§8). MEDIUM for any particular part: the
transceivers this was written for have an active-low reset input, which
is why `ulpi_rst_n` here is active low.

---

## 3. Bus ownership, and the turnaround cycle

This is the rule the rest of ULPI hangs off:

> When `dir` is low, the Link can drive data on the bus. When `dir` is
> high, the PHY can drive data on the bus. A change in `dir` causes a
> turnaround cycle on the bus during which, neither Link nor PHY can
> drive the bus. Data during the turnaround cycle is undefined and must
> be ignored by both Link and PHY.

> *Provenance*: §2.3.1, quoted. HIGH.

So there are **two** turnaround cycles around every time the transceiver
takes the bus: one when `dir` rises and one when it falls. In both of
them the byte on the bus means nothing. A Link that believes the first
byte after `dir` rises will read garbage, and a Link that drives the
first cycle after `dir` falls will fight the transceiver's drivers as
they let go.

`dir` is meant to be wired **straight to the output buffers** of both
ends (§2.3.1), which is why `ulpi_data_oe` in this block is
combinational in `ulpi_dir` rather than registered — but it is
`~dir & ~dir_previous`, so the Link stays off the bus for the falling
turnaround too. That the falling turnaround belongs to neither end is
stated twice: once in §2.3.1 above, and once in §3.8.4.2, which says a
Link resuming after an abort "must drive a TX CMD immediately **after**
the turnaround cycle". HIGH.

The same rule read from the receive side: a byte from the transceiver is
believed only when `dir` was **already** high in the cycle before it. The
transceiver model in `tests/ip_library.rs` drives `0xAA` into every
turnaround cycle it owns, and asserts that the Link never drives one, so
this is not a rule taken on trust.

### Who may interrupt whom

| Situation | What happens | Where |
|---|---|---|
| The transceiver wants the bus while the Link is transmitting | It asserts `dir`; the Link's transfer is aborted. ULPI **does not say** what causes this. | §3.8.4.1. HIGH that it may happen and that ULPI names no cause. |
| The transceiver asserts `dir` during a register read or write | The transaction is aborted and **the Link must retry it when the bus is idle**. | §3.8.3.1. HIGH. |
| A USB receive arrives during a register read | It overrides the read in any cycle of it — which is why `nxt` is never asserted for register read data, so the Link can always tell the two apart. | §3.8.3.2. HIGH. |
| The Link wants the bus while the transceiver has it | It asserts `stp`, and the transceiver must de-assert `dir` in the next cycle. It cannot do this in the same cycle `dir` was asserted. | §3.8.4.2. HIGH. This block never does it; see §10. |

---

## 4. The transmit command byte

The Link starts everything by driving one non-zero byte while it owns the
bus: two bits of command code and six of payload.

| Code | Payload | Name | Meaning |
|------|---------|------|---------|
| `00` | `000000` | NOOP | Nothing. `00h` is the idle value of the bus, and the Link drives it by default. |
| `01` | `000000` | NOPID | Transmit USB data with no PID: chirp and resume signalling. |
| `01` | `00pppp` | PID | Transmit a USB packet whose PID is `pppp`. |
| `10` | `aaaaaa` | REGW | Write the register at this 6-bit address. |
| `10` | `101111` | EXTW | Write an 8-bit address, given in the next cycle. |
| `11` | `aaaaaa` | REGR | Read the register at this 6-bit address. |
| `11` | `101111` | EXTR | Read an 8-bit address, given in the next cycle. |

> *Provenance*: Table 6, §3.8.1.1. HIGH. Anything not in that table is
> reserved and gives undefined behaviour, which is stated too.

So the transmit command for a USB packet is `8'b0100_pppp`. A device
sending an ACK (PID `0010`) drives `42h`; a DATA1 packet (PID `1011`)
drives `4Bh`. **The PID's check nibble is not in it**: the Link hands
over four bits and the transceiver puts the byte on the wire, so the
check nibble is the transceiver's work. HIGH, by the shape of the
encoding — there is nowhere else for it to come from.

## 5. The receive command byte

While `dir` is high and `nxt` is low, the byte the transceiver drives is
a receive command: status, not data.

| Bits | Field | Values |
|------|-------|--------|
| 1:0 | LineState | bit 0 is LineState(0), which is D+; bit 1 is D-. So `01` is J and `10` is K at full speed, `00` is SE0 and `11` is SE1. |
| 3:2 | VbusState | `00` below SessEnd, `01` below SessValid, `10` below VbusValid, `11` VBUS valid. |
| 5:4 | RxEvent | `00` idle, `01` RxActive, `11` RxActive and RxError, `10` host disconnect. |
| 6 | ID | The ID pin, valid 50 ms after IdPullup is set. |
| 7 | alt_int | A non-USB interrupt; the Link must read the Carkit Interrupt Latch to find out which. |

> *Provenance*: Table 7, §3.8.1.2. HIGH.

**Host disconnect must be ignored by a peripheral** — the footnote to
that table says so, and says it must not mask RxActive or RxError
either. A device that treated `10` as "not receiving" would be wrong.
HIGH.

A receive command is sent whenever any of those change, and the Link
"must be able to accept any number of continuous, back-to-back" ones
(§3.8.1.3). It has lower priority than USB data and higher than register
access, so one may be queued and arrive late — but **a queued one always
carries the current values, never an old snapshot**. HIGH. That is what
makes it safe for a Link to keep one copy of the status and update it
whenever a receive command arrives, which is what this block does.

---

## 6. USB packets

### Transmitting

1. The Link drives the transmit command `0100_pppp` and holds it.
2. The transceiver asserts `nxt` — **never in the first cycle of the
   command** — and the Link provides the next byte in the cycle after
   `nxt` was seen high.
3. The payload goes out a byte per `nxt`, the Link holding each byte
   until it is taken.
4. When the last byte has been taken, the Link asserts `stp` for one
   cycle with `00h` on the bus.

> *Provenance*: §3.8.2.2 and Table 2. HIGH.

The transceiver **prepends the SYNC pattern and appends the EOP** for
every PID packet, and works out the long EOP of a high-speed SOF for
itself. HIGH, §3.8.2.2.

**The CRC16 is the Link's.** This is the fact most worth being sure of,
because a Link that assumes otherwise will have every data packet
rejected by the host. The specification never mentions the transceiver
generating or checking a CRC; the only places a CRC appears at all are
"the Link must transmit an inverted CRC" to force a high-speed error
(§3.8.2.3) and a note about a bit-stuff error landing in "the final CRC
byte" of a received packet (§3.8.2.5). Both of those describe the CRC as
something already in the byte stream. HIGH that the Link generates it,
by that absence together with those two mentions; the same follows from
the interface being a UTMI+ wrapper, where CRC generation has always
belonged to the serial interface engine.

So a data packet on a ULPI bus is: the transmit command, the payload,
the CRC16 low byte, the CRC16 high byte, `stp`. A **zero-length** data
packet is the command, `00h`, `00h`, `stp` — the CRC16 of no bytes is
`0x0000` — and a handshake is the command and `stp` with nothing between
them. The last of those three is the one the specification does not draw
a figure for: MEDIUM, though there is nothing else the sequence could be,
since the command carries the PID and §3.8.2.2's rule about not
asserting `stp` "before the first byte has been consumed" is satisfied by
the command's own consumption.

Two more transmit rules:

- A Link that under-runs mid-packet must **drive `FFh` in the same cycle
  as `stp`**, and the transceiver turns that into a full-speed bit-stuff
  error. At most one byte of `FFh`. HIGH, §3.8.2.3. This block cannot
  under-run — its longest packet is eight bytes read out of a table — so
  it never does this.
- After `stp` the Link cannot transmit again until the packet has
  finished on the wire, which the transceiver announces with a receive
  command carrying the SE0-to-J transition. **The transceiver must always
  send that one.** HIGH, §3.8.2.2 and §3.8.1.3. This block does **not**
  wait for it, which §10 says and says why it cannot bite this endpoint.

### Receiving

`dir` and `nxt` rise together; that cycle is the turnaround and carries
nothing. From then on, while `dir` is high:

- `nxt` high: the byte is packet data. The first is the PID, complete
  with its check nibble; the last two are the CRC16.
- `nxt` low: the byte is a receive command.

The packet is over **when a receive command shows RxActive is 0, or when
`dir` is de-asserted, whichever occurs first**. HIGH, §3.8.2.4, quoted
almost word for word. Which of the two a given transceiver uses is not
stated anywhere, so a Link that handles one and not the other is betting
on a part's habits: LOW, and the reason this block handles both and the
tests drive both.

A bit-stuff error, or a packet that ends off a byte boundary, arrives as
receive commands with RxEvent `11` — RxActive and RxError — and the
transceiver de-asserts `dir` when it is done (§3.8.2.5). What a device
must do with such a packet is USB's rule, not ULPI's: say nothing, so the
host retries. This block does that by never producing `rx_eop` for a
packet RxError was seen in.

---

## 7. Register access

### A write

The Link drives `10aaaaaa`, waits for `nxt`, drives the byte in the cycle
after, waits for `nxt` again, and asserts `stp` in the cycle after that.
The transceiver must see `stp` before it will accept another command.
HIGH, §3.8.3.1 and Figure 21.

### A read

The Link drives `11aaaaaa` and waits for `nxt`. In the cycle after `nxt`,
**the transceiver asserts `dir`**; in the cycle after that — the one
after the turnaround — it drives the byte. It does **not** assert `nxt`
for that byte, which is the one place in ULPI where data is not
throttled, and the reason is worth keeping in mind: it leaves `nxt` free
to mean "a USB receive is starting", so a receive can override a read in
any cycle of it. HIGH, §3.8.3.1, §3.8.3.2 and Figure 22.

If either is aborted, the Link retries when the bus is idle. HIGH,
§3.8.3.1, stated for both directions.

### The registers this block uses

| Address | Register | Reset | What matters here |
|---------|----------|-------|-------------------|
| `04h` | Function Control (write; `05h` sets bits, `06h` clears them) | `41h` | XcvrSelect `1:0`, TermSelect `2`, OpMode `4:3`, Reset `5`, SuspendM `6`. |
| `0Ah` | OTG Control (write; `0Bh` / `0Ch`) | `06h` | IdPullup `0`, DpPulldown `1`, DmPulldown `2`, and the VBUS controls above them. |
| `15h` | Debug (read-only) | — | Bits `1:0` are the current LineState. |
| `2Fh` | not a register: the escape to the 8-bit extended address space. | — | Unused here. |

> *Provenance*: Table 19, Table 22, Table 24 and Table 30, §4.1 to §4.2.9.
> HIGH, reset values included.

**A full-speed peripheral wants `04h` = `45h` and `0Ah` = `00h`.**

- XcvrSelect `01` selects the full-speed transceiver, which is already
  the reset value (§4.2.2).
- TermSelect `1` is the part that is not: for a peripheral it connects
  the **1.5 kOhm pull-up on D+**, which is what tells a host a full-speed
  device is attached. The specification spells the combination out —
  "ULPI FS Peripheral (XcvrSelect=01b, DpPulldown=0b, TermSelect=1b)",
  and "the peripheral has the 1.5 kOhm pull-up connected to D+
  (TermSelect set to 1b)". HIGH, §3.8.5.3.2.
- SuspendM `1` is powered, the reset value, and this block leaves it
  there: it does not suspend.
- DpPulldown and DmPulldown reset **set**, and they are a host's 15 kOhm
  pull-downs. A peripheral must clear them, which is the whole of the
  `0Ah` write. HIGH, §3.8.5.3.2 and Table 24.

So the pull-up `usb_device_fs` asks for with a pin is a register bit
here, and a host cannot see this device at all until that write lands.

---

## 8. Starting up

> After power-up, and when the clock starts toggling, the Link must reset
> the PHY by writing to the Reset bit in the Function Control register.
> When this bit is set, the transceiver will assert `dir` and reset the
> UTMI+ core. When the reset completes, the PHY de-asserts `dir` and
> automatically clears the Reset bit. After de-asserting `dir`, the PHY
> must immediately re-assert `dir` and send an RX CMD update to the Link.
> The Link must wait for `dir` to de-assert before using the ULPI bus.
> Does not reset the ULPI interface or ULPI register set.

> *Provenance*: §3.5 and Table 22, quoted and joined. HIGH.

Two things follow. **During that reset the data bus is driven by the
transceiver and the data is undefined**, so the Link must not interpret
anything on it (§3.5, HIGH) — this block interprets nothing on the bus
until its whole start-up sequence has finished. And the register set
survives the reset, so settings written with the Reset bit stay written:
HIGH that the specification says so, but this block does not rely on it —
it writes Function Control again afterwards and then **reads it back**,
and only reports `phy_ready` when the readback is what it wrote. A
transceiver that behaves otherwise is written to again rather than
believed.

There is also a hardware reset pin on the transceivers this was written
for, outside ULPI, and this block holds it for `RESET_CYCLES` first. Both
resets are used because they do different things: the pin resets the
whole part including the interface and the registers, and the register
bit resets the UTMI+ core and nothing else.

**Interface protection.** A transceiver must carry a weak pull-up on
`stp` and must stop interpreting `data` while `stp` is unexpectedly high,
which is how it survives a Link that cannot drive the bus yet; during
power up, or when the clock is not running, it "always asserts `dir`,
protecting its data inputs". HIGH, §3.12. That is why this block waits
for an idle bus — `dir` low for `RESET_CYCLES` — before its first
command, and why it keeps `stp` low except for the single cycles that end
a transfer. A Link that is about to stop driving the bus should pulse
`stp` high first; this one never stops driving it.

---

## 9. Timing, and why no PLL

The interface clock is **60 MHz**. A transceiver must be able to source
it, and an input clock from the Link is an optional feature: "The PHY may
optionally support a 60 MHz input clock from the Link, removing the need
for a crystal. The PHY must drive its internal PLL from the 60 MHz input
clock." HIGH, §3.7.1.2.

That is the arrangement this block is for, and it is why there is no
`usb_device_ulpi_pll` beside it the way there is a `usb_device_fs_pll`.
A full-speed core that does its own line work needs 48 MHz — four samples
of a 12 Mbit/s bit — which no board's oscillator provides, so a PLL makes
it. ULPI needs 60 MHz, which is what the board in question already runs
at. **Nothing needs generating**, so nothing is.

### The Link decision times

ULPI states the inter-packet delays as counts of interface clocks, which
is the one piece of timing a device's control endpoint has to get right:

| Sequence | Full-speed Link | Counted from |
|---|---|---|
| Receive → Transmit | **7 to 18 clocks** | the receive command reporting LineState's SE0-to-J transition of the received packet |
| Transmit → Transmit | 7 to 18 clocks | the same, for its own packet |
| Receive → Receive | at least 1 clock | — |
| Transmit → Receive | 80 clocks | the timeout after which no answer is coming |

> *Provenance*: Table 10, §3.8.2.6.3. HIGH. The specification adds that
> "the timings given ensure inter-packet delays of 2-6.5 bit times",
> which is USB 2.0's own requirement, and that the full-speed maximum is
> one clock less than UTMI's because of the turnaround the receive command
> costs.

`usb_ctrl_ep`'s `TURNAROUND` parameter is exactly that count, which is
why it is a parameter: eight cycles is two bit times of
`usb_device_fs`'s 48 MHz clock, and nine cycles here is in the middle of
ULPI's window. In simulation the device answers 13 to 23 clocks after the
host's end of packet, which is 2.6 to 4.6 bit times.

---

## 10. What this block does, and the choices it made

Implemented: the turnaround in both directions, transmit commands for
packets, receive commands and their five fields, USB packet transmit with
the Link's own CRC16, USB packet receive with both ways a packet can end,
RxError, immediate register reads and writes with retry on abort, the
reset sequence of §3.5 and §3.12's idle-bus wait, LineState tracking, and
bus reset detection from SE0 held for 2.5 us.

Deliberately not implemented, with reasons:

- **High speed.** It needs the chirp handshake of §3.8.5.1 and a device
  that answers a received packet within 1 to 14 interface clocks rather
  than 7 to 18 (Table 10), so under 250 ns. This is a full-speed device.
- **Low speed**, which needs the transceiver told about preambles
  (§3.8.5.2), and **NOPID transmits**, which exist for chirp and resume.
- **Suspend and low power mode** (§3.9). SuspendM is left set and the
  clock runs always, which is what input clock mode wants; a device that
  must draw under 2.5 mA after 3 ms of idle is a board's problem before
  it is an interface's.
- **The extended register set, the interrupt enable registers and
  Carkit mode.** Nothing here has a use for them.
- **Aborting the transceiver with `stp`** (§3.8.4.2). Every transceiver
  must support it, and it is "provided primarily for the Link to shut
  down a babbling port". An endpoint whose longest packet is eight bytes
  cannot babble.
- **VBUS and ID.** The receive command's VbusState and ID bits are read
  and dropped: a bus-powered device is assumed, and no session is
  negotiated.
- **Transmit error injection** (§3.8.2.3). This block cannot under-run.
- **The wait for its own end of packet.** §3.8.2.2 says a Link cannot
  transmit again until the first packet has finished on the wire, which
  the transceiver announces with a receive command carrying the SE0-to-J
  transition. `tx_busy` here falls with the `stp` that ends a packet
  instead, so a device with something to say in the next clock could start
  a second packet over the first. `usb_ctrl_ep` cannot — it answers one
  host packet at a time and has nothing to send until the host has heard
  the last answer — but an endpoint that streams would have to add the
  wait, and this is the first thing to add with one.

Choices where the specification allowed either:

- `ulpi_data_oe` is combinational in `dir`, as §2.3.1 suggests, rather
  than registered a cycle later. It is also low for the falling
  turnaround, which costs one cycle of latency before the Link can drive
  and buys the certainty that the two ends never overlap.
- The start-up sequence reads Function Control back rather than trusting
  the write, and reads the Debug register for LineState rather than
  assuming the bus is at J. Both cost a few clocks once.
- A receive is believed only from the `dir`+`nxt` pair or a receive
  command, never from the turnaround cycle's contents.

---

## 11. What has been established, and what has not

**Nothing here has been near a board.** No Cynthion, no transceiver, no
host. What exists is a simulation, and this is what it establishes:

- `tests/ip_library.rs` contains a **transceiver model** written from this
  document: a full-speed receiver that recovers the bit clock from the
  line, a transmitter that adds the SYNC field, the stuffing, NRZI and
  the EOP, the ULPI bus protocol above them with both turnarounds, and
  the register file with its reset values. It **checks the Link**, and
  complains if the Link ever drives the bus while `dir` is high, drives a
  turnaround cycle, or asserts `stp` while the transceiver owns the bus.
  A test drives the model with a Link that breaks each of those three
  rules, because a model that accepts anything proves nothing.
- On the other side of that model is the **same USB host model that
  enumerates `usb_device_fs`** — real packets, NRZI, bit stuffing, CRC5
  and CRC16 from its own arithmetic, and a decoder that checks every
  answer the way a host does. The enumeration asserted is the same one,
  written once and run against both cores: the device descriptor at
  address 0, SET_ADDRESS taking effect only after its status stage, the
  old address then ignored, short reads, a read of exactly two packets,
  the configuration descriptor in 8 + 8 + 2, SET_CONFIGURATION, and a
  bus reset forgetting the address. The bytes are asserted, not the fact
  that something came back.
- The ULPI-specific cases have tests of their own: the register sequence
  byte for byte, a throttled `nxt`, `0xAA` in every turnaround cycle, a
  packet ended by `dir` alone, a register read a receive overrode, a
  readback that lies, and the transceiver taking the bus three bytes into
  the device's data packet — after which the host hears nothing, asks
  again, and is sent the same packet with the same toggle.
- The device answers between 13 and 23 clocks after the host's end of
  packet, inside the 2 to 6.5 bit times USB allows.

What that does **not** establish is almost as long. Simulation cannot
tell whether the 60 MHz clock leaves the FPGA cleanly enough for the
transceiver's PLL, whether the reset pin is long enough for a particular
part, whether a real transceiver's `nxt` timing matches the reading in §6,
whether a real host's hub tolerates this device's turnaround, or whether
the board is wired the way §12 says. The model was written from the same
document as the block, by the same hand, on the same day: where the
document is wrong, both are wrong together and the tests still pass. That
is the one thing a person plugging a cable in would find out and this
cannot.

### What it would take to see it from a host

Four things, none of them in this package:

1. **Bidirectional pads on the ECP5 backend.** The eight data lines need
   one pad each driven from `ulpi_data_o` and `ulpi_data_oe` and read into
   `ulpi_data_i`. Nothing here needs it to be simulated or measured — the
   footprint takes three separate buffers — but a board does.
2. **The clock out.** `clk_dir='o'` means the 60 MHz has to reach the
   transceiver's clock ball. That is a clock leaving the FPGA, which
   belongs to the design's top and the backend, not to an IP block.
3. **A top level and the board's constraints**: the oscillator, the eight
   data balls, `dir`, `nxt`, `stp` and the reset of whichever of the three
   ports is used, from the platform description in §12.
4. **Loading it**, which `reticle program` already does for this board's
   ECP5 over Apollo.

Then the question is decidable by software rather than by looking at an
LED: the device either appears in the host's device list as `1209:0001`
with an 18-byte device descriptor, or it does not. `lsusb -v` answers it.
That is the next milestone and this is not it.

## 12. What the board says

The board this was written for is a Great Scott Gadgets **Cynthion**,
whose three USB ports each go through their own ULPI transceiver. The
facts below come from its platform description as quoted by this
project's owner, from
`cynthion/python/src/gateware/platform/cynthion_r1_4.py`:

```
data="N16 N14 P16 P15 R16 R15 T15 P14", clk="L14", clk_dir='o',
dir="M16", nxt="M15", stp="L15", rst="L16", rst_invert=True
```

and the two balls wired to the USB data lines of one port, `N4` and `P3`,
are declared **input only** — they are there to watch the bus, not drive
it.

Three readings of that, which shaped this block:

- **`clk_dir='o'` means the FPGA drives the clock to the transceiver**,
  so the transceivers are in ULPI's optional input clock mode (§3.7.1.2)
  and the board's 60 MHz oscillator is the interface clock. No PLL. The
  design's top has to route that clock to the transceiver's clock ball;
  a clock leaves an FPGA through a dedicated path, not through a port of
  an IP block, so this block takes `clk60` as an input and does not
  forward it.
- **`rst_invert=True` means the reset is active low at the ball**, which
  matches the transceivers' own active-low reset input and is why
  `ulpi_rst_n` is active low here.
- **The eight data lines are bidirectional**, turned around by `dir`, so
  they are split into `ulpi_data_i`, `ulpi_data_o` and `ulpi_data_oe` as
  every bidirectional pin in this library is, and the three-state buffers
  belong to the top of the design.

> *Provenance*: quoted from the platform file by this project's owner; not
> read off a board here, and no pin has been driven. MEDIUM for the ball
> names and the directions, HIGH for what `clk_dir` and `rst_invert` mean
> in that description's own terms, and the conclusion that no PLL is
> needed follows from the specification's 60 MHz. Nothing about which
> part is soldered to the board is claimed at all: everything above is
> either ULPI's or the platform file's.

## 13. Reading list

- *UTMI+ Low Pin Interface (ULPI) Specification*, Revision 1.1,
  20 October 2004 — everything marked HIGH above, by section.
- *Universal Serial Bus Specification*, Revision 2.0 — the packets, the
  PIDs, the CRCs, the inter-packet delays and the enumeration, which are
  `usb_ctrl_ep`'s subject rather than this file's.
- The transceiver's own datasheet — for the reset pin, the crystal or
  reference clock, and any vendor-specific register above `30h`. None of
  it is relied on here beyond the reset pin's polarity.
