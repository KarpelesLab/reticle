# A USB host, and what makes one different from a device

`usb_host_ulpi` is a USB 2.0 full-speed **host** behind a ULPI
transceiver. Everything else USB in this library is a peripheral —
`usb_device_fs`, `usb_device_ulpi` and `usb_cdc_acm` are all blocks that
answer somebody else's tokens — and this is the end that sends them: a
frame every millisecond, SETUP, IN and OUT tokens with their CRC5, data
packets with their CRC16, handshakes, a timeout with retries, the bus
reset, and the enumeration of whatever is plugged into the port.

It is written on top of the same bus this repository already has running
on silicon. [`ip/usb_device_ulpi/README.md`](../usb_device_ulpi/README.md)
states ULPI fact by fact with the provenance and confidence of each, and
**that document is not repeated here**: the turnaround, the transmit
command, the receive command, register access, the rule about believing a
byte only when `dir` was already high, and the two things the Microchip
transceiver on a Cynthion does that ULPI does not mention are all the
same at this end of the wire. This document is about the three places
they differ, about the layer above them that a peripheral has no
equivalent of, and — in §9 — about where this block **stops**, which is
further from a board than it was meant to.

This file is laid out the way that one is, because the two kinds of fact
in it are very different: some are read out of a published specification
and some are a reading of it that no device has confirmed. **A third
kind matters more here than it did there**, and it is the transceiver's
own datasheet: the register combination that makes a host drive SE0 is
not in ULPI's register tables in any usable form, and the part's own
Table 5-1 is what says which bits switch the 45 Ohm terminations on.

Nothing was transcribed from anybody's implementation. The encodings
below are read out of published documents; the state machines that use
them are this repository's own, and §8 says which choices were open and
which was taken.

## 1. Confidence, and what it is based on

- **HIGH** — stated in one of three published documents, each named with
  the section it was read from: the *UTMI+ Low Pin Interface
  Specification, Revision 1.1* (20 October 2004), the *Universal Serial
  Bus Specification, Revision 2.0*, or the *USB334x Data Sheet*
  DS00002646A (Microchip, 2009-2018).
- **MEDIUM** — a reading of one of those which it does not state in so
  many words, or a property of one part rather than of a standard.
- **LOW** — inference that explains the rest, with no way to check it
  here.
- **CHECKED** — something a simulation of this block against
  `ip/usb_device_ulpi` has actually produced, with the test that produced
  it named. §9 is the whole of that list, and it is shorter than the
  quotations.

**No fact in this document has been measured on a board.** That is the
difference between this file and the one next door, it is not for want of
trying, and §9 says exactly why.

---

## 2. What a host is that a peripheral is not

Three things, and no more, as far as the ULPI bus is concerned.

### The pull-downs, which live in one register

ULPI 1.1 §3.8.5.3.2 names the full-speed host in its figure's own title
and then in its text:

> **ULPI FS Host (XcvrSelect=01b, DpPulldown=1b, DmPulldown=1b,
> TermSelect=1b)**
>
> "The host has its 15 kOhm pull-downs enabled (DpPulldown and DmPulldown
> set to 1b) and the 45 Ohm terminations disabled (TermSelect set to 1b).
> The peripheral has the 1.5 kOhm pull-up connected to D+ (TermSelect set
> to 1b)."

> *Provenance*: §3.8.5.3.2, quoted. HIGH.

So **Function Control is the same byte for a host as for a peripheral**:
`04h` = `45h`, which is XcvrSelect `01` (full speed), TermSelect `1`,
OpMode `00` and SuspendM `1`. What `TermSelect = 1` *does* is different
at each end — a pull-up on a peripheral, no 45 Ohm terminations on a host
— and the bit is the same bit. The whole of the difference is in OTG
Control:

| | `04h` Function Control | `0Ah` OTG Control |
|---|---|---|
| full-speed **peripheral** | `45h` | `00h` — both pull-downs **cleared** |
| full-speed **host** | `45h` | `06h` — DpPulldown and DmPulldown **set** |

`06h` is also OTG Control's **reset value** (ULPI 1.1 Table 24; USB334x
Table 7-1), so a transceiver that has just come out of reset is already a
host as far as its resistors go. This block writes it anyway, for the
same reason it writes Function Control twice: a readback is worth more
than a reset value.

The part's own datasheet says the same thing as a table of resistors
rather than of register fields, and it is worth having both because the
resistors are what is actually on the wire:

| Signaling Mode | XcvrSelect | TermSelect | OpMode | DpPulldown | DmPulldown | RPU_DP | RPU_DM | RPD_DP | RPD_DM | HSTERM |
|---|---|---|---|---|---|---|---|---|---|---|
| Host Full Speed | `X1b` | `1b` | `00b` | `1b` | `1b` | 0 | 0 | **1** | **1** | 0 |
| Host Chirp | `00b` | `0b` | `10b` | `1b` | `1b` | 0 | 0 | 1 | 1 | **1** |
| Peripheral FS | `01b` | `1b` | `00b` | `0b` | `0b` | **1** | 0 | 0 | 0 | 0 |
| Power-up or VBUS < VSESSEND | `01b` | `0b` | `00b` | `1b` | `1b` | 0 | 0 | 1 | 1 | 0 |

> *Provenance*: DS00002646A §5.2.2, Table 5-1, four of its nineteen rows.
> HIGH. The same section adds a warning worth obeying: "If a ULPI
> Register Setting is configured that does not match a setting in the
> table, the transceiver operation is not maintained and the settings in
> the last row of Table 5-1 will be used." **So the only values this
> block writes are rows of that table.**

The last row above is the reset state, and it is why a transceiver that
has been reset and left alone is a safe thing to leave on a port: the
pull-downs are on, nothing is pulling up, and no terminations are
connected. That is an idle downstream port and nothing else.

### The pair is SE0 and that is the right answer

A peripheral's start-up connects its own 1.5 kOhm pull-up and then has to
**wait** for it to charge the pair — milliseconds, measured, and
`usb_ulpi_link`'s `LINE_TRIES` and §8 of the document next door are the
account of what the missing wait cost. A host has no pull-up. Its idle
port is **SE0**, made by its own two 15 kOhm pull-downs and nothing else,
and it stays there until something is plugged in.

So `usb_ulpi_host_link` reads the Debug register **once** and believes
it. Waiting for the pair to leave SE0 at this end would hang on an empty
socket for ever, and SE0 is not a failure to be retried: it is the
reading.

> MEDIUM, as a reading of §3.8.5.3.2's resistor assignment. CHECKED in
> simulation, which is cheap here because the model can be told to be a
> host's transceiver and then an undriven pair really is SE0:
> `usb_host_ulpi_configures_the_transceiver_as_a_host` asserts the Debug
> register is read exactly once and that the answer is `00h`.

### The register port belongs to the design

A peripheral's Link has a fixed start-up sequence and no reason to let
anything else near the bus. A host's has two reasons, and the second one
is the whole of §3 of this document: the bus reset it must drive **is a
register write**, and a design that has never seen this port before needs
to ask the transceiver what it is before it drives anything at all.

So `reg_*` is a transaction port, and `enum_en` low gives it to the
design:

- **is the transceiver there** — it answers a register read with the byte
  its datasheet gives;
- **does the port have power** — `VbusState`, which no amount of driving
  the bus will tell you and which decides whether a device can pull
  anything up at all;
- **which line is the device on** — `LineState`, which says both that
  something is attached and, since a full-speed device pulls **D+** up,
  whether the board exchanges DP and DM on the way to the connector.

§5 is about that last one, which is the thing most likely to be wrong on
a board nobody has measured.

---

## 3. Driving a bus reset, which is not a transmission

This is the one fact about hosts that is easiest to get wrong, and it is
stated plainly in a place that is easy to miss — the middle of the
high-speed chirp sequence, which a full-speed-only host otherwise has no
use for:

> "2. **Host Drives** – If a host detects a full speed peripheral, it
> resets the peripheral by writing to the Function Control register and
> setting XcvrSelect = 00b (HS) and TermSelect = 0b **which drives SE0 on
> the bus** (D+ and D- connected to ground via 45 Ohm). The host also sets
> OpMode = 10b for correct chirp transmit and receive. The start of SE0
> is labelled T0."

> *Provenance*: ULPI 1.1 §3.8.5.1, step 2, quoted. HIGH.

So SE0 comes from switching the 45 Ohm high-speed terminations **on**
with no transmitter running, and the Link's whole part in it is one byte
in one register. There is no NOPID transmit, no `stp` held low for ten
milliseconds, and nothing on the data bus at all for the duration.

The byte is `50h`: SuspendM `1`, Reset `0`, OpMode `10`, TermSelect `0`,
XcvrSelect `00`. That is exactly the "Host Chirp" row of Table 5-1
above, which is one of only two rows with `HSTERM_EN` asserted on a host
— and `HSTERM_EN` *is* the SE0. `45h` puts it back.

`OpMode = 10b` is in the quotation and this block writes it, although
what it is for — "correct chirp transmit and receive" — is a thing this
block never does. It is written because Table 5-1 is a table of whole
rows and §5.2.2 says a combination that is not one of them is not
supported. MEDIUM that it makes no difference to the resistors; HIGH
that the row is the row.

**Ten milliseconds, and this block holds fifteen.** USB 2.0 §7.1.7.5
asks a host to drive SE0 for at least 10 ms, and `RESET_HOLD` is
`900_000` clocks — 15 ms at 60 MHz — because a device that measures the
reset meanly should not be the thing that decides whether this works.
§7.1.7.3 then allows the device 10 ms to recover and `RESET_RECOVERY` is
20 ms.

