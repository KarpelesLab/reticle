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

**This file was written before any of it was implemented, committed
before a line of the transport existed, and the implementation was
written from this file.** Apollo is BSD-3-Clause and copying it would
have been allowed; it was not copied, for the same reason Reticle's VHDL
library carries its own declarations and native bodies rather than
IEEE's text. A protocol one has written down is one one understands —
and §9 lists four places where it was wrong, three of which writing it
down first is the only reason the mistakes were found in a document rather
than in a board that would not answer. The fourth is the other kind: a
claim the bench appeared to confirm and did not.

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
without reading the code in between. **Four things in this document
were wrong, and §9 says which**; the body has been
corrected and each correction is marked, because a specification that
quietly absorbs its own errors teaches nothing about how far to trust
the rest of it.

---

## 1. Two devices, one board

A Cynthion enumerates as **one of two different USB devices**, depending
on which side of the board owns the port:

| | Vendor | Product | Manufacturer / product string | Serial string | Interfaces |
|---|---|---|---|---|---|
| **Gateware mode** | `0x1d50` | `0x615b` | "Cynthion Project" / whatever the gateware calls itself ("USB Analyzer") | the board's **flash UID** | the gateware's own, plus one *Apollo stub* |
| **Debugger mode** | `0x1d50` | `0x615c` | "Great Scott Gadgets" / "Cynthion Apollo Debugger" | the **microcontroller's own** serial number | a CDC-ACM pair and a DFU runtime interface — see §3 |

Both are the same physical port on the same board. The microcontroller
and the FPGA share it, and exactly one of them drives it at a time.

**The two modes report different serial numbers, and only one of them is
a name for the board.** In gateware mode the string is the board's
**flash UID** — on the board here `2a5a4adf30c460de`, sixteen hex digits,
sixty-four bits. That is the board's stable identity: it is a property of
the configuration flash soldered to it and it does not change with mode.
In debugger mode the string is the microcontroller's own,
`35L6H2CMGJJVCIBAEA3GCLAN74` on the same board — not the flash UID, not
derived from it, and not even the same length.

So the two *are* related, but only in a direction a host cannot use in
both modes. A board in gateware mode hands you its flash UID for nothing,
because it advertises it as its serial string. A board in debugger mode
does not: **no request in this document returns the flash UID**, and §7
says how Apollo's own tooling gets one and why that route is not taken
here. A host that identifies a board by serial number and then hands it
over therefore still cannot find it again by that name; it has to follow
the board across the re-enumeration some other way, such as by being the
debugger that was not on the bus a moment ago.

`0x1d50` is Openmoko's vendor identifier, which pid.codes administers;
Great Scott Gadgets hold `0x615b` and `0x615c` under it. Apollo's host
tooling also recognises a set of `0x1209:…` pid.codes test identifiers
for development builds, which a general implementation may want and
which this one does not use.

> *Provenance*: the vendor and product numbers came from Apollo's host
> package. Both were then **read off the attached board on 2026-09-25**,
> before and after a handover: `0x1d50:0x615b` "Cynthion Project" / "USB
> Analyzer", `bcdDevice` 1.04, serial `2a5a4adf30c460de`, and
> `0x1d50:0x615c` "Great Scott Gadgets" / "Cynthion Apollo Debugger",
> `bcdDevice` 1.04, serial `35L6H2CMGJJVCIBAEA3GCLAN74`. HIGH. The
> differing serial numbers were **not** anticipated when this document
> was written; see §9.
>
> That the gateware's serial string *is* the board's flash UID came later
> still, and not from Reticle: the board's owner ran Apollo's own `info`
> command against the same board in debugger mode and it printed
> `Flash UID: 2a5a4adf30c460de`, character for character the serial the
> board reports in gateware mode. HIGH for this board, by that
> measurement; MEDIUM as a property of every Cynthion, since one board is
> one board. §9 has the whole output and says what sending it cost.

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
**disappears from the bus and re-enumerates** as `0x1d50:0x615c` within
a second or so, which means the handle the request was sent on is dead
the moment it succeeds and the new device has to be found by a fresh
enumeration. Allow several seconds and poll. The transfer itself may
come back as an error rather than a success, because the device leaves
while the host is still looking at it; whether it worked is decided by
whether the debugger turns up, not by that return value.

Nothing about this is persistent. It changes who drives a USB port; it
does not touch the FPGA's configuration memory, the FPGA's flash or the
microcontroller's. A power cycle or a replug puts it back.

The FPGA stays configured and keeps running throughout. It simply no
longer has a USB host to talk to.

**It is, in practice, one-way for the rest of the session.** §6's
request is accepted but does not bring the gateware back on its own:
once a gateware has been told to stop advertising it does not start
again until the part is reconfigured or the board is power cycled. A
host that performs this handover should say so to whoever is holding the
board rather than imply it can undo it.

