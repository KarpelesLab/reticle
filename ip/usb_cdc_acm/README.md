# CDC ACM, and a serial port the operating system already has a driver for

`usb_cdc_acm` is a USB serial port. A device built on it appears as
`/dev/ttyACM*` on Linux, `/dev/cu.usbmodem*` on macOS and a COM port on
Windows, and **nothing has to be installed for that to happen**: the
Communications Device Class is one the operating system already knows, so
the host half of the work is a driver somebody else wrote and ships.

That is the whole reason it is the first class layer in this library.
`ip/usb_device_fs` and `ip/usb_device_ulpi` move bytes over a
vendor-specific interface, which is honest and is also a program's problem:
somebody has to claim the interface and speak to it. This block moves the
same bytes with no program at all — `cat /dev/ttyACM1` is the client.

This document is laid out the way
[`ip/usb_device_ulpi/README.md`](../usb_device_ulpi/README.md) is, and for
the same reason: the facts in it have very different provenance. Some are
read out of a published table. Some are a reading of what a *host driver*
does, which no specification states and only a host can confirm. §1 says
which is which and every claim below is marked.

Nothing was transcribed from anybody's implementation. The specifications
are published documents and the encodings below are read out of them; the
descriptors, the request decoder and the endpoint arrangement are this
repository's own.

---

## 1. Confidence, and what it is based on

- **HIGH** — stated in one of the two published specifications, with the
  table or section named so it can be checked without reading any code:
  - *Universal Serial Bus Class Definitions for Communication Devices*,
    **Revision 1.1**, 19 January 1999 — "CDC 1.1" below.
  - *Universal Serial Bus Communications Class Subclass Specification for
    PSTN Devices*, **Revision 1.2**, 9 February 2007 — "PSTN 1.2" below.
  - *Universal Serial Bus Specification*, **Revision 2.0** — "USB 2.0".
- **MEDIUM** — a reading of one of those which it does not state in so many
  words, or a statement about what a host driver does that is consistent
  with everything observed and is not written down anywhere authoritative.
- **CHECKED** — **measured on a host**, with the output quoted in §5. This
  category does not exist in the ULPI document because that one is about a
  bus; here it is the important one, since the question a class layer has to
  answer is not "is this descriptor legal" but "does the driver bind".
- **LOW** — inference that explains the rest, with no way to check it here.

A thing can be HIGH and wrong about the world: a descriptor can be exactly
what a table says and still not be what a driver looks for. Where this
document has both, it says both.

---

## 2. Why a serial port needs two interfaces

**HIGH** (CDC 1.1 §3.1, §3.2). A communications device is at minimum two
interfaces:

- a **communications class interface** — `bInterfaceClass` `02h` — which
  carries the *management* element (the class-specific control requests,
  over endpoint 0) and optionally a *notification* element (an interrupt
  IN endpoint);
- a **data class interface** — `bInterfaceClass` `0Ah` — which carries the
  bytes.

**HIGH** (CDC 1.1 §3.1.1, Table 33). Nothing about the interface
descriptors themselves says the two belong together. What says it is the
**union functional descriptor**, which names one *master* interface and its
*subordinate* interfaces. A device with two interfaces and no union
descriptor is two unrelated interfaces as far as a host can tell.

**CHECKED**, and precisely this much: a device whose descriptors are the set
in §3 — union descriptor included — is bound by `cdc_acm` on the kernel §5
names, and the port opens and carries bytes.

**MEDIUM**, and no more, for *why*: that `cdc_acm` reads the union descriptor
to decide which interface is the control one and which carries the data, and
that a device without one reaches a path meant for devices in its own quirk
table. Nothing here read that driver's source and nothing here watched the
bus. What would settle it is a device built **without** the union descriptor,
which this project has not put on a board — so the union descriptor is here
because the specification asks for it and because leaving it out is a risk
nobody has measured, not because its absence was seen to fail.

**MEDIUM**. That is why the union descriptor is not optional *in practice*
even though CDC 1.1 Table 33 describes it as one of several functional
descriptors: it is the one whose absence changes whether the device works.

### The subclass

