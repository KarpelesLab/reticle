# A USB hub, and why it cannot be a repeater

`usb_hub` is a full-speed USB 2.0 **hub** with one downstream port. A device
built on it appears to an operating system as a hub — `USB hub found`,
`1 port detected` — and, as with [`ip/usb_cdc_acm`](../usb_cdc_acm/README.md),
**nothing has to be installed for that to happen**: a hub is the one class
every operating system must already know, because a hub is how it finds
anything else at all.

**It is a hub's control endpoint and not a hub.** That sentence is the most
important one in this document and §2 is the whole of why. A real hub repeats
every downstream packet to its enabled ports within about four bit times, and
through a ULPI transceiver the floor is roughly twenty-four one way. What is
behind this block's port instead is a **second USB controller**,
[`ip/usb_host_ulpi`](../usb_host_ulpi/README.md), on its own bus and doing its
own enumeration, and nothing joins the two conversations. So a host finds a
hub, finds something on its port, resets it, and gets no answer from the device
it believes is there. §8 is the kernel log of exactly that, quoted, and it is
the **correct** outcome for the block as it stands rather than a defect to be
apologised for.

This document is laid out the way
[`ip/usb_cdc_acm/README.md`](../usb_cdc_acm/README.md) is, and for the same
reason: the facts in it have very different provenance. Some are read out of a
published table. Some are a reading of what a *host driver* does, which no
specification states and only a host can confirm. §1 says which is which and
every claim below is marked.

Nothing was transcribed from anybody's implementation. The specification is a
published document and the encodings below are read out of it; the descriptors,
the request decoder, the port state machine and the endpoint arrangement are
this repository's own.

---

## 1. Confidence, and what it is based on

- **HIGH** — stated in the *Universal Serial Bus Specification, Revision 2.0*
  ("USB 2.0" below), with the **section** named so it can be checked without
  reading any code.
- **MEDIUM** — a reading of it which it does not state in so many words, or a
  statement about what a host driver does that is consistent with everything
  observed and is not written down anywhere authoritative.
- **CHECKED** — **measured on a host**, with the output quoted in §8. This is
  the category that matters most for a class layer, since the question is not
  "is this descriptor legal" but "does the driver bind".
- **LOW** — inference that explains the rest, with no way to check it here.

**Sections and not table numbers.** USB 2.0 chapter 11 is cited below by
subsection — §11.23.2.1, §11.24.2.7.1 and so on — and its tables are named by
what they hold rather than by number. That is deliberate: a section number
misquoted is findable and a table number misquoted sends a reader to the wrong
table and looks authoritative doing it. Where a table is named in prose it is
named as "the `wPortStatus` field of §11.24.2.7.1" and not as "Table 11-21".

A thing can be HIGH and wrong about the world: a descriptor can be exactly what
a section says and still not be what a driver looks for. Where this document has
both, it says both.

---

## 2. Why this is not a repeater, and must never be built as one

**HIGH** (USB 2.0 §11.1.1, §11.3). A hub is a *repeater*: it is transparent to
the packets passing through it, and §11.1.1's figure is a repeater between an
upstream port and some number of downstream ones. §7.1.14 gives the hub's
contribution to the bus's delay budget as a small number of bit times, and
§11.4 has the hub's own propagation limits.

**MEDIUM**, and it is the engineering fact this whole block's shape follows
from: **a ULPI transceiver cannot do that**. ULPI is a byte-wide interface at
60 MHz with the serialiser in the transceiver, so to get one byte out of one
port and into the other a Link must

1. wait for the transceiver to detect SYNC and hand over the first byte, which
   is eight bit times of SYNC plus a byte — the receive path does not report a
   byte until the byte is complete;
2. put that byte into the other transceiver's transmit command, which prepends
   a fresh SYNC of its own — another eight bit times before the byte reaches
   the wire.

That is around 24 bit times one way before anything can come back, against the
roughly 4 a hub is allowed and the 16 USB 2.0 §7.1.19.1 gives a *host* before
it calls a device's answer a timeout. **So a transparent repeater built on two
ULPI transceivers cannot meet USB turnaround, and no amount of care in the
gateware changes that**: the delay is in the transceivers.

**MEDIUM**. The architecture that does work decouples the two sides and uses
**NAK** as the escape hatch. The PC's side may NAK a transaction until the
data is there — that is what NAK is for and §8.4.5 is the flow control it
describes — so a thing in the middle can take as long as it likes: accept the
host's token, answer NAK, run the transaction on the other bus at its own pace,
and have the answer waiting for the host's retry. That is a **transaction
proxy** and not a repeater, and it is the next round's work. This block is the
part of it a host has to bind to first.

**So the layering is deliberate.** The hub's control endpoint — descriptors,
class requests, port state, status-change endpoint — is a complete and testable
piece of work on its own, and it is what decides whether a host will ever send
a packet towards the port at all. Building it at the same time as the proxy
would have made neither reviewable.

---

## 3. The descriptors, field by field

Two descriptor sets, and the second is the thing a hub has that other classes
do not: a **class-specific descriptor fetched by a request of its own** rather
than carried in the configuration.

### The configuration descriptor: twenty-five bytes

**HIGH** (USB 2.0 §11.23.1). A hub's standard descriptors are a device
descriptor whose `bDeviceClass` is `09h`, one configuration, **one** interface
of class `09h`, and **one** endpoint on it — an interrupt IN for the status
change bitmap. There is nothing else in the configuration: §11.23.2.1's hub
descriptor is fetched by GetHubDescriptor and a hub that put it in the
configuration as well would be describing itself twice.

That makes twenty-five bytes — nine of configuration, nine of interface, seven
of endpoint — which is the **smallest configuration descriptor anything in this
library has**. `ip/usb_cdc_acm`'s is sixty-seven.

`bNumInterfaces`, `bNumEndpoints` and `wTotalLength` are not in the block's
parameter at all: `usb_ctrl_ep` counts them out of the blob at elaboration and
writes them over what is there.
[`docs/ip-library.md`](../../docs/ip-library.md) says why.

#### The device descriptor

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 4 | bDeviceClass | `09h` Hub | **HIGH** USB 2.0 §11.23.1 |
| 5 | bDeviceSubClass | `00h` | **HIGH** §11.23.1 |
| 6 | bDeviceProtocol | `00h` | **HIGH** §11.23.1 |
| 7 | bMaxPacketSize0 | 64 | **HIGH** §5.5.3 allows 8, 16, 32 or 64 |