> HIGH for the two numbers from USB 2.0; the margins are this block's
> choice. **Neither duration is checked by any test**, and that is said
> again in §9: a simulation runs them at hundreds of clocks instead of
> hundreds of thousands, which keeps the *order* and throws away the
> *time*.

---

## 4. Tokens, frames, and where each CRC belongs

### A token is three bytes and a raw transmit

Every transaction starts with a token nobody answers: a PID and two bytes
carrying seven bits of device address, four of endpoint number and a
CRC5 over those eleven bits, least significant bit first on the wire
(USB 2.0 §8.4.1, §8.4.2, §8.3.5; HIGH).

On a ULPI bus that is a transmit command and then **two bytes exactly as
given** — no more. This is why `usb_ulpi_host_link` has three transmit
shapes where the peripheral's Link has two:

| `tx_mode` | What goes out | What it is for |
|---|---|---|
| `TX_HANDSHAKE` | the command and `stp` | an ACK |
| `TX_RAW` | the command, `tx_len` bytes, `stp` | a **token**: the CRC5 is already inside them |
| `TX_DATA` | the command, the payload, the CRC16, `stp` | a data packet |

A Link that appended a CRC16 to a token would put five bytes on the wire
where USB expects three, and nothing would ever answer it. MEDIUM that
`TX_RAW` is the right shape — ULPI draws no figure for a token
specifically — but it follows from §3.8.2.2's transmit being a command
and then whatever bytes the Link hands over, and from the CRC16 being the
Link's own work rather than the transceiver's (the document next door's
§6 has that argument at length).

**The CRC5 is in `usb_host_sie` and the CRC16 is in the Link**, and the
division is not arbitrary: the CRC5 covers an address and an endpoint
number, which the engine knows and the Link does not; the CRC16 covers
the bytes the Link is already handing over one at a time.

### A frame every millisecond

A full-speed host sends a SOF token every millisecond carrying an 11-bit
frame number with its own CRC5, and the number increments every frame
(USB 2.0 §8.4.3; HIGH). This is not decoration: a device that sees no bus
activity for 3 ms enters suspend (§7.1.7.6; HIGH), and an enumeration has
gaps longer than that in it.

`FRAME_CYCLES` is the millisecond in clocks and the counter is free
running, so a SOF this engine could not send on time does not move the
next one. **A SOF can be late**, though: `usb_host_sie` sends one only
from its idle state, so a frame that falls due inside a transaction waits
for the transaction. The worst case is a transaction being retried to
exhaustion — `RETRIES` of 3 plus the first try, each up to
`TIMEOUT_CYCLES`, which at the defaults is 273 us. That is a quarter of a
frame late and not a frame missed, and §8 says what it would take to fix
and why it has not been.

### Where the reply is decoded, and why it is not here

`usb_pkt_rx` decodes it, and that module is **`ip/usb_device_fs`'s**,
reached through this package's `depends` line rather than copied. A PID's
check nibble and a data packet's CRC16 are the same arithmetic in both
directions, and that decoder has been doing it on a board. Two
properties of it matter to a host and are quoted from its own header:

- `pkt` fires for a handshake as well as for a data packet, because "a
  packet that arrived at all, right or wrong, is a reason to stop
  waiting" — which is exactly what a host's timeout needs to end on;
- `dat_ok` is what says a data packet is usable, and a packet whose CRC16
  fails gets **no handshake**, so the device sends it again. That is USB's
  own rule (§8.6.4) and it is why `trn_status` has no code for a bad CRC
  until the retries have run out.

---

## 5. Which line is which, and the one polarity question a board can ask

ULPI 1.1 Table 7 makes LineState bit 0 **D+** and bit 1 D-, so a
full-speed device pulling D+ up reads `01` and a low-speed one reads
`10`. That is the **transceiver's** D+, and a board may exchange DP and
DM between the transceiver and its connector to keep the pair from
crossing over in the layout. A Cynthion does, deliberately, and the
compensation is a bit in a register ULPI reserves and describes not at
all:

> Register **`39h`** "USB IO & Power Management", bit 1 **`SwapDP/DM`**:
> "When asserted, the DP and DM pins of the USB transceiver are swapped.
> This bit can be used to prevent crossing the DP/DM traces on the
> board." Reset value `04h`.

> *Provenance*: DS00002646A §7.1.3.5, quoted. HIGH.

`VENDOR_ADDR` and `VENDOR_DATA` are how this block writes one such
register, before anything on the wire can be affected by it and read back
before the sequence goes on — the same mechanism, the same two
parameters and the same ordering as `usb_ulpi_link`.