> *Provenance*: the request number, direction and recipient came from
> Apollo's host package and were then **sent to the attached board**,
> which re-enumerated as the debugger within a second; HIGH. The
> sideband-advertisement mechanism is the reading that explains why the
> request lives on the gateware and why stopping it is enough — LOW; it
> is an explanation, not an observation, and the protocol works the same
> if the mechanism is something else. That the handover does not undo
> itself is HIGH and was learned the plain way: §6's request was sent,
> was accepted, and the board stayed a debugger.

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

In debugger mode the board presents one configuration whose interfaces
have **nothing to do with this protocol**. On the board here they are:

| # | Class | Subclass | What it is |
|---|---|---|---|
| 0 | `0x02` | `0x02` | CDC communications — the control half of a serial port |
| 1 | `0x0A` | `0x00` | CDC data — the data half |
| 2 | `0xFE` | `0x01` | DFU runtime |

The serial port is the microcontroller's console and the DFU interface
is how its own firmware is updated; **neither is touched here**, and the
DFU one is exactly the sort of surface §7 exists to keep away from.

This layout matters for one practical reason: **Linux binds `cdc_acm` to
interfaces 0 and 1**, so a debugger on the bus already has a kernel
driver on it. That does not get in the way, because:

**Every request in §3, §4 and §6 is a control transfer on endpoint zero
with recipient *device*.**

```
bmRequestType = 0x40   host to device, vendor, recipient device
bmRequestType = 0xC0   device to host, vendor, recipient device
```

That recipient matters. A device-recipient control transfer belongs to
no interface, so nothing has to be claimed and nothing has to be
detached, `cdc_acm` included: the whole of §3 and §4 was performed on
this board with the kernel's serial driver still bound to interfaces 0
and 1, and not one transfer was refused.

The one request that *is* interface-recipient — §2's handover — is sent
to the **gateware**, where the stub interface has no driver on it.

> *Provenance*: the request type came from Apollo's host package and was
> then **used against the board** for everything in §3 and §4; HIGH. The
> interface table was read off the board's descriptors; HIGH. This
> document originally said debugger mode presents "one vendor-specific
> interface", which is wrong — see §9.

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

**None of them returns the board's flash UID**, and that is not an
omission from this table: the identification surface really does stop at
`0xA4`, and nothing in it answers with the sixteen hex digits a gateware
advertises as its serial number. Apollo's `info` command prints a flash
UID anyway; §7 says how, and why an implementation that wants to stay on
the read-only side of a board it does not own cannot copy it.

> *Provenance*: numbers and formats came from Apollo's host package.
> `0xA0`, `0xA2` and `0xA3` were then **issued against the board** and
> answered exactly as described — `Apollo Debug Module`, `v1.1.1`, and
> the two bytes `1`, `2`. HIGH for those three. `0xA1` and `0xA4` are
> listed for completeness and were **not sent**: there is no reason to
> blink a board that is not mine and no use here for a voltage. MEDIUM.

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

**Most firmware stalls `0xB8`, including every Apollo release to date**,
so the fallback is the usual path and not the exception. It is 2048 bits
and no quirks — 2048 bits is 256 bytes, which is the size of each of the
firmware's two buffers.

> *Provenance*: the request number and the eight-byte layout are **the
> host package's alone**: `apollo_fpga/jtag.py` defines
> `REQUEST_JTAG_GET_INFO = 0xb8` and wraps the transfer in a
> `try`/`except IOError`, and `0xb8` does not appear in the firmware's
> request enumeration at all (§7 quotes it in full). So the layout is
> MEDIUM and has never been read off a board, here or anywhere this could
> check. `0xB8` **was** issued against the board and **stalled**;
> Reticle used the fallback, and because the fallback's numbers are what a
> firmware would report the two were indistinguishable until the firmware
> source was read. §9 item 4 is the correction, and
> `usb::Debugger::capability_reported` is how the two are told apart now.
>
> The *numbers* are HIGH by a different route: 2048 bits is the buffer
> size in the firmware, and a 98 473-byte bitstream went through in 385
> consecutive scans of that size. The `wValue`/`wIndex` assignments and the
> 256-byte buffers are HIGH. Nine of the ten requests above were
> exercised; `0xB4` was not until the configuration sequence used it, and
> is now HIGH. Neither quirk bit was set on this board, so both stay
> MEDIUM: the code that handles them is checked against a model and has
> never met a firmware that needs it.

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

> *Provenance*: the numbering came from Apollo's host package. Two of
> the numbers were then **checked against the board**: after walking to
> `1`, `0xB6` answered `1`, and after walking to `4` it answered `4`.
> Number `4` is corroborated by the identifier coming back correctly,
> which it could not have done from any other state. HIGH for `0`, `1`
> and `4`; MEDIUM for the other thirteen, which were never asked for.
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