**HIGH** (USB 2.0 §11.23.1). **`bDeviceProtocol` is the whole statement that
this is a full-speed hub.** `00h` is a hub with no transaction translator, `01h`
a high-speed hub with a single TT and `02h` one with a TT per port — and a
high-speed hub is further required to have two interface alternate settings,
one per TT arrangement. One alternate setting and a zero here say full speed
only, which is what the link layer below this block is.

**MEDIUM**. `bcdUSB` is `0200h`, which `usb_ctrl_ep` writes for every device in
this library. For a full-speed-only USB 2.0 hub that is right rather than
merely tolerated, and it matters to a driver: Linux decides whether to ask for
the SuperSpeed hub descriptor (`2Ah`) or the ordinary one (`29h`) from
`bcdUSB` and `bDeviceProtocol` together, and `0200h` with `00h` gets `29h`,
which is the one this block has.

#### INTERFACE 0 — the hub's only interface (9 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bLength | 9 | **HIGH** USB 2.0 §9.6.5 |
| 1 | bDescriptorType | 4 (INTERFACE) | **HIGH** §9.4 |
| 2 | bInterfaceNumber | 0 | this block's choice |
| 3 | bAlternateSetting | 0 | one setting only; see `bDeviceProtocol` above |
| 4 | bNumEndpoints | *counted* — 1 | derived at elaboration |
| 5 | bInterfaceClass | `09h` Hub | **HIGH** §11.23.1 |
| 6 | bInterfaceSubClass | `00h` | **HIGH** §11.23.1 |
| 7 | bInterfaceProtocol | `00h` | **HIGH** §11.23.1 |
| 8 | iInterface | 0 | no strings |

#### ENDPOINT `81h` — the status change endpoint (7 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bLength | 7 | **HIGH** USB 2.0 §9.6.6 |
| 1 | bDescriptorType | 5 (ENDPOINT) | **HIGH** §9.4 |
| 2 | bEndpointAddress | `81h` — IN, endpoint 1 | **HIGH** §9.6.6 |
| 3 | bmAttributes | `03h` — interrupt | **HIGH** §9.6.6 bits 1:0 |
| 4–5 | wMaxPacketSize | 2, little endian | see below |
| 6 | bInterval | 12 frames | this block's choice |

**`wMaxPacketSize` is two for a bitmap that is one byte**, and the reason is
not the specification's. **HIGH** (§11.12.4): the bitmap is one bit a port plus
bit 0 for the hub, rounded up to a byte, so a one-port hub's is one byte.
**HIGH** (§5.7.3): a full-speed interrupt endpoint may be any size up to 64 —
the "8, 16, 32 or 64" restriction of §5.8.3 is a *bulk* endpoint's — so one
would have been legal. **MEDIUM**: `usb_bulk_ep` masks its buffer's byte index
to the index's own width rather than comparing it against a bound, which needs
the size to be a power of two, and at a size of one that index is **zero bits
wide** — a part-select of nothing, which is not a register any fabric can
build. Two is the smallest size that is both a power of two and a width.

**HIGH** (§5.7.3 again). A host is unaffected: the device sends one byte, which
is a short packet, and a short packet ends an interrupt transfer whatever the
maximum was.

**`bInterval` of 12 is a choice and not a quotation.** **HIGH** (USB 2.0
§9.6.6): for a full-speed interrupt endpoint `bInterval` is a period in frames,
1 to 255, so 12 is 12 ms. What it buys and costs is symmetric: a device plugged
into the port is noticed up to 12 ms late, against one one-byte transaction
every 12 ms of bus time for as long as the hub is configured. Hubs in the field
use anything from 12 to 255. Nothing has measured which is better here.

### The hub descriptor: nine bytes, and the length is the trap

**HIGH** (USB 2.0 §11.23.2.1). The hub's class-specific descriptor, type `29h`,
fetched by GetHubDescriptor:

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bDescLength | 9 | **HIGH** §11.23.2.1 — *computed*, see below |
| 1 | bDescriptorType | `29h` | **HIGH** §11.23.2.1 |
| 2 | bNbrPorts | 1 | this block has one port |
| 3–4 | wHubCharacteristics | `0011h`, little endian | below |
| 5 | bPwrOn2PwrGood | 50 — 100 ms, in 2 ms units | below |
| 6 | bHubContrCurrent | 100 mA | below |
| 7 | DeviceRemovable | `00h` | below |
| 8 | PortPwrCtrlMask | `FFh` | **HIGH** §11.23.2.1 |

**HIGH** (§11.23.2.1). **`bDescLength` is arithmetic and is the field a host
and a hub can disagree about.** The two fields at the end are variable length:
`DeviceRemovable` is one bit per port plus a reserved bit 0, rounded up to a
byte, and `PortPwrCtrlMask` is the same size. One port is one byte each, so the
length is 7 + 1 + 1 = **9**. Nine ports would be 7 + 2 + 2 = 11.

**MEDIUM**, and this is where a hub is most easily rejected. Linux asks for the
whole of its own `struct usb_hub_descriptor` — fifteen bytes, sized for its
maximum port count — and requires at least `7 + 2` bytes back before it will
believe it has a hub descriptor at all. Nine is exactly the floor. What makes
asking for fifteen and getting nine work is `usb_ctrl_ep` capping a class data
stage at `min(wLength, class_len)` and a short packet ending a control read,
which §7 of this file calls out as the one place the class hook's arithmetic is
load-bearing. `usb_hub_answers_the_hub_and_port_class_requests` asks for
fifteen on purpose.

#### wHubCharacteristics

**HIGH** (§11.23.2.1), field by field:

| Bits | Field | This block | Why |
|------|-------|-----------|-----|
| D1:D0 | Logical Power Switching Mode | `01` — individual port power switching | below |
| D2 | part of a compound device | 0 | it is not: nothing is permanently attached |
| D4:D3 | Over-current Protection Mode | `10` — no over-current protection | below |
| D6:D5 | TT Think Time | `00` | no transaction translator |
| D7 | Port Indicators Supported | 0 | none, so SetPortFeature(PORT_INDICATOR) is stalled |

so `0011h`.