**It matters to a host for the mirror of the reason it matters to a
peripheral.** A peripheral with the bit unwritten puts its pull-up on the
wire a host calls D- and is detected as a low-speed device it is not;
`ip/usb_device_ulpi/README.md` §11 has the kernel log of that happening
and of the bit fixing it. A **host** with the bit unwritten reads a
full-speed device's pull-up on the line it calls D- and calls the device
low speed, and then refuses to talk to it at all.

Because the answer is a property of a board rather than of ULPI,
`FS_LINE` is a parameter and not a constant — and whichever way it is
set, `line_state` reports what was actually seen, so a probe settles it.

> CHECKED, in the sense that matters for a parameter:
> `usb_host_ulpi_takes_which_line_is_full_speed_from_its_parameter` sets
> `FS_LINE` to `10`, puts the pair at J, and the host calls it low speed
> and sends nothing. The parameter is live and the failure it prevents is
> the one described.

**What is only quoted, for the Cynthion's TARGET port:** Great Scott
Gadgets' own gateware writes `39h` = `06h` — the reset value with bit 1
set — for **every** port of the board, not per port. It is a
platform-level attribute, `ulpi_extra_registers = {0x39: 0b000110}` in
`cynthion/gateware/platform/core.py`, applied by
`luna/gateware/interface/ulpi.py` to whichever ULPI interface is built.
So the TARGET port is crossed the same way AUX is, on the strength of the
same two files and nothing else. **This has not been read off the TARGET
port**, and the probe built for exactly that purpose has not run (§9).

---

## 6. Waiting for its own packet to finish on the wire

This is the one thing a host must do that a peripheral's link layer gets
away with not doing, and it took the longest to get right.

ULPI 1.1 §3.8.2.2: after `stp` the Link cannot transmit again until the
packet has finished on the wire. A peripheral never has a second packet
ready that soon — `usb_ctrl_ep` and `usb_bulk_ep` each answer one host
packet at a time and have nothing to send until the host has heard the
last answer — and `usb_ulpi_link` therefore does not wait, which its own
header says and says why. A host's SETUP token is followed
**immediately** by its data packet, so it does.

**`tx_busy` is no help.** It falls with the `stp` that ends the *bus*
transfer, and on the part this was written for the bus transfer finishes
long before the wire does: three bytes went across in 32 clocks where the
wire needs 175, and "twelve or more receive commands follow it inside
8.5 us" (`docs/fpga-trellis.md`, measured). The transceiver announces the
real end with a receive command carrying the SE0-to-J transition, and
that one cannot be picked out of the others, because

> "In the case of Full Speed or Low Speed, after STP is asserted each
> FS/LS bit transition will generate a RXCMD since the bit times are
> relatively slow."

> *Provenance*: DS00002646A §6.3.1, quoted. HIGH, and `docs/fpga-trellis.md`
> has the trace in which a Link was handed the backlog.

So the wait is **counted**, from the length of the packet, and only in
the three places where the next thing the engine does is transmit: after
a SOF, after a token that has data to follow it, and after an ACK.
`TX_PER_BYTE` clocks a byte and `TX_OVERHEAD` for the SYNC field, the end
of packet and the transceiver's own latency. At 60 MHz a full-speed bit
is 5 clocks, so a byte is 40 — and the default is **48**, which is 40
plus the worst case of a stuffed zero after every six ones.

There is deliberately **no count after this host's own data packet**, and
the argument is in `usb_host_sie`'s header: what follows one is always
listening, and by the time the listening is over the wire is provably
clear either way.

**What the slack costs, and the first measurement to make on a part.**
The gap this leaves between the host's token and its data packet is the
count's slack plus `GAP_CYCLES`, about ten bit times rather than the two
to six and a half USB 2.0 §7.1.18 names for a *response*. Nothing in
USB 2.0 puts a ceiling on how long a host may leave between two packets
it sends itself, and `ip/usb_device_fs`'s receiver has no timer between a
token and the data after it — which is CHECKED, since that is the device
this host enumerates. A device that did have one would be the first thing
a ULPI trace should be pointed at, and the fix would not be a shorter
count but a **measurement** of the transceiver's
transmit-command-to-wire latency, which is what `TX_OVERHEAD` is standing
in for. MEDIUM that ten bit times is safe; LOW that any particular
device tolerates it.

---

## 7. The enumeration

USB 2.0 §9.1.2's sequence, with §7.1.7.3's and §9.2.6's waits on it.
Every step is one or more control transfers.

1. **Attach.** `LineState` leaves SE0. `FS_LINE` is J and the other is a
   low-speed device, which this host reports and does not talk to.
   §7.1.7.3 asks for 100 ms of stability first, because a plug being
   pushed in bounces; `DEBOUNCE_CYCLES`.
