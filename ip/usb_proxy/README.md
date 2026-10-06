# A USB proxy, and why the PC has to be the one that enumerates

`usb_proxy` is the half of a USB hub that **forwards**.
[`ip/usb_hub`](../usb_hub/README.md) is the other half: a hub's control
endpoint, which a PC's own `hub` driver binds to, and which reports a port with
a device on it. §8 of that document is the kernel log of what happened next —
the PC powered the port, was told something was attached, reset it, and then
`device descriptor read/64, error -71` four times and `unable to enumerate USB
device`. Correct, because nothing carried a packet from one bus to the other.

This is what carries it. The block's own one-line claim is the one thing worth
reading twice: **the device the PC enumerates through our hub is the real
device, with its own descriptors, at an address the PC itself assigned.**
Nothing of ours is in what the PC reads, because nothing of ours is in the
path except timing.

This document is laid out the way
[`ip/usb_hub/README.md`](../usb_hub/README.md) is, and for the same reason: the
facts in it have very different provenance. Some are read out of a published
specification. Some are a reading of what a *host* does, which only a host can
confirm. §1 says which is which and every claim below is marked.

Nothing was transcribed from anybody's implementation. The encodings are read
out of published documents; the state machines, the toggle arrangement and the
addressing decision are this repository's own.

---

## 1. Confidence, and what it is based on

- **HIGH** — stated in the *Universal Serial Bus Specification, Revision 2.0*
  ("USB 2.0" below), the *UTMI+ Low Pin Interface Specification, Revision 1.1*
  ("ULPI") or the *USB334x Data Sheet* DS00002646A, with the section named so
  it can be checked without reading any code.
- **MEDIUM** — a reading of one of those which it does not state in so many
  words, or a statement about what a host driver does that is consistent with
  everything observed.
- **CHECKED** — produced by the simulation in `tests/ip_library.rs`, with the
  test that produced it named. §10 is that list.
- **MEASURED** — a Cynthion r1.4 did it, with the output quoted. §8 is that
  list, and it is the one that decides whether this block is a proxy.
- **LOW** — inference that explains the rest, with no way to check it here.

---

## 2. The architectural decision: pass-through addressing

A proxy has to answer one question before any of the rest: **who assigns the
downstream device its address?**

`ip/usb_host_ulpi` came with an answer that was right for the round that built
it and wrong for this one. `usb_host_enum` is a complete USB 2.0 §9.1.2
enumerator: it debounces the attach, drives the bus reset, reads the device
descriptor twice, assigns **address 1**, reads the configuration descriptor and
sends SET_CONFIGURATION. It did that on this board, and the previous round's
console printed stage 17 — `E_UP` — with a GreatFET in the socket at the same
moment the PC could not reach it at all.

In a proxy that enumerator is **the wrong shape**, and not because it does not
work.

### What translation would have cost

Keep the enumerator and you have **two authorities** for the device's address:
our host, which assigned 1, and the PC, which is about to assign something
else. Everything between them then needs a translation layer, and the pieces of
it are not small:

- a map from the PC's address to ours, in both directions, re-derived whenever
  either end resets;
- a rule for what a GET_DESCRIPTOR answers **while the two disagree** — our
  host has already read the descriptors, so does the PC get our copy or a fresh
  read? One is a cache that can go stale and the other makes the cache
  pointless;
- a second copy of every descriptor, in fabric, because a cached answer has to
  live somewhere;
- an answer to "who owns the configuration": our host has already sent
  SET_CONFIGURATION with the device's own `bConfigurationValue`, and the PC is
  going to send one too, possibly a different one, possibly zero;
- and a decision about the data toggles, which SET_CONFIGURATION resets — ours
  reset them once already, before the PC had ever spoken.

**MEDIUM**, and it is the piece that settles it: the first thing to drift would
be `bMaxPacketSize0`. The lengths a host may legally send depend on a
descriptor, so a proxy that gave the PC a *different* descriptor from the
device's would have to police every length itself, in both directions, for ever.

### What pass-through costs instead

Nothing, and that is the whole argument.

**HIGH** (USB 2.0 §8.4.1). A token carries seven bits of address and four of
endpoint number. **MEDIUM**, and it is the observation the block is built on: on
the upstream bus of a hub, the devices reachable through that port are *the hub
and whatever is behind it*, so **a token whose address is not the hub's own is a
token for something behind the port**. There is nothing else it could be.

So `usb_proxy_relay` forwards the token's address and endpoint verbatim.
`trn_addr` is `tok_addr`; `trn_endp` is `tok_endp`. That includes the PC's own
SET_ADDRESS, which is a control transfer like any other and is relayed like any
other: the device takes the address the PC chose, after its own status stage as
**HIGH** (§9.4.6) requires, which is the device's business and not this block's.
From then on the PC's tokens carry that address and so do ours.

**Both sides agree on the address because there is only one authority for it**,
and there is no table, no map and no cache. And because the descriptors the PC
reads are the device's own bytes, every length on both buses agrees for free:
`bMaxPacketSize0`, `wMaxPacketSize`, `wTotalLength`, all of them.

### So our host stops being a host

`usb_host_enum` is **not instantiated in a proxy at all**.
`usb_hub_proxy_ulpi` takes `usb_ulpi_host_link` and `usb_host_sie` out of
`ip/usb_host_ulpi` and leaves the enumerator behind, which is also what saves
the proxy from paying for logic that would be fighting it.

**What the enumerator is still for**, and it is not nothing:

- `testdata/fpga/cynthion/usb_host_target.v` is the **instrument** that
  established this downstream bus works at all — nine registers of a real
  USB3343 read over sixteen left-edge balls no design of this project had ever
  driven, which is `ip/usb_host_ulpi/README.md` §9;
- `testdata/fpga/cynthion/usb_hub_target.v` uses it to prove a device is in the
  socket and enumerable, which is the measurement this block's result is
  compared against;
- and `usb_host_ulpi_enumerates_usb_device_ulpi` is still the test that proves
  the transaction engine, the tokens and the CRC5 are right, independently of
  any proxy.

A design that wants an autonomous host still has one. A proxy does not want one.

### The one thing pass-through needs in exchange

**A port reset must reach the real device.** Without it the device keeps an
address from a previous session while the PC is talking to address 0, and
nothing answers. §5 is that, and it is why `ip/usb_hub` changed in this round.

---

## 3. Not a repeater, and what NAK buys

**HIGH** (USB 2.0 §11.1 and chapter 7's delay budget). A hub is a *repeater*:
transparent to the packets passing through it, contributing a small number of
bit times to the bus's end-to-end budget.

**MEDIUM**, and it is the engineering fact the whole shape follows from: a ULPI
transceiver cannot do that. To get one byte out of one port and into the other a
Link must wait for the transceiver to detect SYNC and hand over a whole byte —
eight bit times of SYNC plus a byte — and then put that byte into the other
transceiver's transmit command, which prepends a fresh SYNC of its own. About 24
bit times one way, against the roughly 4 a hub is allowed and the 16 USB 2.0
§7.1.19.1 gives a host before it calls an answer a timeout. The delay is in the
transceivers and no gateware moves it.
[`ip/usb_hub/README.md`](../usb_hub/README.md) §2 is that argument at length.

**HIGH** (USB 2.0 §8.4.6). The escape hatch is **NAK**: a device may answer NAK
for as long as it likes while it gets ready, and a host's answer to a NAK is to
ask again. So the thing in the middle may take as long as it likes — accept the
token, answer NAK, run the transaction on the other bus at its own pace, and
have the answer waiting for the retry.

That is a **transaction proxy**, and the shape of every transfer below is that
one sentence applied.

### Nothing is retried in the middle

When a downstream transaction comes back with anything but a usable answer — a
NAK, a timeout, an unusable reply, or a data packet whose toggle says the device
never heard our acknowledgement of the last one — the job is **abandoned** and
the PC is NAKed. The PC asks again, and the asking is what starts the next
attempt.

That is a choice and it is worth stating why it is the right one. A retry loop
in the middle would hammer the downstream bus at its own rate against a device
that is not answering, would need a bound and a counter, and would still have to
NAK the PC in the meantime. Driving it from the PC's retry instead means **the
proxy is exactly as patient as the host is**, which is what a hub should be, and
a device that has gone away produces the PC's own transfer timeout rather than a
storm on the other bus. `usb_host_sie`'s `RETRIES` still covers a packet lost on
the downstream wire, which is the retry that belongs to a bus rather than to a
proxy.

The one exception is the **SETUP**, and it is not a retry: a stage of a control
transfer whose SETUP has not landed downstream re-offers *the SETUP* rather than
the stage. Without that the transfer NAKs for ever, because nothing else would
ever ask again for the one transaction it is waiting on.

---

## 4. What is forwarded, stage by stage

One PC transaction becomes **at most one** downstream transaction. The `job`
register is that one: nothing wanted, wanted, running, or an answer the PC has
not been given yet.

### A SETUP

**HIGH** (USB 2.0 §8.5.3). A device must acknowledge a SETUP. The proxy does so
at once, because it has the eight bytes in a register of its own, and starts a
downstream SETUP with those same eight bytes. Until that is acknowledged
downstream, every later stage of the transfer is NAKed.

A SETUP also **preempts**: it starts a new transfer whatever the last one was
doing, which is what `usb_ctrl_ep` does with one and for the same reason — the
host has moved on.

### A data or status stage

The token the PC sends, forwarded in the same direction: an IN becomes an IN, an
OUT becomes an OUT. Nothing needs to know which stage it is in to do that.

For the one thing that *does* depend on the stage — the data toggle — two
expressions are enough, and they are worth writing down because they are the
whole of the stage machinery this block has:

```
an IN  is the status stage exactly when  ~ct_dir | ct_nodata
an OUT is the status stage exactly when   ct_dir | ct_nodata
```

**HIGH** (§9.3 for the direction bit of `bmRequestType`, §8.5.3 for the stages).
`ct_dir` is that bit — 1 is device to host — and `ct_nodata` is `wLength` being
zero. A read's data stage is INs and its status stage an OUT; a write's data
stage is OUTs and its status stage an IN; a request with no data stage has only
a status IN. There is **no stage register**.

### A short packet ends a transfer, and that is free here

**HIGH** (USB 2.0 §5.5.3 and §8.5.3.2). A data stage ends at the first packet
shorter than `wMaxPacketSize`, or when `wLength` is reached.

**One PC IN fetches exactly one downstream IN**, and exactly the bytes that came
back are forwarded. Nothing reads ahead. So a device that answers 8 bytes to a
64-byte read sends the PC an 8-byte packet and the PC's transfer ends where the
device ended it.

This is written down because a round of this work got it wrong, and it is worth
being exact about *why* this one cannot: a proxy that fetched a whole transfer's
worth before answering would have to decide for itself where the end was, and it
would be deciding from a `wLength` the PC chose and a packet size it had read out
of a descriptor it was also forwarding. On-demand fetching has no opinion to get
wrong. `usb_proxy_forwards_a_data_stage_of_more_than_one_packet` reaches the
*other* way a stage ends too — four whole packets with no short one — which is
the branch something that only handled short packets would hang in.

### Bulk and interrupt

The same shape with no control transfer around it: an IN fetches one downstream
IN and forwards the packet, an OUT is held and forwarded and answered with what
the device said. **HIGH** (§5.7 and §5.8): a bulk and an interrupt transaction
are the same packets and differ only in how a host schedules them, so there is
nothing here that distinguishes them and nothing that needs to.

### What is not forwarded

- **SOF.** `usb_host_sie` sends its own, from its own free-running counter. The
  PC's frame number is not carried across, so a device that times anything from
  the frame number sees a different one on each side. **HIGH** (§5.6): that
  matters to isochronous transfers and to nothing else.
- **Tokens for the hub's own address**, which are the hub's, and
  `usb_dev_core`'s endpoints answer them.
- **Anything at all while the port is not enabled.** §5.

---

## 5. The port reset, and what it needed from the hub

This is the one place the forwarding half had to reach back into the hub half.

**HIGH** (USB 2.0 §11.5.1, the **Resetting** state). A hub drives SE0
downstream for 10 to 20 ms when a host sets PORT_RESET, reports PORT_RESET set
in `wPortStatus` while it does, and sets C_PORT_RESET and enables the port when
it finishes.

`ip/usb_hub` used to do none of the driving, and said so: the reset completed in
the cycle it was asked for, C_PORT_RESET set at once, and **PORT_RESET in
`wPortStatus` was a constant zero** — honest, because there was nothing
downstream to reset. In a proxy that is no longer true, and §2's last subsection
is why it *matters*: pass-through addressing depends on the device forgetting
its address exactly when the PC thinks it has.

So the hub's reset became a handshake of two signals:

| | |
|---|---|
| `port_reset` | a **level**, raised by SetPortFeature(PORT_RESET) and held until the reset is over. It is PORT_RESET in `wPortStatus`, so it is a register now and not a constant |
| `port_reset_done` | one cycle from whatever drove the reset. C_PORT_RESET sets, the port is enabled if something is connected, and `port_reset` falls |

**Tying `port_reset_done` high is exactly the old behaviour**, which is what
`testdata/fpga/cynthion/usb_hub_target.v` does and what the hub's own tests do.
A port that loses its power or its connection while resetting stops resetting,
because §11.5.1's Powered-off state has no reset in progress.

### What drives it

`usb_proxy_dn`, and the SE0 is a **register write** and not a transmission:

> "2. **Host Drives** – If a host detects a full speed peripheral, it resets the
> peripheral by writing to the Function Control register and setting
> XcvrSelect = 00b (HS) and TermSelect = 0b **which drives SE0 on the bus** (D+
> and D- connected to ground via 45 Ohm). The host also sets OpMode = 10b for
> correct chirp transmit and receive."

> *Provenance*: ULPI 1.1 §3.8.5.1, step 2, quoted. HIGH.

`50h` into `04h` drives it and `45h` puts it back, which are the "Host Chirp"
and "Host Full Speed" rows of DS00002646A Table 5-1 — **HIGH**, and §5.2.2 of
that datasheet says a combination which is not a row of it is not supported,
which is why `OpMode = 10b` is written although nothing here ever chirps.
`RESET_HOLD` is 15 ms and `RESET_RECOVERY` 20 ms against the 10 ms each of USB
2.0 §7.1.7.5 and §7.1.7.3, because a device that measures the reset meanly
should not be what decides whether this works.

### The gate, which is the port being enabled

**HIGH** (§11.5.1). A downstream port reaches **Enabled** only by being reset.

So the relay claims a token only while `configured & port_enabled`, and a port
is enabled only when a reset completed. That means **nothing is ever forwarded
to a device the PC has not reset through this port**, which is the condition
pass-through addressing needs and is asserted by
`usb_proxy_forwards_nothing_until_the_host_has_reset_the_port` — including that
a powered, connected, *unreset* port answers a token for another address with
**nothing at all** rather than with a NAK, because a NAK would be claiming an
endpoint.

### Attach, detach, and the counter they share

**HIGH** (§7.1.7.3). 100 ms of stability before an attach is believed, because a
plug being pushed in bounces. `DEBOUNCE_CYCLES`.

**The same counter debounces a detach**, and that is not symmetry for its own
sake: SE0 between two packets is an **end of packet** and not an empty socket,
so a detach has to be the pair being SE0 for as long as it took to decide it had
arrived. `usb_host_enum` never re-armed after a detach at all — its own README
lists that as the first thing to add — and a hub whose device can be unplugged
has to.

**HIGH** (§7.1.5). Which line the device pulled up says which speed it is, and
`FS_LINE` says which of the two *this board* calls D+, because a Cynthion
exchanges DP and DM between each transceiver and its connector.
`ip/usb_host_ulpi/README.md` §5 is the whole of that, and a low-speed device is
**reported and not spoken to**: there is no PRE token here.

---

## 6. The data toggles: four arrays, because there are two buses

**HIGH** (USB 2.0 §8.6). The data toggle is how a host and a device agree that a
packet arrived exactly once: the sender alternates DATA0 and DATA1, the receiver
discards a packet whose toggle is not the one it expects, and both sides advance
only on a handshake they heard.

**The two buses are independent and their toggles do not stay in step by
themselves.** A packet can be acknowledged downstream and then have to be sent
upstream more than once — the PC's ACK was lost — and each time only one side's
toggle moves. So there are four, one bit per endpoint number:

| | |
|---|---|
| `up_tog_in` | what the next IN packet this block sends the PC will carry |
| `up_tog_out` | what it expects the PC's next OUT packet to carry |
| `dn_tog_in` | what it expects the device's next IN packet to carry |
| `dn_tog_out` | what its next OUT packet to the device will carry |

Each side's toggle moves when **that side** acknowledges: `up_tog_in` when the
PC acknowledges a packet we sent, `up_tog_out` when we acknowledge a packet the
PC sent, `dn_tog_in` when the engine acknowledges a packet the device sent, and
`dn_tog_out` when the device acknowledges one we sent. A packet whose toggle is
not the expected one is a copy the far end is sending again because it did not
hear the last acknowledgement: upstream it is acknowledged and dropped,
downstream the job is abandoned and the PC asks again.

### Sixteen bits and not fifteen

Each array is sixteen bits indexed by the endpoint number as it is, **not**
fifteen indexed by the number less one — and endpoint 0's bit is live.

**HIGH** (§8.5.3). A control transfer's data stage starts at DATA1 in whichever
direction it goes. So a SETUP sets all four of that endpoint's bits, the data
stage then alternates out of the same array every other endpoint uses, and a
status stage is always DATA1 and is **forced** rather than taken from an array.

The result is that no bit of any of the four is a bit nothing writes. That is
the shape this family has cost this project eight rounds of investigation over —
a flip-flop with a constant data input, and an unrouted slice input on an ECP5
reads as a one — and `CLAUDE.md` carries the rule. Fifteen bits indexed by
`ep - 1` would have been the other way to avoid it and would have put an
out-of-range index one guard away.

### What resets a toggle, and what this block does about each

**HIGH** (§9.4.5 for SET_CONFIGURATION, §9.4.1 for CLEAR_FEATURE(ENDPOINT_HALT),
§9.4.9 for SET_INTERFACE, §11.5.1 for a port reset).

| | |
|---|---|
| a bus reset on the upstream bus | all four arrays cleared |
| a port reset, or the port being disabled | all four arrays cleared |
| the PC's **SET_CONFIGURATION** | all four cleared, when the transfer's status stage completes |
| the PC's **CLEAR_FEATURE(ENDPOINT_HALT)** | the one endpoint it names, in the direction its endpoint address names, on both sides |
| the PC's **SET_INTERFACE** | **not reflected**, and §9 says so |

The two in the middle are read out of the stored SETUP when the status stage
completes, and the reason they have to be is worth stating: the PC resets its
toggle then, and so does the device, because the request is forwarded. A proxy
that did not would be the only thing on the bus still holding the old one.

SET_INTERFACE is the one gap. It resets the toggles of the endpoints of the
interface it selects, and **which endpoints belong to which interface is in a
descriptor this block forwards without reading**. Clearing all four arrays
instead would be wrong in the other direction, on a device with two interfaces
and a stream on each. §9 lists it as not done, and a device whose driver uses
alternate settings may need the port reset the PC would send anyway.

---

## 7. STALL propagates, in both directions

**MEDIUM**, and it is the reason one of this block's decisions goes the slower
way: a device that refuses a request and a proxy that swallows the refusal look
**completely different** to a host. The first is a driver finding out that what
it asked for does not exist, in microseconds. The second is a transfer that
hangs for five seconds and then a device the kernel gives up on.

So a downstream STALL becomes an upstream STALL on the transaction that caused
it, and for a control transfer it also sets a sticky flag which STALLs every
later stage of that transfer until the next SETUP — which is **HIGH** (§8.5.3)
what a device that has stalled a control transfer must do.

### And it is why an OUT is NAKed rather than acknowledged on sight

It would be easy, and one transaction per packet cheaper, to acknowledge the
PC's OUT data, buffer it, and forward it afterwards. But then a device that
STALLs the packet has already had it acknowledged on the PC's behalf, and the
refusal can only be reported against some *later* transaction — or not at all,
if the PC never sends one.

So an OUT is held, NAKed, forwarded, and acknowledged — or stalled — on the PC's
retry. That is a round trip of a few microseconds and it makes the answer the
device's own. **MEDIUM** that it halves the best case throughput of a bulk OUT
stream; nothing here has measured what it costs on a part, and §9 says so.

---

## 8. What a host said

A Great Scott Gadgets Cynthion r1.4 holding
`testdata/fpga/cynthion/usb_proxy_target.v`, built with `VBUS_AUX = 1`:
`ip/usb_proxy` with the hub on the **AUX** transceiver and the downstream port on
the **TARGET** one, and a Great Scott Gadgets **GreatFET** in the TARGET-A
socket. Linux 6.18.41-gentoo, `xhci_hcd`, the Cynthion on a full-speed downstream
port of a hub. Everything below is **quoted**, not paraphrased.

### The one before, and the one after

This is the whole round in nine lines, and they are from the same `dmesg` buffer
on the same machine with the same device in the same socket — the first from
`usb_hub_target.v`, which forwards nothing, and the second from
`usb_proxy_target.v`, which does.

**Before.** `ip/usb_hub/README.md` §8 quotes this at length and it is the
correct outcome of a hub with no proxy behind it:

```text
usb 7-5.1: new full-speed USB device number 25 using xhci_hcd
usb 7-5.1: device descriptor read/64, error -71
usb 7-5.1: device descriptor read/64, error -71
usb 7-5-port1: attempt power cycle
usb 7-5.1: new full-speed USB device number 26 using xhci_hcd
usb 7-5.1: Device not responding to setup address.
...
usb 7-5-port1: unable to enumerate USB device
```

**After:**

```text
usb 7-5: new full-speed USB device number 28 using xhci_hcd
usb 7-5: New USB device found, idVendor=1209, idProduct=0001, bcdDevice= 1.00
usb 7-5: New USB device strings: Mfr=0, Product=0, SerialNumber=0
hub 7-5:1.0: USB hub found
hub 7-5:1.0: 1 port detected
usb 7-5.1: new full-speed USB device number 29 using xhci_hcd
usb 7-5.1: not running at top speed; connect to a high speed hub
usb 7-5.1: New USB device found, idVendor=1d50, idProduct=60e6, bcdDevice= 1.00
usb 7-5.1: New USB device strings: Mfr=1, Product=2, SerialNumber=3
usb 7-5.1: Product: GreatFET
usb 7-5.1: Manufacturer: Great Scott Gadgets
usb 7-5.1: SerialNumber: 000000000000000057cc67e6341d3457
```

**MEASURED**, and there are five things in it worth pointing at.

**`idVendor=1d50, idProduct=60e6`** is the GreatFET's own pair and not
`1209:0001`. The descriptors the kernel read are the **device's**, byte for byte,
because with pass-through addressing there is nothing of this project in them.
§2 is why that was the architecture chosen and this line is what it buys.

**The three strings.** `Mfr=1, Product=2, SerialNumber=3` are string descriptor
*indices*, and the three lines after them are the strings themselves — so the
kernel sent GET_DESCRIPTOR(STRING) four times (a language table and three
strings) and got each one back through the proxy. **Nothing in this library
implements a string descriptor**: `usb_ctrl_ep` stalls one and `ip/usb_hub` has
none, which is why the hub's own three lines above read `Mfr=0, Product=0,
SerialNumber=0`. A proxy that answered for the device rather than forwarding to
it could not have produced those thirty-two hexadecimal digits of serial number.

**No retries at all.** The `before` log has four `error -71` and two
`Device not responding to setup address` across three device numbers. The
`after` log has one device number and no errors: the device descriptor read, the
SET_ADDRESS, the configuration descriptor read and the strings all completed
first time.

**`not running at top speed; connect to a high speed hub`** is the kernel
observing that a full-speed device on a full-speed hub could have been faster.
Both halves of that are true and neither is this block's to fix: `usb_host_sie`
is a full-speed engine with no chirp and `bDeviceProtocol` of `00h` says so.

**And the device number is `29` where the hub's is `28`** — one address apart,
both assigned by the kernel, with nothing in between. That is the pass-through
of §2 seen from the host's side: the kernel allocated an address for the device
and the device took it.

### The power budget, which is the one thing that looked like success and was not

The first load of this design produced every line above **and two more**:

```text
usb 7-5.1: rejected 1 configuration due to insufficient available bus power
usb 7-5.1: no configuration chosen from 1 choice
```

**MEASURED**, and it is worth writing up because it is a failure mode that looks
exactly like a working proxy right up to the last line. The device enumerated
perfectly — descriptors, address, strings, all of it — and was then left
unconfigured, so nothing could use it.

The reason is nothing to do with forwarding. A GreatFET's one configuration
declares `MaxPower 500mA`; **HIGH** (USB 2.0 §11.13) a bus-powered hub may offer
each of its ports 100 mA, because every milliamp it hands downstream comes out of
the allowance its own upstream cable granted it; and
[`ip/usb_hub`](../usb_hub/README.md)'s configuration descriptor said bus powered,
100 mA, which was true of every other design in this library.

**It is not true of this board.** A Cynthion's own supply and its Apollo debug
microcontroller come in on the **CONTROL** port, so the hub controller in the
FPGA draws nothing at all from the AUX cable. `usb_proxy_target.v` therefore has
a `SELF_POWERED` parameter, default 1, which sets bit 6 of `bmAttributes` — and
that parameter's own comment carries the inaccuracy it leaves behind, because the
current the *port* draws does come out of the AUX cable, through the
bidirectional switch `VBUS_AUX` closes. A strictly compliant self-powered hub
would supply its ports from its own rail. What makes it safe in practice is that
the switch is closed by the bitstream and not by the host: whatever the socket
draws, it drew before the PC ever asked, and no class request can change it.

**MEDIUM**, and it is a pleasing consequence of §7 of
[`ip/usb_hub/README.md`](../usb_hub/README.md) rather than a coincidence: the
same byte is what the **standard** GET_STATUS of §9.4.5 reports. `usb_ctrl_ep`
derives bit 0 of its two bytes from bit 6 of `CFG_ATTR`, so a host that reads the
configuration descriptor and a host that asks GET_STATUS cannot be told different
things about whether this device is self powered. That is the whole argument for
putting a standard request in endpoint 0 instead of in a class, and the next
subsection is `lsusb` reading both halves of it off the part.

### What it read off the part

With `SELF_POWERED = 1` the configuration is chosen and the device is usable.
`lsusb -v`, through our hub, of the device in the socket:

```console
$ lsusb -d 1d50:60e6 -v
Bus 007 Device 031: ID 1d50:60e6 OpenMoko, Inc. replacement for GoodFET/FaceDancer - GreatFet
Negotiated speed: Full Speed (12Mbps)
Device Descriptor:
  bLength                18
  bDescriptorType         1
  bcdUSB               2.00
  bDeviceClass            0 [unknown]
  bDeviceSubClass         0 [unknown]
  bDeviceProtocol         0
  bMaxPacketSize0        64
  idVendor           0x1d50 OpenMoko, Inc.
  idProduct          0x60e6 replacement for GoodFET/FaceDancer - GreatFet
  bcdDevice            1.00
  iManufacturer           1 Great Scott Gadgets
  iProduct                2 GreatFET
  iSerial                 3 000000000000000057cc67e6341d3457
  bNumConfigurations      1
  Configuration Descriptor:
    bLength                 9
    bDescriptorType         2
    wTotalLength       0x0020
    bNumInterfaces          1
    bConfigurationValue     1
    iConfiguration          0
    bmAttributes         0x80
      (Bus Powered)
    MaxPower              500mA
    Interface Descriptor:
      bLength                 9
      bDescriptorType         4
      bInterfaceNumber        0
      bAlternateSetting       0
      bNumEndpoints           2
      bInterfaceClass       255 Vendor Specific Class
      bInterfaceSubClass    255 Vendor Specific Subclass
      bInterfaceProtocol    255 Vendor Specific Protocol
      iInterface              0
      Endpoint Descriptor:
        bLength                 7
        bDescriptorType         5
        bEndpointAddress     0x81  EP 1 IN
        bmAttributes            2
          Transfer Type            Bulk
          Synch Type               None
          Usage Type               Data
        wMaxPacketSize     0x0040  1x 64 bytes
        bInterval               0
      Endpoint Descriptor:
        bLength                 7
        bDescriptorType         5
        bEndpointAddress     0x02  EP 2 OUT
        bmAttributes            2
          Transfer Type            Bulk
          Synch Type               None
          Usage Type               Data
        wMaxPacketSize     0x0040  1x 64 bytes
        bInterval               0
Device Qualifier (for other device speed):
  bLength                10
  bDescriptorType         6
  bcdUSB               2.00
  bDeviceClass            0 [unknown]
  bDeviceSubClass         0 [unknown]
  bDeviceProtocol         0
  bMaxPacketSize0        64
  bNumConfigurations      2
```

**MEASURED.** Four things in it are the proxy's and not the device's to claim.

**`wTotalLength 0x0020` is thirty-two bytes and all thirty-two are there**, with
the interface descriptor and both endpoint descriptors under it. The host read it
twice — once for its nine-byte header and once for the whole of it — and §4's "a
short packet ends a transfer" is what makes the first of those two stop at nine
rather than running on into the second.

**`MaxPower 500mA` and `bmAttributes 0x80`** are the **device's** power
declaration and not the hub's, and they are the two bytes the subsection above is
about. The hub's own read `0xc0` and `100mA`. Two devices on one bus with
different answers, each reporting its own, is pass-through addressing in two
lines.

**The three strings.** `iManufacturer 1 Great Scott Gadgets`,
`iProduct 2 GreatFET` and thirty-two hexadecimal digits of serial number are four
more control reads — a language table and three strings — of a descriptor type
**nothing in this library implements**. `usb_ctrl_ep` stalls a string descriptor
and `ip/usb_hub` has none, which is why the hub's own line in the kernel log above
reads `Mfr=0, Product=0, SerialNumber=0`.

**`Device Qualifier`**, which `lsusb` fetches with GET_DESCRIPTOR of type `06h` —
a request **this library stalls** — and which the device answered with ten bytes.
There is no clearer single statement that what the host is talking to is the
device and not the proxy.

And the hub's own port, read at the same moment:

```console
$ lsusb -d 1209:0001 -v | grep -A 3 'Hub Port Status'
 Hub Port Status:
   Port 1: 0000.0103 power enable connect
Device Status:     0x0001
```

**MEASURED**, and both lines changed in this round.

`0103h` is PORT_POWER, PORT_ENABLE and PORT_CONNECTION. The reading
`ip/usb_hub/README.md` §8 quotes is `0101h` — powered and connected and **not
enabled**, because that hub's port was enabled by a reset and then disabled again
when the kernel failed to enumerate through it. **HIGH** (§11.5.1): a port is
Enabled only by a reset completing, so `0103h` is §5's reset having reached the
real device and the kernel having got what it wanted through the port.

`Device Status: 0x0001` is the **standard** GET_STATUS of §9.4.5 with bit 0 — Self
Powered — set, answered by `usb_ctrl_ep` out of bit 6 of `CFG_ATTR`. The same
reading in `ip/usb_hub/README.md` §8 is `0x0000`, from a class block's hook, and
the byte that produced it was a literal zero in `usb_hub_req`. It is arithmetic
over a parameter now, in endpoint 0, and it agrees with the configuration
descriptor by construction.

### The gateware's own view, over the T14 console

**MEASURED.** The console `usb_proxy_target.v` puts on ball T14 — which Apollo
bridges to `/dev/ttyACM0` and which exists because AUX is the hub and cannot be a
serial port as well — printed, with the kernel's enumeration already over and then
again after a bulk attempt from userspace:

```text
PFCD14A2B      (and two more of the same)
PFCD17A2E      (and five more of the same)
```

That design's header is the field table. Decoded:

| | byte 0, the hub | byte 1, the port | byte 2 | byte 3 |
|---|---|---|---|---|
| `FC D1 4A 2B` | ready, configured, addressed, bus reset seen, port powered **and enabled**, not suspended, not resetting | ready, attached, full speed, **proxied**, no refused register write, VbusState `00`, **a transaction forwarded outside a control transfer** | a control transfer held, job **idle**, `dn_stage` **10** | LineState `01`, **11** SETUPs forwarded |
| `FC D1 7A 2E` | the same | the same | job **3** — an answer the PC has not come back for | **14** SETUPs forwarded |

**`dn_stage` 10 is `D_UP`**, which `usb_proxy_dn` only reaches by driving a bus
reset at the device and waiting out its recovery. **`setups` 11 and then 14** is
the count climbing by exactly the three control transfers `tests/usb_proxy.rs`
does between the two readings — its device descriptor read and its two
configuration descriptor reads — which is the forwarding counted rather than
inferred.

**`job` 3 in the second reading is correct and is worth explaining**, because it
looks like a stuck state machine. The relay keeps a NAK answer until the PC asks
that endpoint again, and the PC had stopped asking: the bulk attempt gave up. §3's
"nothing is retried in the middle" is why that is the right behaviour — the next
thing that moves the job is the host, and if the host has lost interest there is
nothing to do.

**One reading in that is the same one `ip/usb_hub/README.md` §8 explains and it is
still not wrong.** Byte 1 bits 2:1 are the TARGET transceiver's `VbusState` and
they read `00`, which USB334x Table 6-3 makes "below SessEnd" — no power on the
port at all — while a device is attached, enumerated and configured through it.
**The TARGET transceiver does not sense the connector its power flows through**:
`ip/usb_host_ulpi/README.md`'s "VBUS switching" traced that from the published
Cynthion PCB design, and on TARGET `VbusState` does not mean "is my device
powered".

### Bytes over bulk: what was established and what was not

The GreatFET declares a bulk pair — `81h` IN and `02h` OUT, 64 bytes each — and
`tests/usb_proxy.rs` tries both. Both time out.

**That is not evidence of a broken proxy and it is not evidence of a working one
either**, which is why that part of the test reports rather than asserts: **HIGH**
(§8.4.6) a device may NAK for as long as it likes, a host turns a NAK for ever
into a timeout, and from the host's side an endpoint the proxy never reached and
one the device NAKed are both `operation timed out`.

**The console is what tells them apart.** `saw_data_fwd`, byte 1 bit 0, is set:
`usb_proxy_relay` handed the engine a transaction that was **not** part of a
control transfer, so the bulk token went out on the downstream wire. That bit
exists because of this measurement — it was `ctrl_active` in the first version of
this console, which said nothing about bulk at all.

So what is established is that **a bulk transaction is forwarded downstream**, and
what is **not** established is whether the GreatFET NAKed it or said nothing. Its
firmware's libgreat command pipe is a vendor control request and these bulk
endpoints are armed on demand, so a NAK until a buffer is queued is what a reading
of that firmware would predict — but this round did not read it and does not claim
it. Distinguishing the two needs `trn_status` on the console, which is three more
bits and the obvious next instrument.

**What *is* asserted about bulk is in simulation**, and it is asserted exactly:
`usb_proxy_moves_bytes_through_the_port` sends four packets of four different
lengths — one byte, eight, a full sixty-four and four — through
`usb_device_ulpi`'s own loopback and gets each one back byte for byte, with the
PC's two data toggles asserted independently and **eight downstream transactions
counted**, one per packet each way. A relay that held a packet and handed it over
twice would not read eight.

---

## 9. What this block does not do

Everything in this list is a thing a reader might reasonably expect and will not
find.

**One transaction at a time.** Anything else is NAKed while one runs, so a
device with several endpoints is polled in turn rather than in parallel. That is
a throughput limit and not a correctness one: a job is only ever started by a
token the PC sent, so a job that is waiting is waiting for something the PC
asked for and will ask for again.

**One device behind the port.** The forwarding is per token and keeps no
per-device state at all, so several addresses would work — but the toggle arrays
are one set, and two devices sharing an endpoint number would share a toggle. A
second device needs the arrays indexed by address too, and there is nowhere for
one to be: the hub has one port.

**No isochronous transfers, and no frame alignment.** §4's last subsection.

**No PING, no split transactions, no low speed and no high speed**, all of which
follow from `usb_host_sie` being a full-speed engine with no PRE token and no
chirp. A low-speed device on the port is reported and never spoken to.

**No suspend.** The PC's SetPortFeature(PORT_SUSPEND) moves a bit in
`ip/usb_hub` and nothing downstream stops; the downstream SOFs keep the device
awake. ULPI §3.8.5.3.2's suspend is the change, and it needs a decision about
what a resume does to a relay with a job in flight.

**No SET_INTERFACE toggle reset.** §6's last subsection.

**Nothing switches a VBUS pin.** The socket behind the port is powered by a
switch in the design's own top level, and on the board this was written for
those switches are *bidirectional* between connectors, so closing two of them
ties two hosts' supplies together. `ip/usb_hub/README.md` §4 states that as a
deliberate refusal.

**No throughput figure.** Nothing here has measured bytes a second on a part,
and §7's last paragraph is the one place a number would be interesting. A
throughput figure in this project is printed by an `#[ignore]`d test and never
asserted.

**No descriptor rewriting of any kind.** A proxy that wanted to present a
*different* device to the PC than the one in the socket would have to, and this
one is the opposite of that by construction: §2.

---

## 10. What each test would and would not catch

Seven tests of this block in `tests/ip_library.rs`, three that cover every block
in the library and reach it with the rest, and one that needs a board.

| Test | What it would catch | What it would not |
|------|--------------------|-------------------|
| `usb_proxy_enumerates_the_device_behind_the_port` | a control transfer not forwarded in either direction, a SETUP the device never saw, a data stage cut in the wrong place, an address the device did not take, a port reset that did not reach it, a descriptor byte that is the proxy's rather than the device's, and a relay left holding an answer nobody asked for | that a **kernel** enumerates it; any of the times, which `PROXY_TEST_PARAMS` is explicit about |
| `usb_proxy_enumerates_through_the_transceivers_that_are_on_the_board` | the same against three transceiver models that each hear their own transmission and each report `LineState` **late**, out of a backlog that outlives the packet — both measured on a Microchip USB3343 on a Cynthion r1.4. It matters more here than anywhere else in the library, because `LineState` is what decides when the downstream port is a port at all: a proxy that believed a stale reading would reset a device that had not arrived, or never reset one that had | the same; and that a board has one transceiver behaving one way where the model has four |
| `usb_proxy_forwards_a_data_stage_of_more_than_one_packet` | a toggle this proxy did not forward or did not track, which is invisible at one packet a stage; a short packet treated as the middle of a transfer; and a transfer that needed a short packet to end and hung without one. It runs against a device built with `MAXPKT0 = 8`, the smallest §5.5.3 allows, so the eighteen-byte device descriptor arrives in three packets and the thirty-two-byte configuration descriptor in four — and four is **exactly** the length asked for, so there is no short packet to stop on | a packet size the device declares and the PC does not believe, which pass-through makes impossible |
| `usb_proxy_moves_bytes_through_the_port` | a toggle wrong in either direction, a packet forwarded twice, a packet dropped, a length lost, and a buffer whose bytes come out in the wrong order — over four packets of four lengths including a one-byte short one and a full 64-byte one, through the device's own loopback, with the PC's two toggles asserted independently and asserted to come back to DATA0 after four | throughput, which is not asserted anywhere |
| `usb_proxy_propagates_a_stall_from_the_device` | a proxy that swallowed a refusal, which is §7's whole subject; a STALL that is not sticky, where §8.5.3 wants one; and a STALL that outlives the transfer it belonged to, which would refuse everything after it. The STALL is a **real** one — a string descriptor the device behind the port has not got — and not a condition the test manufactures | a STALL from a **bulk** endpoint, because nothing in this library halts one |
| `usb_proxy_forwards_nothing_until_the_host_has_reset_the_port` | a relay that claimed tokens before the PC had enabled the port, which would answer for a device whose address is anybody's guess; a NAK where silence belongs, which would claim an endpoint; and a port reset that moved a bit in the hub without reaching the transceiver — asserted by the device's **own** `address` output going back to 0 on a second reset | whether a kernel resets a port in that order |
| `usb_proxy_is_one_clock_domain` | a crossing added anywhere in a design with two ULPI buses in it | — |
| `every_block_maps_to_the_logic_it_was_mapped_from` | this block's LUT4 and LUT6 mapping differing from the logic it came from, **proved** rather than sampled | anything after mapping: placement, routing, the bitstream |
| `footprints_match_the_documentation` | the block suddenly costing twice as much | — |
| `blocks_synthesise_cleanly` | a latch, and any diagnostic at or above a warning | — |

**What none of them can catch, and it is the thing that matters most: whether a
kernel enumerates the device through it.** A host model written from the same
specification as the proxy can agree with it about something they are both wrong
about — that is the sentence `ip/usb_device_ulpi/README.md` §11 wrote before that
block had a board, and the one time it came true is written up in the same
section. §8 is the other half.

**And one more test, which is a board and not a model.** `tests/usb_proxy.rs` is
`#[ignore]`d, needs `--features program`, and skips with a reason when no hub of
ours with a device behind it is attached. Its own module comment says what each
of its four parts would and would not catch; the one that matters is part three,
which is a single boolean — **a child of our hub exists in sysfs** — and which
fails rather than skips if the port reports a device and the kernel enumerated
nothing through it. That is the state `ip/usb_hub/README.md` §8 quotes the kernel
log of, and it is the one assertion in this whole round that simulation could not
reach at all.

The bulk half of it **reports** rather than asserts, and §8's last subsection is
why: a host cannot tell an endpoint the proxy never reached from one the device
NAKed, because both are `operation timed out`. The thing that can is
`saw_data_fwd` on the board's console, and that bit exists because of this.

---

## 11. Reading list

- *Universal Serial Bus Specification, Revision 2.0* — §8.4.1 for a token's
  address and endpoint, §8.4.6 for the handshakes and what a NAK means, §8.5.3
  for a control transfer's stages and its toggles, §8.6 for the toggle itself,
  §9.1.2 for the enumeration a host does, §9.4.6 for when SET_ADDRESS takes
  effect, §11.1 for what a hub is, §11.5.1 for the downstream port's state
  machine, §7.1.7 for the reset timings.
- *UTMI+ Low Pin Interface Specification*, Revision 1.1 — §3.8.5.1 for the bus
  reset a host drives out of its terminations, §3.8.5.3.2 for a full-speed
  host's register values, Table 10 for the inter-packet windows.
- *USB334x Data Sheet*, Microchip DS00002646A — Table 5-1, which is the only
  place the resistors behind a register combination are written down.
- [`ip/usb_hub/README.md`](../usb_hub/README.md) — the other half, and §8 of it
  the kernel log this round's §8 is the sequel to.
- [`ip/usb_host_ulpi/README.md`](../usb_host_ulpi/README.md) — the transaction
  engine and the Link, and §5 of it the one polarity question a board can ask.
- [`ip/usb_device_ulpi/README.md`](../usb_device_ulpi/README.md) — ULPI, fact by
  fact, with the provenance of each. Not repeated here.
- [`docs/ip-library.md`](../../docs/ip-library.md) — what every block in the
  library costs.