**D1:D0 is `01` and not `1X`, and that is the one field whose value decides
whether this hub works at all.** **HIGH**: `00` is ganged power switching, `01`
is individual, `1X` is reserved and means a hub with no power switching.
**MEDIUM**: Linux's `hub_power_on` sends SetPortFeature(PORT_POWER) to every
port of a hub whose mode is `00` or `01` and to **none** of a hub whose mode is
`1X`. §4 is why this block's port is not connected until it is powered, so a
hub that declared no power switching here would have a port nothing ever turned
on, for ever. `01` is also the common value in the field. This is a reading of
a driver, marked MEDIUM, and §8 is the measurement that confirms the request
arrives.

**D4:D3 is `10` and that is the honest value.** **HIGH** (§11.23.2.1): "no
over-current protection" is allowed only for a bus-powered hub that does not
implement any, which is what this is — `bmAttributes` of the configuration
descriptor says bus powered and there is no current sensor anywhere in the
design. The alternative values claim a detector and then `wPortStatus`'s
PORT_OVER_CURRENT would be a bit that is always zero because nothing looks,
rather than because there is nothing to look at.

#### bPwrOn2PwrGood, bHubContrCurrent and the two masks

**MEDIUM**. `bPwrOn2PwrGood` is 50, which §11.23.2.1 makes 100 ms. **Nothing
here sequences a supply** — SetPortFeature(PORT_POWER) moves a register bit and
the bit is good the cycle after — so a smaller number would be true and would
buy nothing: a host waits this once, at enumeration. Understating it is the only
way this field can cause a fault, because a host that looks before a real supply
is good sees a port with nothing on it.

**MEDIUM**. `bHubContrCurrent` is 100 mA, which is the same current the
configuration descriptor's `bMaxPower` declares, so the two agree. §4 is what
this does and does not say about the current the port's socket draws.

**HIGH** (§11.23.2.1). `DeviceRemovable` is one bit a port with bit 0 reserved,
`0` removable and `1` non-removable, so `00h` — whatever is in the downstream
socket can be unplugged. `PortPwrCtrlMask` is `FFh` because the section says so
in as many words: "This field exists for reasons of compatibility with software
written for 1.0 compliant devices. All bits in this field should be set to 1B."

---

## 4. The requests, and which features are honoured

USB 2.0 §11.24.2 is the hub's class requests. They reuse the standard request
codes of §9.4 with a class `bmRequestType` rather than defining their own, and
the recipient field is what makes a request about a **port**: `11b` — "other" —
with the port number, from one, in `wIndex`.

| Request | `bmRequestType` `bRequest` | Section | This block |
|---------|---------------------------|---------|------------|
| GetHubDescriptor | `A0h` `06h` | §11.24.2.5 | **answered** — the nine bytes of §3 |
| GetHubStatus | `A0h` `00h` | §11.24.2.6 | **answered** — four zero bytes |
| GetPortStatus | `A3h` `00h` | §11.24.2.7 | **answered** — §5 is what is in it |
| SetPortFeature | `23h` `03h` | §11.24.2.12 | **three features**, below |
| ClearPortFeature | `23h` `01h` | §11.24.2.2 | **eight features**, below |
| ClearHubFeature | `20h` `01h` | §11.24.2.1 | accepted, and nothing to clear |
| SetHubFeature | `20h` `03h` | §11.24.2.11 | **stalled** |
| SetHubDescriptor | `20h` `07h` | §11.24.2.10 | **stalled** |
| GetBusState | `A3h` `02h` | §11.24.2.4 | **stalled** |
| ClearTTBuffer | `23h` `08h` | §11.24.2.3 | **stalled** |
| ResetTT | `23h` `09h` | §11.24.2.9 | **stalled** |
| GetTTState | `A3h` `0Ah` | §11.24.2.8 | **stalled** |
| StopTT | `23h` `0Bh` | §11.24.2.13 | **stalled** |

**HIGH** (§11.24.2). The four transaction-translator requests belong to a
high-speed hub and this is a full-speed one, which `bDeviceProtocol` says;
SetHubDescriptor and SetHubFeature are ones a hub need not implement; and
GetBusState is optional and for debugging. A host that reads `bDeviceProtocol`
and `wHubCharacteristics` sends none of them, and one that sends them anyway is
not claimed and gets a STALL from endpoint 0 — which is the right answer for a
request the device never offered. This is `ip/usb_cdc_acm`'s pattern: **what a
block does not implement is said in a descriptor field a host reads**, so the
STALL is a second line of defence and not the statement.

### Honoured, accepted-and-ignored, and stalled

**Honoured** — the request changes something a later GetPortStatus reports:

| Feature | Section | What happens |
|---------|---------|--------------|
| SetPortFeature(PORT_POWER) | §11.24.2.12 | the port's own power bit goes on, and a connection may then appear |
| ClearPortFeature(PORT_POWER) | §11.24.2.2 | off again, and §11.5.1.1's powered-off port reports no connection and no enable |
| SetPortFeature(PORT_RESET) | §11.24.2.12 | completes **at once**; see below |
| SetPortFeature(PORT_SUSPEND) | §11.24.2.12 | the suspend bit |
| ClearPortFeature(PORT_SUSPEND) | §11.24.2.2 | the resume, and C_PORT_SUSPEND when it is complete — §11.5.1.8 |
| ClearPortFeature(PORT_ENABLE) | §11.24.2.2 | the enable bit off |
| ClearPortFeature(C_PORT_CONNECTION) | §11.24.2.2 | the change bit |
| ClearPortFeature(C_PORT_SUSPEND) | §11.24.2.2 | the change bit |
| ClearPortFeature(C_PORT_RESET) | §11.24.2.2 | the change bit |

**Accepted and ignored** — claimed and acknowledged, and nothing changes,
because the bit named is a **constant zero** in this hub and a host clearing a
zero has got what it asked for:

| Feature | Why it is a constant zero |
|---------|---------------------------|
| ClearHubFeature(C_HUB_LOCAL_POWER) | a bus-powered hub has no local supply to lose |
| ClearHubFeature(C_HUB_OVER_CURRENT) | no current sensor; `wHubCharacteristics` says so |
| ClearPortFeature(C_PORT_ENABLE) | §11.24.2.7.2 sets it when a port is disabled by an **error** — babble, a lost device — and nothing here detects one |
| ClearPortFeature(C_PORT_OVER_CURRENT) | as above |

**MEDIUM**, and it is the reason these four are a category of their own rather
than either honoured or stalled: **a register that nothing can set is a
flip-flop with a constant data input**, and on an ECP5 an unrouted slice input
reads as a one. That shape cost this project eight rounds of investigation once
and has been caught twice since; `CLAUDE.md` and `usb_ctrl_ep`'s own header
carry the account. So there is no register for any of the four, the
corresponding bits in `wHubStatus` and `wPortChange` are literal zeros, and
clearing one is a request that is claimed and does nothing at all.