2. **Reset.** §3 above. Then §7.1.7.3's recovery.
3. **The device descriptor, twice.** Eight bytes first, because until
   byte 7 has been read the host does not know `bMaxPacketSize0` and must
   assume the smallest a full-speed device may declare, which §5.5.3
   makes **8**. Then all eighteen with the real size — which is also the
   first thing that proves a multi-packet data stage and its toggle.
4. **An address.** `SET_ADDRESS` is the one request whose effect must
   wait for its own status stage (§9.4.6): the device answers at its
   **old** address and only then changes. So the transfer finishes at
   address 0 and everything after it is at `DEV_ADDR`, with §9.2.6.3's
   2 ms in between.
5. **The device descriptor again**, at the new address. Nothing in USB
   asks for this. It is here because it is the one transaction that
   distinguishes "the address was accepted" from "the device is still
   answering at 0", and the two look identical until something addresses
   it.
6. **The configuration descriptor, twice**, for the same reason as the
   device descriptor: the first nine bytes carry `wTotalLength`.
   `DESC_MAX` caps what is read.
7. **A configuration.** `SET_CONFIGURATION` with the
   `bConfigurationValue` byte 5 of that descriptor gave — **not** a
   constant 1. The value is the device's to choose and a host that
   assumes is wrong on a device that numbered it anything else.

> *Provenance*: USB 2.0 §9.1.2, §9.4.3, §9.4.6, §9.4.7, §5.5.3, §7.1.7.3,
> §9.2.6.3 and Tables 9-8 and 9-10 for the field offsets. HIGH.

**The descriptor bytes leave as a stream**, each with its offset in its
own descriptor and which descriptor it is. A consumer that writes
`desc_data` at `base[desc_tag] + desc_index` never has to undo anything:
a transfer that failed is retried and the retry writes the same offsets
again, and `desc_done` is the only thing that says a descriptor is whole.
Nothing is buffered in the block — the three fields the sequence needs
for itself are caught out of the stream by their offsets as they go past.

**The data toggle is the sequencer's**, because it is a property of a
transfer: a control transfer's data stage starts at DATA1 whichever way
it points (§8.5.3) and alternates, and the status stage is always DATA1.
`usb_host_sie` deliberately holds no toggle, because one toggle there
would be one toggle for every endpoint.

**A NAK is not a failure.** A device may answer NAK for as long as it
likes while it gets ready, so a NAK re-sends the transaction and the
bound on that is a **clock** and not a count: `NAK_CYCLES` for the whole
control transfer. A count would be a different limit for a slow device
than for a fast one.

---

## 8. What this block does, and the choices it made

Implemented: a host's register values and their readback, the vendor
register a board can need, the bus reset driven from the terminations,
SOF with an 11-bit frame number and CRC5, SETUP/IN/OUT tokens with CRC5,
data packets with the Link's CRC16, handshake decoding, the data toggle
across a control transfer, a reply timeout with retries, attach detection
with a debounce, low-speed detection, and the enumeration of §7 with the
descriptor bytes streamed out.

Deliberately not implemented, with reasons:

- **High speed.** It needs the chirp handshake of §3.8.5.1 — the host
  driving SE0, the peripheral answering with a chirp K, the host
  answering with alternating K and J of 40 to 60 us each — and a Link
  that turns a received packet round in 1 to 14 interface clocks instead
  of 7 to 18 (Table 10), so under 250 ns.
- **Low speed.** It needs `XcvrSelect = 11b` and the transceiver
  prepending a full-speed preamble to every packet (DS00002646A
  §6.4.1.3). A low-speed device is **detected and reported** rather than
  ignored, because telling the difference between "nothing is attached"
  and "something this host cannot talk to" is the useful part.
- **Hubs, split transactions and the PRE token**, which follow from the
  above. This host talks to the one device on its port.
- **Isochronous and interrupt transfers**, and bulk: the transaction
  engine can do a bulk transfer and nothing drives it to. What is missing
  is a schedule, which is §8's next entry.
- **A schedule.** A host controller proper keeps a list of endpoints and
  a budget per frame. This keeps one transaction and a frame counter, and
  whatever drives `trn_start` decides what to ask for; `usb_host_enum` is
  the one thing in this package that does.
- **Suspend, resume and remote wake-up** (§3.9, §3.8.5.3.2). SuspendM is
  left set and the clock runs always, which is what ULPI's input clock
  mode wants.
- **High-speed disconnect detection**, which is the transceiver's only in
  high speed anyway: "When in FS or LS modes, the Link is expected to
  handle all disconnect detection" (DS00002646A §6.3.2.1). A full-speed
  detach is SE0 on `line_state`, and **nothing acts on it once the device
  is up** — see below.
- **VBUS switching.** The receive command's `VbusState` is reported and
  nothing is done with it. Whether a port has power is a board's
  question, settled by a switch outside the transceiver, and on the board
  this was written for it is a *bidirectional* switch between connectors
  where the wrong combination ties two hosts' supplies together. This
  block neither reads a switch nor drives one; a design that drives one
  does it in its own top level where the board's own comment about it can
  be read.