**HIGH** (CDC 1.1 Table 16). `bInterfaceSubClass` `02h` is the **Abstract
Control Model**, which is the subclass whose management element is the
line-coding and control-line requests rather than a full modem's. PSTN 1.2
§3.6 is where the model is described.

### The protocol byte

**HIGH** (CDC 1.1 Table 17). `bInterfaceProtocol` `00h` is "No class
specific protocol required" and `01h` is "AT Commands: V.250 etc".

**This block says `00h`**, and that is a deliberate choice rather than a
default: it answers no AT commands, and a descriptor that claimed V.250
would be a false statement about the device. Most CDC ACM devices in the
world say `01h`.

**CHECKED**. `cdc_acm` binds on the class and the subclass; it does not
read `bInterfaceProtocol` to decide. §5 is a device with `00h` bound by it.

---

## 3. The descriptor set, field by field

Fifty-eight bytes, in descriptor order. With the nine bytes of the
configuration descriptor that `usb_ctrl_ep` writes, `wTotalLength` is
**67**.

`bNumInterfaces`, both `bNumEndpoints` fields and `wTotalLength` are not in
the block's parameter at all: `usb_ctrl_ep` counts them out of the blob at
elaboration and writes them over what is there.
[`docs/ip-library.md`](../../docs/ip-library.md) says why. The `0` sitting
in byte 4 of each interface descriptor below is a placeholder.

### INTERFACE 0 — the communications interface (9 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bLength | 9 | **HIGH** USB 2.0 Table 9-12 |
| 1 | bDescriptorType | 4 (INTERFACE) | **HIGH** USB 2.0 Table 9-5 |
| 2 | bInterfaceNumber | 0 | this block's choice |
| 3 | bAlternateSetting | 0 | one setting only |
| 4 | bNumEndpoints | *counted* — 1 | derived at elaboration |
| 5 | bInterfaceClass | `02h` | **HIGH** CDC 1.1 Table 15 |
| 6 | bInterfaceSubClass | `02h` Abstract Control Model | **HIGH** CDC 1.1 Table 16 |
| 7 | bInterfaceProtocol | `00h` no class protocol | **HIGH** CDC 1.1 Table 17 |
| 8 | iInterface | 0 | no strings; §6 |

### Header functional descriptor (5 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bFunctionLength | 5 | **HIGH** CDC 1.1 Table 26 |
| 1 | bDescriptorType | `24h` CS_INTERFACE | **HIGH** CDC 1.1 Table 24 |
| 2 | bDescriptorSubtype | `00h` Header | **HIGH** CDC 1.1 Table 25 |
| 3–4 | bcdCDC | `0110h` — 1.10, little endian | **HIGH** CDC 1.1 Table 26 |

**HIGH** (CDC 1.1 §5.2.3). The header functional descriptor must be the
first of the class-specific descriptors of a communications interface.

### Call Management functional descriptor (5 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bFunctionLength | 5 | **HIGH** PSTN 1.2 Table 3 |
| 1 | bDescriptorType | `24h` | **HIGH** |
| 2 | bDescriptorSubtype | `01h` Call Management | **HIGH** CDC 1.1 Table 25 |
| 3 | bmCapabilities | `00h` | see below |
| 4 | bDataInterface | 1 | the data interface's number |

**HIGH** (PSTN 1.2 Table 3). `bmCapabilities` D0 is "device handles call
management itself" and D1 is "device can send/receive call management
information over a Data Class interface". Both are clear: a serial port
manages no calls.

**MEDIUM**. `bDataInterface` is required to be a real interface number even
when no call management happens over it, so it names interface 1.

### Abstract Control Management functional descriptor (4 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bFunctionLength | 4 | **HIGH** PSTN 1.2 Table 4 |
| 1 | bDescriptorType | `24h` | **HIGH** |
| 2 | bDescriptorSubtype | `02h` Abstract Control Management | **HIGH** CDC 1.1 Table 25 |
| 3 | bmCapabilities | `02h` | see below |

**HIGH** (PSTN 1.2 Table 4). The four bits of `bmCapabilities`:

| Bit | What it claims | This block |
|-----|----------------|-----------|
| D0 | Set_Comm_Feature, Clear_Comm_Feature, Get_Comm_Feature | **clear** — not implemented |
| D1 | Set_Line_Coding, Set_Control_Line_State, Get_Line_Coding, and the Serial_State notification | **set** |
| D2 | Send_Break | **clear** — not implemented |
| D3 | the Network_Connection notification | **clear** |

**D1 is one bit over four things and this block does three of them.** It
answers all three requests and never sends a Serial_State notification; §4
says why it cannot. Setting D1 is therefore not perfectly true, and it is
set anyway, for a reason worth writing down:

**CHECKED**. With D1 set, **Linux sends SET_LINE_CODING and its seven bytes
arrive**: `stty -F /dev/ttyACM1 115200` and then GET_LINE_CODING asked of the
part reads `115200` back out of the device's own registers, whose reset value
is 9600. §5 has it. That is the only measurement anywhere of this hook's
host-to-device data stage on real hardware, and it is the reason the request
group is claimed rather than left out.

**MEDIUM**. That `cdc_acm` keeps D1 as `USB_CDC_CAP_LINE` and *gates*
SET_LINE_CODING on it — so a device that cleared D1 to be pedantic about the
notification would stop being asked the questions it *can* answer. That is
documented driver behaviour and this project has not built the device that
would test it.

So the descriptor claims the request group, which is what a host reads the bit
for, and this paragraph is where the notification quarter of that claim is
retracted in words.

D2 clear is the honest half of the same coin: `usb_cdc_req` does not claim
SEND_BREAK, so endpoint 0 stalls it, and a host that read D2 never sends
one.

### Union functional descriptor (5 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bFunctionLength | 5 | **HIGH** CDC 1.1 Table 33 |
| 1 | bDescriptorType | `24h` | **HIGH** |
| 2 | bDescriptorSubtype | `06h` Union | **HIGH** CDC 1.1 Table 25 |
| 3 | bControlInterface | 0 | the communications interface |
| 4 | bSubordinateInterface0 | 1 | the data interface |

§2 is why this one is the descriptor that matters most.

### ENDPOINT `82h` — the notification endpoint (7 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 0 | bLength | 7 | **HIGH** USB 2.0 Table 9-13 |
| 1 | bDescriptorType | 5 (ENDPOINT) | **HIGH** USB 2.0 Table 9-5 |
| 2 | bEndpointAddress | `82h` — IN, endpoint 2 | **HIGH** USB 2.0 Table 9-13 |
| 3 | bmAttributes | `03h` — interrupt | **HIGH** USB 2.0 Table 9-13 bits 1:0 |
| 4–5 | wMaxPacketSize | 8, little endian | §4 |
| 6 | bInterval | 16 frames | **HIGH** USB 2.0 Table 9-13 |

**HIGH** (USB 2.0 Table 9-13). For a full-speed interrupt endpoint
`bInterval` is a period in frames, 1 to 255, so 16 is 16 ms.

### INTERFACE 1 — the data interface (9 bytes)

| Byte | Field | Value | Provenance |
|------|-------|-------|------------|
| 4 | bNumEndpoints | *counted* — 2 | derived at elaboration |
| 5 | bInterfaceClass | `0Ah` Data | **HIGH** CDC 1.1 Table 18 |
| 6 | bInterfaceSubClass | `00h` | **HIGH** CDC 1.1 §4.5 — unused for data |
| 7 | bInterfaceProtocol | `00h` "No class specific protocol" | **HIGH** CDC 1.1 Table 19 |

### ENDPOINT `01h` and ENDPOINT `81h` — the bytes (7 bytes each)

`bmAttributes` `02h` is bulk (**HIGH**, USB 2.0 Table 9-13 bits 1:0) and
`wMaxPacketSize` is 8 both ways.

**HIGH** (USB 2.0 §5.8.3). A full-speed bulk endpoint's maximum packet size
must be 8, 16, 32 or 64. Eight is the smallest legal one and is what
`usb_bulk_ep` holds.