> *Provenance*: from Apollo's host package and firmware, and then
> **checked on the board in the one way that can tell the difference**.
> A 32-bit identifier read as a single scan and the same identifier read
> as two 16-bit scans — the first without bit 0, the second with it —
> gave the same value, `0x21111043`. A scan that ended the shift
> whether or not the flag was set would have made the second half read
> a freshly captured register instead of the rest of the first; a scan
> that never ended it would have left the walk out of step. HIGH for
> bit 0 and for both buffer lifetimes. MEDIUM for bit 1, which was never
> set here.

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

> *Provenance*: this was the fact the reading of Apollo's host package
> was **least** sure of — the host code moves between a bit-level
> representation and bytes in more than one place, and a wrong answer
> here produces a stream of the right length that decodes to nonsense,
> which looks exactly like a dead board. It was written down as above
> because that is what the firmware's buffers and the quirk's existence
> imply, and because IEEE 1149.1 is LSB-first, and it was flagged as the
> thing to settle on the bench first.
>
> It was settled there, and it was right. The four bytes `0xB2` returned
> for a 32-bit scan after `Test-Logic-Reset` were `43 10 11 21`, which
> read as a little-endian `u32` give `0x21111043`: bit 0 set, as IEEE
> 1149.1 requires of an identification register, and a JEDEC
> manufacturer field of `0x021`, which is Lattice — on a board whose FPGA
> is a Lattice ECP5. No other arrangement of those four bytes produces a
> well-formed identifier of any manufacturer. HIGH, by measurement.

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
> taken from anywhere. It is what the implementation was written
> against, and it **ran in exactly this order on the attached board**,
> twice, producing `0x21111043` both times. HIGH.

---

## 6. Putting the board back

`0xC2` — host to device, recipient device, no data — tells Apollo that
it *may* let the FPGA have the USB port again.

**That is all it does, and on its own it is not enough.** It was sent to
this board, twice, and was accepted both times with no error; the board
stayed a debugger. Permission is not the same as a request: the port
moves when the FPGA asks for it, and a gateware that has been told to
stop advertising (§2) does not start again by itself. Apollo's own
tooling pairs this request with a reconfiguration of the FPGA, which is
`0xC0` and which §7 says this project does not send.

So the honest way to end a session is: send `0xC2`, and tell whoever is
holding the board that a replug or a power cycle brings the gateware
back. Nothing is lost by leaving it: the FPGA is still configured with
whatever it was configured with, and which side owns a USB port is not a
state of the part.

> *Provenance*: the request number came from Apollo's host package. Its
> effect — accepted, and insufficient — is **HIGH and was measured**: the
> transfer returned success and `lsusb` showed `0x1d50:0x615c` still
> there afterwards, both immediately and later. The explanation of *why*
> it is insufficient is LOW; the observation is not. A failure of the
> request should be non-fatal: a board that has already gone cannot
> answer, and that is not an error worth failing a read over.

---

## 7. Requests this project does not send

The following exist and are documented here so that an implementer knows
to avoid them, not so they can be used. None of them is ever sent. They
do appear in Reticle's code, in exactly one place and for exactly that
reason: `apollo::NOT_SENT` is this table, so that a test can assert the
crate never reaches one.

| `bRequest` | What it does | Why not |
|---|---|---|
| `0xC0` | reconfigure the FPGA from its flash | throws away the configuration that is in the part. Not this project's board to reconfigure. |
| `0xC1` | force the FPGA offline | holds the FPGA out of the way for flash work. Reading an identifier does not need it, and it stops whatever the board was doing. |
| `0xA1` | set the LED pattern | harmless, and pointless: nobody is watching. |
| `0xA4` | read an ADC channel | harmless, and there is no use here for a voltage. |
| — | anything in Apollo's flash-bridge or DFU surface | **writes something that a power cycle does not undo.** That is the line this project does not cross on hardware it does not own. |

The same list is in the code as `apollo::NOT_SENT`, with a test asserting
that nothing the crate compiles or issues appears in it — the same guard
`gowin::FORBIDDEN` gets. A promise about what is never put on the wire is
worth more as an assertion than as a paragraph.

### The flash UID, and why reading it is on the wrong side of that line

This is the one thing a reader of §1 will want that is not here, so it is
worth saying exactly where it went. **There is no vendor request that
returns the flash UID**, and that is now a statement about the firmware's
own source rather than an inference. Apollo's request set is one `enum` in
`firmware/src/vendor.c` and it is, in full:

| Range | Requests |
|---|---|
| information | `0xA0` `GET_ID`, `0xA1` `SET_LED_PATTERN`, `0xA2` `GET_FIRMWARE_VERSION`, `0xA3` `GET_USB_API_VERSION`, `0xA4` `GET_ADC_READING` |
| JTAG | `0xB0` `CLEAR_OUT_BUFFER`, `0xB1` `SET_OUT_BUFFER`, `0xB2` `GET_IN_BUFFER`, `0xB3` `SCAN`, `0xB4` `RUN_CLOCK`, `0xB5` `GOTO_STATE`, `0xB6` `GET_STATE`, `0xB7` `BULK_SCAN`, `0xBE` `STOP`, `0xBF` `START` |
| programming | `0xC0` `TRIGGER_RECONFIGURATION`, `0xC1` `FORCE_FPGA_OFFLINE`, `0xC2` `ALLOW_FPGA_TAKEOVER_USB` |
| debug SPI (`_BOARD_HAS_DEBUG_SPI` only) | `0x50` `DEBUG_SPI_SEND`, `0x51` `DEBUG_SPI_READ_RESPONSE`, `0x52` `FLASH_SPI_SEND`, `0x53` `TAKE_FLASH_LINES`, `0x54` `RELEASE_FLASH_LINES` |
| other | `0xE0` `GET_RAIL_VOLTAGE` (declared, not implemented), `0xEE` `GET_MS_DESCRIPTOR` |

**There is nothing in `0xA5`..`0xAF`**, nothing that mentions a UID, and
the whole firmware tree has no reference to the flash's unique identifier
at all. Two more things fall out of reading it, and both matter here:

- `0xB7` is declared and has **no case in the dispatcher**, so it stalls.
  `0xB8` is not in the firmware's enumeration at all — it is the *host*
  package's `REQUEST_JTAG_GET_INFO` — which is why §4's capability reply
  is a fallback in practice and not a measurement. §9 corrects that.
- the `0x50`..`0x54` block is compiled only for a board that defines
  `_BOARD_HAS_DEBUG_SPI`, which among the Cynthion revisions is the
  SAMD21 one. A Cynthion r1.4 has a SAMD11 and **stalls all five**, so the
  one vendor request that can put an arbitrary opcode on the configuration
  flash does not exist on this board. That is luck rather than design and
  it changes nothing about the list above.

> *Provenance*: `greatscottgadgets/apollo` at `main`, read from a checkout:
> `firmware/src/vendor.c` for the enumeration and the dispatcher,
> `firmware/src/boards/*/apollo_board.h` for `_BOARD_HAS_DEBUG_SPI`, and
> `apollo_fpga/__init__.py` for the host's mirror of it. HIGH, and it is a
> *negative* claim checked the only way a negative claim can be: the whole
> tree searched for the thing that is not in it.
>
> **Checked again on 2026-09-26**, because a negative claim about somebody
> else's firmware goes stale when they release. The checkout above is from
> the 1.1.1 tree (its `CHANGELOG.md` tops out at `[1.1.1] - 2024-11-19`,
> which is the firmware this board reports), so it was compared against the
> *current* `main` of the host package, fetched fresh: the request
> constants are still exactly `0xA0`..`0xA4`, `0xC0`..`0xC2` and `0xF0`,
> `read_flash_uid` is still in `apollo_fpga/ecp5.py` and still built out of
> `_enter_background_spi` plus the flash opcode `0x4B`, and there is still
> nothing anywhere that asks the microcontroller for a UID. So the answer
> to "which vendor request returns the flash UID" remains **none of them**,
> on two versions and from both sides of the protocol.

What Apollo's `info` command does instead is:

1. `0xC1` — force the FPGA offline, so the flash bridge can have the
   pins;
2. take the JTAG pins and shift the ECP5 instruction
   `LSC_ENTER_BACKGROUND_SPI` (`0x3A`) with its unlock operand, which
   turns the part into a pass-through to the board's configuration SPI
   flash;
3. issue the JEDEC `READ_UID` opcode (`0x4B`) to that flash and read the
   answer back.

The read itself takes nothing away — `READ_UID` is as read-only as an
`IDCODE` — but the **route** to it is two of the rows in the table above:
`0xC1`, and the flash bridge. It also has a visible cost, which is how the
provenance below came to be known: forcing the FPGA offline stops the
gateware that was running.

So Reticle does not read a flash UID. It reads the one a board in gateware
mode advertises for free, and for a board already in debugger mode it says
so rather than pretending the identifier is unavailable for some vaguer
reason. `reticle program --device <flash-uid>` against an attached
debugger is an error that names `0xC1` and points here.

> *Provenance*: a reading of Apollo's host package
> (`apollo_fpga/__init__.py` and `apollo_fpga/ecp5.py`, BSD-3-Clause),
> consulted to answer one question — "is there a request for this?" — and
> then closed. The answer was no, and the three steps above are what is
> there instead: MEDIUM, and **not** put on the wire from here, so nothing
> confirms the operand or the opcode. That `0xC1` really does stop the
> running gateware is HIGH, and was learned the expensive way by the
> board's owner: their `apollo info` needed `--force-offline` to print a
> UID at all, and it left the board a debugger.

### `0xC1` is a JTAG scan, which is why it is not needed

There is a related fact that turned out to matter a great deal, because it
is what lets this project configure an ECP5 without sending anything on
§7's list.