- **String descriptors**, which need a language ID read first and are the
  one part of enumeration whose length nothing bounds.
- **Re-arming after a detach.** `LineState` going back to SE0 after a
  device was configured is a detach, and `usb_host_enum` does not go
  round again: `up` stays high and the report stays true of a device that
  may have gone. This is the first thing to add, and it needs a decision
  about what the report then says.
- **Interrupting a transaction to keep a SOF on time.** It needs the
  retry logic to be able to resume rather than restart.

Choices where a document allowed either:

- The reply timeout is counted **from the start of the host's own
  transmit** and not from the end of its packet on the wire, so the
  packet's own wire time is inside the window and the window has to be
  longer than the longest packet. Counting from the end would need to
  know where the end is, which is §6's whole subject.
- `TIMEOUT_CYCLES` is 4096 where USB 2.0 §7.1.19.1 gives a host 16 bit
  times — 80 clocks — after the end of its packet. The asymmetry is the
  reason: a timeout too long only delays the retry of a transaction that
  was going to fail anyway, and a timeout too short abandons an answer
  that was on its way. It also has to cover a **model** of a transceiver
  as well as a transceiver, and the model in `tests/ip_library.rs` is
  store and forward.
- A register transaction aborted by the transceiver is retried
  `REG_TRIES` times and then **reported** with `reg_ok` low, where the
  peripheral's Link retries for ever. §3.8.3.1 says "must retry" and puts
  no number on it; the peripheral has nothing to go on to if its start-up
  cannot finish, and a host's register port belongs to something that is
  waiting for an answer.
- The bus is believed from the end of the transceiver's own reset rather
  than from the end of the start-up, which §9 explains and which is the
  difference between being able to read `VbusState` at all and not.

---

## 9. What has been established, and what has not

### What has been established: our host enumerates our own device

`tests/ip_library.rs` puts `usb_host_ulpi` and `usb_device_ulpi` on the
same D+ / D- pair, each behind its own transceiver model, with the pair
resolved between them by the only three things that drive it — the host's
45 Ohm terminations, whichever end is transmitting, and the device's own
pull-up. `usb_host_ulpi_enumerates_usb_device_ulpi` runs the whole of §7
and asserts **bytes**:

- the eighteen-byte device descriptor, compared with
  `expected_device_descriptor`, which is the same function the device's
  own tests compare a *host model's* reading against;
- the thirty-two-byte configuration descriptor, likewise;
- `wTotalLength` read out of bytes 2 and 3 of the first nine, and
  `bMaxPacketSize0` out of byte 7 of the first eight;
- `bConfigurationValue` out of byte 5, and that `SET_CONFIGURATION` used
  **that** and not a constant;
- the address: the host says 1 and the **device's own `address` output**
  says 1, which is the only thing that distinguishes an address that was
  accepted from one that was sent;
- the device's `configured` output;
- that a SOF went out while this was happening;
- and that neither transceiver model complained about its Link, and that
  the two ends never drove the pair in the same cycle.

`usb_host_ulpi_enumerates_through_the_transceiver_that_is_on_the_board`
does it again with both models told to behave the way the part on the
board does: each hears its own transmission (`hears_itself`) and each
reports LineState **late**, one transition at a time out of a backlog
that outlives the packet (`reporting_stale_line`). Both of those were
measured on a Microchip USB3343 on a Cynthion r1.4 and both are things
ULPI either permits or forbids and the part does anyway. The device has
enumerated through them on a board; the host now does it in simulation.

`usb_host_ulpi_reads_a_descriptor_in_packets_of_eight` does it a third
time against a device built with `MAXPKT0 = 7'd8`, which is the smallest
§5.5.3 allows, and it is there because of what the first two **cannot**
reach: our device declares 64, so every descriptor it sends fits in one
packet and a data stage of one packet proves nothing about a toggle. With
eight, the eighteen bytes arrive in three packets and the thirty-two in
four — and the second of those also reaches the *other* way a data stage
ends, because four full packets is exactly `wLength` and there is no short
packet to stop on, which is the branch a host that only handled short
packets would hang in. The toggle is checked without counting packets: a
DATA0 packet **with a payload** can only come out of endpoint 0 in a stage
of more than one packet, since a stage starts at DATA1 and a status stage
carries nothing, and with 64-byte packets there is not one of them.

**And the packets are compared with a second implementation of the
arithmetic.** `usb_token`, `usb_data` and `usb_sof` in that file are what
the *host model* builds its packets with, and `usb_crcs_match_the_catalogue_and_the_wire`
holds their CRC5 and CRC16 to the published catalogue's check values over
`"123456789"` before anything is held to them. So the SETUP token, its
eight-byte `GET_DESCRIPTOR` data packet and the first two SOFs are
asserted byte for byte against a second implementation, and that one is
pinned to a third. The SOF is worth having in that list on its own: it is
the one token whose CRC5 covers an eleven-bit frame number rather than an
address and an endpoint.