**MEDIUM**. `bInterval` is meaningless for a full-speed bulk endpoint and is
written as 0.

### The device descriptor

**HIGH** (CDC 1.1 Table 14). A communications device says `02h` in the
**device** descriptor's `bDeviceClass`, with `bDeviceSubClass` and
`bDeviceProtocol` `00h`. That is what tells a host that the interfaces
belong to one function before it has read any functional descriptor, and it
is why no interface association descriptor is needed for a device with one
function.

---

## 4. The notification endpoint, and the notification it never sends

The communications interface has an interrupt IN endpoint. What a CDC ACM
device would send on it is a **SERIAL_STATE** notification, which reports
the state of the incoming control lines and any break, parity or overrun
error the device has seen.

**HIGH** (PSTN 1.2 §6.5.4, Table 31). SERIAL_STATE is
`bmRequestType` `A1h`, `bNotification` `20h`, `wValue` 0, `wIndex` the
interface, `wLength` 2, and then **two bytes** of `UART State Bitmap`. With
the eight-byte notification header (PSTN 1.2 §6.5) that is a **ten-byte**
packet.

**This block never sends one, and could not.** `usb_bulk_ep` holds eight
bytes a packet, because the length field both of this library's
transmitters take is four bits wide. Ten does not fit. Sending one needs a
wider length field in `usb_fs_tx`, in `usb_ulpi_link` and in the endpoint —
not a parameter. `wMaxPacketSize` in the descriptor is therefore **8** and
not 16, because a descriptor says what a device does.

So the endpoint answers every poll with a NAK, for ever.

**MEDIUM**. That is the ordinary resting state of a CDC ACM device: an
interrupt endpoint is polled on a schedule whether or not the device has
anything to say, and NAK is how a device says "not now". There is no
requirement anywhere in PSTN 1.2 that a device ever send a SERIAL_STATE; §6.5
describes what a notification means, not that one must arrive.

**MEDIUM**. What is *not* optional is the endpoint's existence. Linux's
`cdc_acm` takes `endpoint[0]` of the communications interface and requires
it to be an interrupt IN endpoint; a communications interface with no
endpoints does not reach the driver's normal path.

**CHECKED**. §5 is a port that was opened, written to, read from and closed
with no notification ever sent. That establishes the behaviour of **one
driver on one kernel version**, which is a weaker statement than the
specification-level ones above and is marked differently on purpose. A
host that polled an endpoint that never answers and gave up would show
here as the port failing to open, and it does not.

**What a design that needs SERIAL_STATE should do** is not "set a parameter
in this block". It should widen the packet length through both link layers
first; the eight-bytes-a-packet limit is in `usb_bulk_ep`'s own "What it
does not do" for that reason.

---

## 5. What a host said

A Great Scott Gadgets Cynthion r1.4 on its AUX port, holding
`testdata/fpga/cynthion/usb_cdc_uart.v` — this block behind
`ip/usb_device_ulpi`'s link layer, with its bytes bridged to `ip/uart` and
that UART's transmit line looped into its own receiver. Linux 6.18,
`xhci_hcd`, the device on a full-speed downstream port of a hub. Everything
below is quoted, not paraphrased.

### The kernel bound its own driver

```text
usb 7-5: new full-speed USB device number 81 using xhci_hcd
usb 7-5: New USB device found, idVendor=1209, idProduct=0001, bcdDevice= 1.00
usb 7-5: New USB device strings: Mfr=0, Product=0, SerialNumber=0
cdc_acm 7-5:1.0: ttyACM1: USB ACM device
```

That last line is the whole point of this block: **`cdc_acm`**, the driver
that ships with the operating system, claimed interface 0 and made a
terminal. Nothing of this project runs on the host.

```console
$ readlink -f /sys/class/tty/ttyACM1/device/driver
/sys/bus/usb/drivers/cdc_acm
$ ls -l /dev/serial/by-id/
usb-1209_0001-if00 -> ../../ttyACM1
```

**`ttyACM1` and not `ttyACM0`**, because a Cynthion's own Apollo debugger is
itself a CDC ACM device and holds `ttyACM0`. The number is not a property of
this device and nothing should look for one; `tests/usb_cdc_acm.rs` walks
sysfs for `1209:0001` instead.