`0xC1` `FORCE_FPGA_OFFLINE` is not a pin, a reset line or a special
power-up path. `firmware/src/fpga.c` implements it as **four JTAG
operations**: walk to `TEST_LOGIC_RESET`, walk to `SHIFT_IR`, shift the
ECP5 instruction `ISC_ENABLE` (`0xC6`), walk to `SHIFT_DR`, shift `0x00`.
That is exactly step 2 of any ECP5 SRAM configuration sequence, including
the one in `src/program/lattice.rs`.

So a host that is going to configure the part **does not need the
request**: it shifts the instruction itself, through `0xB1` and `0xB3`
like every other scan, and the part goes offline because that is what
`ISC_ENABLE` does to it. `reticle program` therefore configures a Cynthion
without `0xC1` ever appearing on the wire, and §7's list stands unchanged
rather than being quietly relaxed to admit it. The distinction is real and
not a technicality: what §7 refuses is *sending Apollo a request whose
effects this project has not accounted for*, and an instruction shifted
deliberately, as part of a sequence whose every step is written down, is
not that.

Apollo's firmware also **watches the bytes going into the out buffer** and
recognises `ISC_ENABLE` and `ISC_DISABLE` (`0x26`) in order to track
whether the FPGA is online. An implementation that shifts an instruction
register is therefore visible to the firmware, which is worth knowing
before doing it. It was done, and nothing went wrong: §9 has the run.

> *Provenance*: `firmware/src/fpga.c` and `firmware/src/jtag.c` in
> `greatscottgadgets/apollo` at `main`, read from a checkout. HIGH for what
> the code does. That shifting `ISC_ENABLE` from the host has the same
> effect the request would is HIGH **by measurement**: it is what §9's
> configuration run did, and the status register showed `ISC_ENABLE` set
> afterwards.

---

## 8. Configuring the part

This section said a configuration sequence was not attempted, and listed
three things it would need. All three arrived, and the part has been
configured over this transport. What is here now is what that takes *of
the transport*; `src/program/lattice.rs` has the sequence and
`docs/programming.md` the run.

Nothing in §3 or §4 changed to make it possible. A configuration is the
same requests a `IDCODE` read uses, in greater quantity:

- the **instruction register is 8 bits**, so each instruction is one
  `0xB1` of one byte and one `0xB3` of eight bits, walked to `Shift-IR`
  by `0xB5` first;
- the **bitstream is one long DR scan**, which `0xB3` cannot do in one
  request: it is split into chunks of at most §4's scan limit, with the
  advance-state flag clear on all but the last so the TAP stays in
  `Shift-DR`. That path was already exercised by reading a 32-bit
  register as two 16-bit scans (§9), which is why it was known to work
  before a bitstream went through it;
- an all-zero chunk is a `0xB0` `CLEAR_OUT_BUFFER` and no data stage
  rather than a `0xB1` of zeros, which on a nearly-empty bitstream is
  most of them;
- the waiting between steps is **polling, not sleeping**: the ECP5's own
  `LSC_CHECK_BUSY` read through `0xB3`, counted in reads. A plan cannot
  be told how fast TCK is, and Apollo is never told either, so a number
  of TCK cycles is not a duration.

Two things about the transport that only a long write reveals:

- **the pins must be held for the whole sequence.** `0xBE` `JTAG_STOP`
  does not merely stop driving them: `firmware/src/boards/cynthion_d11/jtag.c`
  restores the pin multiplexing, which on this board is a UART. A part in
  configuration mode whose TAP is clocked by something else between two
  steps is a part in an unknown state. `usb::Debugger::jtag_session` is
  what holds them.
- **`0xC1` is not needed**, for the reason §7 now gives: the request is
  itself a scan of `ISC_ENABLE`, and a configuration sequence shifts that
  instruction anyway.

What the transport cost, measured on this board: 98 473 bytes of
compressed bitstream in 0.5 s, and 238 282 bytes in 1.1 s, over a
full-speed control-transfer link with a 2048-bit scan limit.

---

## 9. Verified

On **2026-09-25** a Great Scott Gadgets Cynthion attached to this
machine was taken from its analyzer gateware to the Apollo debugger and
its ECP5's identifier was read, by Reticle's own code, written from the
sections above and from nothing else. The run is
`tests/program_apollo.rs::read_the_ecp5_idcode`. It was performed
several times from two different starting states: the first from
gateware mode, which exercised §2's handover, and the rest from
debugger mode, which did not. Every run read the same identifier.

What the board said:

```
Cynthion 2a5a4adf30c460de is in gateware mode
Apollo on 35L6H2CMGJJVCIBAEA3GCLAN74 (the gateware was asked to give up the USB port)
  identifier: Apollo Debug Module
  firmware: v1.1.1
  USB API: 1.2
  max scan 2048 bits, quirks 0x00000000

IDCODE 0x21111043
IDCODE 0x21111043: manufacturer 0x021 (Lattice), part number 0x1111, version 2 — LFE5U-12F (or LAE5U-12F)
the TAP rests in RunTestIdle
read again in 16-bit chunks: 0x21111043
after asking for Shift-DR the firmware says Some(ShiftDr)
```