**Stalled** — not claimed, so endpoint 0 answers the STALL it was going to:

- SetPortFeature(PORT_TEST), §11.24.2.12 — there are no test modes here;
- SetPortFeature(PORT_INDICATOR) — D7 of `wHubCharacteristics` is clear, so
  this hub never offered indicators;
- SetPortFeature of PORT_CONNECTION, PORT_ENABLE, PORT_OVER_CURRENT,
  PORT_LOW_SPEED or any change bit — **HIGH** (§11.24.2.12): a host has no way
  to enable a port but by resetting it, and the rest are status and not
  features;
- ClearPortFeature of anything not in the two tables above;
- **anything aimed at a port number this hub does not have**, which for a
  one-port hub is anything but 1 — including 0.

### The reset that takes no time, and why that is honest here

**HIGH** (USB 2.0 §11.5.1.5). A hub drives SE0 downstream for 10 to 20 ms when
a host sets PORT_RESET, reports PORT_RESET set in `wPortStatus` while it does,
and sets C_PORT_RESET and enables the port when it finishes.

**This block drives nothing downstream at all.** What is downstream of it is
not a port of this hub yet — it is `ip/usb_host_ulpi`, with its own bus, its own
reset sequence and its own enumeration — and joining the two is §2's proxy. So:

- `port_reset` is **one cycle** on the block's port, for whatever wants to know;
- C_PORT_RESET is set, so a host polling for the reset to finish sees it
  finished;
- the port becomes enabled if something is connected;
- and **PORT_RESET in `wPortStatus` is a constant zero** — a bit no expression
  in the block can set, so there is no register for it, for the reason the
  previous subsection gives.

**MEDIUM**. A host therefore sees the reset already complete at its first
GetPortStatus, enumerates the port, and gets nothing back from the device it
believes is there. That is this round's expected outcome and §8 is it happening.

### What a host may not have a say in

**`SetPortFeature(PORT_POWER)` does not reach a VBUS switch.** On the board this
block runs on, the socket behind the port is powered by `aux_vbus_en`, one of
three **bidirectional** switches onto a shared node, and the board's own
platform description warns that closing two of them ties two hosts' supplies
together. Which one is closed is a parameter of the bitstream —
`testdata/fpga/cynthion/usb_hub_target.v`'s header has the long form — and
**not** something a host asks for. So the power this request switches is the
hub's own idea of the port, not a supply.

That is a deliberate refusal rather than an omission, and it is worth saying why
in two directions. A host that powers the port off and expects a device to
disappear will be disappointed: the device is still attached and
`port_attached` still says so, so the connection does come back when the port is
powered again — which is what §5's gating produces and is indistinguishable
from a device that was there all along. And a host cannot brown out the board by
powering a port on, because it was never able to.

Likewise `bMaxPower` of 100 mA and `bHubContrCurrent` of 100 mA are the hub
controller's own draw. A strictly compliant bus-powered hub supplying its
downstream ports from its upstream cable would declare their current too; this
one does not supply them from there, and the inaccuracy is recorded here rather
than papered over.

---

## 5. The port, and where its state comes from

**HIGH** (USB 2.0 §11.24.2.7.1 and §11.24.2.7.2). GetPortStatus returns four
bytes: `wPortStatus` then `wPortChange`, each little endian.

| Bit | `wPortStatus` | This block |
|-----|---------------|-----------|
| 0 | PORT_CONNECTION | `port_power & port_attached` |
| 1 | PORT_ENABLE | set by a port reset completing, cleared by a disconnect, a power-off or ClearPortFeature(PORT_ENABLE) |
| 2 | PORT_SUSPEND | SetPortFeature(PORT_SUSPEND) |
| 3 | PORT_OVER_CURRENT | **0** — no detector |
| 4 | PORT_RESET | **0** — a reset is over in the cycle it is asked for |
| 8 | PORT_POWER | SetPortFeature / ClearPortFeature(PORT_POWER) |
| 9 | PORT_LOW_SPEED | `connection & port_low_speed` |
| 10 | PORT_HIGH_SPEED | **0** — a full-speed hub |
| 11 | PORT_TEST | **0** |
| 12 | PORT_INDICATOR | **0** |

| Bit | `wPortChange` | This block |
|-----|---------------|-----------|
| 0 | C_PORT_CONNECTION | `connection` changed |
| 1 | C_PORT_ENABLE | **0** — nothing here disables a port on an error |
| 2 | C_PORT_SUSPEND | a resume completed |
| 3 | C_PORT_OVER_CURRENT | **0** |
| 4 | C_PORT_RESET | a reset completed |

**HIGH** (§11.24.2.6). `wHubStatus` and `wHubChange` are four more bytes, and
all four are zero here: bit 0 of the status is a local power supply this
bus-powered hub does not have and bit 1 is an over-current condition it cannot
detect, so neither can change.

### Where `port_attached` comes from

`port_attached` and `port_low_speed` are **inputs** of `usb_hub`, and on the
design this block was written for they are `ip/usb_host_ulpi`'s `attached` and
`low_speed`:

- **`attached`** is that block's own **debounced** sight of its bus leaving
  SE0 — its `DEBOUNCE_CYCLES` is 100 ms at 60 MHz — which is a device present
  on the socket and is exactly what §11.24.2.7.1 makes PORT_CONNECTION.
- **`low_speed`** is which line that device pulled up: **HIGH** (USB 2.0
  §7.1.5.1) a full-speed device pulls D+ up through 1.5 kOhm and a low-speed one
  pulls D- up, so the host's `LineState` says which. It is only meaningful while
  something is pulling, which is why the block gates PORT_LOW_SPEED on
  `connection` — the specification calls it the speed of the *attached* device
  for the same reason.

**`attached` and not `up`.** `ip/usb_host_ulpi` also reports `up`, meaning it
finished enumerating the device for itself. Whether *our* host has enumerated it
is not what a PC's PORT_CONNECTION means, and reporting `up` would hide a device
our host failed on from a host that might have done better.

### Why a connection is gated on power, and what it buys

**HIGH** (USB 2.0 §11.5.1.1). A port in the Powered-off state has no meaningful
connection status, and a hub's ports are powered off until the host powers them.

**MEDIUM**, and this is the design's neatest consequence: gating `connection`
on `port_power` is what makes **the first connection reportable at all**.