### And what it read off the part

```console
$ lsusb -d 1209:0001 -v
Device Descriptor:
  bLength                18
  bDescriptorType         1
  bcdUSB               2.00
  bDeviceClass            2 Communications
  bDeviceSubClass         0 [unknown]
  bDeviceProtocol         0
  bMaxPacketSize0         8
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
    wTotalLength       0x0043
    bNumInterfaces          2
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
      bInterfaceClass         2 Communications
      bInterfaceSubClass      2 Abstract (modem)
      bInterfaceProtocol      0
      iInterface              0
      CDC Header:
        bcdCDC               1.10
      CDC Call Management:
        bmCapabilities       0x00
        bDataInterface          1
      CDC ACM:
        bmCapabilities       0x02
          line coding and serial state
      CDC Union:
        bMasterInterface        0
        bSlaveInterface         1
      Endpoint Descriptor:
        bLength                 7
        bDescriptorType         5
        bEndpointAddress     0x82  EP 2 IN
        bmAttributes            3
          Transfer Type            Interrupt
          Synch Type               None
          Usage Type               Data
        wMaxPacketSize     0x0008  1x 8 bytes
        bInterval              16
    Interface Descriptor:
      bLength                 9
      bDescriptorType         4
      bInterfaceNumber        1
      bAlternateSetting       0
      bNumEndpoints           2
      bInterfaceClass        10 CDC Data
      bInterfaceSubClass      0 [unknown]
      bInterfaceProtocol      0
      iInterface              0
      Endpoint Descriptor:
        bLength                 7
        bDescriptorType         5
        bEndpointAddress     0x01  EP 1 OUT
        bmAttributes            2
          Transfer Type            Bulk
          Synch Type               None
          Usage Type               Data
        wMaxPacketSize     0x0008  1x 8 bytes
        bInterval               0
      Endpoint Descriptor:
        bLength                 7
        bDescriptorType         5
        bEndpointAddress     0x81  EP 1 IN
        bmAttributes            2
          Transfer Type            Bulk
          Synch Type               None
          Usage Type               Data
        wMaxPacketSize     0x0008  1x 8 bytes
        bInterval               0
```

`lsusb`'s own decoding of the functional descriptors — "CDC Header", "CDC
Call Management", "CDC ACM", "CDC Union" — is worth noticing for itself: it
is a second, independent parser agreeing that those five bytes are a union
functional descriptor naming interfaces 0 and 1.

All sixty-seven bytes, as the kernel cached them:

```console
$ xxd /sys/bus/usb/devices/7-5/descriptors
00000000: 1201 0002 0200 0008 0912 0100 0001 0000  ................
00000010: 0001 0902 4300 0201 0080 3209 0400 0001  ....C.....2.....
00000020: 0202 0000 0524 0010 0105 2401 0001 0424  .....$....$....$
00000030: 0202 0524 0600 0107 0582 0308 0010 0904  ...$............
00000040: 0100 020a 0000 0007 0501 0208 0000 0705  ................
00000050: 8102 0800 00                             .....
```

**Every bit of that is accounted for.** The device descriptor is the first
eighteen bytes and the configuration descriptor is the remaining sixty-seven,
so the file is eighty-five bytes and there is nothing else in it. Read
against §3's tables: `wTotalLength` `0x0043` is 67 and is the sum of the nine
bytes `usb_ctrl_ep` writes and the fifty-eight in `IFACE_DESC`;
`bNumInterfaces` is 2 and `bNumEndpoints` is 1 and 2, all three counted at
elaboration and none of them in the parameter; the four functional
descriptors are `05 24 00`, `05 24 01`, `04 24 02` and `05 24 06` in the
order CDC 1.1 §5.2.3 gives; and the three endpoint descriptors are `82h`
interrupt, `01h` bulk and `81h` bulk. Nothing is unexplained.

### Bytes, with no program of this project's involved

```console
$ stty -F /dev/ttyACM1 115200 raw -echo clocal
$ cat -v /dev/ttyACM1 &
$ printf 'hello, world\r\n' > /dev/ttyACM1
hello, world^M
```

