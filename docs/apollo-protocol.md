# The Apollo debugger's USB protocol

Apollo is the firmware on the debug microcontroller of a Great Scott
Gadgets **Cynthion**. The board's FPGA — a Lattice ECP5 — has no JTAG
header and no FTDI part on it. Its TAP is wired to the microcontroller,
and the only way to a scan chain is to ask the microcontroller to
perform one over USB. This document describes that request set well
enough to write a JTAG transport against it without reading Apollo's
code.

It is a specification, not a transcription. Every statement below says
where it came from and how sure of it this project is, in the same style
`src/fpga/devices/xc7.dev` uses, because the two kinds of fact here are
very different: some were confirmed against the board on the bench, and
some are a reading of a public source that no device has yet
contradicted.

**This file was written before any of it was implemented, and the
implementation was written from this file.** Apollo is BSD-3-Clause and
copying it would have been allowed; it was not copied, for the same
reason Reticle's VHDL library carries its own declarations and native
bodies rather than IEEE's text. A protocol one has written down is one
one understands.

## Confidence, and what it is based on

- **HIGH** — confirmed against the Cynthion attached to this machine, or
  read directly off its USB descriptors. Each such statement says which.
- **MEDIUM** — a reading of Apollo's public host tooling and firmware
  (`github.com/greatscottgadgets/apollo`, BSD-3-Clause), consulted to
  learn the protocol and then closed. No device has contradicted it and
  no device has confirmed it.
- **LOW** — inference that explains the rest, which this project has no
  way to check.

A *Verified* section at the end records what the bench later said, so
this document's claims and the run that tested them can be compared
without reading the code in between.

---

## 1. Two devices, one board

A Cynthion enumerates as **one of two different USB devices**, depending
on which side of the board owns the port:

| | Vendor | Product | Product string | Interfaces |
|---|---|---|---|---|
| **Gateware mode** | `0x1d50` | `0x615b` | whatever the gateware calls itself ("USB Analyzer") | the gateware's own, plus one *Apollo stub* |
| **Debugger mode** | `0x1d50` | `0x615c` | "Apollo Debugger" | one vendor-specific interface |

Both are the same physical port on the same board. The microcontroller
and the FPGA share it, and exactly one of them drives it at a time.

`0x1d50` is Openmoko's vendor identifier, which pid.codes administers;
Great Scott Gadgets hold `0x615b` and `0x615c` under it. Apollo's host
tooling also recognises a set of `0x1209:…` pid.codes test identifiers
for development builds, which a general implementation may want and
which this one does not use.

> *Provenance*: the vendor and product numbers are from Apollo's host
> package — MEDIUM. `0x1d50:0x615b` with the product string "USB
> Analyzer", manufacturer "Cynthion Project", `bcdDevice` 1.04 and serial
> `2a5a4adf30c460de` was **read off the attached board on 2026-09-25**;
> HIGH for that one.

### The Apollo stub interface

The gateware-mode device carries an extra interface that does nothing
but exist and answer one request. It is recognised **by its descriptor,
not by its number**:

- `bInterfaceClass` = `0xFF` (vendor specific)
- `bInterfaceSubClass` = `0x00`
- `bNumEndpoints` = `0`

The gateware's own interfaces are also vendor specific but use a
non-zero subclass, so the subclass is what tells them apart. An
implementation should scan the active configuration descriptor for the
first interface matching the triple above and use **that interface's
number** as `wIndex`; assuming that interface 1 is right, as it happens
to be on the analyzer gateware, is assuming a property of one gateware
rather than of the protocol.