The problem it solves is the one `ip/usb_cdc_acm` got wrong. A device may well
already be plugged into the downstream socket before the PC has even seen the
hub, so there is no edge of `port_attached` for the hub to notice. An edge
detector would miss it. A one-shot at configuration would catch it and would be
the same latch that went wrong next door. But `port_power` starts at zero and
the host sets it — `hub_power_on` runs immediately after the hub is configured —
so `connection` rises **when the host powers the port**, which is to say at the
moment the host is listening and expecting to be told. The change is recorded
then, with no edge detector and no one-shot.

The other half of it is that **a hub that is not configured has powered-off
ports**: `configured` going low, on a bus reset or SET_CONFIGURATION 0, clears
the power, the enable, the suspend and all three change bits. So a second
enumeration goes round the same path as the first and the device is reported all
over again. `ip/usb_cdc_acm` needed an extra trigger —
SET_CONTROL_LINE_STATE — for exactly the case this covers for nothing.

---

## 6. The status-change endpoint, and the latch that is not there

**HIGH** (USB 2.0 §11.12.4). The hub reports changes as a **bitmap** on its
interrupt IN endpoint: bit 0 for the hub itself and bit n for port n, which for
one port is one byte. A host polls it, is given the bitmap when something has
changed, and is NAKed when nothing has.

**Bit 0 is a constant zero here.** §11.24.2.6 puts two things in `wHubChange`
and this hub has neither, so a one-port hub's bitmap is `02h` or nothing.

**There is no state in the sender at all.** It is one wire:

```verilog
wire owed = configured & (change_map != 8'h00);
```

The bitmap is a function of the change bits, those bits are **sticky** — set
when something changes, cleared only by the host's own
ClearPortFeature(C_PORT_*) — and a packet is armed whenever any of them is set
and the endpoint has room. The one byte is handed over and committed in the
same cycle, so a one-byte packet goes out of a two-byte endpoint rather than
waiting for a second byte that is not coming.

### Why there is no record of having reported

**This block has been bitten once already by a notification endpoint with a
latch in it, and the account is worth reading before changing anything here.**
[`ip/usb_cdc_acm/README.md`](../usb_cdc_acm/README.md) §4 is it in full. The
short of it: that endpoint had a register meaning "the host has been told", it
was set once per configuration, and a host that was not listening at that moment
— or a driver bound a second time without a bus reset — never heard again.
Measured on a part as `TIOCMGET = 0x026`, no DCD and no DSR, on three
consecutive opens of a port that was working.

The reasoning that produced that defect looked complete, and the thing wrong
with it was an assumption nobody wrote down: **that the host which receives a
notification is the host that will act on it.** Nothing in a device can know
that.

So this block holds no such record. What a hub reports is state the host must
clear, and the host's ClearPortFeature **is** the acknowledgement — which USB
2.0 asks for anyway, since §11.24.2.2 is how a change bit is cleared and there
is no other way. A design with no latch cannot have the defect, and
`usb_hub_status_change_endpoint_reports_every_change` asserts a **second**
change report and the same bitmap twice with the state deliberately unchanged,
so that nothing but the absence of a one-shot can produce them.

### What having no latch costs

Two things, both bounded and both in the open.

**A host that has read the bitmap and not yet cleared the change is sent it
again.** The change bit is still set, so a packet arms as soon as the host's ACK
frees the buffer. That is one extra one-byte packet per poll until the
ClearPortFeature lands, which on Linux is a few milliseconds and one or two
polls. The driver ORs the bitmaps together and acts once.

**A packet already armed when the host clears the change goes out with the old
bitmap.** `usb_bulk_ep` is a store-and-forward packet buffer and has no way to
unsay a packet it has been given. Exactly one, and deterministically one: the
host acknowledged the previous packet, the bit was still set, so a packet armed;
then the ClearPortFeature landed; then that packet went. A host does one
GetPortStatus over it, finds nothing changed, and carries on.
`StatusPipe::settles` in `tests/ip_library.rs` **asserts** it rather than
tolerating it, so that anything which changed it fails there and is read.

The alternative to both is a latch that says "this bitmap has already gone",
and that is the defect above with a different name. The extra packet is paid.

---

## 7. What the class hook could and could not express

`usb_ctrl_ep`'s class hook is nine ports and the contract that `class_claim`,
`class_len` and `class_byte` are combinational in `class_setup`. The hub needed
more of it than the first class did, and this section is what it found.

**Everything the hub needs fits.** Six class requests decode from eight bytes
against constants, which is what the hook is for; three of them read and their
longest data stage is the nine-byte hub descriptor, which `CLASS_MAX = 9` sets
the width of endpoint 0's counters from; and three of them carry nothing at all,
so the hook's host-to-device half is not taken. The one asymmetry of the hook —
a class request that writes may carry at most one eight-byte packet — does not
bind, because **no hub request has an OUT data stage** except SetHubDescriptor,
which §11.24.2.10 makes optional and this block stalls.

**One thing the hub needs is a standard request, and it is why this block
claims one.** Linux's `hub_configure` sends the **standard** GET_STATUS of USB
2.0 §9.4.5, device recipient — `80h 00h`, `wLength` 2 — with the comment "power
budgeting mostly matters with bus-powered hubs", and takes its failure path with
`can't get hub status` if the transfer does not complete. `usb_ctrl_ep`
implements five standard requests and GET_STATUS is not one of them, so without
a claim the hub stalls it and is not a hub as far as that driver is concerned.

The hook offers it: its own header says `class_req` is raised for everything but
the five, "string descriptors and GET_STATUS included, so a class that wants
those can have them without this file changing again". So `usb_hub_req` claims
it and answers two zero bytes — bus powered, remote wake-up not enabled, which
agrees with the configuration descriptor's `bmAttributes` and with there being
no SET_FEATURE(DEVICE_REMOTE_WAKEUP) anywhere in this library.

**And that is a layering smudge, reported rather than worked around.** A
standard request belongs in endpoint 0, and the right change is to `usb_ctrl_ep`:
GET_STATUS to a device is two bytes derived from `CFG_ATTR` and a remote-wakeup
bit nothing sets, it is the same for every device in the library, and putting it
there would mean the next class does not have to think about it. It is **not**
made in this round because it widens `std_req` on the two cores that already run
on silicon, and this round has no measurement that would catch a regression in
them — the hub's own tests would pass either way. It is a small, well-understood
change for whoever has a board and the CDC tests in front of them.