`printf` and `cat`. The characters went out of endpoint 1 OUT, through
`ip/uart`'s transmitter at 115200 baud, back in through its receiver, out of
endpoint 1 IN, and up through `cdc_acm` and the terminal layer into `cat`.

And as a test, with the descriptors checked against the sources on the way:

```console
$ cargo test --features program --test usb_cdc_acm -- --ignored --nocapture
the kernel gave 1209:0001 the terminal /dev/ttyACM1
configuration descriptor (67 bytes): [09, 02, 43, 00, 02, 01, 00, 80, 32, ...]
bDeviceClass is 02h, bDeviceSubClass and bDeviceProtocol 00h
48 bytes out, 48 bytes back
host -> USB -> UART transmit -> UART receive -> USB -> host, 48 bytes, byte for byte
test a_serial_port_this_compiler_built_is_bound_by_the_kernels_own_driver ... ok
```

### The host set the line coding, and the device kept it

```console
$ cargo test --features program --test usb_cdc_acm -- --ignored --nocapture
...
GET_LINE_CODING: 115200 baud, bCharFormat 0, bParityType 0, bDataBits 8
`cdc_acm` is back on /dev/ttyACM1
```

`stty` asked for 115200 and `usb_cdc_req` comes up at **9600**, so 115200 in
those registers can only have got there through SET_LINE_CODING. That makes
this the one measurement of the class hook's **host-to-device data stage** on
real silicon: the round trip above proves the bulk endpoints, and the port
opening at all proves SET_CONTROL_LINE_STATE, because `cdc_acm` fails `open`
if that stalls — but SET_LINE_CODING is the only request here with a data
packet behind it, and nothing else would notice if its seven bytes had never
arrived.

Getting it took the interface away from `cdc_acm` for the length of one
control transfer, because usbfs refuses a request addressed to an interface
another driver holds:

```text
GET_LINE_CODING was not asked (submit urb: resource busy (os error 16))
```

so the test detaches the driver, claims, asks, releases and attaches it
again — and then checks the terminal came back, which the second line above
is. The line coding survives the detach because it is a register in the
device and not anything the host was keeping.

### And every bit of the bitstream belongs to something

The design is 1386 lookup tables, 649 flip-flops and 20 pads on an LFE5U-12F,
and its bitstream was decoded back through the same Project Trellis records the
router read:

```text
usb_cdc_uart: 57636 configuration bit(s) set, 0 unexplained, 19145 arc(s)
```

**Nothing is unexplained**: every one of those 57636 bits belongs to a feature
the database names, so no bit was set for a reason the database does not know —
which is how a wrong tile rule looks from the inside. The count is also what
`reticle fpga` reports for the same bitstream, so the writer and the decoder
were asked separately and agree.

That check is `tests/fpga_trellis.rs`'s
`the_bitstream_decodes_back_to_the_arcs_the_router_chose`, which makes it of a
small design on every run and was **not weakened**. It was run by hand over
this one, which is two minutes of place and route in a release build and too
slow to keep in the gate.

### The notification endpoint never sent anything

The port enumerated, was opened, carried bytes and was closed, with
**nothing ever sent on endpoint 2 IN**. That is not an observation about
timing — it is the design: `usb_cdc_acm` ties `notif_valid` low, so
`usb_bulk_ep` has nothing armed and answers every poll with a NAK, and there
is no path by which a notification could be sent. §4 says why there could not
be one even if a design wanted it.

So: **CHECKED** — Linux's `cdc_acm` binds, opens, transfers and closes with no
SERIAL_STATE notification ever arriving. That is one driver on one kernel and
is not a statement about PSTN 1.2, which does not require one either.

`clocal` in the `stty` line above is the other half of the same fact, and it
is the one place the omission is visible: a CDC ACM device reports carrier
*through* SERIAL_STATE, so this device never asserts DCD, and an `open`
without `clocal` would wait for a carrier that never comes. A design that
needs a host to see carrier needs the ten-byte packet §4 describes.

### What two of these lines cost to get right