Later the same day the same read was performed **from the command line**,
which is what took the transport from something only a test could reach to
something the tool can be pointed at. `reticle program --probe --device
<serial>` now dispatches on which kind of adapter carries that serial
before it opens anything, so an FTDI cable and a Cynthion can be attached
at once and each is reached by name. Against the board here, in debugger
mode:

```
$ reticle program --list
35L6H2CMGJJVCIBAEA3GCLAN74  Cynthion, Apollo debugger  [the microcontroller's own USB serial number]

$ reticle program --probe --device 35L6H2CMGJJVCIBAEA3GCLAN74
adapter 35L6H2CMGJJVCIBAEA3GCLAN74 (Cynthion, Apollo debugger)
Apollo on 35L6H2CMGJJVCIBAEA3GCLAN74
  identifier: Apollo Debug Module
  firmware: v1.1.1
  USB API: 1.2
  max scan 2048 bits, quirks 0x00000000
IDCODE 0x21111043: manufacturer 0x021 (Lattice), part number 0x1111, version 2 — LFE5U-12F (or LAE5U-12F)
nothing further was read and nothing was written: reading a status register means shifting some vendor's instruction, and there is no ECP5 configuration sequence in this crate
```

Same identifier, same firmware, through the same requests; the only new
thing on the wire was nothing at all. The board was already a debugger, so
this run did not perform §2's handover and did not send §6's `0xC2`.

### The part was configured, over this transport

Later the same day the transport carried a bitstream. **This is the first
time this project has configured anything Lattice.** Three files went in,
in this order, each into the volatile configuration SRAM and nothing else:

| File | Bytes | Where it came from | Status afterwards |
|---|---|---|---|
| `analyzer.bit` | 238 282 | Great Scott Gadgets' own build for this board | `0x00200100` — `DONE`, no fault |
| `leds_alternate.bit` | 98 473 | built by Reticle from `testdata/fpga/cynthion/leds_alternate.v` | `0x00200100` |
| `leds.bit` | 98 473 | built by Reticle from `testdata/fpga/cynthion/leds.v` | `0x00200100` |

The first was the transport's own test: a bitstream nobody here wrote,
which the board is configured with every time it is plugged in, so a
failure would have been the sequence's and not the compiler's. The other
two came out of `reticle fpga`; `docs/fpga-trellis.md` is their story.

What was on the wire, in order, and none of it is new to §3 or §4:
`0xBF` once; then per step `0xB5` walks, `0xB1`/`0xB0` and `0xB3` scans,
`0xB4` run-clocks, and `0xB2` reads for each status register; then `0xBE`.
**No `0xC0`, no `0xC1`, no `0xC2`, no `0xA1`, no `0xA4`, nothing in
`0x50`..`0x54`, and nothing near the DFU interface.**

What the part said along the way, which is the evidence that each step did
what it was supposed to:

```
status before: 0x00200100 (DONE)
after LSC_REFRESH, IDCODE 0x21111043
status after ISC_ENABLE: 0x00200f10 (DONE, ISC_ENABLE)
status after ISC_ERASE: 0x00200e10 (ISC_ENABLE)
98473 bytes of bitstream shifted in 0.5 s
status after ISC_DISABLE: 0x00200100 (DONE)
```

`ISC_ENABLE` appearing in the status register is the measurement behind
§7's claim that shifting the instruction does what `0xC1` would. `DONE`
going low after `ISC_ERASE` and high again at the end is the part saying it
really did clear its configuration and really did take a new one.

### 2026-09-26: the same transport, a corrected bitstream

The configuration sequence was run again, unchanged, with a `leds.bit`
rebuilt after a defect was found in the *compiler* rather than in this
protocol — `docs/fpga-trellis.md` has that story. It is recorded here only
because it is another run of everything above:

```
$ reticle program --list
35L6H2CMGJJVCIBAEA3GCLAN74  Cynthion, Apollo debugger  [the microcontroller's own USB serial number]

$ reticle program --device 35L6H2CMGJJVCIBAEA3GCLAN74 leds.bit
status before: 0x00200100 (DONE)
after LSC_REFRESH, IDCODE 0x21111043
status after ISC_ENABLE: 0x00200f10 (DONE, ISC_ENABLE)
status after ISC_ERASE: 0x00200e10 (ISC_ENABLE)
98474 bytes of bitstream shifted in 0.5 s
status after ISC_DISABLE: 0x00200100 (DONE)
```

Same requests, same statuses, same timing. The board was already a
debugger, so no handover and no `0xC2`. Note what the status register is
and is not evidence of: it was `0x00200100` after the run that lit nothing
and `0x00200100` after this one, so `DONE` says the part took a bitstream
and says nothing about what the bitstream does.