Nine more tests — twelve in all — cover what an enumeration that works
does not reach: the start-up sequence byte for byte with `0Ah` = `06h` and
**one** Debug read, the vendor register written and read back before
anything else, the probe reading seven registers with nothing put on the
USB, an empty port not being an attachment and getting no frame, a
low-speed device being reported and not spoken to, a device that never
answers being given up on after four tries with the four SETUP tokens
asserted byte for byte, a device that leaves during the reset being
reported as that and not as a timeout, `FS_LINE` being live, and the block
being one clock domain.

**What that establishes about the host is the host's logic and the
bytes.** Four real defects in this block came out of it and are in the
commit history: a state register shared between two waits so that a
transaction never ended; a register read's answer latched as a receive
command; the start-up sitting out the transceiver's own reset and then
reading the receive command that followed it as the reset; and a register
written at two widths from two places. The last three are **inherited
from `usb_ulpi_link`, where they are unreachable rather than absent** —
that Link reads no register after `phy_ready` and has no use for a
receive command it cannot get another of — and they are not fixed there,
because changing a block a host has enumerated on a board, to remove a
condition that provably cannot arise in it, is not a trade this project
makes. They are written down here instead, which is where the next person
to write a Link will be.

### What it does not establish, and the three things only a board can say

- **None of the times.** The simulation runs 100 ms of debounce as 200
  clocks, 15 ms of SE0 as 400 and 2 ms after `SET_ADDRESS` as 200, a
  ratio of about fifteen thousand to one. That **keeps** every ordering,
  because each is a state and not a duration, and it keeps the bus reset
  long enough to be one — 400 clocks is more than `usb_device_ulpi`'s own
  `SE0_CYCLES` of 150, so the device really does see a reset and really
  does forget its address. It establishes **nothing** about whether 100 ms
  of debounce is 100 ms or whether a device given 2 ms has enough. Those
  are numbers a host must get right against a *device's* patience.
- **Nothing about the inter-packet gap of §6** against any device but
  ours. Ten bit times between a token and its data is what this block
  leaves and what `usb_device_fs` does not mind.
- **Both halves are this repository's.** The device side is a fixed point
  rather than a second guess, because a real host has enumerated it on a
  real board — but where this document is wrong about ULPI, the host, the
  device and the model can still be wrong together. That is the sentence
  `ip/usb_device_ulpi/README.md` §11 wrote before it had a board, and the
  one time it came true there is written up in the same section.

### Where it stops, which is one edge of a die

**It has not run on the part, and it cannot, with this backend as it
stands.** This is not a matter of trying harder and it is worth being
exact about, because it is the one thing in this document that a reader
will want to act on.

The Cynthion r1.4's TARGET transceiver is on these balls, from the
board's own platform description:

```
target_phy  data="R2 R1 P2 P1 N3 N1 M2 M1" clk="T4" clk_dir='o',
            dir="R3" nxt="T2" stp="T3" rst="R4" rst_invert=True
```

Every one of those thirteen balls is at **column 0** of the die in
Project Trellis' `iodb.json` for the caBGA-256 — the **left** edge. So
are the three VBUS switch balls, `K5`, `L1` and `L2`. The AUX port, which
this project's designs have been using all along, is at column 72: the
right edge. The LEDs and the oscillator are at row 0: the top edge.

`src/fpga/trellis`'s `Edge::of` described **two** of the four edges when
this block was written, and the left was not one of them, so a design on
those balls was refused:

```
error: `tgt_data$io0` is constrained to package pin `R2`, which the
       architecture maps to no usable site
```

That refusal was the honest thing and it is now gone: **all four edges are
described.** `docs/fpga-trellis.md` has the account. The part worth
carrying here is why a mirror of the right edge would have been wrong: the
**column** mirrors, the **rows do not**, so a mirrored rule would have put
every left-edge pad's bits in the tile belonging to a *different ball of
the same edge* — and it would have decoded perfectly against itself while
driving the wrong pin. It was settled the way the other two edges were,
against bitstreams Lattice's own packer wrote for this board: 477 bits at
absolute frame positions across all three reference files, and no
`PIO<s>.BASE_TYPE` set anywhere in column 0 that is not a mapped ball's
tile.

So `testdata/fpga/cynthion/usb_host_target.v` now builds **with its TARGET
balls constrained**, unchanged from the version written against the
refusal: 36 pads, 128 408 configuration bits, 41 882 arcs, **0
unexplained**, and the arcs the bits select are exactly the router's.