> *Provenance*: the matching rule (class `0xFF`, subclass `0x00`, no
> endpoints) is from Apollo's host package — MEDIUM. The attached board's
> descriptors were **read here** and match it exactly: interface 0 is
> class `0xFF` subclass `0x10` with one bulk IN endpoint of 512 bytes
> (the analyzer's), interface 1 is class `0xFF` subclass `0x00` with no
> endpoints. No kernel driver was bound to either. HIGH for the board.

---

## 2. Getting from gateware mode to debugger mode

While the FPGA is configured, its gateware continuously *advertises* to
the microcontroller — over a sideband signal on the board, not over USB
— that it wants the USB port. The microcontroller yields the port for as
long as that advertisement continues. Stopping the advertisement is
therefore the whole handover, and it is a request the **gateware**
answers, not Apollo:

| Field | Value |
|---|---|
| `bmRequestType` | `0x41` — host to device, vendor, **recipient interface** |
| `bRequest` | `0xF0` (stop advertising) |
| `wValue` | `0x0000` |
| `wIndex` | the stub interface's number |
| `wLength` | `0` |

There is no data stage and no reply beyond the status stage. The device
should **disappear from the bus and re-enumerate** as `0x1d50:0x615c`
within a second or so, which means the handle the request was sent on is
dead the moment it succeeds and the new device has to be found by a
fresh enumeration. Allow several seconds and poll.

Nothing about this is persistent. It changes who drives a USB port; it
does not touch the FPGA's configuration memory, the FPGA's flash or the
microcontroller's. A power cycle, a replug, or the request in §6 puts it
back.

The FPGA stays configured and keeps running throughout. It simply no
longer has a USB host to talk to.

> *Provenance*: the request number, direction and recipient are from
> Apollo's host package — MEDIUM. The sideband-advertisement mechanism is
> the reading that explains why the request lives on the gateware and why
> stopping it is enough — LOW; it is an explanation, not an observation,
> and the protocol works the same if the mechanism is something else.

### If there is no stub interface

A gateware that does not implement the stub cannot be asked to yield,
and neither can a board whose FPGA is unconfigured and silent. Apollo's
own answer to that is a physical one — holding the **PROGRAM** button
while plugging the board in brings up the debugger directly. An
implementation that cannot press buttons should say so rather than
guess. This one says so.

> *Provenance*: the button route is the documented Cynthion procedure —
> MEDIUM. It was not needed here and was not tried.

---

## 3. Debugger mode: the device

In debugger mode the board presents one configuration with a single
vendor-specific interface. **Every request in §3, §4 and §6 is a control
transfer on endpoint zero with recipient *device*.**

```
bmRequestType = 0x40   host to device, vendor, recipient device
bmRequestType = 0xC0   device to host, vendor, recipient device
```

That recipient matters. The one request that uses recipient *interface*
is the handover in §2, and it is answered by the FPGA rather than by
Apollo.

Because everything is a device-recipient control transfer, no interface
needs to be claimed for any of it. Whether a kernel driver binds to the
debugger's interface is a question for the host and is answered on the
bench, not here; if one does, it has to be detached like any other.

> *Provenance*: the request type is from Apollo's host package —
> MEDIUM.

### Identification requests

| `bRequest` | Dir | `wValue` | `wIndex` | Data stage |
|---|---|---|---|---|
| `0xA0` | IN | 0 | 0 | a NUL-terminated ASCII string naming the firmware; ask for 256 bytes and stop at the first NUL |
| `0xA1` | OUT | LED pattern | 0 | none |
| `0xA2` | IN | 0 | 0 | a NUL-terminated ASCII string: the firmware version |
| `0xA3` | IN | 0 | 0 | two bytes: major, then minor, of the *USB API* version |
| `0xA4` | IN | 0 | 0 | an ADC reading |

`0xA0` is the one to use as a sanity check: its string contains the word
`Apollo`. The USB API version from `0xA3` is what tells an
implementation whether a newer request exists; `0xA2` is a human-facing
string and should not be parsed for capability.

> *Provenance*: numbers and formats from Apollo's host package — MEDIUM.
> `0xA1` and `0xA4` are listed for completeness and are not sent by
> Reticle: there is no reason to blink a board that is not mine and no
> use here for a voltage.

---

## 4. The JTAG request set

Apollo does not expose TMS. It exposes a **TAP controller**: the
microcontroller holds the state, walks it on request, and shifts data in
and out of whichever shift state it is in. An implementation therefore
does not encode TMS sequences the way it would for an FTDI MPSSE; it
names states and lengths. This is the single biggest difference between
the two transports and the reason a byte buffer of MPSSE commands is the
wrong thing to hand an Apollo.

| `bRequest` | Dir | `wValue` | `wIndex` | Data stage |
|---|---|---|---|---|
| `0xB0` | OUT | 0 | 0 | none — zeroes the out buffer |
| `0xB1` | OUT | 0 | 0 | the bits to shift out of TDI |
| `0xB2` | IN | 0 | 0 | the bits that came in on TDO during the last scan |
| `0xB3` | OUT | bits to scan | flags | none |
| `0xB4` | OUT | TCK cycles | 0 | none — clock without leaving the state |
| `0xB5` | OUT | state number | 0 | none — walk the TAP there |
| `0xB6` | IN | 0 | 0 | one byte: the current state number |
| `0xB8` | IN | 0 | 0 | eight bytes of capability, see below |
| `0xBE` | OUT | 0 | 0 | none — release the JTAG pins |
| `0xBF` | OUT | 0 | 0 | none — take the JTAG pins |

### `0xB8`, the capability reply

Eight bytes, two little-endian 32-bit fields:

| Bytes | Meaning |
|---|---|
| 0–3 | the largest number of **bits** one `0xB3` may scan |
| 4–7 | quirk flags |

Quirk flags:

| Bit | Name | Meaning |
|---|---|---|
| 0 | flip bits in whole bytes | the shift engine moves each byte most-significant bit first; the host must bit-reverse every **whole** byte in both directions to compensate, leaving a trailing partial byte alone |
| 1 | always bit-bang | the firmware wants every scan done the slow way; set `wIndex` bit 1 on every `0xB3` |

A firmware old enough not to have `0xB8` stalls it. The fallback is 2048
bits and no quirks — 2048 bits is 256 bytes, which is the size of each of
the firmware's two buffers.

> *Provenance*: the request numbers, the `wValue`/`wIndex` assignments,
> the eight-byte layout, the quirk bits and the 256-byte buffers are all
> from Apollo's host package and firmware — MEDIUM throughout.

### `0xB5` and `0xB6`, the state numbers

The sixteen states of IEEE 1149.1 figure 6-1, numbered in the order the
standard draws them: the two stable states, then the DR column, then the
IR column.

| # | State | # | State |
|---|---|---|---|
| 0 | `Test-Logic-Reset` | 8 | `Update-DR` |
| 1 | `Run-Test/Idle` | 9 | `Select-IR-Scan` |
| 2 | `Select-DR-Scan` | 10 | `Capture-IR` |
| 3 | `Capture-DR` | 11 | `Shift-IR` |
| 4 | `Shift-DR` | 12 | `Exit1-IR` |
| 5 | `Exit1-DR` | 13 | `Pause-IR` |
| 6 | `Pause-DR` | 14 | `Exit2-IR` |
| 7 | `Exit2-DR` | 15 | `Update-IR` |

This is the same order as Reticle's own `jtag::TapState`, which is why
the mapping in `src/program/apollo.rs` is an index into a table that
names both sides rather than a translation — and why a test pins all
sixteen pairs, since "the same order" is exactly the kind of claim that
is true until someone reorders an enum.

Walking to `Test-Logic-Reset` is the reset: it is reached from any state,
known or unknown, and a part with an identification register loads
`IDCODE` on arriving there.

> *Provenance*: the numbering is from Apollo's host package — MEDIUM.
> The correspondence with IEEE 1149.1's own figure is this document's
> reading of why that numbering is what it is — LOW, and irrelevant: the
> numbers are what matter and they are listed.

### `0xB3`, a scan

`wValue` is the number of **bits** to shift, not bytes. `wIndex` is a
flag word:

| Bit | Meaning when set |
|---|---|
| 0 | advance the TAP on the last bit: TMS goes high with it, so the scan ends in `Exit1-DR` or `Exit1-IR` |
| 1 | bit-bang this scan rather than using the hardware shift engine |

With bit 0 clear the TAP stays in the shift state and a following scan
continues the same register, which is how a scan longer than the
firmware's buffer is split: every chunk but the last with bit 0 clear,
the last with it set.

The out buffer is **not** consumed or cleared by a scan, and the in
buffer is **overwritten** by every scan. So `0xB2` must be read before
the next `0xB3`, and `0xB0` sent before any `0xB1` that does not fill
the whole buffer.

> *Provenance*: from Apollo's host package and firmware — MEDIUM.

### Bit order

Bit *i* of the register, counting from the first bit through TDI, is
**bit `i mod 8` of byte `i / 8`** — least significant bit of the first
byte first, which is the order IEEE 1149.1 itself shifts in. The same
packing applies to the bits that come back from `0xB2`, so a 32-bit
`IDCODE` is four bytes that read directly as a little-endian `u32`.

For a scan of *N* bits, `0xB1` carries `ceil(N/8)` bytes and `0xB2`
returns `ceil(N/8)` bytes. Bits beyond *N* in the last byte are
meaningless in both directions.

When quirk bit 0 is set, every one of the first `N / 8` whole bytes is
bit-reversed by the host on the way out and on the way back; the
trailing `N mod 8` bits are left as they are.

> *Provenance*: this is the fact the reading of Apollo's host package was
> **least** sure of — the host code moves between a bit-level
> representation and bytes in more than one place, and a wrong answer
> here produces a stream of the right length that decodes to nonsense,
> which looks exactly like a dead board. It is written down as the
> above because that is what the firmware's buffers and the quirk's
> existence imply, and because IEEE 1149.1 is LSB-first. MEDIUM, and
> **flagged as the thing to settle on the bench first**: an identification
> register has bit 0 set by the standard and a known manufacturer field,
> so a single 32-bit read has a right answer that only one byte order
> produces.

---

## 5. Bringing up a link, and reading an identifier

The whole sequence, from a board in gateware mode to a 32-bit `IDCODE`:

1. Find the gateware device and its stub interface; send `0xF0` to the
   stub (§2). Wait for `0x1d50:0x615c`.
2. Open the debugger. Optionally `0xA0` to confirm it is Apollo and
   `0xA3` for the API version.
3. `0xB8` for the maximum scan length and the quirks; fall back to 2048
   bits and no quirks if it stalls.
4. `0xBF` — take the JTAG pins.
5. `0xB5` with `wValue` 0 — walk to `Test-Logic-Reset`. Every part with
   an identification register now has `IDCODE` selected, whatever its
   instruction register's width. **No instruction is shifted**, which is
   the point: shifting one means knowing the width, and the width is what
   is unknown about an unknown part.
6. `0xB5` with `wValue` 4 — walk to `Shift-DR`. The walk passes through
   `Capture-DR`, which is what loads the identifier into the shift path.
7. `0xB0` — clear the out buffer, so the 32 bits shifted in are zeros.
8. `0xB3` with `wValue` 32 and `wIndex` 1 — scan 32 bits and leave the
   shift state on the last one.
9. `0xB2` asking for 4 bytes — the identifier, least-significant bit
   first, which is a little-endian `u32`.
10. `0xB5` with `wValue` 1 — back to `Run-Test/Idle`.
11. `0xBE` — release the JTAG pins.

> *Provenance*: the sequence is assembled from the requests above, not
> taken from anywhere — MEDIUM, and it is what the implementation was
> written against.

---

## 6. Putting the board back

`0xC2` — host to device, recipient device, no data — tells Apollo to let
the FPGA have the USB port again. The gateware resumes advertising, the
debugger disappears from the bus, and the board comes back as whatever
it was before §2 was ever sent.

This is the polite end of a session and costs nothing, but it is not
required: unplugging, or a power cycle, has the same effect, and so does
nothing at all once the host stops talking — the board simply stays in
debugger mode.

> *Provenance*: the request number is from Apollo's host package —
> MEDIUM. A failure of it should be non-fatal: a board that has already
> gone cannot answer, and that is not an error worth failing a read
> over.

---

## 7. Requests this project does not send

The following exist and are documented here so that an implementer knows
to avoid them, not so they can be used. None of them appears anywhere in
Reticle's code.

| `bRequest` | What it does | Why not |
|---|---|---|
| `0xC0` | reconfigure the FPGA from its flash | throws away the configuration that is in the part. Not this project's board to reconfigure. |
| `0xC1` | force the FPGA offline | holds the FPGA out of the way for flash work. Reading an identifier does not need it, and it stops whatever the board was doing. |
| `0xA1` | set the LED pattern | harmless, and pointless: nobody is watching. |
| — | anything in Apollo's flash-bridge or DFU surface | **writes something that a power cycle does not undo.** That is the line this project does not cross on hardware it does not own. |

There is a related trap worth naming even though it is not a request.
Apollo's firmware **watches the bytes going into the out buffer** and
recognises the ECP5 configuration instructions `ISC_ENABLE` (`0xC6`) and
`ISC_DISABLE` (`0x26`) in order to track whether the FPGA is online. An
implementation that shifts an instruction register is therefore visible
to the firmware in a way it may not expect. Reading `IDCODE` after
`Test-Logic-Reset` shifts **no instruction at all**, so none of this
applies to what Reticle does; it would apply the moment anyone wrote an
ECP5 configuration sequence.

> *Provenance*: the request numbers and the `ISC_ENABLE`/`ISC_DISABLE`
> sniffing are from Apollo's host package and firmware — MEDIUM, and
> deliberately untested: the whole point of the list is that none of it
> was put on the wire.

---

## 8. What a configuration sequence would still need

Reading is the milestone and writing is not attempted. For the record,
what is missing is not the transport:

- an **ECP5 configuration sequence**. Its instruction register is 8 bits
  wide and the flow is a sequence of `ISC_*` and `LSC_*` instructions
  with the bitstream shifted through DR. None of it is in Reticle and
  none of it is in this document, because none of it was needed to read
  an identifier, and writing it down from memory would be exactly the
  kind of unverified claim this file exists to avoid;
- a **bitstream** for the part, which means an ECP5 fabric database and
  a place-and-route target. Reticle has neither;
- the honest accounting that `0xC1` would be wanted first, to stop the
  running gateware — and that is one of the requests §7 says this project
  does not send.

So the transport is done and the target is not. That is the right way
round: a transport with nothing to say is testable, and a configuration
sequence with nowhere to send it is not.

---

## 9. Verified

*This section is written after the fact and says what the bench
confirmed, changed or contradicted. Until it exists, nothing above is
HIGH except what §1 marks as read off the descriptors.*

Pending.