Also run, and worth recording because it is the point of the flash UID:

```
$ reticle program --probe --device 2a5a4adf30c460de
error: no attached adapter reports the serial number `2a5a4adf30c460de`. It is
spelled like a Cynthion's flash UID, which is what a board advertises as its USB
serial number *in gateware mode* — a board already in Apollo debugger mode reports
the microcontroller's serial number instead, and Reticle will not read a debugger's
flash UID, because Apollo reads it by forcing the FPGA offline (vendor request 0xc1)
and driving the board's configuration flash over JTAG, which this project does not
do. Name the debugger instead: 35L6H2CMGJJVCIBAEA3GCLAN74
```

That is the flash UID of the board that was in front of it, and the answer
is an explanation rather than "no such device". What is **not** shown by
that run is the case it exists for — a board in gateware mode, whose USB
serial string *is* that flash UID and therefore matches. This board has
stayed a debugger since 2026-09-25 for the reasons in *What the board is
now*, so the matching half was last exercised then.

### Confirmed

- §1's two identities, and the stub interface's descriptor triple.
- §2: `0xF0` to the stub interface hands the port over, and the board
  re-enumerates as the debugger in well under a second.
- §3: every request there is a device-recipient control transfer and
  none of them needs an interface claimed, with `cdc_acm` bound
  throughout. `0xA0`, `0xA2`, `0xA3` answer as described.
- §4: the state numbers `0`, `1` and `4`, with `0xB6` reading back what
  `0xB5` was given; `0xB3`'s advance-state flag, proved by reading one
  32-bit register as two 16-bit scans and getting the same value; the
  buffer lifetimes. **Not** `0xB8` — see the correction below.
- §4's **bit order**, which was the flagged risk and was right. It was
  then confirmed a second and much harder way: the same packing carried
  238 282 bytes of somebody else's bitstream into the part, and a part
  whose bits arrive in the wrong order does not assert `DONE`.
- §4's **chunking**, which the 16-bit experiment showed and a 98 473-byte
  scan then leaned on 385 times in a row.
- §5's sequence, run in order, twice, with the same result.
- §8: the whole configuration sequence, three times, with the status
  register agreeing at every step.
- §7's account of `0xC1`: `ISC_ENABLE` shifted from the host puts the part
  in configuration mode, which the status register shows.

### Wrong, and corrected above

1. **§1 and §3: the two modes report different serial numbers**, and
   this document did not say so because it had not occurred to the
   author that they would. The gateware reports `2a5a4adf30c460de` and
   the microcontroller reports `35L6H2CMGJJVCIBAEA3GCLAN74`. Reticle's
   first implementation matched on the serial number across the
   handover and could not have found the board it had just handed over;
   it now follows the board by being the debugger that was not there
   before. This is the error that a document *not* written from the
   descriptors would have carried into the field.

   **The correction itself then needed correcting.** This section said
   the two strings were *unrelated*, which was an inference from their
   looking nothing alike, and it was wrong. The board's owner ran
   Apollo's own `info` command against the same board, in debugger mode,
   and one device printed both:

   ```
   Hardware: Cynthion r1.4
   Product: Cynthion Apollo Debugger
   Serial number: 35L6H2CMGJJVCIBAEA3GCLAN74
   Vendor ID: 1d50   Product ID: 615c   bcdDevice: 0104
   Firmware version: v1.1.1
   USB API version: 1.2
   ADC reading: 3214
   Flash UID: 2a5a4adf30c460de
   ```

   `2a5a4adf30c460de` is exactly what the board reports as its USB serial
   number in gateware mode. So the gateware's serial string **is** the
   board's flash UID: the board's stable identity, the same in both modes,
   and the one of the two names that means "this board" rather than "this
   board while it is in this mode". §1 now says which string is which kind
   of thing.

   What does *not* change is the conclusion drawn from the error. A host
   still cannot follow a board across the handover by name, because a
   debugger does not report the flash UID and there is no request that
   returns it — §7 says what Apollo does instead and why this project
   stops there. The heuristic stays, and `src/program/usb.rs` now says in
   as many words that it is one, and that it would pick the wrong board if
   two were handed over at once.

   Two things to note about that output, since it is evidence and evidence
   has a cost. It was produced by **Apollo's tooling, not by Reticle**: the
   `ADC reading` is request `0xA4`, and the `Flash UID` needed
   `--force-offline`, which is `0xC1` followed by the flash bridge. Both
   are on §7's list and neither has ever been sent from here. And
   `--force-offline` stops a running gateware, so that run is one of two
   reasons this board has stayed a debugger; *What the board is now*, below,
   says what the other is and why they cannot be told apart after the fact.