**What that does not settle, and this is the part that still matters.**
Nothing has been loaded. So the probe of §2 has not been run and the
enumeration has not been attempted on silicon, which means every "quoted"
in this document that a measurement would have turned into "checked" is
still quoted — including the one that matters most, whether the TARGET
port crosses DP and DM the way AUX does. `FS_LINE` is a parameter for
exactly that reason and one probe run settles it.

A board would also add the thing no bitstream can: that `X0Y38/PIOC` is
the ball wired to the transceiver and not the ball a row away. **This
board cannot give that directly** — its LEDs are on the top edge, its
USER button on the right, and there is nothing observable on the left at
all. The transceiver itself is the instrument available: it will not raise
`nxt` without the clock the FPGA drives out of `T4`, so a register read
that answers at all is evidence the left-edge rule put the clock on the
right ball.

**The two signals in "4423 of 4425"** in the earlier measurement are not
unrouted nets, and the difference is worth saying because it looks
alarming. `Netlist::is_routable` requires a signal to have both a driver
and a sink, and a pad driven by a constant has neither a driving *pin* nor
anything to route to. That design drives three output pads from constants
— the three VBUS switches — so two or three such signals is what is
expected. The arithmetic is right; which signals they are has not been
pinned down, and the line that carries the weight is the last one, where
every set bit decodes and the arcs are exactly the router's.

---

## 10. What the board says

The board this was written for is a Great Scott Gadgets **Cynthion
r1.4**, whose three USB ports each go through their own ULPI transceiver.
The facts below are read out of
`cynthion/gateware/platform/cynthion_r1_4.py` in the installed `cynthion`
package, and the part is the same `USB3343-CP` the AUX port's
`ip/usb_device_ulpi` has been talking to.

```
ULPIResource("target_phy", 0,
    data="R2 R1 P2 P1 N3 N1 M2 M1", clk="T4", clk_dir='o',
    dir="R3", nxt="T2", stp="T3", rst="R4", rst_invert=True,
    attrs=Attrs(IO_TYPE="LVCMOS33", SLEWRATE="FAST")),
```

- **`clk_dir='o'` means the FPGA drives the clock to the transceiver**, so
  the transceiver is in ULPI's optional input clock mode (§3.7.1.2) and
  the board's 60 MHz oscillator is the interface clock. No PLL. The
  design's top routes that clock to the transceiver's clock ball.
- **`rst_invert=True` means the reset is active low at the ball**, which
  matches the part's own active-low reset input and is why `ulpi_rst_n`
  here is active low.
- **`SLEWRATE="FAST"` is asked for on all thirteen pins** and this flow
  writes it when a constraint asks (`set_io -slew fast`).

And the thing that is a host's and not a device's:

```
# VBUS on each of the Type-C ports can be connected to TARGET A through
# a bidirectional switch. If any of these switches is enabled, TARGET A
# is considered an output. An additional switch can be enabled to pass
# VBUS through to another port in addition to TARGET A.

Resource("target_c_vbus_en",   0, Pins("K5", dir="o"), ...)
Resource("control_vbus_en",    0, Pins("L1", dir="o"), ...)
Resource("aux_vbus_en",        0, Pins("L2", dir="o"), ...)
```

A device plugged into the full-size TARGET-A socket has no power until
one of those switches is closed, and **two of them closed at once tie two
host ports' VBUS together**, which is the comment's own warning and the
reason this block has nothing to do with them.
`testdata/fpga/cynthion/usb_host_target.v` is where a switch is driven,
it drives exactly one from a parameter and the other two to zero with no
parameter at all, and its header says which one, why it is the AUX port's
and not the CONTROL port's, and what state a loaded bitstream leaves it
in.

> *Provenance*: quoted from the platform file as installed. HIGH for what
> the file says; **nothing below it has been driven**, and §9 says why.

---

## 11. Reading list

- *UTMI+ Low Pin Interface (ULPI) Specification*, Revision 1.1,
  20 October 2004 — the bus, the host's register values (§3.8.5.3.2), the
  bus reset (§3.8.5.1) and the inter-packet windows (Table 10).
- *Universal Serial Bus Specification*, Revision 2.0 — the tokens, the
  PIDs, the CRCs, the frame, the reset timing (§7.1.7) and the
  enumeration (§9.1.2).
- *USB334x Data Sheet*, Microchip DS00002646A — Table 5-1, which is the
  only place the resistors behind a register combination are written down,
  and §6.3.1, §6.3.2.1, §6.4.1 and §7.1.3.5 for what the part does that
  ULPI does not describe.
- [`ip/usb_device_ulpi/README.md`](../usb_device_ulpi/README.md) — ULPI
  fact by fact, and the account of getting a device through this same bus
  on this same board.
- [`docs/fpga-trellis.md`](../../docs/fpga-trellis.md) — the ECP5 backend,
  including the two edges of the die it describes and the two it does not.