**One thing the hook cannot express and the hub does not need.** There is no way
for a class to STALL a request it recognises: `class_claim` promises an answer.
A hub has a use for that which `ip/usb_cdc_acm` did not — §11.24.2.12 wants a
STALL for a feature request made while the port is in the wrong state, which is
a condition the feature selector alone does not decide. This block does not
reach it, because none of the three features it sets has a state precondition it
can fail. A hub that implemented suspend properly would, and the answer would
be either a tenth port on the hook or a feature decoder that declines to claim
on a condition — the second of which is what `usb_cdc_req` already does for
`wIndex`, so it is probably enough.

---

## 8. What a host said

A Great Scott Gadgets Cynthion r1.4 holding
`testdata/fpga/cynthion/usb_hub_target.v`, built with `VBUS_AUX = 1`: this
block behind `ip/usb_device_ulpi`'s link layer on the **AUX** transceiver,
`ip/usb_host_ulpi` on the **TARGET** one, and a Great Scott Gadgets GreatFET
(`1d50:60e6`) in the TARGET-A socket. Linux 6.18.41-gentoo, `xhci_hcd`, the
Cynthion on a full-speed downstream port of a hub. Everything below is
**quoted**, not paraphrased.

### The kernel bound its own hub driver

```text
usb 7-5: USB disconnect, device number 107
usb 7-5: new full-speed USB device number 109 using xhci_hcd
usb 7-5: New USB device found, idVendor=1209, idProduct=0001, bcdDevice= 1.00
usb 7-5: New USB device strings: Mfr=0, Product=0, SerialNumber=0
hub 7-5:1.0: USB hub found
hub 7-5:1.0: 1 port detected
```

**CHECKED.** Those last two lines are the whole point of this block: **`hub`**,
the driver that ships with the operating system, claimed interface 0, read the
hub descriptor of §3, believed it, and made a port. Nothing of this project
runs on the host.

```console
$ readlink -f /sys/bus/usb/devices/7-5/7-5:1.0/driver
/sys/bus/usb/drivers/hub
$ cat /sys/bus/usb/devices/7-5/maxchild
1
```

**CHECKED.** `maxchild` is the kernel's own record of `bNbrPorts`, taken out of
the hub descriptor it fetched for itself, so that one attribute says both that
the driver bound and that it read and believed the class-specific descriptor.
`tests/usb_hub.rs` asserts exactly that, for the same reason
`tests/usb_cdc_acm.rs` walks sysfs for a `/dev/ttyACM*`: the bus and device
numbers are not properties of this device and nothing should look for one.

### And then it tried to enumerate through the port, and failed

```text
usb 7-5.1: new full-speed USB device number 110 using xhci_hcd
usb 7-5.1: device descriptor read/64, error -71
usb 7-5.1: device descriptor read/64, error -71
usb 7-5.1: new full-speed USB device number 111 using xhci_hcd
usb 7-5.1: device descriptor read/64, error -71
usb 7-5.1: device descriptor read/64, error -71
usb 7-5-port1: attempt power cycle
usb 7-5.1: new full-speed USB device number 112 using xhci_hcd
usb 7-5.1: Device not responding to setup address.
usb 7-5.1: Device not responding to setup address.
usb 7-5.1: device not accepting address 112, error -71
usb 7-5.1: WARN: invalid context state for evaluate context command.
usb 7-5.1: new full-speed USB device number 113 using xhci_hcd
usb 7-5.1: Device not responding to setup address.
usb 7-5.1: Device not responding to setup address.
usb 7-5.1: device not accepting address 113, error -71
usb 7-5.1: WARN: invalid context state for evaluate context command.
usb 7-5-port1: unable to enumerate USB device
```

**This is the correct outcome and it is not being apologised for.** §2 is why:
nothing forwards a packet from the AUX bus to the TARGET bus, so the device the
kernel has been told about cannot answer. The round that builds the transaction
proxy is the round in which these lines change, and until then any other result
would mean something was being faked. `usb 7-5.1` is the *name the kernel gave
a device behind our port*, and the fact that it got that far — a device number,
a reset, a SET_ADDRESS attempt — is the measure of how much of a hub this
block is.

**CHECKED**, and it is worth saying for whoever is looking at their own kernel
log: this stops. The kernel tries twice, power-cycles the port, tries twice
more, gives up, and says nothing further for as long as the port's connection
does not change. It is not a log that fills.

**MEDIUM**. `error -71` is `EPROTO`, which is the host controller reporting that
the transaction did not complete, and `attempt power cycle` is
ClearPortFeature(PORT_POWER) followed by SetPortFeature(PORT_POWER) — so the
kernel exercised both directions of a feature this block honours while failing
at the thing it does not do.

### What it read off the part

```console
$ lsusb -d 1209:0001 -v
Bus 007 Device 109: ID 1209:0001 Generic pid.codes Test PID
Negotiated speed: Full Speed (12Mbps)
Device Descriptor:
  bLength                18
  bDescriptorType         1
  bcdUSB               2.00
  bDeviceClass            9 Hub
  bDeviceSubClass         0 [unknown]
  bDeviceProtocol         0 Full speed (or root) hub
  bMaxPacketSize0        64
  idVendor           0x1209 Generic
  idProduct          0x0001 pid.codes Test PID
  bcdDevice            1.00
  iManufacturer           0
  iProduct                0
  iSerial                 0
  bNumConfigurations      1
  Configuration Descriptor:
    bLength                 9
    bDescriptorType         2
    wTotalLength       0x0019
    bNumInterfaces          1
    bConfigurationValue     1
    iConfiguration          0
    bmAttributes         0x80
      (Bus Powered)
    MaxPower              100mA
    Interface Descriptor:
      bLength                 9
      bDescriptorType         4
      bInterfaceNumber        0
      bAlternateSetting       0
      bNumEndpoints           1
      bInterfaceClass         9 Hub
      bInterfaceSubClass      0 [unknown]
      bInterfaceProtocol      0 Full speed (or root) hub
      iInterface              0
      Endpoint Descriptor:
        bLength                 7
        bDescriptorType         5
        bEndpointAddress     0x81  EP 1 IN
        bmAttributes            3
          Transfer Type            Interrupt
          Synch Type               None
          Usage Type               Data
        wMaxPacketSize     0x0002  1x 2 bytes
        bInterval              12
Hub Descriptor:
  bLength               9
  bDescriptorType      41
  nNbrPorts             1
  wHubCharacteristic 0x0011
    Per-port power switching
    No overcurrent protection
  bPwrOn2PwrGood       50 * 2 milli seconds
  bHubContrCurrent    100 milli Ampere
  DeviceRemovable    0x00
  PortPwrCtrlMask    0xff
 Hub Port Status:
   Port 1: 0000.0101 power connect
Device Status:     0x0000
  (Bus Powered)
```