Both faults were invisible in simulation and both are written up where they
belong; they are here because "what a host said" is also the record of what
it said when the device was wrong.

**`config 1 has 1 interface, different from the descriptor's value: 2`.** One
byte of sixty-seven — `bInterfaceNumber` of the data interface, 0 where it
should be 1 — and `cdc_acm` refused the device. The cause was the **LUT4
cover of the descriptor ROM** in `usb_ctrl_ep`, not anything in this block:
the same design was byte-perfect in simulation, and the same descriptor set
in `usb_device_fs` with no class layer at all fails identically. §7 has it,
and `tests/ip_library.rs`'s
`usb_descriptors_survive_lookup_table_mapping` is the test that was
missing — nothing in this repository had ever asked a *mapped* netlist to do
anything.

**Forty-eight bytes back as fourteen, some seven times over.** A UART
transmitter takes its byte on its own `tx_valid && !busy`, so a bridge that
throttles only the endpoint's `out_ready` leaves `tx_valid` asserted and the
transmitter sends the same byte again. That one is the board design's wiring
and nothing to do with these blocks;
`testdata/fpga/cynthion/usb_cdc_uart.v` carries the explanation at the two
lines that fix it.

---

## 6. What this block does not do

- **No strings.** `iManufacturer`, `iProduct`, `iSerialNumber`,
  `iConfiguration` and both `iInterface` are 0, so the port has no name in
  `lsusb` and no stable `/dev/serial/by-id/` entry of its own beyond the
  vendor and product identifiers. String descriptors are the one thing
  `usb_ctrl_ep` stalls that the class hook could now answer, and this block
  does not answer them, because a product name is a device's property and
  not a serial port's.
- **No SERIAL_STATE, no SEND_BREAK, no Comm_Feature.** §3 and §4.
- **Nothing acts on the line coding.** `baud`, `char_format`, `parity` and
  `data_bits` are what the host asked for and are brought out for a design
  to use. Following `dwDTERate` means dividing a clock by a run-time value,
  which is a design's business.
- **Eight bytes a packet**, an eighth of the largest a full-speed bulk
  endpoint may have. A host is limited in *transactions* a frame rather than
  in bytes, so that is close to a factor of eight off what a 64-byte endpoint
  reaches; no figure is quoted because nothing here measured one. It also
  means a host must read **one packet at a time**: a bulk IN transfer ends on
  a short packet or a full buffer, so a read of 64 bytes answered with 8 is
  not finished, and a host that asks again gets a NAK and a timeout from a
  device that is behaving perfectly.
- **No FIFO.** The endpoint holds one packet each way and NAKs while it is
  full, which is what bulk means. A bridge to something as slow as a UART
  wants depth on its receive side, and `ip/fifo_sync` is the block for it —
  see §7.
- **One serial port.** Two would need two of everything and an interface
  association descriptor above them.
- **No suspend, no remote wake-up, no SOF tracking, no high speed.** Those
  are the device core's limits and `ip/usb_device_fs` states them.

---

## 7. What this took, and what it found

### The class hook outlives this block

`usb_ctrl_ep` stalled every request it did not implement itself, which is a
control endpoint no class can sit on. The hook that replaced that is nine
ports and is described at length in that module's header; the shape of it
is the part worth repeating here, because the next class will use it and
not this block:

- the eight bytes of a SETUP, with a one-cycle `class_req` that is raised
  **only** for requests endpoint 0 does not implement, so a class cannot
  shadow SET_ADDRESS;
- `class_claim` and `class_len` read back **combinationally**, in that same
  cycle, because endpoint 0 chooses the transfer's stage in the cycle the
  SETUP's data packet ends — and because a request decoder is a comparison
  of eight bytes against constants with nothing in it to sequence;
- a device-to-host data stage fetched a byte at a time through
  `class_index`, capped at the host's `wLength` exactly as a descriptor's
  is;
- a host-to-device data stage handed over as one packet, up to eight bytes,
  which is every CDC ACM request and is the limit that is written down
  rather than assumed.