2. **§3: debugger mode is not "one vendor-specific interface".** It is a
   CDC-ACM pair and a DFU runtime interface, and Linux binds `cdc_acm`
   to two of the three. The protocol is unaffected — every request is
   device-recipient — but the claim was false and would have sent an
   implementer looking for an interface to claim.
3. **§6 overpromised.** `0xC2` was written up as the request that puts
   the board back. It is accepted and it does not put the board back;
   §6 now says what it actually does.
4. **§4's `0xB8` was never answered by this board, and this document
   claimed its reply had been confirmed.** It had not. `0xB8` is not in the
   firmware's request enumeration at all — it is defined by Apollo's *host*
   package, which wraps it in a `try`/`except` for exactly that reason —
   and a Cynthion r1.4 running v1.1.1 stalls it. Reticle fell back to
   `DEFAULT_MAX_SCAN_BITS` and no quirks, which is what §4 says the
   fallback is, **and those are indistinguishable in a printed line from a
   firmware that reported exactly that**. So every run above that said
   `max scan 2048 bits, quirks 0x00000000` was reporting Reticle's own
   assumption as though it were a measurement.

   Two things changed. `usb::Debugger::capability_reported` now says which
   of the two it is, and the line prints it:

   ```
   max scan 2048 bits, quirks 0x00000000 (assumed; this firmware has no case for that request)
   ```

   And §4 now says the reply is a layout read from the host package and not
   from any board. The layout may well be right — a firmware that
   implements it would have to match its host — but nothing here has seen
   one. The *consequence* is nil, because 2048 bits is what the firmware's
   buffer actually is and both quirks are off, which the configuration run
   above then demonstrated across 385 consecutive scans.

   This is the second time in this document that a thing which "obviously
   worked" turned out to be a default that happened to be right. The first
   was the serial numbers in item 1.

### Still unverified

- `0xA1`, `0xA4`, `0xC0`, `0xC1`: **never sent from here.** The board's
  owner's own run of Apollo's tooling sent `0xA4` and `0xC1`, and what they
  answered is in item 1 above; that is a measurement of the board, not of
  Reticle, and nothing in this crate can reach either request. `0xB4` has
  left this list: the configuration sequence in §8 uses it.
- `0xB8`'s eight-byte layout, which no board has ever answered — see item 4
  above. This is not a deliberate omission like §7's, it is a claim that
  was believed to be checked and was not.
- The flash-UID sequence in §7: a reading of Apollo's host package and
  nothing more. The operand of `LSC_ENTER_BACKGROUND_SPI` and the JEDEC
  opcode have not been on a wire from here and will not be.
- Both quirk bits: this firmware reports neither, and it does not report
  anything, so the code that handles them has only ever been checked
  against a model.
- Eleven of the sixteen state numbers. The configuration sequence walks to
  `Test-Logic-Reset`, `Run-Test/Idle`, `Shift-IR` and `Shift-DR`, and reads
  one back.
- Everything in §7, deliberately: the point of that list is that none of
  it was put on the wire.
- The PROGRAM-button route in §2, which was not needed.

### What the board is now

**In debugger mode, with the FPGA configured by Reticle and not by its
flash.** That is a change from what this section used to say, and it is
worth being exact about it.

The configuration SRAM now holds `leds.bit`, built from
`testdata/fpga/cynthion/leds.v`, which drives six pads to zero and should
light the board's six FPGA LEDs. **The analyzer gateware has been
displaced.** A power cycle or a replug reloads it from the board's flash,
which nothing here touched: no flash was written, on either chip, and the
one JTAG instruction that could reach one is named in
`program::lattice::NOT_SHIFTED` with a test asserting no plan carries it.

The copy in it as of 2026-09-26 is the **rebuilt** one, 61 configuration
bits rather than 60, after the bank-rail defect in
`docs/fpga-trellis.md`. Whether the LEDs are on has not been reported
since that load.

Worth recording, because it was measured rather than assumed: when the
first configuration of the day started, the part's status register read
`0x00200000` — `DONE` **clear**, the configuration SRAM empty. So the
analyzer was not running at that point either, and the LEDs were dark. What
emptied it was one of the two things named below, most likely the owner's
`apollo info --force-offline`, which §7 explains is a shifted
`ISC_ENABLE`. The first `LSC_REFRESH` of the configuration sequence then
reloaded the part from flash — `DONE` went high — before `ISC_ERASE`
cleared it again for the bitstream.

Two separate things asked this board to stop driving that USB port, and
they are worth keeping apart because only one of them is Reticle's. §2's
handover, `0xF0`, was sent from here, and on its own it accounts for the
board staying a debugger: §2 says a gateware told to stop advertising does
not start again until the part is reconfigured or power cycled. The board's
owner's `apollo info --force-offline` — `0xC1`, which is on §7's list and
has never been sent from here — would have had the same effect on its own.
Which of the two is load-bearing cannot be told apart after the fact, and
nothing rests on the answer. Every identifier read since, including every
command-line one, has found the board already a debugger and performed no
handover at all.