Five things in that are worth pointing at.

**`bDeviceProtocol 0 Full speed (or root) hub`** is `lsusb`'s own words for the
field §3 says is the whole statement of what kind of hub this is. **CHECKED**:
a second decoder, written by someone else, read that byte the way §11.23.1 says
to.

**`wHubCharacteristic 0x0011`** with `lsusb`'s own decoding under it —
`Per-port power switching`, `No overcurrent protection` — is the independent
confirmation of §3's bit-field reading that this project cannot give itself.
The block states `0011h` from a reading of §11.23.2.1; `lsusb` takes the same
two bytes apart into the same two claims.

**`wTotalLength 0x0019`** is twenty-five, and `bLength 9` / `nNbrPorts 1` with
`DeviceRemovable 0x00` and `PortPwrCtrlMask 0xff` is the nine-byte hub
descriptor of §3 read back field by field — including the two variable-length
masks whose size `bDescLength` has to agree with.

**`Hub Port Status: Port 1: 0000.0101 power connect`** is `wPortChange` and
`wPortStatus`: `0101h` is PORT_POWER and PORT_CONNECTION, with no change
outstanding. **CHECKED**: the kernel's hub driver sent
SetPortFeature(PORT_POWER) — which is §3's warning about `wHubCharacteristics`
D1:D0 coming out right — and the port is reporting the GreatFET on the other
side of the die.

**`Device Status: 0x0000 (Bus Powered)`** is the **standard** GET_STATUS of §7,
the one request `usb_ctrl_ep` does not implement and this class claims on the
hook. A hub that stalled it would not have reached the lines above at all.

### The port reported a device, lost it, and reported it again

**CHECKED**, and this is the on-the-part half of §6's assertion. Over usbfs,
without claiming the interface away from the kernel's own hub driver —
`tests/usb_hub.rs` is the whole of it, and §10 says why it reads the change
bits through GetPortStatus rather than off the endpoint:

```text
wPortStatus 0x0101, wPortChange 0x0000        as the kernel's driver left it
powered off: wPortStatus 0x0000, wPortChange 0x0001
powered on again: wPortStatus 0x0101, wPortChange 0x0001
left as found: wPortStatus 0x0101, wPortChange 0x0000
```

ClearPortFeature(PORT_POWER) took the port's power away, and with it the
connection — §11.5.1.1's powered-off port, which reports nothing at all — and
C_PORT_CONNECTION said so. ClearPortFeature(C_PORT_CONNECTION) cleared it.
SetPortFeature(PORT_POWER) brought both back, and **C_PORT_CONNECTION was set a
second time**. A hub with a one-shot anywhere in it fails that last step, and
the one-shot is the defect `ip/usb_cdc_acm/README.md` §4 writes up from this
same board.

The device never went anywhere: the socket's real VBUS is the board's own
`aux_vbus_en`, which no class request reaches, which is §4's last subsection and
is why the connection comes straight back.

### And the interrupt endpoint carried it

**CHECKED**. The power cycle above was sent from userspace, and about a second
later the kernel started enumerating the port again of its own accord:

```text
usb 7-5.1: new full-speed USB device number 114 using xhci_hcd
usb 7-5.1: device descriptor read/64, error -71
...
usb 7-5-port1: unable to enumerate USB device
```

**MEDIUM**, for the causality rather than the log: nothing told the kernel that
the port had changed except the status-change endpoint. The hub driver does not
poll `GetPortStatus` on a timer for a hub that is awake — its interrupt URB on
endpoint `81h` is the only thing watching — and the requests that caused the
change went over usbfs, which the kernel does not inspect. So the bitmap of §6
went out on real silicon, the driver read it, and it acted.

That matters because it is the one thing about this block that otherwise had no
measurement: a status-change endpoint that NAKed for ever would have left every
assertion above passing and the hub unable to notice anything, and §10 was going
to say so. It does not have to.

**What is still only simulated** is the endpoint's *packet*: ten reports in a
row, the NAKs in between, the bitmap's own byte. Reading endpoint `81h` directly
would mean taking the interface off the kernel's hub driver, which
`tests/usb_hub.rs` refuses to do.

### The gateware's own view, over the T14 console

**CHECKED.** The console `testdata/fpga/cynthion/usb_hub_target.v` puts on ball
T14 — which Apollo bridges to `/dev/ttyACM0` and which exists because AUX is
the hub and cannot be a serial port as well — printed, in order, over the
seventeen seconds its drive window is open:

```text
H90D01120
HFDD01120
HFDD01120
HF9D01120      (and twelve more of the same)
```

That design's header is the field table. Decoded:

| | byte 0, the hub | byte 1, the host | stage | line |
|---|---|---|---|---|
| `90 D0 11 20` | ready, **not yet configured**, bus reset seen | ready, attached, full speed, enumerated | 17 | J |
| `FD D0 11 20` | ready, configured, addressed, **port powered and enabled** | the same | 17 | J |
| `F9 D0 11 20` | the same, port powered and **not** enabled | the same | 17 | J |

which is the kernel's own sequence seen from the other side: the PC reset the
AUX bus and configured the hub, powered the port, reset the port — which is
what enabled it — and then, having failed to enumerate anything through it, left
the port powered and disabled. `ip/usb_host_ulpi`'s half of the board is
unchanged throughout: `attached`, full speed, `up`, stage 17, LineState `01`.

**One reading in that is not explained and is left visible.** Byte 1 bits 2:1
are the TARGET transceiver's `VbusState` and they read `00`, which USB334x
Table 6-3 makes "below SessEnd" — no power on the port at all — while
`LineState` reads `01` and a device is attached, enumerated and answering. The
two disagree, and what settles it is not in this round: whether that
transceiver's VBUS sense pin is connected to the TARGET-A node on this board is
a question for its schematic. `attached`, `up` and `LineState` are the readings
this section relies on and none of them depends on it.

---

## 9. What this block does not do

Everything in this list is a thing a reader might reasonably expect and will not
find.