The one thing it deliberately does **not** have is a way for a class to
STALL a request it recognises: `class_claim` promises an answer. A class
that must refuse leaves the claim low and takes the STALL endpoint 0 was
going to send.

### A control endpoint's stage register did not grow a bit

The obvious way to add a host-to-device class data stage is a fifth stage
in `usb_ctrl_ep`'s `stage` register, which would take it from two bits to
three — and then, for every design with no class layer, the third bit is a
bit no expression can set. This project has paid eight rounds of
investigation for exactly that shape once.

So there is no fifth stage. A class request that writes is a status-stage
transfer with one data packet expected first, and what recognises that
packet is a register **bit**, `class_out_wait`, which is a bit either way.

### The endpoints generalised, and it cost almost nothing

`usb_dev_core` had one bulk pair and a one-bit `owner`. It now has a second,
IN-only endpoint and two bits — `own_data` and `own_notif` — which are the
`sel` signals themselves rather than an encoded index. That was chosen over
an index for two reasons: an index costs a decoder at every `sel` and at
the transmitter multiplexer, and an index leaves a **high bit** of a
register unset when there is no second endpoint, which is the shape above.
Two bits that are each provably constant-or-not let the whole second
endpoint disappear when `NOTIF_ENDP` is 0.

`usb_bulk_ep` took `WITH_OUT` and `WITH_IN`, and a direction that is zero is
**not answered at all** rather than NAKed. That is the right answer and not
a shortcut: a host must hear nothing from an endpoint the descriptors do
not declare. Nothing is `generate`d away — the registers of a direction
that cannot be asked for have no reader, so synthesis removes them.

### `ip/fifo_sync` cannot be placed on an ECP5

Found while bridging this block to `ip/uart` on the Cynthion, and it is a
gap in the FPGA backend rather than in either block:

```
error: the design needs 2 `lutram` site(s) and the part has 0
```

`fifo_sync`'s storage is an array indexed by a variable, which becomes a
distributed RAM, and `src/fpga/devices/ecp5.dev` declares
`bel TRELLIS_DPR16X4 lutram` with **no `count`**. So `fpga::place` sees zero
sites of it and refuses.

Measured on `ecp5-12f-CABGA256`, the Cynthion's part: depth 16 with `FWFT`,
depth 32 with `FWFT`, and depth 64 without it — **2**, **4** and **8**
`lutram` sites asked for and refused, so neither the depth nor the read
port's shape is what it turns on.
`fpga::synthesize_for` is happy with every one of them, which is why
`tests/ip_library.rs`'s `small_memories_become_logic_after_the_fpga_flow`
passes: it stops before placement.

**Inferred**, not measured: that the other two ECP5 devices in that file
behave the same way, since all three declare the bel without a `count` and
that is the cause. Only the 12F was tried, because it is the only ECP5 this
flow has a Project Trellis part for.

The effect is that **no design in this repository can instantiate
`fifo_sync` on this part**, which is worth knowing independently of serial
ports. `testdata/fpga/cynthion/usb_cdc_uart.v` works around it by carrying
one byte at a time, and says so.

---

## 8. Reading list

- *Universal Serial Bus Class Definitions for Communication Devices*,
  Revision 1.1, 19 January 1999. §3 for the interface arrangement, §5.2.3
  for the order of the functional descriptors, Tables 14 to 19 for the
  class codes, Tables 24 to 26 and 33 for the functional descriptors.
- *Universal Serial Bus Communications Class Subclass Specification for
  PSTN Devices*, Revision 1.2, 9 February 2007. §3.6 for the Abstract
  Control Model, §6.3 for the requests, §6.5 for the notifications, Tables
  3, 4, 13, 17 and 31.
- *Universal Serial Bus Specification*, Revision 2.0. §9.6 and Tables 9-5,
  9-12 and 9-13 for the standard descriptors, §5.8.3 for bulk packet
  sizes, §8.5.3 for the stages of a control transfer.
- [`ip/usb_device_ulpi/README.md`](../usb_device_ulpi/README.md) for the
  bus below this, and [`docs/ip-library.md`](../../docs/ip-library.md) for
  the device core and how the descriptors are carried.