**It does not forward anything.** §2 is the whole of why, and it is the next
round's work. A device on the port is reported and never spoken to.

**One port.** `bNbrPorts` is 1 and it is a localparam, not a parameter, because
a parameter with one legal value is a lie about what has been built. A second
port is a second copy of five registers, a wider `wIndex` comparison, a second
bit of the change bitmap and two more bytes of hub descriptor whose lengths
§11.23.2.1 makes depend on the count. None of it is hard and none of it is here.

**No downstream signalling of any kind**: no SE0 for a port reset, no suspend or
resume on the port, no frame forwarding, no packet repeating, no connect or
disconnect detection of its own — `port_attached` is an input.

**No transaction translator and no high speed**, which `bDeviceProtocol` says
and the four stalled TT requests follow from.

**No over-current detection, no local power supply, no port indicators and no
test modes**, each of which is a bit in `wHubCharacteristics` or a stalled
feature that says so.

**Nothing switches a VBUS pin from a class request.** §4's last subsection.

**No strings**, so the hub has no product name in `lsusb`, for the same reason
`ip/usb_cdc_acm` has none: a string descriptor is a device's property and not a
class's.

**No suspend of the hub itself**, which is `usb_ctrl_ep`'s limit and not this
block's.

---

## 10. What each test would and would not catch

Ten tests in `tests/ip_library.rs`, and what they are each for.

| Test | What it would catch | What it would not |
|------|--------------------|-------------------|
| `usb_hub_enumerates_as_a_hub`, and the two ULPI variants | every byte of the device and configuration descriptors wrong, in either order, through short reads and a read that ends on `wLength`, and a second enumeration after a bus reset | that a driver binds to them |
| `usb_hub_descriptors_carry_what_a_host_hub_driver_binds_on` | the three properties Linux's `hub_probe` refuses an interface over — a subclass that is not 0 or 1, a count of endpoints that is not one, an endpoint that is not interrupt IN — and a hub descriptor whose `bDescLength` disagrees with its length | the same; it is an assertion about an expectation written in Rust, so it pins **why** those bytes and adds nothing about the block |
| `usb_hub_answers_the_hub_and_port_class_requests`, and the ULPI variant | a request not claimed, a wrong byte in any of the three reads, a feature that does not reach the state it names, a change bit that does not clear, the port's state wrong for any of eight combinations of power, connection, enable, suspend and speed | whether a host sends them in that order, or at all |
| `usb_hub_stalls_the_class_requests_it_does_not_claim` | a claim that is too wide — a port number this hub does not have, a feature it never offered, a TT request, a feature request with a payload — and a standard request shadowed by the class | a claim that is too narrow: a request a host sends and this block stalls fails on a host and not here |
| `usb_hub_status_change_endpoint_reports_every_change`, and the ULPI variant | **a one-shot**: a second change not reported, a bitmap sent once where the state has not moved, a change reported while the port is powered off, a report after the host cleared the change bit, or no report at all after a bus reset and a second configuration | whether a host's driver acts on the bitmap, and whether 12 frames is often enough |
| `usb_hub_is_one_clock_domain` | a crossing added to either wrapper | — |
| `every_block_maps_to_the_logic_it_was_mapped_from` | the hub's LUT4 and LUT6 mapping differing from the logic it came from, **proved** rather than sampled | anything after mapping: placement, routing, the bitstream |
| `usb_descriptors_survive_lookup_table_mapping` | a **mapped** netlist answering GET_DESCRIPTOR with the wrong bytes, which is how a one-byte defect in `ip/usb_cdc_acm` was found after a kernel refused the device | the hub descriptor, which is a `case` in `usb_hub_req` and not a ROM, and the ULPI wrapper, and the array shape of the status endpoint's buffer |
| `footprints_match_the_documentation` | the block suddenly costing twice as much | — |

**What none of them can catch, and it is the thing that matters most:
whether the driver binds.** A descriptor set can be exactly what §11.23.1
describes and still not be what a kernel's hub driver looks for, and no model
of a host written from the same specification as the device can tell you
otherwise. §8 is the other half, and §3's own warning about
`wHubCharacteristics` D1:D0 is the clearest example of a field no simulation has
an opinion about: both values are legal, both pass every test above, and only a
host decides which one makes the port work.

**And one more test, which is a board and not a model.**
`tests/usb_hub.rs` is `#[ignore]`d, needs `--features program`, and skips with a
reason when no `1209:0001` is attached. It asserts the three things §8 quotes:
that the driver named `hub` holds the interface and the kernel recorded one
port, that the descriptors the part reports are the ones the sources describe
— including the nine-byte hub descriptor asked for as fifteen — and that the
port reports a connection, loses it to ClearPortFeature(PORT_POWER), and
**reports it a second time** when the power comes back. What it cannot do is
read endpoint `81h`, because that would mean taking the interface off the
kernel's own hub driver; §8's last-but-one subsection is what established the
endpoint instead.

**And what the simulation establishes that a board cannot.** The stale-line
transceiver — `UlpiPhy::reporting_stale_line`, which reproduces a real
misbehaviour of the Microchip part on this board against ULPI §3.8.1.3 — runs
every hub assertion through a transceiver that reports `LineState` a clock late.
A board has one transceiver behaving one way; the model has that one and three
others.

---

## 11. Reading list

- *Universal Serial Bus Specification, Revision 2.0*, chapter 11 — the hub.
  §11.1.1 and §11.3 for what a hub is, §11.5.1 for the port's state machine,
  §11.12.4 for the status change bitmap, §11.23.1 for the standard descriptors,
  §11.23.2.1 for the hub descriptor, §11.24.2 for the class requests.
- The same, §5.7.3 and §5.8.3 for what sizes an interrupt and a bulk endpoint
  may declare; §7.1.19.1 for how long a host waits; §8.4.5 for NAK as flow
  control; §9.4.5 for the standard device status.
- [`ip/usb_device_ulpi/README.md`](../usb_device_ulpi/README.md) — ULPI, fact by
  fact, with the provenance of each. Not repeated here.
- [`ip/usb_host_ulpi/README.md`](../usb_host_ulpi/README.md) — the other end of
  the wire, which is what this block's downstream port reads.
- [`ip/usb_cdc_acm/README.md`](../usb_cdc_acm/README.md) — the first class layer,
  and §4 of it the notification-endpoint defect this block's §6 is shaped
  against.
- [`docs/ip-library.md`](../../docs/ip-library.md) — the class hook, the
  footprint table, and what every block in the library costs.
