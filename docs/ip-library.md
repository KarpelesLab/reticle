# The Reticle IP library

The first-party half of phase 8. [`docs/ip.md`](ip.md) describes the
machinery — the manifest formats, the resolver, the bus model, the black
boxes — and [`docs/writing-a-cpu.md`](writing-a-cpu.md) describes how to
package a processor, using this library's two as the worked examples.
This document describes the **blocks**: twenty-five pieces of HDL
that drop into a design the way a crate drops into a Rust program, each
with a manifest, a Rust co-simulation test, and a resource footprint that
was measured rather than guessed.

They live at the top of the repository, in `ip/`, one directory per
package:

```text
ip/
  axil_gpio/     reticle.ip  rtl/axil_gpio.v
  cdc_pulse/     reticle.ip  rtl/cdc_pulse.v
  cdc_sync/      reticle.ip  rtl/cdc_sync.v
  dvi_tx/        reticle.ip  rtl/tmds_encoder.v  rtl/video_timing.v  rtl/dvi_tx.v
  dvi_tx_pll/    reticle.ip  rtl/dvi_tx_pll.v
  eth_mac_rgmii/ reticle.ip  rtl/eth_mac_rgmii.v
  eth_mac_rmii/  reticle.ip  rtl/eth_mac_tx.v  rtl/eth_mac_rx.v  rtl/eth_mac_rmii.v
  fifo_async/    reticle.ip  rtl/fifo_async.v
  fifo_sync/     reticle.ip  rtl/fifo_sync.v
  hyperram_ctrl/ reticle.ip  rtl/hyperram_ctrl.v
  i2c_master/    reticle.ip  rtl/i2c_master.v
  mos6502/       reticle.ip  rtl/mos6502.v
  ppu2c02/       reticle.ip  rtl/ppu_palette.v  rtl/ppu2c02.v  README.md
  pwm/           reticle.ip  rtl/pwm.v
  ram_wrapper/   reticle.ip  rtl/ram_sp.v  rtl/ram_sdp.v
  rv32i/         reticle.ip  rtl/rv32i.v
  sdram_ctrl/    reticle.ip  rtl/sdram_ctrl.v
  spi_master/    reticle.ip  rtl/spi_master.v
  spiflash_xip/  reticle.ip  rtl/spiflash_xip.v
  timer/         reticle.ip  rtl/timer.v
  uart/          reticle.ip  rtl/uart_tx.v  rtl/uart_rx.v  rtl/uart.v
                 rtl/uart_baud_div.v
  usb_device_fs/ reticle.ip  rtl/usb_fs_rx.v  rtl/usb_fs_tx.v  rtl/usb_ctrl_ep.v  rtl/usb_device_fs.v
                 (usb_ctrl_ep.v holds four modules; see below)
  usb_device_fs_pll/ reticle.ip  rtl/usb_device_fs_pll.v
  usb_device_ulpi/ reticle.ip  README.md  rtl/usb_ulpi_link.v  rtl/usb_device_ulpi.v
  usb_host_ulpi/ reticle.ip  README.md  rtl/usb_ulpi_host_link.v  rtl/usb_host_sie.v
                 rtl/usb_host_enum.v  rtl/usb_host_ulpi.v
  usb_cdc_acm/   reticle.ip  README.md  rtl/usb_cdc_req.v  rtl/usb_cdc_acm.v
                 rtl/usb_cdc_acm_fs.v  rtl/usb_cdc_acm_ulpi.v
  vga_out/       reticle.ip  README.md  rtl/vga_out.v
```

`ip/` is in the `exclude` list of `Cargo.toml`, so the published `.crate`
does not carry it. The library is HDL, not Rust: nothing under `src/`
reads it — a block reaches a design through `ip::resolve` like any other
package, and the crate stays sans-I/O — so shipping the tree would add
`.v` files to every download without making the crate do anything more.
It is distributed as part of the repository instead.

## The blocks

| Block | Top module | What it is | Depends on |
|-------|------------|------------|------------|
| `fifo_sync` | `fifo_sync` | synchronous FIFO, full / empty / occupancy count, optional first-word fall through | — |
| `cdc_sync` | `cdc_sync` | N-flop clock domain crossing synchroniser, parameterised width and depth | — |
| `cdc_pulse` | `cdc_pulse` | one pulse across two domains through a toggle and a full handshake | `cdc_sync` |
| `fifo_async` | `fifo_async` | asynchronous FIFO, gray-coded pointers, pointer synchronisers | `cdc_sync` |
| `uart` | `uart`, `uart_tx`, `uart_rx` | 8N1 UART, run-time or parameterised baud divisor, ready / valid | — |
| `uart` | `uart_baud_div` | clocks per bit from a bit rate, by restoring long division, with the rates it refuses | — |
| `spi_master` | `spi_master` | byte-level SPI master, any CPOL / CPHA | — |
| `i2c_master` | `i2c_master` | byte-level I²C master, 7-bit addressing, clock stretching tolerated | — |
| `pwm` | `pwm` | counter-comparator PWM, duty latched once per period | — |
| `timer` | `timer` | prescaled auto-reload down-counter with a pulse and a sticky interrupt | — |
| `axil_gpio` | `axil_gpio` | AXI4-Lite GPIO subordinate: data, direction and set registers | `cdc_sync` |
| `ram_wrapper` | `ram_sdp`, `ram_sp` | portable block RAM wrappers, single and simple dual port, optional output register | — |
| `rv32i` | `rv32i` | the whole RV32I base integer set, multi-cycle, machine-mode CSRs, traps and interrupts | — |
| `mos6502` | `mos6502` | the documented MOS 6502: 56 mnemonics, 13 addressing modes, decimal mode, RES / NMI / IRQ / BRK, documented cycle counts | — |
| `eth_mac_rmii` | `eth_mac_rmii`, `eth_mac_tx`, `eth_mac_rx` | Ethernet MAC over RMII: preamble, frame check sequence, inter-frame gap | — |
| `spiflash_xip` | `spiflash_xip` | execute-in-place SPI flash reader, read only and cache-less | — |
| `sdram_ctrl` | `sdram_ctrl` | SDR SDRAM controller for x16 parts: power-up sequence, refresh, open rows per bank, datasheet timings in nanoseconds | — |
| `hyperram_ctrl` | `hyperram_ctrl` | HyperBus controller: command-address, fixed or variable latency, DDR data and RWDS, register access | — |
| `dvi_tx` | `dvi_tx`, `tmds_encoder`, `video_timing` | DVI output: 640x480, 800x600 and 1280x720 timings, TMDS 8b/10b with DC balance, 10:1 serialisation through DDR outputs | — |
| `dvi_tx_pll` | `dvi_tx_pll` | `dvi_tx` with its five-times clock from the device's PLL | `dvi_tx` |
| `vga_out` | `vga_out` | VGA output: the same three timings, colour truncated to the board's bits per channel, blanking forced to black, syncs at the mode's polarity | `dvi_tx` |
| `eth_mac_rgmii` | `eth_mac_rgmii` | gigabit Ethernet MAC over RGMII: the RMII MAC's frame logic an octet a cycle behind DDR IO, optional IO delays | `eth_mac_rmii` |
| `usb_device_fs` | `usb_device_fs`, `usb_fs_rx`, `usb_fs_tx`, `usb_dev_core`, `usb_pkt_rx`, `usb_ctrl_ep`, `usb_bulk_ep` | USB full-speed device: NRZI, bit stuffing, SYNC and EOP, CRC5 and CRC16 checked, a control endpoint that enumerates, and a bulk endpoint pair with a byte interface | — |
| `usb_device_fs_pll` | `usb_device_fs_pll` | `usb_device_fs` with its 48 MHz from the device's PLL and a 12 MHz board clock | `usb_device_fs` |
| `usb_device_ulpi` | `usb_device_ulpi`, `usb_ulpi_link` | the same device behind a ULPI transceiver, which does the line work in silicon: the bus turnaround, transmit and receive commands, register access, one 60 MHz clock and no PLL | `usb_device_fs` |
| `usb_host_ulpi` | `usb_host_ulpi`, `usb_ulpi_host_link`, `usb_host_sie`, `usb_host_enum` | the **other end of the wire**: a USB full-speed host behind a ULPI transceiver — a frame every millisecond, tokens with CRC5, data packets with CRC16, handshakes, a timeout with retries, a bus reset driven from the transceiver's own terminations, and an enumeration that reads the device and configuration descriptors and sets an address and a configuration | `usb_device_fs` |
| `usb_cdc_acm` | `usb_cdc_acm`, `usb_cdc_req`, `usb_cdc_acm_fs`, `usb_cdc_acm_ulpi` | a USB serial port the operating system's own driver binds to: two interfaces with the union functional descriptor, the line-coding and control-line requests, a notification endpoint that sends SERIAL_STATE and a 64-byte bulk pair, behind either link layer | `usb_device_fs`, `usb_device_ulpi` |
| `usb_hub` | `usb_hub`, `usb_hub_req`, `usb_hub_fs`, `usb_hub_ulpi` | a USB 2.0 full-speed **hub**, which is the other class every operating system already has a driver for: the hub and port class requests of USB 2.0 §11.24.2, the hub descriptor of §11.23.2.1, one interrupt IN status-change endpoint and **no bulk endpoint at all**, with a port whose state is read from a second USB controller on the other side of the die | `usb_device_fs`, `usb_device_ulpi` |
| `ppu2c02` | `ppu2c02`, `ppu_palette` | NES-compatible picture unit: 256x240 raster, nametables and attributes, scrolling through `v`/`t`/`x`/`w`, 8x8 sprites with per-line evaluation, priority and sprite zero hit | — |

`ppu2c02` is the block whose *subject* needs a statement rather than only
its behaviour, so it has a page of its own,
[`ip/ppu2c02/README.md`](../ip/ppu2c02/README.md): it is implemented from
the published description of a machine, and no game data, character data
or lockout logic is in this repository. It is used by
[`examples/nes`](../examples/nes), which runs a demo written for that
example and nothing else. It is not in the footprint table below, which
measures the blocks `tests/ip_library.rs` takes through the flow; its
numbers are on its own page and in `tests/nes.rs`.

Two other blocks carry a page. [`ip/vga_out/README.md`](../ip/vga_out/README.md)
says what truncating colour to a board's bits per channel costs a picture.
[`ip/usb_device_ulpi/README.md`](../ip/usb_device_ulpi/README.md) is a
different kind of page again: it is **the protocol, written down before
the block was**, the way [`docs/apollo-protocol.md`](apollo-protocol.md)
was written before the Apollo transport — every fact of ULPI 1.1 the block
relies on, with the section it came from and how sure of it this project
is, then what the block leaves out and why, then what simulation
established and what it cannot. A link layer written from a reading
nobody wrote down is a link layer nobody can check.

[`ip/usb_cdc_acm/README.md`](../ip/usb_cdc_acm/README.md) is the third of
that kind and is about a different sort of fact again. ULPI's document is a
bus read out of a specification; a class layer's question is not "is this
descriptor legal" but **"does the driver bind"**, which no specification
answers and only a host can. So that page has a fourth confidence level
beside HIGH, MEDIUM and LOW — **CHECKED**, meaning measured on a host with
the output quoted — and it is careful about the difference. **CHECKED** is
that `cdc_acm` binds to this descriptor set, that the port opens and carries
bytes, that the **SERIAL_STATE notification arrives and the driver acts on it**
— the ten bytes read off endpoint `82h` and `TIOCMGET` reporting DCD and DSR,
which `cdc_acm` can only get from that notification — and that
SET_LINE_CODING's seven bytes reach the device: 115200 read back out of its
own registers, which is the only measurement anywhere of the class hook's
host-to-device data stage on silicon. **MEDIUM** is the *why* of any of it:
that the driver reads the union functional descriptor to tell the interfaces
apart, that it wants an interrupt IN endpoint on the communications interface,
that it gates SET_LINE_CODING on `bmCapabilities` D1. Nothing here read that
driver's source or watched the bus, and the devices that would settle those —
one without a union descriptor, one without the endpoint — have not been on a
board. The page also records where the specification and the driver pull in
opposite directions and which way the block went: D1 is one bit over four
things, the block does three of them, and it is set anyway.

[`ip/usb_host_ulpi/README.md`](../ip/usb_host_ulpi/README.md) is the
fourth, and it is the first page here about a block that is **not a
peripheral**. Everything else USB in this library answers somebody else's
tokens; this one sends them. Its page is deliberately short where ULPI's
is long — the bus is the same bus and that document is not repeated — and
long in three places a device's never had to be:

- **where the host's registers differ from a device's**, which is less
  than it looks. ULPI 1.1 §3.8.5.3.2 names the full-speed host
  "XcvrSelect=01b, DpPulldown=1b, DmPulldown=1b, TermSelect=1b", so
  Function Control is the **same byte** at both ends and the whole
  difference is two pull-downs in `0Ah`;
- **how a host drives a bus reset**, which is the fact easiest to get
  wrong because it is not a transmission at all: §3.8.5.1 step 2 has the
  host write Function Control with "XcvrSelect = 00b (HS) and TermSelect
  = 0b which drives SE0 on the bus (D+ and D- connected to ground via
  45 Ohm)", and the USB334x datasheet's Table 5-1 is the only place the
  resistors behind that combination are written down;
- **where it stops**, which is one edge of a die and is in §9 of that
  page. The Cynthion's TARGET transceiver is on column 0 of the
  caBGA-256 — the **left** edge — and `src/fpga/trellis` describes the
  top and right edges only, for a reason its own doc comment gives. So
  the design places, routes and produces a bitstream whose every bit
  decodes, and the one thing it cannot do is name the right balls. That
  page says what the measurement to take is and which reference
  bitstreams would settle it.

[`ip/usb_hub/README.md`](../ip/usb_hub/README.md) is the fifth, and it is
the page whose **first** section is an argument about why the block is not
the thing its name suggests. A hub is a repeater — USB 2.0 §11.1.1 — and
through a ULPI transceiver the floor for a byte in and a byte out is about
24 bit times against the 4 a hub is allowed, because the transceiver does
not report a byte until the byte is complete and then prepends a fresh SYNC
on the way out. So `usb_hub` is a hub's **control endpoint**: the
descriptors, the class requests, the port state and the status-change
endpoint, with a second USB controller behind the port and nothing joining
the two conversations. That page's §2 is the timing, §8 is the kernel log of
a host finding the hub, finding something on its port and failing to
enumerate it, and both are written as the correct outcome of this round
rather than as a defect. It also carries one deliberate departure from the
other four: it cites chapter 11 by **section** and never by table number,
because a section number misquoted is findable and a table number misquoted
sends a reader somewhere else and looks authoritative doing it.

`usb_host_ulpi`'s fourth confidence level is **CHECKED** in a different
sense from `usb_cdc_acm`'s: not "a host did this" but "**our host did this
to our own device**", which is a real test and a smaller claim.
`tests/ip_library.rs` puts `usb_host_ulpi` and `usb_device_ulpi` on one
D+ / D- pair, each behind its own transceiver model, and asserts the
eighteen bytes of the device descriptor and the thirty-two of the
configuration descriptor against the same `expected_*` functions the
device's own tests compare a host *model's* reading against. It found
four defects in the host and two in the model, and three of the four are
inherited from `usb_ulpi_link` where they are unreachable rather than
absent — which is the clearest argument this library has for writing a
second thing against the same bus.

`rv32i`, `mos6502`, `eth_mac_rmii` and `spiflash_xip` are the **larger
blocks**, and they are larger in a particular way: each is a whole
protocol or a whole machine rather than a part of one, so each is where
a shortcut would have been invisible. They also fit together.
`spiflash_xip` presents the memory port `rv32i` puts on its instruction
side, so a processor executing straight out of a serial flash is the two
of them and one wire.

The **two processors are deliberately nothing alike**, which is the
reason there are two. `rv32i` is a 32-bit load/store machine with
fixed-width instructions, thirty-two registers and one addressing mode;
its core is a three-state sequencer and an ALU, and what its tests prove
is arithmetic and traps. `mos6502` is an 8-bit accumulator machine from
1975 with three registers, variable-length instructions, **thirteen**
addressing modes and a cycle count per instruction that programs were
written to depend on. So its core is a forty-state sequencer where every
state is one bus access, and what its tests prove is *timing* as much as
arithmetic: an indexed read that crosses a page costs a cycle, a taken
branch costs a cycle, a branch onto another page costs two, and a
read-modify-write writes its address three times. It also has to
reproduce the part's documented faults rather than fix them — `JMP
($xxFF)` takes its high byte from `$xx00` — and it has packed
binary-coded decimal arithmetic, which is where most 6502
implementations are wrong and which `DECIMAL_MODE` can compile out. The
two of them between them exercise almost disjoint parts of the
toolchain.

What is general about packaging a processor and what is specific to each
of those two — the manifest, the bus contract, testing a core so the test
cannot agree with a wrong core, cycle accuracy, interrupts and reset,
reproducing documented quirks, and what the cores cost — is drawn out in
[`writing-a-cpu.md`](writing-a-cpu.md), with both of them as the worked
examples. Each is also dropped into a whole system: `rv32i` in
[`examples/soc`](../examples/soc) and `mos6502` in
[`examples/mos6502_computer`](../examples/mos6502_computer).

`sdram_ctrl` is the first of the blocks that **need a device
primitive**, the ones phase 8 waited on the FPGA backend for: it forwards
its clock to the part through a double-data-rate output register, which
it asks for with a `ddr` attribute on the port, so the ECP5 row of its
footprint carries an `ODDRX1F` and the iCE40 row an `SB_IO` in DDR mode.
`hyperram_ctrl` puts every HyperBus pin through one — nine `IDDRX1F` and
ten `ODDRX1F` on the ECP5 — and asks for an IO delay on CK, the
`DELAYG` that moves each clock edge into the middle of the byte it
clocks. `dvi_tx` serialises through them, and `dvi_tx_pll` gets its
five-times pixel clock from a `clock_mhz` attribute on a net nothing
drives, which is how a design asks for a PLL: an `SB_PLL40_CORE` or an
`EHXPLLL` appears in its footprint, fed from the board's 25 MHz.
`eth_mac_rgmii` is `eth_mac_rmii`'s frame logic — the same `eth_mac_tx`
and `eth_mac_rx`, which the RMII block was split into so both could
share them, eight bits a cycle instead of two — behind DDR registers on
every RGMII pin, with the clock skew RGMII needs available from the IO
delay element: six `DELAYG` in its ECP5 row. The split cost the RMII
block nothing; its footprint did not move by a cell.
`usb_device_fs` needs no DDR at all — full speed is 12 Mbit/s, sampled
four times a bit — but it needs 48 MHz, which is exactly what
`usb_device_fs_pll` asks the PLL to make from the 12 MHz oscillator
most small boards carry, with no error on either family.

`usb_device_ulpi` needs **neither**, which is the interesting half of why
it exists. It is for a board whose USB lines never reach the FPGA at all:
on a Great Scott Gadgets Cynthion all three ports go through their own
ULPI transceiver, and the only balls wired to a pair are declared input
only, for watching the bus. An encoder and a serialiser have nothing to
drive there, so `usb_fs_rx` and `usb_fs_tx` are replaced by
`usb_ulpi_link`, a byte-parallel bus to the transceiver — and the
transceiver does the line work in silicon. That bus runs at 60 MHz, which
is what the board's oscillator already is, so there is no
`usb_device_ulpi_pll` beside it: nothing needs generating.

What the two cores **share** is the part worth sharing, and it is now a
whole device rather than one endpoint. `usb_device_fs` was split the way
`eth_mac_rmii` was, and `usb_dev_core` is the shared half: `usb_pkt_rx`
for the packet decoding — the PID check nibble, the CRC5 of tokens, the
CRC16 of data packets and the payload — `usb_ctrl_ep` for endpoint 0 and
the standard requests, `usb_bulk_ep` for the endpoint that moves bytes,
and the one transmitter those two endpoints share. Both cores instantiate
it, `usb_device_ulpi` through a `depends` line on `usb_device_fs` exactly
as `eth_mac_rgmii` depends on `eth_mac_rmii`. It was a split rather than a
copy on purpose: a control endpoint is the part of a USB device that is
hardest to get right and the part a test proves most about, and two copies
of one drifting apart is a cost that arrives later and is paid by whoever
is unlucky.

The decoder came out of `usb_ctrl_ep` when the second endpoint arrived,
for the same reason: an endpoint that decoded packets for itself would
carry its own CRC16 generator, its own byte counter and its own PID check,
which is about a hundred LUT4 of duplication and three chances for two
statements of one thing to disagree. Both endpoints read one pulse, `pkt`,
with everything about the packet valid in that cycle.

**The arbitration between them is one register.** A USB device only speaks
when a token asks it to, so the endpoint that may answer is the one the
last token named — `owner <= (tok_endp != 0)` — and everything after that
token belongs to the same endpoint, because a host does not interleave
transactions on one device. `sel` into each endpoint is that bit, it gates
the endpoint's turnaround counter, and the rest of the arbitration is a
multiplexer. There is no request and grant and no round robin, because
there is never a second answer waiting: the endpoint that has not been
asked has nothing to say.

The **byte interface** both cores bring out is `usb_bulk_ep`'s, and it is
deliberately a stream and not a FIFO. `out_data` / `out_valid` /
`out_last` / `out_ready` is what the host sent, a byte at a time with
`out_last` on the last byte of each packet; `in_data` / `in_valid` /
`in_ready` / `in_commit` is what it will be sent, and `in_commit` is what
sends a short packet as a short packet — or, given no bytes at all, a
zero-length one, which is how a host is told a transfer has ended.
Flow control is the host's problem, which is what bulk means: an OUT
packet that arrives before the last was taken is answered with NAK and the
host repeats it, an IN token with nothing ready is answered with NAK and
the host asks again, and neither loses a byte. `fifo_sync` is already a
block here, so a design that wants depth puts one on either side rather
than paying for it inside every USB device.

Every output of that interface is a function of the endpoint's own
registers, and no input reaches any of them combinationally. That is not
a convenience, it is what lets a top level wire `out_ready` to `in_ready`
and `in_valid` to `out_valid` and have a loopback rather than a
combinational loop, which is exactly what
`testdata/fpga/cynthion/usb_ulpi_device.v` does.

The **descriptors** are the class's. The configuration descriptor used to
be eighteen bytes of `case` inside `usb_ctrl_ep`, with `wTotalLength`
typed out as `18` four lines above the `9` and the `9` it is the sum of,
and a device that moves bytes has endpoint descriptors that no control
endpoint should know about. So a design states its interface and endpoint
descriptors in one wide parameter, `IFACE_DESC`, written in descriptor
order — a Verilog concatenation lists its most significant part first,
which is the order `lsusb -v` prints — and `usb_ctrl_ep` states the nine
bytes that wrap them, because those nine bytes are **arithmetic over the
rest**:

| Field | Where it comes from |
|-------|---------------------|
| `wTotalLength` | `9 + IFACE_BYTES`, computed |
| `bNumInterfaces` | the INTERFACE descriptors in the parameter, counted by a constant function at elaboration |
| `bNumEndpoints` | the ENDPOINT descriptors after each INTERFACE descriptor, counted the same way and **written over** whatever byte 4 of that interface descriptor held |

None of the three can disagree with the descriptors, because none of them
is read from the descriptors. A wide parameter was chosen over a
descriptor module with a byte port because a descriptor is not logic: as a
parameter it is a constant those functions can walk at elaboration, so the
counting costs nothing at run time, and a design that wants another class
changes one instantiation instead of adding a module and four wires to its
top level. A descriptor module would have put the length back in two
places — its own bytes and the core's idea of how many there are — which
is the thing being fixed. The one number a design still states twice is
`IFACE_BYTES`, the concatenation's own width; get it wrong and the value
is truncated at its top, byte 0 stops being a `bLength`, and
`bNumInterfaces` comes out wrong, which the test compares against a
descriptor written forwards in Rust.

Those constant functions are written with shifts and masks and **no
part-selects**, which looks perverse and is not: an indexed part-select
whose base a width checker cannot bound warns — `part-select [-1:-8] is
outside [39:0]` was the first attempt at reading the blob backwards — and
`blocks_synthesise_cleanly` allows no warnings.

## A class on top

Everything above is a device that moves bytes and says nothing about what
they mean. `usb_cdc_acm` is the first block that says what they mean, and it
picks the meaning an operating system already has a driver for: a **serial
port**. A device built on it appears as `/dev/ttyACM*`, `/dev/cu.usbmodem*`
or a COM port with nothing installed, so the host half of a demonstration is
`cat`.

Three things were genuinely new, and only one of them is the serial port.

**The class hook, which outlives this block.** `usb_ctrl_ep` stalled every
request it did not implement, which is a control endpoint no class can be
built on: CDC ACM needs three requests answered, a human interface device
needs two others, and the next class needs something else again. What they
share is a shape, and the shape is nine ports — the eight bytes of a SETUP
with a one-cycle `class_req`, a `class_claim` and `class_len` read back, a
device-to-host data stage fetched a byte at a time through `class_index`, and
a host-to-device data stage handed over as one packet.

Two decisions in it are worth more than the rest.

`class_claim` and `class_len` are read **combinationally**, in the same cycle
`class_req` is high. That is not a shortcut: endpoint 0 chooses the
transfer's stage in the very cycle the SETUP's data packet ends, so a class
that answered a cycle later would need a fifth stage there, a handshake back,
and a rule about what happens if the host's next token arrives first. A
request decoder is a comparison of eight bytes against constants — there is
nothing in it to sequence — so asking for it combinationally asks for nothing
a class cannot give. `ip/usb_cdc_acm/rtl/usb_cdc_req.v` is that decoder and it
is fifteen lines of `assign`.

And `class_req` is **not** raised for the five requests endpoint 0 implements,
so a class cannot shadow SET_ADDRESS or GET_DESCRIPTOR by claiming them.
Everything else is offered, string descriptors and GET_STATUS included, so a
class that wants those can have them without that file changing again.

There is one thing the hook deliberately does not have: a way for a class to
STALL a request it recognises. `class_claim` promises an answer. A class that
must refuse leaves the claim low and takes the STALL endpoint 0 was going to
send — which is how `usb_cdc_req` refuses SEND_BREAK, a request its own
descriptor never offered.

**A fifth control stage was not added, on purpose.** The obvious way to
serve a host-to-device class data stage is a fifth value of `stage`, which
takes that register from two bits to three — and then, for every design with
no class layer, the third bit is a bit no expression can set. That is the
shape that cost this project eight rounds of investigation once and is
written out at length in `usb_ctrl_ep`'s own header. So there is no fifth
stage: a class request that writes is a status-stage transfer with one data
packet expected first, and what recognises that packet is a register **bit**,
which is a bit either way.

**More than one endpoint pair.** CDC needs a bulk pair *and* an interrupt IN,
so `usb_dev_core`'s one bulk endpoint and one-bit `owner` had to generalise.
It gained a second, IN-only endpoint at `NOTIF_ENDP`, and `owner` became two
bits — `own_data` and `own_notif` — which are **the `sel` signals themselves
rather than an encoded index**. Three owners need two bits either way, so
the choice was about what the two bits are: an index costs a decoder at every
`sel` and at the transmitter multiplexer, and an index leaves the high bit of
a register unset when there is no second endpoint, which is the shape above.
Two bits that are each a `sel` let the whole second endpoint disappear when
`NOTIF_ENDP` is `4'd0`: `own_notif` is then a flip-flop whose data input is
the constant zero, `synth::opt::FfOpt` replaces it with that constant, and
the endpoint, its half of the multiplexer and its buffers go with it.

`usb_bulk_ep` took `WITH_OUT` and `WITH_IN`, and a direction that is zero is
**not answered at all** rather than NAKed — silence, which is what a host must
hear from an endpoint the descriptors do not declare. Nothing is generated
away by hand: the registers of a direction that cannot be asked for have no
reader, so synthesis removes them, and the two directions stay one piece of
readable logic instead of two halves behind a `generate`.

**Two interfaces, and the descriptor that ties them together.** A serial port
is a communications interface carrying the control requests and a data
interface carrying the bytes, and nothing in either interface descriptor says
they belong to one another. The **union functional descriptor** is what says
it (CDC 1.1 Table 33), and Linux's `cdc_acm` is understood to read it to tell
the two apart, with a device that omits one reaching a path meant for devices
in the driver's own quirk table — understood rather than measured, which that
block's README marks. Fifty-eight bytes of `IFACE_DESC` hold two interface
descriptors, four functional descriptors and three endpoint descriptors,
which with the nine `usb_ctrl_ep` writes is a `wTotalLength` of 67 — six
bytes under the `DESC_MAX` of 64 that "a CDC ACM descriptor needs headroom
for" was about.

**The notification endpoint sends SERIAL_STATE.** Ten bytes — eight of
header and two of `wSerialState` — which is PSTN 1.2 §6.5.4 and is what did
not fit when the length field both transmitters took was four bits. It fits
now: `wMaxPacketSize` on that endpoint is **16**, the smallest power of two
that holds ten, and USB 2.0 §5.7.3 lets a full-speed interrupt endpoint be
any size up to 64 rather than one of the four a bulk endpoint may be.

**One goes out whenever the host's idea of the line state could be stale**, and
the endpoint NAKs every poll in between. Three things make it stale and all
three are triggers: the host configuring the device, the host **opening or
reconfiguring the port** — SET_CONTROL_LINE_STATE or SET_LINE_CODING — and
`serial_state` changing. The middle one is the part that is not in PSTN 1.2 and
is not optional either: a state-change notification only reaches a host that was
listening when the state changed, and `cdc_acm` does not start listening until
`open`. Sending one per configuration and no more meant that the single packet
went to whoever polled first and the carrier was then wrong for ever: measured
as `TIOCMGET = 0x026` — no DCD, no DSR — on three consecutive opens, which
`ip/usb_cdc_acm/README.md` §4 writes up as the defect it was.

`serial_state` is a **port** of `usb_cdc_acm` and not a constant, seven bits
wide because §6.5.4 defines seven and reserves the other nine: DCD, DSR,
break, ring, framing, parity and overrun. A device with no modem lines ties it
to `7'b000_0011` — both carriers, no errors — and the argument is the
specification's own words for those two bits, since a port whose far end is in
the same die has its carrier present and its data set ready from the moment it
exists. `ip/usb_cdc_acm/README.md` §4 and §5 say what a host was observed to
do with it and what is only quoted.

**What the class layer cost.** On the ECP5, `usb_device_fs` is 853 LUT4, 355
flip-flops and 16 `TRELLIS_DPR16X4`, and `usb_cdc_acm_fs` is 1118, 474 and 18,
so a serial port is **+265 LUT4, +119 flip-flops and +2 distributed RAMs** over
the vendor device it is built on; `usb_device_ulpi` to `usb_cdc_acm_ulpi` is
+265, +119 and +2, which is the same thing twice and is the point of sharing the
core. The flip-flops are the line coding — `dwDTERate` alone is thirty-two —
DTR and RTS, `class_active` and `class_out_wait` in endpoint 0, the
notification endpoint's turnaround counter, and the SERIAL_STATE sender's
`nidx`, `reported` and `ever`. The two extra RAMs are that endpoint's own
sixteen-byte buffer, which is 16 x 8 bits and so exactly two of them.

(Those figures were +423 LUT4 and +252 flip-flops until the endpoint buffers
became arrays; "The endpoint buffers are arrays now" below has both sets and
what moved between them.)

**And the hook cost the designs that do not use it nothing.** With
`NOTIF_ENDP` and `class_claim` tied off, `own_notif`, the second endpoint, its
buffers and `class_active` are all constant and all removed; that was measured
when the hook arrived and is why `usb_device_ulpi`'s numbers did not move for
it.

## What the packet size is worth, measured and not calculated

Every endpoint of these blocks used to be **eight bytes**, and that was the
four-bit length both transmitters took rather than a decision. It is 64 now on
the bulk endpoints and on endpoint 0, and 16 on the CDC notification endpoint.
Both halves of the trade were measured rather than reasoned about.

**What it costs.** The footprint table carries `usb_device_fs` four times —
`MAXPKT = 8` and the default 64, each with the buffers as arrays and as shift
registers — so every difference below is one subtraction and is checked by the
same test that generates the rest. The shift-register column is what 64 bytes
cost when the buffers were first widened:

| ECP5 45F, `BUF_RAM = 0` | 8 bytes | 64 bytes | difference |
|-------------------------|---------|----------|------------|
| LUT4 | 893 | 1830 | **+937** |
| flip-flops | 459 | 1379 | **+920** |

The flip-flops are the two buffers: 64 bytes each way is 1024 of them where
eight bytes was 128, and the rest is a handful of wider counters. **The lookup
tables are the byte multiplexer and nothing else.** A LUT4 is a 2-to-1
multiplexer with a select, so a 64-to-1 byte multiplexer is 63 of them per bit
and 504 per direction; two directions is 1008, against 112 for the 8-to-1
multiplexers they replace. That cost belongs to the *interface* and not to the
way the packet is stored — a buffer written as sixty-four named byte registers
and a `case` needs exactly the same multiplexer, plus a six-to-sixty-four
decoder for the write enables that the shift register does not need.

Two cheaper structures existed and **one of them is now what the block does**;
"The endpoint buffers are arrays now" below is the whole of it, and this is the
shape of the argument that got there.

A buffer that **shifts a byte out** as the consumer takes it needs no
multiplexer at all, because the byte is always at the bottom. What it needs is
for the packet to be *aligned* to the bottom before the first byte is read, and
its length is not known until it has all arrived — so the alignment is up to 63
more shifts after the packet ends, which is a padding state in both directions
and a new rule about when a second packet may start. This one is still only
written down.

A **distributed RAM** — `reg [7:0] buf [0:63]` — needs neither a write decoder
nor a read multiplexer, and was the obvious answer. It was not available: when
this was written `src/fpga/devices/ecp5.dev` declared that bel with no site
count, so `fpga::place` counted zero of them and refused any design that needed
one, which is the same gap `testdata/fpga/cynthion/usb_cdc_uart.v` records about
`ip/fifo_sync`. It is available now — 3036 sites on the Cynthion's part — and
taking it gave back 977 of those 937 lookup tables and 1024 of the flip-flops
as well.

**What it buys.** Measured on a Cynthion's AUX port, through
`testdata/fpga/cynthion/usb_ulpi_device.v` — the bulk loopback, with no UART in
the way — by `tests/usb_loopback.rs`, which times 256 write-then-read round
trips and prints the rate. The device holds one packet each way, so each round
trip is one OUT transaction, one IN transaction and two trips through the
host's own stack:

| `wMaxPacketSize` | round trips/s | bytes/s each way | slowest of six |
|------------------|---------------|------------------|----------------|
| 8 | 8460 | 67 700 | 57 600 |
| 64 | 3990 | 255 500 | 243 700 |

Those are medians of six runs each. The slowest run of a set is always one of
the first two, and after that they cluster inside 2 %, so the spread is the
host's scheduler settling rather than the device varying — which is worth
saying because the two low readings at eight bytes are 15 % off the median and
a reader comparing single runs could get 57 600 against 258 600 and call it
four and a half times.

**3.8 times, and not eight.** The transactions do fall by eight, but the round
trip rate falls with them — 8460 a second to 3990 — because a 64-byte packet
takes 43 microseconds on a 12 Mbit/s wire where an eight-byte one takes 5, so
the wire stops being free. What is left over each round trip is about 110
microseconds of host and scheduler, unchanged by the packet size, and that is
now the larger half of the cost. A host that pipelined its transfers would see
more of the eight; this one waits for each.

**Why the serial port's own figure is not this one.**
`testdata/fpga/cynthion/usb_cdc_uart.v` carries **one byte at a time** through
its UART — that file's header says why, and the short version is that it was
written while `ip/fifo_sync` could not be placed on this part — so it sends
one-byte packets whatever `wMaxPacketSize` says, and a wider packet does
nothing for it at all. The placement gap is closed now, so that is a design
this round did not revisit rather than a constraint it was under.
The bulk loopback is the design that measures the endpoint rather than the
bridge above it.

**What these numbers are and are not.** They are two runs of one host on one
machine against one part, repeatable to a tenth of a percent over four runs
each, and they are **printed by a test and never asserted**: no number here is
compared against a clock, and `tools/check.sh` does not run that test. What
they do not measure is a host that pipelines transfers instead of waiting for
each one — a queue of URBs would overlap the round trips and go faster at both
sizes — so they are the floor of what the endpoint can do and not the ceiling.

**What it found in this compiler, and it is fixed now.** `ip/fifo_sync` could
not be placed on an ECP5 at all. Its storage is an array indexed by a
variable, which becomes a distributed RAM, and nothing in the ECP5 fabric
model had a site for one to go in, so `fpga::place` counted zero of them and
refused at any depth and with `FWFT` either way:

```text
error: the design needs 2 `lutram` site(s) and the part has 0
```

The first reading of this blamed `src/fpga/devices/ecp5.dev`, which does
declare `bel TRELLIS_DPR16X4 lutram` with no `count`. **That was not the
cause.** A `.dev` bel `count` is a resource budget; `fpga::place` counts
`RoutingGraph::sites`, which come from the architecture, and an ECP5's
architecture is loaded from Project Trellis by `src/fpga/trellis`, which
created three kinds of bel and no more: `lut`, `ff` and `io`. There was no
`lutram` site for the placer to find, so adding a `count` would have changed
nothing.

What it took was modelling a slice's distributed-RAM mode in the Trellis
loader — the bel, its wires and its configuration bits — and
`docs/fpga-trellis.md`'s first section is the whole account. In short: a
`TRELLIS_DPR16X4` is **slices A, B and C of one logic tile**, held together by
one bit of that tile (`F50B11`), so a tile holds exactly one and it costs six
of the tile's eight lookup tables. `ip/fifo_sync` now places, routes and comes
out as a bitstream at depths 16, 32 and 64, with every bit of the image
decoding back through the database into a feature it names — 97 bits per RAM,
which is `ecppack`'s own number for the 111 distributed RAMs in this board's
reference bitstreams.

**The 7 series is still in the old position**, and it was looked at far
enough to say so precisely rather than by analogy.
`src/fpga/devices/xc7.dev` declares `RAM64X1D lutram` without a `count`, and
`src/fpga/xray/parse.rs` makes nine kinds of bel — `lut`, `ff`, `carry`,
`site`, `io`, `gb`, `bram`, `dsp`, `other` — and **no `lutram`**. That flow
does reach a real bitstream, checked feature for feature against Vivado's own
output, so a 7-series design with a distributed RAM does not stop early: it
maps to `RAM64X1D` (which `logicram_xc7` proves) and then dies at
`place.rs`'s `the design needs N lutram site(s) and the part has 0`, exactly
as the ECP5 did. `docs/fpga-xray.md` has the verdict on what the work would
take; one family at a time.

`testdata/fpga/cynthion/usb_cdc_uart.v` still carries one byte at a time
through the UART, which needs one holding register instead of a queue, and
says so in its header. That workaround is no longer forced.

**What the endpoints cost.** `usb_device_fs` went from 549 LUT4 and 244
flip-flops on the ECP5 to 916 and 438, and `usb_device_ulpi` from 638 and
250 to 1001 and 444, when the endpoints arrived and a packet was eight bytes.
Two eight-byte packet buffers are 128 of those flip-flops and are the price of
an endpoint that needs no FIFO; `usb_bulk_ep` on its own is 281 LUT4 and 167
flip-flops, and the control endpoint grew by about a hundred LUT4 for indexing
a descriptor blob instead of an eighteen-entry `case`. ("What the packet size
is worth" above is what the same two blocks cost now that a packet is 64
bytes.) Shrinking `DESC_MAX` from 64 bytes to
32 saves seven of those, which is not worth the headroom a CDC ACM
descriptor needs.

The first measurement of it was **three `MULT18X18D` and 1560 LUT4**, which
is a hard multiplier in a USB device and is worth recording as a gap in
this compiler rather than in these blocks: `obuf[ordx * 8 +: 8]` is a byte
index scaled to a bit index, and `src/synth` does not strength-reduce a
multiply by a constant power of two, so the `mul` cell survives to
technology mapping. Written `{ordx, 3'b000}` the same expression is wires.
That gap is still open and still worth knowing about, but only
`BUF_RAM = 0` meets it now: an array is indexed by the byte index itself and
there is no scaling to strength-reduce.
Any design that indexes an array by a scaled index would meet it, and the
blocks are where it showed.

Four modules share `rtl/usb_ctrl_ep.v`, which is the one thing in this
library that is not one module to a file. `tests/fpga_trellis.rs`
elaborates the two Cynthion top levels from a list of paths written out in
Rust, and a module those paths do not reach used to **not fail to
elaborate** — it became a black box whose outputs were undefined, and the
first of those two tests then reported `FAIL: LED 1 is lit and no host has
configured anything` about a design that was perfectly well. That silent
black box is now a diagnostic: `ir::Design::check_instance_targets` names
the module and the instance, and `fpga::synthesize_for`,
`asic::synthesize_asic` and `sim::Simulator` all refuse rather than run on a
design with a hole in it (`docs/ir.md`). A black box something *declares* —
an encrypted package, a `blackbox module`, a device primitive — still works,
which is the distinction that makes the check safe. Splitting the file back
into four is still a rename and three lines in each of
those two lists. The 370 lines of NRZI, bit
stuffing and serialising in `usb_fs_rx` and `usb_fs_tx` are what ULPI
replaces and are *not* shared, because there is nothing there a ULPI
design can use. The one thing the split needed was a parameter:
`TURNAROUND`, how long after a host's packet the answer starts, which is
eight cycles of 48 MHz on the full-speed core and is stated by ULPI
itself as 7 to 18 clocks of 60 MHz (ULPI 1.1 Table 10). That counter is
in **each** endpoint rather than shared, which is two statements of one
rule and was chosen: a shared counter has to tell an endpoint that its
packet went out, a cycle after it did, and ten lines of counter in each
endpoint is cheaper to be sure of than a handshake between three modules.

Everything is **Verilog-2005**, deliberately: it is the path this
compiler exercises hardest, and it is the dialect every other tool reads.
Each source starts with a README-style header saying what the block does
**and what it does not** — the second half is the useful one, because an
IP block's limits are what a user needs before they commit to it.

## The endpoint buffers are arrays now

`usb_bulk_ep` kept its two packets in **shift registers** because the shape
that wants to be an array could not be placed: an array indexed by a register
is a distributed RAM and the ECP5 backend had no site to put one on. That is
closed — `TRELLIS_DPR16X4` has 3036 sites on the Cynthion's part — so the
buffers are `reg [7:0] buf [0:MAXPKT-1]`, the write pointer and the read index
are the memory's own addresses, and **neither the write decoder nor the 64-way
byte multiplexer is built at all**. `BUF_RAM` selects the shape, it is 1 by
default, and `usb_device_fs`, `usb_device_ulpi` and `usb_cdc_acm` all carry it
so a design can choose.

**What it saved, and where it cost.** Every row is from the footprint table
below, which the gate regenerates:

| Target | 64 bytes, shift | 64 bytes, array | 8 bytes, shift | 8 bytes, array |
|--------|-----------------|-----------------|----------------|----------------|
| ECP5 45F, LUT4 | 1830 | **853** | 893 | **770** |
| ECP5 45F, flip-flops | 1379 | **355** | 459 | **331** |
| ECP5 45F, `TRELLIS_DPR16X4` | 0 | **16** | 0 | **4** |
| ECP5 45F, LUT depth | 10 | 10 | 11 | 11 |
| iCE40 HX1K, SB_LUT4 | **1804** | 2869 | **849** | 983 |
| iCE40 HX1K, flip-flops | 1379 | 1379 | 459 | 459 |
| iCE40 HX1K, LUT depth | 10 | 10 | 10 | 10 |

On the ECP5 that is **−977 LUT4 and −1024 flip-flops** at 64 bytes, for 16
distributed RAMs. Sixteen is what to expect and it is worth saying why: a
`TRELLIS_DPR16X4` is 16 words of 4 bits, a 64-byte buffer is 64 words of 8, so
it is two wide by four deep — eight per buffer, two buffers, sixteen. At
`MAXPKT = 8` a buffer is 8 x 8, which is two RAMs with their upper half
unused, so four; the saving is smaller there (**−123 LUT4**) because an 8-to-1
multiplexer was never the expensive part.

**On the iCE40 it is worse, and the shift register stays available for that
reason.** These buffers are read combinationally, and the flow says so in as
many words — `u_dev.u_ep1.g_ram.obuf -> flip-flops, 512 flip-flop(s) (a block
RAM reads on a clock edge and this memory has an asynchronous read port)`. On
the ECP5 that refusal lands on `TRELLIS_DPR16X4`; the iCE40 database declares no
distributed RAM at all, so it lands on flip-flops instead: the same 1024 bits,
*plus* a write-enable per word per bit, which the shift register did not need
because a shift register writes at a fixed end. The flip-flop count is
identical in both shapes and the lookup tables go **up by 1065**, which is
most of an HX1K's 1280 on its own. The penalty does not vanish at
`MAXPKT = 8` either — 983 against 849, **+134** — so a design on a family with
no LUT RAM should say `BUF_RAM = 0` whatever its packet size. The **+1065** is
also larger than a decoder ought to be: 64 words of write enable is a
6-to-64 decoder and ought to be shared across the eight bits, and the count is
consistent with one enable gate per word *per bit*. That is a note about
`fpga::primitives`' fallback rather than about these blocks, and it is written
here rather than acted on.

**The generic LUT4 and LUT6 rows in the table are not a saving and must not be
read as one.** `MapOptions::lut(k)` covers logic and leaves a memory alone, so
those rows go from `1840 x lut` to `804 x lut, 2 x memory 64x8, 2 x memrd,
2 x memwr`: the thousand lookup tables did not disappear, they moved into a
cell the generic mapping does not lower. What a *device* does with that cell is
the ECP5 and iCE40 rows, and those are the honest ones. It also means
`every_block_maps_to_the_logic_it_was_mapped_from` now proves a **smaller**
network for every USB block — `usb_device_fs` at LUT4 is 1701 AIG nodes and
831 cells where it was near twice that — because the buffers are no longer part
of the logic it maps. No block entered or left that check and every one of them
is still proved equivalent.

**Depth did not move on the ECP5 or the iCE40**, which is the answer to "a RAM
read has a different shape from a 64-to-1 mux": both are on the path from a
buffer to the transmitter, and neither is the deepest path in the block —
`usb_pkt_rx`'s CRC16 is. The one depth that did move is `usb_cdc_acm_fs` at
LUT6, from 11 to **10**.

**Neither buffer is cleared on reset any more**, because a distributed RAM has
no reset pin and an array cleared on `rst_n` cannot be one. Nothing needs the
clear: a byte of the OUT buffer is read only while `olen != 0` and one of the
IN buffer only while `armed`, and both of those are flip-flops that reset to
zero. The one read outside a packet is the transmitter's fetch one past the
end, which `usb_fs_tx` throws away, and a zero-length packet is that fetch and
nothing else.

That last byte is **X in a four-state simulation**, and it costs one test a
parameter. In the design as elaborated the X stops at the byte multiplexer
`usb_dev_core` shares between its endpoints, because `CellKind::Mux` answers
from the input its select chose; every behavioural test in `tests/ip_library.rs`
runs on the default shape and passes. After **covering**, that multiplexer is
`lut` cells, and `CellKind::Lut` in `src/sim/sched.rs` answers X as soon as any
input bit is X whether its function depends on that input or not — so one
unwritten byte silences the whole device and GET_DESCRIPTOR returns nothing.
`usb_descriptors_survive_lookup_table_mapping` is the one test that drives a
host at a mapped netlist, so its three cases take `BUF_RAM = 0`; the descriptor
ROM it is about is a constant in `usb_ctrl_ep` that `BUF_RAM` does not reach, so
it covers exactly what it covered before. **What nothing covers is the array
shape through a mapped netlist in simulation**, and an X-optimal `lut`
evaluation in `src/sim` would close that.

**`usb_ctrl_ep` and `usb_pkt_rx` were looked at and neither changes.**
`usb_ctrl_ep` has no packet buffer: what it sends is `IFACE`, a `localparam`
blob, indexed by an offset — a ROM, with no write port, so there is no memory
to infer, and a distributed RAM would need its contents written in through
`dpram_init_word` where the cover of a constant is both cheaper and already
proved (`the_descriptor_rom_cone_maps_to_its_own_function`). What it receives is
`usb_pkt_rx`'s eight-byte word, wired in parallel. `usb_pkt_rx` keeps that word
in `d0`..`d7`, eight named byte registers shifted along, and brings all
sixty-four bits out at once as `dat`: there is no index and no multiplexer to
remove, and an array would need eight read ports, which is eight copies of a
distributed RAM. Both are shapes the array does not help.

**On the part.** `testdata/fpga/cynthion/usb_ulpi_device.v` through the whole
flow to a `.bit`, on an `ecp5-12f-CABGA256`:

| | shift register | array |
|---|---|---|
| LUT4 | 2004 | **1028** |
| `TRELLIS_FF` | 1432 | **408** |
| `TRELLIS_DPR16X4` | 0 | **16** |
| LUT depth | 10 | 10 |
| signals routed | 3448 of 3448 | 1512 of 1512 |
| configuration bits set | 86 845 | 44 355 |
| bits that do not decode | 0 | **0** |

Both were loaded and both enumerate, loop bytes back and report `zero_probe`
zero. The array image is half the bits of the shift-register one and every one
of its 44 355 still decodes back through the database into a feature it names —
14 361 arcs, 2537 fields and 1123 words, with **nothing unexplained** — and the
arcs are exactly the ones the router chose.

The **serial port** was built and loaded too, because a bulk loopback and a
class device are not the same traffic: `testdata/fpga/cynthion/usb_cdc_uart.v`
comes out as 1383 lookup tables, 604 flip-flops and **18** distributed RAMs —
the bulk pair's sixteen and the notification endpoint's two — with all 59 995 of
its bits decoding and nothing unexplained. On the part the kernel's own
`cdc_acm` binds it on `/dev/ttyACM1`, the ten bytes of SERIAL_STATE arrive off
endpoint `82h` with both carriers set, 48 bytes go out and come back byte for
byte through the UART, and GET_LINE_CODING answers 115200 8N1. That is
`tests/usb_cdc_acm.rs`, and it is `#[ignore]`d like the other two.

**The address permutation is right in silicon, and this is the first thing that
could say so.** `docs/fpga-trellis.md` records the gap: a distributed RAM's
addresses are permuted (`dpram_init_word`, nextpnr's `dram_to_comb`), every RAM
in every reference bitstream starts empty, so every word reads alike and no
check in this repository could tell a right permutation from a wrong one. These
buffers are the first non-trivial contents this project has put in one on a
part and read back. 256 bytes went through endpoint 1 in packets of 64, 63, 8,
5 and 1 byte, and came back **byte-identical** — through one distributed RAM on
the way in and another on the way out, at all 64 addresses of each, across all
four 16-word banks of the depth expansion, with the write pointer and the read
index counting independently. A permutation that disagreed between the write
port and the read port, or a bank select that did not, would have returned those
bytes shuffled inside each group of sixteen. It did not.

**What that does and does not settle.** It settles that the write-address and
read-address decodings of a `TRELLIS_DPR16X4` agree with each other, and that
the depth expansion picks the same bank on both ports, for every address of a
64-deep RAM. It does **not** settle `dpram_init_word`: this design writes no
initial contents, so nothing here says that an `initial` block's word *n* lands
at address *n*. That half of the gap is still open and still needs the
experiment `docs/fpga-trellis.md` names.

**Throughput did not change, which is the answer worth having.** Both
bitstreams above were loaded on the same board, the same host and the same
afternoon, and `tests/usb_loopback.rs` timed six runs of each:

| buffers | round trips/s | bytes/s each way | spread of six |
|---------|---------------|------------------|---------------|
| shift register | 4000 | 256 008 | 256 002 – 256 028 |
| array | 4000 | 256 006 | 256 003 – 256 031 |

The two are inside 0.01 % of each other and inside each other's spread, so the
endpoint's shape is not what the rate is made of: round one's own account of
the figure says the wire is 43 microseconds of a round trip and the host's
stack is about 110, and none of that moved. Round one measured 3990 and
255 500 as the median of six on another day, which is 0.2 % below both columns
here — the difference between two afternoons, not between two designs. The
8-byte figure in "What it buys" above was not re-measured and is left as round
one recorded it.

As before, those numbers are **printed and never asserted**: nothing here is
compared against a clock and `tools/check.sh` does not run that test.

## The second class, and the endpoint it does not have

`usb_hub` is the second class layer, and what is interesting about it is almost
all subtraction. A serial port needed two interfaces, five functional
descriptors, three endpoints and sixty-seven bytes of configuration descriptor.
A hub needs **one interface, one endpoint and twenty-five bytes** — USB 2.0
§11.23.1 is one page and that is all of it — because a hub's one class-specific
descriptor is fetched by a request of its own, §11.23.2.1, and is nine bytes it
hands over through the class hook rather than nine bytes in a parameter.

**It is the first block in this library with no data endpoint**, and that is a
parameter `usb_dev_core` gained for it: `DATA_ENDP = 4'd0`, the same convention
`NOTIF_ENDP = 4'd0` already had, with both directions of `usb_bulk_ep` turned
off, `own_data` a flip-flop whose data input is the constant zero and the
buffers, the multiplexer arm and the byte interface all removed with it. The
alternative was to leave a bulk pair in the fabric that no descriptor declares
and no host would ever address, which at 64 bytes a direction is over a thousand
flip-flops or sixteen `TRELLIS_DPR16X4` for nothing.

**So a hub costs less than the vendor-specific device it is built on.** On the
ECP5, `usb_device_fs` is 853 LUT4, 355 flip-flops and 16 `TRELLIS_DPR16X4`, and
`usb_hub_fs` is **843, 317 and 2** — ten fewer lookup tables, thirty-eight fewer
flip-flops and fourteen fewer distributed RAMs than the block it is a class layer
on top of. `usb_device_ulpi` to `usb_hub_ulpi` is the same subtraction twice: 944
and 361 and 16 against 932 and 323 and 2. Against the other class,
`usb_cdc_acm_ulpi`'s 1209 and 480 and 18, the hub is 277 lookup tables and 157
flip-flops smaller. A class layer is not necessarily an addition.

(The two distributed RAMs hold **sixteen bits** — the status-change endpoint's
two-byte packet buffer — because one `TRELLIS_DPR16X4` is four bits of width and
the width is eight. `BUF_RAM = 0` would put those sixteen bits in flip-flops and
is very likely the better choice at that size on this family; the footprint table
does not measure it and the parameter is there so a design can.)

**And two things the hub found that the serial port had not.**

The first is that **the class hook is reached for a standard request**.
`usb_ctrl_ep` implements five and offers the rest, and its header says so in as
many words — "string descriptors and GET_STATUS included, so a class that wants
those can have them without this file changing again". A hub is the first class
to need one: Linux's `hub_configure` sends the standard GET_STATUS of USB 2.0
§9.4.5 during hub probe, with the comment "power budgeting mostly matters with
bus-powered hubs", and takes its failure path if the transfer does not complete.
So `usb_hub_req` claims it and answers two zero bytes. That is a layering smudge
and it is written up as one in that block's §7: the right home for a standard
request is endpoint 0, the change is small and well understood, and it is
reported rather than made because it widens `std_req` on the two cores that
already run on silicon and this round has no measurement that would catch a
regression in them.

The second is **how not to report a change**. `ip/usb_cdc_acm`'s notification
endpoint had a register meaning "the host has been told", it was set once per
configuration, and a host that was not listening at that moment never heard
again — measured on a part as no DCD and no DSR on three consecutive opens. The
hub has no such register at all: what it reports is **sticky state the host must
clear**, the host's own ClearPortFeature(C_PORT_*) is the acknowledgement, and
the sender is one wire —

```verilog
wire owed = configured & (change_map != 8'h00);
```

— so the defect is not avoided, it is unrepresentable. What that costs is one
extra one-byte packet per poll while a change is outstanding, plus exactly one
stale bitmap after a host clears a change that a packet had already been armed
with, and `StatusPipe::settles` in `tests/ip_library.rs` **asserts** the stale
one rather than tolerating it. The alternative is a latch saying "this bitmap has
already gone", which is the same defect with a different name, because nothing in
a device can know whether the host that received a bitmap is the host that will
act on it.

The same shape handles re-enumeration for free, which the serial port needed an
extra trigger for: a hub that is not configured has powered-off ports, a
powered-off port's connection is meaningless (§11.5.1.1), so the port's
connection rises when the **host** powers it — which is the moment the host is
listening — and a device already plugged in before the host ever looked is
reported with no edge detector and no one-shot anywhere.

## Using one

A block is an ordinary IP package, so a project reaches it with a
`depends` line and nothing else:

```text
# reticle.proj
name blinky
top top
device ice40-hx1k-tq144

source rtl/top.v

depends uart      ^1.0.0 path ../reticle/ip/uart
depends fifo_sync ^1.0.0 path ../reticle/ip/fifo_sync
```

`cdc_sync` is not named there and does not have to be: `axil_gpio`,
`cdc_pulse` and `fifo_async` declare it themselves and `PathProvider`
finds it in the directory next to the one it came from. `docs/ip.md` has
the whole resolution story.

Parameters are overridden at instantiation, as Verilog parameters always
are:

```verilog
fifo_sync #(
    .WIDTH (16),
    .DEPTH (64),
    .FWFT  (1)
) u_fifo (
    .clk (clk), .rst_n (rst_n),
    .wr_en (wr_en), .wr_data (wr_data), .full (full),
    .rd_en (rd_en), .rd_data (rd_data), .empty (empty),
    .count (count)
);
```

Three blocks carry parameters marked *derived* — `ADDR_WIDTH` and
`CNT_WIDTH` on the FIFOs and the RAMs. They are `$clog2` of the depth,
and they are parameters rather than localparams only because
Verilog-2005 has no way to put a computed width in an ANSI port list
otherwise. Never override them; overriding `DEPTH` re-evaluates them,
which `derived_parameters_follow_the_depth_they_come_from` checks.

`sdram_ctrl` takes the same approach further. A manifest has no
arithmetic, so the timings are declared the way a datasheet states them —
`T_RCD_NS`, `T_RP_NS`, `T_RAS_NS`, `T_RC_NS`, `T_RFC_NS`, `T_WR_NS`,
`T_RRD_NS`, `T_REFI_NS` and `T_INIT_US` next to `CLK_MHZ` — and the
cycle counts `TRCD` to `TINIT` are derived parameters whose defaults do
the division: rounded up for a minimum, rounded down for the refresh
interval, which is a maximum. A design states the part and the clock and
never the cycles.

## How it is tested

`tests/ip_library.rs` is the whole of it, and it runs in the default
build. Every block goes through all of:

| Test | What it proves |
|------|----------------|
| `manifests_parse` | every `reticle.ip` parses, names its own directory, lists files that exist and round-trips through `IpManifest::to_text` |
| `packages_resolve_and_elaborate` | every block builds through `ip::resolve` and `ip::elaborate` from a generated project, dependencies included |
| `blocks_synthesise_cleanly` | `synth::run` reports nothing at all — no error, no warning, and no inferred latch |
| `footprints_match_the_documentation` | the table below is the one this run measured |
| `axil_gpio_matches_the_axi4lite_definition` | `bus::match_ports` finds all nineteen AXI4-Lite signals on the GPIO at the widths its parameters imply |

and then a behavioural co-simulation test through `sim::Simulator`, which
is the part that matters:

- **`fifo_sync`** — occupancy counted word by word as it fills and
  drains, a write while full ignored, a simultaneous read and write, and
  the first-word-fall-through timing: with `FWFT = 1` the word is on
  `rd_data` in the cycle after the write, with no read strobe at all.
- **`cdc_sync`** — `q` follows `d` after exactly STAGES cycles, not one
  fewer.
- **`cdc_pulse`** — two clocks whose periods share no factor, three
  requests offered only while the block says it is free, and exactly
  three pulses counted in the destination domain.
- **`fifo_async`** — sixteen words through a four-word FIFO between a
  50-tick and a 71-tick clock, arriving in order and unmangled.
- **`uart`** — the transmitter's own output wired into the receiver, two
  bytes back to back, both recovered with no framing error; and a
  hand-driven line with a broken stop bit, which `rx_error` catches. Then
  the same loop with the divisor driven from the `div` port instead of
  the parameter, at a value neither divides the other, so a bit period
  that came from the port and one that came from the parameter cannot be
  confused; and the three divisors it refuses, each falling back to
  CLK_DIV rather than stopping the port.
- **`uart_baud_div` and `uart` together** — the last link in the claim
  that a host setting a rate changes a waveform. The divider is run until
  it has an answer, the number is handed to `uart`'s `div` port, and the
  gaps between the edges of a frame on `tx` are measured: seven rates a
  terminal program offers, each coming out a whole number of its own bit
  periods, with the transmitter's CLK_DIV set to a value that is *wrong*
  for every one of them so a period from the parameter could not be
  mistaken for one from the port. The worst slip is one part in a thousand.
- **`uart_baud_div`** — fourteen rates a host asks for, each divided and
  rounded here as `round(60e6 / rate)` rather than listed, so the block's
  own worked table and the assertion cannot drift apart without one of
  them being wrong about arithmetic; every accepted rate is checked to
  land inside the 2% an 8N1 frame survives; and the four refusals are
  listed one by one, because each is a different reason.

  **The divisor is latched once per character, not read every bit**, and
  that is a measurement rather than a preference. Comparing the bit
  counter against a run-time value needs a sixteen-bit magnitude
  comparison inside the loop, and `uart` came out at 229 `LUT4` and a
  logic depth of **19** that way — against 120 and **6** before the port
  existed. Latching the limit makes the loop a sixteen-bit *equality*
  against a register: 215 `LUT4` at depth **8**, in the table below. So
  the run-time divisor costs about 95 lookup tables and two levels of
  depth, and the magnitude comparison would have bought eleven more
  levels and nothing else — a rate that changes mid-character costs that
  character either way. Latching also gives the better semantics: a
  character in flight keeps the rate it started at, so a rate change can
  never corrupt a byte.
- **`spi_master`** — modes 0 and 3, with a slave model that samples
  `mosi` on the rising edge and presents `miso` on the falling one, so
  the bits are checked where a real slave would look at them; `cs_n` is
  checked to fall before the first edge and rise after the last.
- **`i2c_master`** — a whole seven-bit addressed exchange over an
  open-drain bus model: address for writing, a data byte, a repeated
  start, address for reading, one byte read with a closing NACK and a
  stop. The slave acknowledges each byte and **holds SCL down in the
  middle of one**, which the master has to wait out.
- **`pwm`** — the duty counted over a whole sixteen-cycle period at five
  settings, with the period tick counted too.
- **`timer`** — the gap between interrupts measured and checked against
  `(reload + 1) * (prescale + 1)`, and the sticky flag checked to stay
  until it is cleared.
- **`axil_gpio`** — real AXI4-Lite transactions from Rust: the direction
  register written and read back, the output register reaching the pins,
  the pin state arriving in the input register through its synchroniser,
  the set register ORing bits in, and bits above WIDTH dropped.
- **`ram_wrapper`** — both ports of both shapes, with and without the
  output register, and the read-first behaviour on a write.
- **`rv32i`** — real machine code, assembled in the test and run. See
  [below](#what-the-processor-actually-executes).
- **`mos6502`** — real 6502 machine code, assembled in the test from the
  documented opcode matrix and run against a 64 KiB memory, with the
  cycle count of every instruction checked against the reference.
  See [below](#and-what-the-6502-executes).
- **`eth_mac_rmii`** — the transmitter's own pins looped into the
  receiver, and the frame that comes out compared with the one that went
  in; the same frame with one dibit flipped in the payload, which still
  arrives and whose `rx_crc_ok` is low; the transmitted pins decoded
  independently in Rust and checked against seven octets of 0x55, a
  0xD5 delimiter, the payload and a check sequence computed by the
  testbench's own CRC, with the inter-frame gap counted after it; and a
  hand-driven frame that ends in the middle of an octet, which
  `rx_error` catches and delivers nothing for.
- **`spiflash_xip`** — five word reads against a flash model in the
  style of the `spi_master` slave, which samples `mosi` on the rising
  edge and presents `miso` on the falling one. The command octet and the
  address the flash saw are checked, the word is checked to be the four
  octets assembled little-endian, and `cs_n` is checked to fall a
  divisor before the first edge and to stay low past the last. Then the
  configuration registers are rewritten — a different command, eight
  dummy cycles and half the clock rate — and the reads are checked
  again, with the model expecting the dummy cycles too.
- **`sdram_ctrl`** — against an SDRAM model written in the test that
  **enforces the datasheet** rather than storing whatever it is given.
  It is clocked by the forwarded clock, so it samples the command pins
  where the part does, and it records a violation for a command before
  the power-up wait, an ACTIVE before the initialisation sequence or to
  an open bank, a READ or WRITE to a closed bank or inside tRCD, a
  PRECHARGE inside tRAS or tWR, an ACTIVE inside tRP, tRC or tRRD,
  anything inside tRFC or tMRD, a REFRESH with a row open, a refresh
  interval longer than tREFI, a mode word that is not the one asked for,
  and the controller driving `dq` while the part does. Read data is on
  the bus for exactly the one cycle the CAS latency says, with garbage
  either side. The cycle counts it enforces are computed in Rust from the
  nanoseconds, independently of the block. Two parts run the same
  workload — a Micron MT48LC16M16A2 at 75 MHz and CAS latency 2, an ISSI
  IS42S16400 with a smaller geometry at 48 MHz and CAS latency 3: a row
  written and read back with one ACTIVE for all thirty-two accesses,
  each byte enable alone, a row miss and back, every bank at the top of
  the address space, four hundred pseudo-random accesses across banks
  and rows, then four refresh intervals of idling and the data read
  again. And because a model that accepts anything proves nothing,
  `the_sdram_model_catches_a_controller_that_breaks_the_datasheet` gives
  the model a part slower than the controller was built for, one timing
  at a time — tRCD, tRP, tRAS, tRC, tWR and the refresh interval — and
  requires the model to name the rule each one breaks.
- **`hyperram_ctrl`** — against a HyperRAM model that **enforces the
  initial latency**: it counts CK edges from the fall of CS#, reads the
  command-address from the first six, and expects a write's first byte on
  exactly the edge its latency names, so data a clock early is DQ driven
  during the latency count and data a clock late is a first beat with
  nothing on it. It answers reads with RWDS toggling from the first beat,
  drives RWDS during CA the way the part does, doubles every third
  transaction in variable-latency mode as a refresh collision would, and
  checks CS#'s recovery time, whole words, and that the two ends never
  drive the bus together. Between them sit the IO registers as the FPGA
  backend builds them — a DDR output that launches its two halves in
  the next cycle, a DDR input that samples both edges, CK a quarter cycle
  late through the IO delay — with the testbench stepping in quarter
  cycles. The identification and configuration registers are read, and
  then every latency the part has, three to seven clocks, fixed and
  variable, is written to configuration register 0 and used for writes,
  masked writes and reads checked against a reference and against the
  model's own contents. A controller built believing the part is at
  five clocks, or at seven, is caught by the model, and so is one that
  does not wait out the recovery time.
- **`dvi_tx`** — the TMDS encoder is where these blocks are usually
  wrong, so it is tested **exhaustively against the specification's own
  algorithm**, written out in Rust from the DVI 1.0 flow chart with an
  unbounded disparity. A search from zero finds every running disparity
  the algorithm can reach (−8 to +8, in steps of two), and for each of
  those nine states and each of the 256 bytes the encoder is driven into
  the state by the shortest byte sequence, checked on the way, and then
  given the byte: the symbol and the disparity it leaves, read from the
  register, must be the specification's, and the symbol must decode back
  to the byte through an independent decoder. The four control symbols
  are checked too. `video_timing` is walked through a whole frame of
  each of the three modes, every pixel compared with where `de`, the two
  syncs, `x`, `y` and `frame` must be for VESA's and CEA-861's numbers.
  And the whole transmitter is run for two lines of 640 x 480 with a
  pattern on every lane: the serial bits are collected from the DDR
  ports, the clock lane is checked to be 1111100000 at every symbol, the
  symbols are cut at its edges and compared with the reference encoder's
  for every visible pixel on all three lanes, and blanking is checked to
  carry hsync on lane 0 exactly where the mode puts it. The transmitter
  is also asserted to be **one clock domain** — the pixel rate is an
  enable — so `timing::analyze_cdc` finds nothing crossing, and
  `dvi_tx_pll` is taken through the FPGA flow for both families, which
  must build the PLL from the 25 MHz reference within 1 % of 126 MHz and
  put every lane through a DDR register on the PLL's clock.
- **`vga_out`** — the same raster with none of the DVI machinery, for a
  board with a resistor ladder and no TMDS connector. It instantiates
  `dvi_tx`'s `video_timing` through a `depends` line rather than copying
  it, so there is one statement of the timings and the sync polarities
  for both video paths. A whole 640 x 480 frame is walked at the pins
  against VESA's numbers typed into the test; a second whole frame is
  walked with white driven into *every* pixel slot, which must reach the
  pins in the picture and nowhere else — 76 800 blanked pixels across the
  visible lines and 36 000 below them, counted separately, because a
  monitor takes its black level from the back porch and colour during
  blanking makes it roll. Each of the three modes' syncs are checked at
  the pins to idle at that mode's polarity and to pulse the other way for
  exactly the mode's sync width. And a 64-pixel pattern with a different
  ramp on each channel is read back at four, eight and one bits a
  channel: the pins carry the **top** bits of each byte, since the block
  truncates rather than rounds. `ip/vga_out/README.md` says why, and what
  it costs a picture. `examples/apple2` and `examples/nes` both build on
  it for the Basys 3, and the NES is where the truncation shows: its
  palette is in colour, and `examples/nes/README.md` names the one pair
  of its sixty-four entries that stops being two at four bits a channel.
- **`eth_mac_rgmii`** — the transmitter's pins looped into the
  receiver's, as the RMII test does, through the DDR registers modelled
  as the backend builds them: what the output registers take at one edge
  is on the pins, low nibble then high, for the next cycle, and the input
  registers hand the pair over at the edge after. A 64-octet frame and a
  one-octet frame come back intact with a good check sequence, and the
  transmitter takes an octet **every** cycle, which is what gigabit
  means. The pins are decoded in Rust: TX_CTL's two halves always agree,
  TXC is 2'b01 so it rises with every low nibble, and the octets are
  seven 0x55, 0xD5, the payload and the testbench's own check sequence,
  followed by a gap of at least twelve cycles. A bit flipped in the
  payload arrives with `rx_crc_ok` low, and RX_CTL's halves made to
  disagree mid-frame — the PHY's receive error — raise `rx_error` and
  withhold the frame's last octet. `timing::analyze_cdc` must find
  exactly two domains, `tx_clk` and `rgmii_rxc`, and nothing crossing
  between them.
- **`usb_device_fs`** — driven by a **USB host model** that sends real
  packets: SYNC, the bytes least significant bit first, a zero stuffed
  after every six ones, NRZI from idle J and an EOP, with CRC5 and CRC16
  from the host's own arithmetic, which is itself first held to the
  CRC catalogue's check values and to the bytes every analyser shows for
  a SETUP to address 0 (`2D 00 10`) and for the first GET_DESCRIPTOR
  (`… DD 94`). What the device sends back is decoded the way a host
  does — SYNC, stuffing, EOP, the PID's check nibble and the CRC16 — and
  the time it took to answer must fall between two and six and a half
  bit times. A whole enumeration runs: a bus reset, the device
  descriptor at address 0 with wLength 64, SET_ADDRESS taking effect
  only after its status stage, the old address then ignored, short
  reads, a read of exactly two packets, the configuration descriptor
  in 8 + 8 + 2 with runs of ones in the request to make the device
  unstuff, SET_CONFIGURATION, and a second bus reset forgetting the
  address. The same enumeration runs against a host whose clock is
  0.4 % slow and one 0.4 % fast — more than the 0.25 % the
  specification allows — which is what found the receiver sampling each
  bit in its last cycle instead of its middle. Packets with a bad CRC16,
  a bad CRC5, a bad PID check nibble or broken bit stuffing, and a
  SETUP to an endpoint that does not exist, all go unanswered; GET_STATUS
  and a string descriptor are stalled; the next SETUP is served as if
  none of it had happened. A data packet the host does not acknowledge
  is sent again with the same toggle, and the next one follows once it
  is. The device is one clock domain, and `usb_device_fs_pll` must build
  a PLL giving exactly 48 MHz from 12 on both families.

  Then **the bytes**. `bulk_loopback` is the one statement of moving them,
  the way `enumerate` is the one statement of enumerating, and it runs
  against the full-speed core, the full-speed core with the host's clock
  0.4 % slow and 0.4 % fast, the ULPI core, and the ULPI core behind the
  transceiver that reports LineState a clock late. A full **64-byte**
  packet, one a byte short of full, an eight-byte one, a five-byte one and a
  one-byte one go out and come back, each read before the next is sent, with
  `FF` and `07` in the payload so the bit stuffing is exercised inside a data
  packet and not only inside a descriptor. The 63-byte one is there because it
  is the only length at which a packet fills the buffer and does not start at
  the bottom of it, which is what a base counter off by one gets wrong. The bytes are checked at the byte interface as well as at
  the host, packet by packet, with `out_last` where the host put the end
  of each one, and the pair is in **loopback** — `out_*` wired into
  `in_*` — which is the wiring the board has rather than a testbench's
  private arrangement.

  Beside that, the endpoint's own rules, each with a test. The interface
  driven as a consumer would drive it rather than looped back, which is
  where the **zero-length packet** is in both directions: a loopback
  cannot carry one, since an OUT of no bytes hands nothing over, and
  `in_commit` alone is the only way to send one. The OUT endpoint NAKing
  while the last packet has not been taken, with the NAK **counted and
  never timed** — one attempt, one NAK — and both packets arriving in
  order once the consumer starts again. A packet the host sends twice with
  the same toggle, acknowledged twice and delivered once, which an endpoint
  that looked at its buffer before its toggle would NAK for ever. An IN
  packet the host does not acknowledge, sent again with the same toggle and
  released only by the ACK. And CLEAR_FEATURE(ENDPOINT_HALT) putting one
  direction's toggle back to DATA0, which is how a host and a device agree
  on a toggle again without a bus reset and is what the host-side loopback
  uses `clear_halt` for.

  The derived fields get two tests of their own, both by overriding
  `IFACE_DESC`: an interface that claims **five** endpoints and carries
  two, which the device must report as two, and two interfaces of one
  endpoint each both claiming **nine**, which must come back as two
  interfaces with one endpoint apiece and a `wTotalLength` of 41 — the
  second of which also proves the walk along the chain of `bLength` fields
  does not stop at the first descriptor.

  **All of the above runs on the buffers as arrays**, which is the default,
  and every one of them would catch a buffer that answered from the wrong
  address: `bulk_loopback`'s 63-byte packet in particular, which is the one
  length that fills a buffer and, in the shift-register shape, does not start
  at the bottom of it. What **none** of them reaches is the shape as it is
  built for a device: the array becomes a memory cell, and a memory cell is
  the backend's to lower — `the_logic_fallback_answers_like_the_memory_it_replaced`
  in `tests/fpga_flow.rs` is what answers for that, and a board is what
  answers for the silicon. `BUF_RAM = 0` gets `bulk_loopback` too, in
  `usb_device_fs_loops_bytes_with_the_buffers_as_shift_registers`, because the
  two shapes share all four of their counters and differ in a subtraction — the
  shift register reads at `index - written` where the array reads at `index`,
  and an off-by-one there is what the 63-byte packet is for. Its other rules,
  the toggle and the NAKs and the zero-length packet, are not re-run on that
  shape: none of them touches a buffer's addressing.

  **None of these would have caught the fault that cost the last round.**
  Every one of them passes against a three-bit state register for four
  states, because a simulator has no opinion about a flip-flop whose data
  input is a constant and will not bring one up holding a one. What catches
  that is the ECP5 backend refusing such a flip-flop, and it earned its
  keep immediately: it refused this round's first two bitstreams, for
  `in_total` at seven bits when the longest descriptor is 32 bytes and for
  a four-bit `pend_pid` in an endpoint whose four answers all have a zero
  in bit 2. Both are now as wide as their values, the second by registering
  two bits of *answer* and decoding the PID nibble in wires.
- **`usb_device_ulpi`** — the **same host model and the same
  enumeration**, with a **ULPI transceiver model** between them. The
  model is a transceiver, not a stub: a full-speed receiver that recovers
  the bit clock off the pair as a transceiver does rather than by
  counting the device's clock, a transmitter that adds the SYNC field, the
  bit stuffing, NRZI and the end of packet, and the ULPI bus above the
  two — both turnaround cycles, transmit commands, receive commands with
  LineState, VbusState, RxActive and RxError, and a register file with
  the reset values ULPI 1.1 gives. The enumeration is written once, in
  `enumerate`, and both cores are put through it, so the ULPI core is
  held to the bytes of every descriptor, the address taken only after its
  status stage, the short reads, the 8 + 8 + 2 configuration read and the
  bus reset that forgets the address. What the device puts on the *ULPI*
  bus is asserted too: the transmit command byte of each packet (`42h`
  for an ACK, `4Bh` for a DATA1), the packet the transceiver was handed
  with the CRC16 the device computed, and the two `00h` bytes of a
  zero-length data packet's CRC. The start-up sequence is asserted byte
  for byte — Function Control `65h`, OTG Control `00h`, Function Control
  `45h`, the readback, the Debug register for LineState — and the reset
  pin is checked to have been held. **The pair the model presents starts at
  SE0, not J**, because a full-speed bus is at J only because a device pulls
  D+ up and that pull-up has to charge the pair: so the start-up's LineState
  read is asserted to have been *repeated* while it said SE0 and answered
  once, which is what a Microchip transceiver on a real board does and what a
  model with the pair already at J could not show. A second test gives that
  model no pull-up at all, so its pair never leaves SE0, and asserts that the
  device still reports `phy_ready` and never calls it a bus reset.

  Then the parts ULPI adds, each with a test: `0xAA` driven into **every
  turnaround cycle** of every one of these tests, which the Link must
  ignore; `nxt` deasserted every other cycle, throttling it; a packet
  ended by `dir` falling with no closing receive command, which
  §3.8.2.4 allows instead; a register read a USB receive overrode, which
  the Link must retry; a readback that lies, after which the Link writes
  the registers again rather than believing them; and the transceiver
  taking the bus three bytes into the device's data packet, after which
  the host hears nothing, asks again, and is sent the same packet with
  the same toggle. The model **checks the Link** as well as answering
  it — it complains if the Link ever drives the bus while `dir` is high,
  drives a turnaround cycle, or asserts `stp` while the transceiver owns
  the bus — and
  `the_transceiver_model_catches_a_link_that_drives_a_bus_that_is_not_its`
  drives it with a Link that breaks each of those three, because a model
  that accepts anything proves nothing. The device answers 13 to 23
  clocks after the host's end of packet, inside the 2 to 6.5 bit times
  USB allows and inside ULPI's own 7-to-18-clock window. It is one clock
  domain and asks for no PLL. And it has **run on a board**:
  [`ip/usb_device_ulpi/README.md`](../ip/usb_device_ulpi/README.md) §11
  says what a Linux host read out of it and what our own host code moved
  through endpoint 1.

- **`usb_host_ulpi`** — not the host model at all, because this block
  **is** the host. Three of the twelve tests put it and `usb_device_ulpi`
  on **one D+ / D- pair**, each behind its own transceiver model, with
  the pair resolved between them by the only three things that drive it:
  the host's 45 Ohm terminations, whichever end is transmitting, and the
  device's own 1.5 kOhm pull-up. The first runs through well-behaved
  models; the second through two told to behave the way the part on the
  board does — each hearing its own transmission, each reporting LineState
  late out of a backlog that outlives the packet; and the third against a
  device built with `MAXPKT0 = 7'd8`, the smallest USB 2.0 §5.5.3 allows,
  which is the only one of the three that enters a **multi-packet data
  stage** and so the only one that puts the toggle under the host — and
  which also reaches the other way a data stage ends, since four full
  packets of eight is exactly `wLength` and there is no short packet to
  stop on. All three assert **bytes**: the
  eighteen of the device descriptor and the thirty-two of the
  configuration descriptor, compared with the same `expected_*` functions
  the device's own tests compare a host *model's* reading against;
  `wTotalLength` out of bytes 2 and 3 of a nine-byte read and
  `bMaxPacketSize0` out of byte 7 of an eight-byte one;
  `bConfigurationValue` out of byte 5 and `SET_CONFIGURATION` using
  **that** and not a constant 1; and the address said by the host *and by
  the device's own `address` output*, which is the only thing that
  distinguishes an address accepted from one sent.

  The other nine cover what an enumeration that works does not reach. The
  start-up sequence byte for byte, with `0Ah` = `06h` — the two 15 kOhm
  pull-downs that are the whole register difference between a host and a
  peripheral — and with the Debug register read **once**, because SE0 on a
  host's port is the answer and not something to retry. The board's
  vendor register written and read back before anything else. The probe:
  seven registers read through the block's own port with `enum_en` low,
  the part's Vendor and Product IDs among them, and **nothing put on the
  USB** while it happens. An empty port is not an attachment and gets no
  frame. A device that pulls D- up is reported as low speed and is sent
  nothing. A device that never answers is given up on after four tries
  and reported as a timeout rather than an error, with the four SETUP
  tokens counted. A device that leaves during the reset is reported as
  that. And `FS_LINE` is live: set to `10` with the pair at J, the host
  calls a full-speed device low speed — which is the failure a board that
  crosses DP and DM causes, from the other end of the same wire.

  **What it found is the argument for writing it.** Four defects in the
  host, three of them inherited from `usb_ulpi_link` where they are
  unreachable rather than absent: a register read's answer latched as a
  receive command, a start-up that sat out the transceiver's own reset and
  then read the receive command §3.5 promises *afterwards* as the reset
  itself, and one register written at two widths from two places. And two
  in the **model** every USB test here runs against: its receiver
  assembled its own transmission into bytes, because the guard was the
  ULPI bus state and a packet leaves that state over and over while
  receive commands go out; and it cleared RxActive at the first SE0 of an
  end of packet rather than at the SE0-to-J transition, which is what the
  part's own datasheet says it does **not** do and what keeps two bit
  times of turnaround from being violated. Neither had ever been asked,
  because the device's harness hands the model `None` while the device
  transmits.

  It has **not run on a board**, and that is not for want of trying:
  [`ip/usb_host_ulpi/README.md`](../ip/usb_host_ulpi/README.md) §9 has
  the whole of why, which is that the Cynthion's TARGET transceiver is on
  the left edge of the die and this backend describes the top and right
  edges only. The design places, routes and writes a bitstream all 128632
  of whose set bits decode with nothing unexplained; what it cannot do is
  put a pad on the right ball. One consequence is not about pins at all:
  the three bidirectional VBUS switches onto the TARGET A node are on that
  same edge, so **no design this flow can build can put power on that
  socket**.

- **`usb_cdc_acm`** — the **same host model again**, through both link
  layers and through the transceiver that reports LineState late, because a
  class layer that only works with the clean model is not finished. Twelve
  tests: a whole enumeration against the serial port's own descriptor set,
  which is nine packets of data stage and four times longer than anything the
  plain device sends; the descriptor set compared with sixty-seven bytes
  written **forwards in Rust from CDC 1.1 and PSTN 1.2**, the derived fields
  as the arithmetic and not the answer; the three class requests answered,
  with the line coding set to 115200 two-stop odd seven-bit — deliberately
  not 8N1, so a block reporting a constant is caught — and read back byte for
  byte; the requests it does **not** claim stalled, with the line coding
  proved untouched by them; SET_ADDRESS and CLEAR_FEATURE still working with a
  class on the hook, which is the hook's safety property; the **SERIAL_STATE
  notification** — the ten bytes of PSTN 1.2 §6.5.4 on the first poll after
  SET_CONFIGURATION, twenty NAKs after it because a state that has not changed
  is not news, one more with the other toggle when the line state does change,
  another after a bus reset because the host has forgotten, and an OUT to the
  endpoint answered with nothing at all since it has no OUT direction; and the
  bytes, both directions and looped back, with a full 64-byte packet among
  them.

  Four mutations of the implementation were each checked to fail those tests:
  GET_LINE_CODING left unclaimed, `class_req` raised for standard requests
  too, the notification endpoint given an OUT direction, and the union
  functional descriptor removed.

  What none of it could reach is whether a **real driver binds**, and that is
  the interesting half of a class layer. `tests/usb_cdc_acm.rs` is that half:
  it asks the kernel, through sysfs, whether `cdc_acm` claimed the device and
  made a terminal, and then writes bytes to that terminal and reads them back.
  [`ip/usb_cdc_acm/README.md`](../ip/usb_cdc_acm/README.md) §5 is what a host
  said, quoted — including the two faults that were invisible in simulation
  and what each of them was.

### What the processor actually executes

`rv32i` is the one block where "it simulates" would mean nothing on its
own, so its tests assemble RISC-V machine code — from the base ISA's own
field layout, never from the core's decoder — load it into the
instruction memory and check the architectural state. Both register-file
flavours run every program.

| Test | The program |
|------|-------------|
| `rv32i_builds_constants_and_pc_relative_addresses` | LUI, AUIPC, ADDI, and a write to `x0` that is dropped; the register file read after each instruction |
| `rv32i_computes_every_register_immediate_operation` | all nine of ADDI, SLTI, SLTIU, XORI, ORI, ANDI, SLLI, SRLI, SRAI, including SRAI shifting the sign in and SLTIU sign-extending its immediate before comparing unsigned |
| `rv32i_computes_every_register_register_operation` | all ten of ADD, SUB, SLL, SLT, SLTU, XOR, SRL, SRA, OR, AND, plus a shift amount above 31 to prove only `rs2[4:0]` counts |
| `rv32i_takes_and_declines_every_branch` | each of the six branches given a pair that makes it jump and a pair that makes it fall through, twelve in all, counted rather than positioned |
| `rv32i_jumps_and_links` | JAL and JALR linking the right address, jumping to the right one, and JALR clearing bit 0 of its target |
| `rv32i_loads_and_stores_every_width` | LB, LBU, LH, LHU, LW at every offset in the word, sign extension checked against zero extension, SB and SH landing in one lane and two without touching the rest, and a negative offset |
| `rv32i_survives_memories_that_make_it_wait` | the same program with zero, one and three wait states on both ports |
| `rv32i_traps_on_a_misaligned_access_and_on_nonsense` | a handler at `mtvec` that adds up `mcause` and returns past the faulting instruction, driven through causes 4, 6, 2, 3 and 11, then a JALR to an address that is not a multiple of four, which is cause 0 blamed on the jump |
| `rv32i_reads_and_writes_its_machine_csrs` | CSRRW / CSRRS / CSRRC and the immediate forms over `mstatus`, `mtvec`, `mie`, `mip`, `mhartid`; `mcycle` and `minstret` read twice and their difference checked against the core's own timing; an unknown CSR and a write to a read-only one, both illegal |
| `rv32i_takes_a_timer_interrupt_and_returns_from_it` | a spin loop with MIE and MTIE set, the timer line raised, the handler entered once with cause 0x80000007 and `mepc` inside the loop, MRET returning into it, and the external and software lines with their own causes |
| `rv32i_sums_an_array_in_a_loop` | eight words summed through a `lw` / `add` / `addi` / `bne` loop and the total stored past the end of the array |
| `rv32i_runs_a_recursive_function_on_the_stack` | Fibonacci of ten by the definition: two nested calls per frame, the return address and the argument saved on a stack, 177 calls and about two thousand instructions, and the stack pointer back where it started |

### And what the 6502 executes

`mos6502` is tested the same way and harder, because on a 6502 being
right is not only about the answer. `tests/mos6502_asm/mod.rs` is a
second assembler, built on the **documented opcode matrix typed out in
hexadecimal order** — mnemonic, addressing mode, opcode byte and the
cycle count the reference prints for it — with a two-pass front end over
it for labels and the thirteen operand syntaxes. Nothing in it comes
from the core's decoder, and the cycle counts in it are the
reference's, not the hardware's, so
`mos6502_counts_the_cycles_of_every_instruction` is a comparison between
two independent statements of the same table.

| Test | What it runs |
|------|--------------|
| `mos6502_loads_stores_and_transfers` | the three loads, the three stores and all six transfers, with N and Z after each, and TXS proved to be the one transfer that sets no flag |
| `mos6502_reaches_every_addressing_mode` | one load per mode, each from an address only that mode computes, so a mode that lands anywhere else reads a zero |
| `mos6502_traces_what_it_retired` | `dbg_retire` pulsing once per instruction with `dbg_pc` naming the opcode it came from, over a jump that skips one |
| `mos6502_counts_its_index_registers` | INX, INY, DEX and DEY over both wrap-arounds, with the flags each leaves |
| `mos6502_stores_through_every_mode` | every mode each of the three stores has, including STX's zero page,Y and STY's zero page,X |
| `mos6502_computes_every_logical_operation` | ORA, AND and EOR with their flags |
| `mos6502_sets_overflow_for_every_sign_combination` | the four sign combinations of ADC and of SBC, each with a result that overflows and one that does not, plus the carry in as part of the sum |
| `mos6502_adds_and_subtracts_in_decimal_mode` | fourteen known ADC results and nine known SBC ones in packed BCD, including the three places the NMOS part is famously surprising: Z from the *binary* sum, N and V from the intermediate before the high nibble is corrected, and every SBC flag being the binary subtraction's. Then all of them again with `DECIMAL_MODE = 0`, where D is still a flag and the arithmetic is binary |
| `mos6502_compares_with_cmp_cpx_and_cpy` | nine pairs through each of the three, with V set beforehand and D set too, so a comparison that touched either is caught |
| `mos6502_tests_bits_with_bit` | N and V from bits 7 and 6 of *memory* whatever the accumulator holds, and Z from the conjunction |
| `mos6502_shifts_and_rotates_through_carry` | all four, with the carry both ways, over the accumulator and over memory, which must agree |
| `mos6502_reads_modifies_and_writes_memory` | INC, DEC, ASL, LSR, ROL and ROR in zero page, zero page,X, absolute and absolute,X, with INC and DEC proved to leave the carry alone |
| `mos6502_writes_a_read_modify_write_byte_back_before_the_result` | the bus trace of `INC $10`: read, write what was read, write the result — the double write a memory-mapped register can see |
| `mos6502_wraps_zero_page_indexing_and_its_pointers` | `LDA $FF,X` reaching `$0001` and never `$0101`, and both halves of a `(zp,X)` and a `(zp),Y` pointer wrapping inside page zero |
| `mos6502_wraps_the_stack_inside_page_one` | S walked past both ends, with every write checked to stay inside `$0100`–`$01FF` |
| `mos6502_reproduces_the_indirect_jmp_page_bug` | `JMP ($10FF)`, whose bus trace must read `$10FF` and then `$1000`, so the jump lands where the part sends it and not where the arithmetic says |
| `mos6502_counts_the_cycles_of_every_instruction` | all 143 non-branch encodings, each assembled, checked to be the opcode byte the table names, run, and its cycles compared with the documented count |
| `mos6502_spends_an_extra_cycle_when_an_indexed_read_crosses_a_page` | every indexed read at a base that crosses and one that does not, and the indexed writes and read-modify-writes proved to spend that cycle either way |
| `mos6502_times_branches_by_whether_they_are_taken_and_cross` | two cycles not taken, three taken, four across a page, forwards and backwards |
| `mos6502_takes_and_declines_every_branch` | each of the eight given a flag state that makes it jump and one that makes it fall through |
| `mos6502_calls_and_returns_through_the_stack` | a nested JSR / RTS, with the pushed address checked to be the JSR's own last byte and the stack pointer back where it started |
| `mos6502_pushes_and_pulls_the_status_byte` | PHP pushing bits 4 and 5 set, PLP taking no flag from either, and PHP setting them again over a status byte pulled as zero |
| `mos6502_treats_an_undocumented_opcode_as_a_nop` | six of the 105 undocumented opcodes, each two cycles with no register and no flag touched |
| `mos6502_takes_an_irq_between_instructions` | the line raised at the top of a known instruction so the sequence's own seven cycles can be counted, the frame on the stack with bit 4 clear, the handler entered with I set, and a level that stays high firing again after RTI |
| `mos6502_masks_an_irq_with_the_interrupt_flag` | the same line held high for three hundred cycles with I set, and nothing taken |
| `mos6502_takes_an_nmi_on_its_edge_and_through_the_mask` | an NMI taken although I is set, a line held high counting as one edge and not many, a second edge taken, and a one-cycle pulse latched rather than lost |
| `mos6502_prefers_an_nmi_to_an_irq` | both raised at once; the NMI vector wins |
| `mos6502_breaks_and_returns_from_the_interrupt` | BRK's seven cycles, the address past its padding byte pushed, bit 4 pushed *set* where an interrupt pushes it clear, and RTI returning with I and C as they were |
| `mos6502_delays_the_effect_of_cli_and_sei_by_one_instruction` | an IRQ pending across CLI, which does not let it in until the instruction after; and one raised during SEI, which is taken anyway |
| `mos6502_multiplies_sixteen_bits_by_shift_and_add` | a 16 x 16 multiply in a subroutine: the multiplier shifted right a bit at a time through `LSR` / `ROR`, the multiplicand shifted left through `ASL` / `ROL`, a 16-bit add when the bit was set, five operand pairs, and the stack level again |
| `mos6502_sums_an_array_into_sixteen_bits` | sixteen bytes summed into a 16-bit total through a subroutine called once per byte, so the carry into the high byte and the whole call sequence are proved together |
| `mos6502_runs_a_recursive_function_on_the_stack` | Fibonacci of ten by the definition, with the argument and the first result kept on the stack and read back through `TSX` and absolute,X — which is how a 6502 reaches its own frame, since nothing there fits in a register |
| `mos6502_survives_a_memory_that_makes_it_wait` | the same loop with zero, one and three wait states, which must cost clocks and **not** bus cycles: all three take the same number of cycles and reach the same answer |

The **quirks are the point** of half of that list. A 6502 core that gets
the answers right and the timing wrong is not a 6502 core, because the
programs written for it counted cycles — so the extra cycle of a
page-crossing indexed read, the branch penalties, the stack's wrap
inside page one, the zero-page wrap of an indexed address and of a
pointer's two halves, the indirect `JMP` page bug and the
one-instruction delay of `CLI` and `SEI` each have a test named after
them, and the core's header names the test next to the quirk.

Where a block crosses clock domains, `timing::analyze_cdc` is run over
the synthesised and flattened netlist and the result is *asserted*, not
just printed: `cdc_pulse` must show two two-flop synchronisers and
`fifo_async` two gray buses with the generator found, and neither may
show a single `Unsynchronised` crossing. That is a real check on how the
HDL is written, not on whether it compiles — a synchroniser written as
one shift register instead of separate flops fails it.

### A note on testbench discipline

The two-domain tests present their inputs a fixed `SETUP` before each
edge rather than in the same instant. That is not decoration: an
event-driven simulator is entitled to process a clock edge before the
combinational cone feeding it has finished re-evaluating, so a value
changed in the same instant as the edge that samples it is a race, in
Reticle exactly as in any other simulator and in a real circuit. The
first version of the asynchronous FIFO test drove `wr_en` at the edge and
watched the FIFO overflow, because the flip-flop holding `wr_full`
sampled a stale comparison while the pointer beside it sampled a fresh
one. Present inputs early.

## Resource footprints

Measured, not guessed. Every row below comes from running the block
through this crate's own tools, at the parameters the row names:

- **LUT4** and **LUT6** — `synth::run` followed by
  `synth::techmap::map_module` with `MapOptions::lut(4)` and
  `MapOptions::lut(6)`. Storage is not mapped, so flip-flops appear as
  `dff` cells (one per register, whatever its width) and memories as
  `memory <depth>x<width>` with their ports.
- **iCE40 HX1K** and **ECP5 45F** — the whole `fpga::synthesize_for`
  flow for `ice40-hx1k-tq144` and `ecp5-45f-CABGA381`: generic synthesis,
  block RAM / carry / IO / clock-buffer inference, LUT covering and the
  rewrite to the family's own cells.

Each block is measured **as the top of its own design**, with the
constraints its own source states as attributes merged in, as
`Constraints::merge_attrs` does in a user's flow. So every port takes an
IO buffer — that is why `axil_gpio` shows 178 `SB_IO` — and a `ddr`
port takes its double-data-rate register. Dropped into a design, the
buffers disappear; the logic and the flip-flops do not.

`LUT depth` is the depth of the mapped combinational network in cells,
which is the rough shape of the critical path before place and route.

`mos6502` is measured twice, which is what `DECIMAL_MODE` is for.
Decimal mode is a ten-bit adder and a five-bit one for ADC, two
five-bit subtractors for SBC and four comparators, and the two pairs of
rows below say what that costs: **64 more LUT4 and 34 more LUT6** (about
four per cent either way), and on the iCE40 92 more `SB_LUT4` and 54
more `SB_CARRY`. The LUT depth does not move, because the decimal path
is beside the binary one and not in front of it, so the parameter buys
area back and nothing else. Off, D is still a flag — SED, CLD, PHP and
PLP all see it — and ADC and SBC simply ignore it.

**The Xilinx 7 series is not in this table**, although
`src/fpga/devices/xc7.dev` describes the Artix-7 of the Digilent Basys 3
and `reticle fpga --device xc7a35t-cpg236` takes a design all the way to
the files Vivado reads. Two reasons, both honest: that family declares
no double-data-rate register, so every block that asks for one is
*correctly refused* on it rather than measured — the run stops at
`sdram_ctrl`, the first of them — and a third device makes this test
take about four and a half minutes.
Adding it is one line — the `DEVICES` list in `tests/ip_library.rs` —
once `IDDR` and `ODDR` are in the device file. `docs/fpga.md` says what
that family does and does not support.

**A mapper bug moved ten of these rows, all of them LUT6.** The technology
mapper used to compute some cut functions wrongly — `src/synth/techmap/cuts.rs`
has the whole story, and `usb_cdc_acm` shipped three wrong descriptor bytes
to a host because of it. A wrong function came from reading an input the cut
did not separate as constant zero, and a function with a constant folded into
it has *smaller support*, so those cuts were reduced below their true size and
looked cheaper than any correct cut. Removing them costs area where the
mapper was buying it with wrong logic:

- `rv32i` at LUT6 went from 2044 to **2180** cells and from depth 28 to 29
  (+6.7 %), and with `REGFILE_BRAM=1` from 2075 to 2182 (+5.2 %). That is the
  largest regression in the table and it is the honest price of the fix: the
  cheaper cover did not compute the ISA.
- `usb_cdc_acm_ulpi` went from 1018 to 1028 cells but from depth 11 to
  **10**, and `usb_device_fs` from 765 to 767.
- `mos6502` went *down*, 1336 to 1320 with decimal mode and 1302 to 1286
  without, `dvi_tx` and `dvi_tx_pll` from 363 to 357, and
  `usb_device_ulpi` from 869 to 866. Composition does not only remove cuts:
  a merged cut whose cone collapsed to a constant under the zero substitution
  used to be **thrown away** as constant, and a cut whose leaf really does
  cancel out is now found where a corrupted table hid it. With eight cuts
  kept per node, a different eight survive and some nodes get a better one.

No LUT4 row moved with them, and no iCE40 or ECP5 row — those flows map onto
LUT4 too. That is not because LUT4 was unaffected (LUT4 is what broke
`usb_ctrl_ep`) but because a four-leaf merge has fewer ways to lose a path than
a six-leaf one, so on these blocks the LUT4 covers came out the same.

**Then sixteen more rows moved, because a block changed.** With the mapper
fixed, `usb_ctrl_ep`'s `desc()` reads its descriptor blob with one part-select
again instead of a page at a time, and the four USB blocks were re-measured.
The page split is **not** worth keeping on its own merits: taking it out is
smaller in eleven of the sixteen rows and **shallower in fourteen of them**,
because selecting one of four 128-bit pages and then a byte out of that is a
level of muxing the direct part-select does not need.

| Block | Target | Page split | One part-select |
|---|---|---|---|
| `usb_device_fs` | LUT4 | 913, depth 11 | 917, depth **10** |
| `usb_device_fs` | LUT6 | 767, depth 9 | **758**, depth **8** |
| `usb_device_fs` | iCE40 HX1K | 889, depth 12 | 891, depth **9** |
| `usb_device_fs` | ECP5 45F | 912, depth 11 | 916, depth **10** |
| `usb_device_ulpi` | LUT4 | 1003, depth 10 | **1000**, depth **9** |
| `usb_device_ulpi` | LUT6 | 866, depth 10 | **861**, depth 10 |
| `usb_device_ulpi` | iCE40 HX1K | 976, depth 11 | 976, depth **9** |
| `usb_device_ulpi` | ECP5 45F | 1004, depth 10 | **1001**, depth **9** |
| `usb_cdc_acm_fs` | LUT4 | 1108, depth 14 | **1098**, depth **12** |
| `usb_cdc_acm_fs` | LUT6 | 910, depth 10 | **884**, depth **9** |
| `usb_cdc_acm_fs` | iCE40 HX1K | 1095, depth 14 | 1101, depth **12** |
| `usb_cdc_acm_fs` | ECP5 45F | 1107, depth 14 | **1097**, depth **12** |
| `usb_cdc_acm_ulpi` | LUT4 | 1199, depth 13 | **1181**, depth **11** |
| `usb_cdc_acm_ulpi` | LUT6 | 1028, depth 10 | **1010**, depth 10 |
| `usb_cdc_acm_ulpi` | iCE40 HX1K | 1171, depth 13 | 1167, depth **11** |
| `usb_cdc_acm_ulpi` | ECP5 45F | 1201, depth 13 | **1183**, depth **11** |

The five rows that grew do so by 2 to 6 cells, and every one of them gets a
shorter critical path in exchange. So the workaround was costing area *and*
timing, which is the usual way round: it was written to dodge a compiler bug,
not because it was better logic.

The table is generated by `footprints_match_the_documentation` in
`tests/ip_library.rs` and compared byte for byte, so it cannot drift.
Run the tests with `UPDATE_EXPECT=1` to refresh it after an intended
change, and read the diff: a block that suddenly costs twice as much is
exactly what this table is for.

<!-- footprints: generated by tests/ip_library.rs -->
| Block | Top | Parameters | Target | Cells | LUT depth |
|-------|-----|------------|--------|-------|-----------|
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | LUT4 | 3 x dff, 27 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | LUT6 | 3 x dff, 22 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 2 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 18 x SB_DFFER, 1 x SB_GB, 27 x SB_IO, 290 x SB_LUT4 | 4 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | ECP5 45F | 1 x DCCA, 27 x LUT4, 2 x TRELLIS_DPR16X4, 18 x TRELLIS_FF, 27 x TRELLIS_IO | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT4 | 2 x dff, 27 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT6 | 2 x dff, 22 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 2 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 10 x SB_DFFER, 1 x SB_GB, 27 x SB_IO, 290 x SB_LUT4 | 4 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | ECP5 45F | 1 x DCCA, 27 x LUT4, 2 x TRELLIS_DPR16X4, 10 x TRELLIS_FF, 27 x TRELLIS_IO | 3 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | LUT4 | 2 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | LUT6 | 2 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | iCE40 HX1K | 2 x SB_DFFR, 4 x SB_IO, 1 x SB_LUT4 | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | ECP5 45F | 2 x TRELLIS_FF, 4 x TRELLIS_IO | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | LUT4 | 3 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | LUT6 | 3 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | iCE40 HX1K | 24 x SB_DFFR, 1 x SB_GB, 18 x SB_IO, 1 x SB_LUT4 | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | ECP5 45F | 1 x DCCA, 24 x TRELLIS_FF, 18 x TRELLIS_IO | 0 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | LUT4 | 6 x dff, 4 x lut | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | LUT6 | 6 x dff, 4 x lut | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | iCE40 HX1K | 1 x SB_DFFER, 5 x SB_DFFR, 7 x SB_IO, 6 x SB_LUT4 | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | ECP5 45F | 4 x LUT4, 6 x TRELLIS_FF, 7 x TRELLIS_IO | 1 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | LUT4 | 10 x dff, 41 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 4 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | LUT6 | 10 x dff, 37 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 41 x SB_DFFR, 1 x SB_DFFS, 2 x SB_GB, 24 x SB_IO, 298 x SB_LUT4 | 4 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | ECP5 45F | 2 x DCCA, 41 x LUT4, 2 x TRELLIS_DPR16X4, 42 x TRELLIS_FF, 24 x TRELLIS_IO | 4 |
| `uart` | `uart` | CLK_DIV=104 | LUT4 | 16 x dff, 213 x lut | 8 |
| `uart` | `uart` | CLK_DIV=104 | LUT6 | 16 x dff, 187 x lut | 6 |
| `uart` | `uart` | CLK_DIV=104 | iCE40 HX1K | 33 x SB_CARRY, 60 x SB_DFFER, 24 x SB_DFFES, 34 x SB_DFFR, 2 x SB_DFFS, 1 x SB_GB, 40 x SB_IO, 247 x SB_LUT4 | 8 |
| `uart` | `uart` | CLK_DIV=104 | ECP5 45F | 1 x DCCA, 215 x LUT4, 120 x TRELLIS_FF, 40 x TRELLIS_IO | 8 |
| `uart` | `uart_baud_div` | (defaults) | LUT4 | 9 x dff, 282 x lut | 23 |
| `uart` | `uart_baud_div` | (defaults) | LUT6 | 9 x dff, 235 x lut | 17 |
| `uart` | `uart_baud_div` | (defaults) | iCE40 HX1K | 35 x SB_CARRY, 181 x SB_DFFER, 3 x SB_DFFES, 1 x SB_GB, 52 x SB_IO, 263 x SB_LUT4 | 23 |
| `uart` | `uart_baud_div` | (defaults) | ECP5 45F | 1 x DCCA, 282 x LUT4, 184 x TRELLIS_FF, 52 x TRELLIS_IO | 23 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT4 | 10 x dff, 87 x lut | 6 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT6 | 10 x dff, 80 x lut | 4 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | iCE40 HX1K | 22 x SB_CARRY, 35 x SB_DFFER, 1 x SB_DFFES, 17 x SB_DFFR, 1 x SB_GB, 25 x SB_IO, 73 x SB_LUT4 | 3 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | ECP5 45F | 1 x DCCA, 88 x LUT4, 53 x TRELLIS_FF, 25 x TRELLIS_IO | 6 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT4 | 14 x dff, 116 x lut | 7 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT6 | 14 x dff, 90 x lut | 4 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | iCE40 HX1K | 18 x SB_CARRY, 20 x SB_DFFER, 4 x SB_DFFES, 17 x SB_DFFR, 1 x SB_GB, 30 x SB_IO, 117 x SB_LUT4 | 3 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | ECP5 45F | 1 x DCCA, 116 x LUT4, 41 x TRELLIS_FF, 30 x TRELLIS_IO | 7 |
| `pwm` | `pwm` | WIDTH=8 | LUT4 | 2 x dff, 23 x lut | 6 |
| `pwm` | `pwm` | WIDTH=8 | LUT6 | 2 x dff, 17 x lut | 4 |
| `pwm` | `pwm` | WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 8 x SB_DFFER, 8 x SB_DFFR, 1 x SB_GB, 21 x SB_IO, 21 x SB_LUT4 | 6 |
| `pwm` | `pwm` | WIDTH=8 | ECP5 45F | 1 x DCCA, 23 x LUT4, 16 x TRELLIS_FF, 21 x TRELLIS_IO | 6 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT4 | 4 x dff, 61 x lut | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT6 | 4 x dff, 51 x lut | 4 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 17 x SB_DFFER, 9 x SB_DFFR, 1 x SB_GB, 47 x SB_IO, 56 x SB_LUT4 | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | ECP5 45F | 1 x DCCA, 61 x LUT4, 26 x TRELLIS_FF, 47 x TRELLIS_IO | 5 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | LUT4 | 11 x dff, 47 x lut | 2 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | LUT6 | 11 x dff, 38 x lut | 1 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | iCE40 HX1K | 116 x SB_DFFER, 16 x SB_DFFR, 1 x SB_GB, 178 x SB_IO, 49 x SB_LUT4 | 2 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | ECP5 45F | 1 x DCCA, 48 x LUT4, 132 x TRELLIS_FF, 178 x TRELLIS_IO | 2 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT4 | 1 x lut, 1 x memory 256x8, 1 x memrd, 1 x memwr | 1 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT6 | 1 x lut, 1 x memory 256x8, 1 x memrd, 1 x memwr | 1 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | iCE40 HX1K | 27 x SB_IO, 1 x SB_LUT4, 1 x SB_RAM40_4K | 1 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | ECP5 45F | 1 x DP16KD, 1 x LUT4, 27 x TRELLIS_IO | 1 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT4 | 1 x memory 256x8, 1 x memrd, 1 x memwr | 0 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT6 | 1 x memory 256x8, 1 x memrd, 1 x memwr | 0 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | iCE40 HX1K | 36 x SB_IO, 1 x SB_RAM40_4K | 0 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | ECP5 45F | 1 x DP16KD, 36 x TRELLIS_IO | 0 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | LUT4 | 14 x dff, 2436 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | LUT6 | 14 x dff, 2180 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 29 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | iCE40 HX1K | 220 x SB_CARRY, 1024 x SB_DFFE, 292 x SB_DFFER, 66 x SB_DFFR, 1 x SB_GB, 208 x SB_IO, 5048 x SB_LUT4 | 36 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | ECP5 45F | 1 x DCCA, 2487 x LUT4, 32 x TRELLIS_DPR16X4, 358 x TRELLIS_FF, 208 x TRELLIS_IO | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT4 | 16 x dff, 2445 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT6 | 16 x dff, 2182 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 28 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | iCE40 HX1K | 220 x SB_CARRY, 2 x SB_DFFE, 292 x SB_DFFER, 66 x SB_DFFR, 1 x SB_GB, 208 x SB_IO, 2356 x SB_LUT4, 4 x SB_RAM40_4K | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | ECP5 45F | 1 x DCCA, 4 x DP16KD, 2446 x LUT4, 360 x TRELLIS_FF, 208 x TRELLIS_IO | 34 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | LUT4 | 25 x dff, 1710 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | LUT6 | 25 x dff, 1320 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | iCE40 HX1K | 179 x SB_CARRY, 132 x SB_DFFER, 4 x SB_DFFES, 3 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 1733 x SB_LUT4 | 16 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | ECP5 45F | 1 x DCCA, 1720 x LUT4, 139 x TRELLIS_FF, 57 x TRELLIS_IO | 16 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | LUT4 | 25 x dff, 1646 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | LUT6 | 25 x dff, 1286 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | iCE40 HX1K | 125 x SB_CARRY, 132 x SB_DFFER, 4 x SB_DFFES, 3 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 1641 x SB_LUT4 | 16 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | ECP5 45F | 1 x DCCA, 1661 x LUT4, 139 x TRELLIS_FF, 57 x TRELLIS_IO | 16 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT4 | 26 x dff, 305 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT6 | 26 x dff, 283 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | iCE40 HX1K | 10 x SB_CARRY, 127 x SB_DFFER, 64 x SB_DFFES, 3 x SB_DFFR, 1 x SB_GB, 34 x SB_IO, 301 x SB_LUT4 | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | ECP5 45F | 1 x DCCA, 305 x LUT4, 194 x TRELLIS_FF, 34 x TRELLIS_IO | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT4 | 9 x dff, 177 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT6 | 9 x dff, 166 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | iCE40 HX1K | 7 x SB_CARRY, 97 x SB_DFFER, 3 x SB_DFFES, 8 x SB_DFFR, 1 x SB_GB, 140 x SB_IO, 173 x SB_LUT4 | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | ECP5 45F | 1 x DCCA, 178 x LUT4, 108 x TRELLIS_FF, 140 x TRELLIS_IO | 4 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | LUT4 | 36 x dff, 429 x lut | 10 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | LUT6 | 36 x dff, 348 x lut | 9 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | iCE40 HX1K | 24 x SB_CARRY, 180 x SB_DFFER, 3 x SB_DFFR, 36 x SB_DFFS, 1 x SB_GB, 121 x SB_IO, 419 x SB_LUT4 | 10 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | ECP5 45F | 1 x DCCA, 428 x LUT4, 1 x ODDRX1F, 219 x TRELLIS_FF, 121 x TRELLIS_IO | 10 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | LUT4 | 26 x dff, 214 x lut | 7 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | LUT6 | 26 x dff, 187 x lut | 7 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | iCE40 HX1K | 13 x SB_CARRY, 102 x SB_DFFER, 4 x SB_DFFES, 11 x SB_DFFR, 1 x SB_DFFS, 1 x SB_GB, 91 x SB_IO, 215 x SB_LUT4 | 8 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | ECP5 45F | 1 x DCCA, 1 x DELAYG, 9 x IDDRX1F, 216 x LUT4, 10 x ODDRX1F, 118 x TRELLIS_FF, 91 x TRELLIS_IO | 7 |
| `dvi_tx` | `dvi_tx` | MODE=0 | LUT4 | 18 x dff, 480 x lut | 9 |
| `dvi_tx` | `dvi_tx` | MODE=0 | LUT6 | 18 x dff, 357 x lut | 7 |
| `dvi_tx` | `dvi_tx` | MODE=0 | iCE40 HX1K | 187 x SB_CARRY, 72 x SB_DFFER, 52 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 570 x SB_LUT4 | 8 |
| `dvi_tx` | `dvi_tx` | MODE=0 | ECP5 45F | 1 x DCCA, 481 x LUT4, 4 x ODDRX1F, 124 x TRELLIS_FF, 57 x TRELLIS_IO | 9 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | LUT4 | 18 x dff, 480 x lut | 9 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | LUT6 | 18 x dff, 357 x lut | 7 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | iCE40 HX1K | 187 x SB_CARRY, 72 x SB_DFFER, 52 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 570 x SB_LUT4, 1 x SB_PLL40_CORE | 8 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | ECP5 45F | 1 x DCCA, 1 x EHXPLLL, 481 x LUT4, 4 x ODDRX1F, 124 x TRELLIS_FF, 57 x TRELLIS_IO | 9 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | LUT4 | 7 x dff, 80 x lut | 4 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | LUT6 | 7 x dff, 68 x lut | 3 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | iCE40 HX1K | 22 x SB_CARRY, 36 x SB_DFFER, 2 x SB_DFFES, 1 x SB_GB, 67 x SB_IO, 76 x SB_LUT4 | 3 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | ECP5 45F | 1 x DCCA, 80 x LUT4, 38 x TRELLIS_FF, 67 x TRELLIS_IO | 4 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | LUT4 | 28 x dff, 396 x lut | 5 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | LUT6 | 28 x dff, 349 x lut | 4 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | iCE40 HX1K | 10 x SB_CARRY, 119 x SB_DFFER, 64 x SB_DFFES, 7 x SB_DFFR, 2 x SB_GB, 39 x SB_IO, 394 x SB_LUT4 | 5 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | ECP5 45F | 2 x DCCA, 6 x DELAYG, 5 x IDDRX1F, 395 x LUT4, 6 x ODDRX1F, 190 x TRELLIS_FF, 39 x TRELLIS_IO | 5 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | LUT4 | 88 x dff, 804 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | LUT6 | 88 x dff, 697 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 9 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 64 x SB_CARRY, 1024 x SB_DFFE, 300 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 2869 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 1 x DCCA, 853 x LUT4, 16 x TRELLIS_DPR16X4, 355 x TRELLIS_FF, 39 x TRELLIS_IO | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | LUT4 | 88 x dff, 779 x lut, 2 x memory 8x8, 2 x memrd, 2 x memwr | 11 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | LUT6 | 88 x dff, 667 x lut, 2 x memory 8x8, 2 x memrd, 2 x memwr | 8 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | iCE40 HX1K | 51 x SB_CARRY, 128 x SB_DFFE, 276 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 983 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | ECP5 45F | 1 x DCCA, 770 x LUT4, 4 x TRELLIS_DPR16X4, 331 x TRELLIS_FF, 39 x TRELLIS_IO | 11 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | LUT4 | 90 x dff, 1840 x lut | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | LUT6 | 90 x dff, 1581 x lut | 9 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | iCE40 HX1K | 64 x SB_CARRY, 1324 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 1804 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | ECP5 45F | 1 x DCCA, 1830 x LUT4, 1379 x TRELLIS_FF, 39 x TRELLIS_IO | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | LUT4 | 90 x dff, 902 x lut | 11 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | LUT6 | 90 x dff, 762 x lut | 8 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | iCE40 HX1K | 51 x SB_CARRY, 404 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 849 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | ECP5 45F | 1 x DCCA, 893 x LUT4, 459 x TRELLIS_FF, 39 x TRELLIS_IO | 11 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | LUT4 | 88 x dff, 804 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 10 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | LUT6 | 88 x dff, 697 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 9 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 64 x SB_CARRY, 1024 x SB_DFFE, 300 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 2869 x SB_LUT4, 1 x SB_PLL40_CORE | 10 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 1 x DCCA, 1 x EHXPLLL, 853 x LUT4, 16 x TRELLIS_DPR16X4, 355 x TRELLIS_FF, 39 x TRELLIS_IO | 10 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | LUT4 | 82 x dff, 904 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 10 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | LUT6 | 82 x dff, 804 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 11 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 97 x SB_CARRY, 1024 x SB_DFFE, 316 x SB_DFFER, 37 x SB_DFFES, 8 x SB_DFFR, 1 x SB_GB, 55 x SB_IO, 2954 x SB_LUT4 | 10 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 1 x DCCA, 944 x LUT4, 16 x TRELLIS_DPR16X4, 361 x TRELLIS_FF, 55 x TRELLIS_IO | 10 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | LUT4 | 96 x dff, 1295 x lut | 10 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | LUT6 | 96 x dff, 1158 x lut | 11 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | iCE40 HX1K | 132 x SB_CARRY, 482 x SB_DFFER, 35 x SB_DFFES, 25 x SB_DFFR, 1 x SB_GB, 159 x SB_IO, 1229 x SB_LUT4 | 10 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | ECP5 45F | 1 x DCCA, 1299 x LUT4, 542 x TRELLIS_FF, 159 x TRELLIS_IO | 10 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | LUT4 | 113 x dff, 1088 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 12 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | LUT6 | 113 x dff, 910 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 10 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 84 x SB_CARRY, 1152 x SB_DFFE, 413 x SB_DFFER, 44 x SB_DFFES, 14 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 104 x SB_IO, 3417 x SB_LUT4 | 12 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 1 x DCCA, 1118 x LUT4, 18 x TRELLIS_DPR16X4, 474 x TRELLIS_FF, 104 x TRELLIS_IO | 12 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | LUT4 | 107 x dff, 1163 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 11 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | LUT6 | 107 x dff, 1022 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 11 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 117 x SB_CARRY, 1152 x SB_DFFE, 429 x SB_DFFER, 42 x SB_DFFES, 9 x SB_DFFR, 1 x SB_GB, 120 x SB_IO, 3481 x SB_LUT4 | 11 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 1 x DCCA, 1209 x LUT4, 18 x TRELLIS_DPR16X4, 480 x TRELLIS_FF, 120 x TRELLIS_IO | 11 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | LUT4 | 92 x dff, 841 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 11 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | LUT6 | 92 x dff, 716 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 8 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 41 x SB_CARRY, 16 x SB_DFFE, 260 x SB_DFFER, 39 x SB_DFFES, 15 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 23 x SB_IO, 839 x SB_LUT4 | 10 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 1 x DCCA, 843 x LUT4, 2 x TRELLIS_DPR16X4, 317 x TRELLIS_FF, 23 x TRELLIS_IO | 11 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | LUT4 | 86 x dff, 930 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 10 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | LUT6 | 86 x dff, 818 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 11 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 74 x SB_CARRY, 16 x SB_DFFE, 276 x SB_DFFER, 37 x SB_DFFES, 10 x SB_DFFR, 1 x SB_GB, 39 x SB_IO, 927 x SB_LUT4 | 10 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 1 x DCCA, 932 x LUT4, 2 x TRELLIS_DPR16X4, 323 x TRELLIS_FF, 39 x TRELLIS_IO | 10 |
<!-- end footprints -->

### Seven things writing these blocks found

All seven were gaps in Reticle itself rather than in the blocks, and
each is pinned down by a test. Two were found by the original eleven
blocks, two by writing the three larger ones, two by the blocks that
need device primitives, and one by the 6502. **The first six have since
been fixed**, and their tests now hold the fix rather than the gap; the
seventh is described last and is still open.

**iCE40 flip-flops refused an active-low reset. Fixed.** Every block
resets on `negedge rst_n`, which is the convention the rest of this
repository's IP uses and the one nearly all real HDL uses. Every
`SB_DFF*` primitive the iCE40 database declares resets *high*, and
`fpga::techcells` used to match polarity exactly instead of putting an
inverter in front of the reset net, so it reported `F0310` and left
generic `dff` cells behind — which is why the iCE40 rows used to show
`dff` where the ECP5 rows show `TRELLIS_FF`, and why the iCE40 target
could not finish a design written the usual way.

The fix is the one `asic::library` has always applied to a polarity no
standard cell has: use the primitive with the other polarity and invert
the net feeding it. The inverter is one of the device's own LUTs, and
there is **one per net**, not one per flip-flop, so `cdc_sync`'s reset
costs one `SB_LUT4` between its two flops and `rv32i`'s costs one
between three hundred. The same is done for a clock-enable polarity a
family lacks, which is now expressible in a `.dev` `mode` clause as
`enable_low`. What is *not* inverted is the clock: a wrong edge would
mean a second clock network with its own skew, which is a
physical-design decision rather than a mapper's, so it is still reported.
`fpga::CellMapReport::inverted` lists the nets and
`ice40_flip_flops_take_an_active_low_reset_through_one_inverter` holds
the fix.

**A memory below the block-RAM threshold was left generic. Fixed.**
`fpga::primitives` decided that a memory too small for a block RAM would
be built from flip-flops or LUT RAM, recorded the decision in the report
— and nothing performed it. The `$memrd` and `$memwr` cells stayed, and
`fpga::check_nextpnr_json`, this crate's own netlist checker, then said
`a memory is left: it did not become block RAM or logic`. It was visible
in the table as the `memory 16x8, memrd, memwr` entries on the two FIFOs,
whose 128 bits fall under the 256-bit threshold; the 256 x 8 RAMs above
it mapped to `SB_RAM40_4K` and `DP16KD` cleanly.

The fallback is performed now, and which of the two it is comes from the
device file rather than from the family name. A device that declares a
distributed RAM primitive with a usable port map gets one — the ECP5's
`TRELLIS_DPR16X4`, whose shape Reticle reads off that port map (four
`dout` pins wide, four `raddr` pins deep) so that nothing about 16 x 4 is
written in Rust. A device that declares none, as the iCE40 database does,
gets one flip-flop per bit with a write enable decoded per word and a
multiplexer per read port, and a clocked read port gets its output
register. Several read ports mean several copies of a distributed RAM,
since it has one read port; one array of flip-flops serves them all with
one multiplexer each.

A memory the fallback still cannot build is *named*, not silently
mangled: initial contents that flip-flops cannot be preloaded with, a
write that is not clocked, write ports on different clocks, or more than
`MapOptions::max_logic_bits` (4096 by default, lifted by an explicit
`ram_style`) — a few thousand flip-flops are almost never what was
meant, and "it does not fit" is the useful answer.
`the_logic_fallback_answers_like_the_memory_it_replaced` in
`tests/fpga_flow.rs` drives the same stimulus into a design before and
after the lowering and insists the two answer alike;
`small_memories_become_logic_after_the_fpga_flow` holds the library
half.

**A function call inside an asynchronously reset process warned about
the function's own locals. Fixed.** Writing a CRC step, a decode table or
a sign extension as a Verilog `function` is the readable way to do it,
and calling one from inside `always @(posedge clk or negedge rst_n)` made
flip-flop inference report every argument and every local of that
function as a register that failed to get an asynchronous reset,
`S0013`, once per name. The netlist was always correct, since the call is
inlined into pure combinational logic; the diagnostic was wrong, and it
was enough to stop a block passing `blocks_synthesise_cleanly`, which
insists on no warning at all.

The fix is to warn only about nets the process assigns non-blockingly. A
blocking assignment inside a clocked block is a temporary, computed and
consumed within the cycle, and needs no reset; an inlined function leaves
one per local and per argument.
`function_locals_are_not_reported_as_unreset_registers` holds an
eighteen-line reproduction and now checks the warning is absent.

**A register file with two read ports was declined with a reason that
read like an acceptance. Fixed, twice over.**
`rv32i` keeps x1..x31 in one array with two read ports and one write
port. On the ECP5 the block RAM mapper turned it down with

```text
`DP16KD` has 2 read and 2 write port(s), the memory needs 2 and 1
```

Every comparison in that sentence holds — two reads wanted and two
available, one write wanted and two available — and yet it was a refusal,
because the constraint it left out is that a `DP16KD` port is *either* a
read or a write. The decision was right; the explanation could not be
acted on. It now names the constraint that applies:

```text
`DP16KD` has 2 port(s) and each serves either a read or a write,
but the memory needs 3 (2 read, 1 write)
```

The mapping a real flow applies here is **duplication**: one copy of the
contents per read port, each block with one read and one write port of
its own, all written together from the one writer. That is what the
mapper does now, whenever the memory has exactly one write port and more
read ports than a block can serve — two writers would need the copies
kept in step between them, which the blocks cannot do, and that case is
still declined with the sentence above. `BramMapping::copies` reports the
factor.

With `REGFILE_BRAM = 1` the two reads are clocked, which is the shape a
block RAM has, and the register file maps onto **four `DP16KD`**: two
copies, two blocks wide each, since thirty-two bits do not fit one
block's eighteen. With `REGFILE_BRAM = 0` the same array is read
combinationally and no block RAM does that — the core's own header says
that variant wants distributed RAM or flip-flops — so it takes the logic
fallback instead, thirty-two `TRELLIS_DPR16X4` on the ECP5 and a thousand
flip-flops on the iCE40. Both are in the table.

That asynchronous check is new too, and it matters beyond the register
file: before it, *any* memory over the threshold with a combinational
read was given a block whose clock pin nothing drove.
`a_two_read_port_register_file_is_duplicated_across_block_rams` and
`an_asynchronous_register_file_takes_the_logic_fallback` hold the two
halves, and `regfile_ecp5` in `testdata/fpga/` takes the same shape
through the whole flow.

**A project lost a top that its own sources instantiate with a
parameter override. Fixed.** `dvi_tx`'s package first shipped the
transmitter and a wrapper around it, `dvi_tx_pll`, which instantiates it
as `dvi_tx #(.MODE(MODE))`. A project whose top was `dvi_tx` then failed
to build with `P0401`, "the project's top `dvi_tx` is not in the
design". `ip::elaborate` never passed the project's `top` to the
Verilog elaborator, which chose its own roots, the modules nothing
instantiates, and elaborated everything else only as instances; an
instance with a parameter override gets a name derived from the
override (`leaf$W_1`), so no module kept the plain name.
`ip::elaborate` now passes the project's top to whichever frontend
defines it, so a VHDL top in a mixed project is left to the VHDL side.
`a_project_top_that_is_also_instantiated_with_an_override_keeps_its_name`
holds the fix, with and without the override. The wrapper packages
`dvi_tx_pll` and `usb_device_fs_pll`, split out to work around this,
stay as they are: a separate package for a PLL-wrapped variant is a
reasonable shape in its own right.

**A zero-step IO delay built a delay element. Fixed.**
`eth_mac_rgmii`'s TX_DELAY and RX_DELAY default to 0, for a PHY that
adds the RGMII clock skew itself, and a parameterised block can only
spell its delay as `(* io_delay = TX_DELAY *)`: Verilog has no way to
make an attribute conditional, so zero is how it says "none". The mapper
used to take the zero literally, giving the ECP5 a `DELAYG` set to
nothing on every such pin and the iCE40 an `F0304` warning for a delay
nobody asked for. A zero-step delay now builds nothing and warns about
nothing. `a_zero_step_io_delay_builds_nothing` holds the fix on both
families.

**A comment above a parameter moves into the port list. Open.**
Both processors document their parameters the way every block documents
its ports — a `//` line above the declaration, since that is where a
user looks first:

```verilog
module mos6502 #(
    // 1 builds the packed binary-coded decimal arithmetic ADC and SBC
    // use when the D flag is set; 0 leaves D a flag nothing reads.
    parameter DECIMAL_MODE = 1
) (
    input wire clk,
```

`reticle fmt` lifts those lines out of the `#(...)` list and stacks them
in front of the first entry of the `(...)` list, so `DECIMAL_MODE`'s
sentence comes back sitting above `clk`. No comment is *lost* — the
formatter's own corpus property is that the set of comments survives, and
it does — but each one ends up documenting something else, and the file
still looks right, which is the worst way for a formatter to be wrong.
It reproduces in ten lines:

```verilog
module m #(
    // How wide the data bus is.
    parameter WIDTH = 8
) (
    // The clock.
    input  wire clk,
    output wire q
);
    assign q = clk;
endmodule
```

becomes

```verilog
module m #(
  parameter WIDTH = 8
) (
  // How wide the data bus is.
  // The clock.
  input  wire clk,
  output wire q
);
  assign q = clk;
endmodule
```

No golden file under `testdata/verilog/format/` has a comment inside a
parameter list, which is why it had not been seen. The fix is to anchor
a leading comment to the parameter it precedes in
`verilog::format::comments`, the same way one is already anchored to a
port. Until then neither core's source goes through the formatter, and
`a_comment_above_a_parameter_still_moves_into_the_port_list` asserts
that the gap is still there, so closing it fails that test and points
at this paragraph.

## What is not here yet

The roadmap's blocks that waited on device primitives are all here now:
`sdram_ctrl`, `hyperram_ctrl`, `dvi_tx`, `eth_mac_rgmii` and
`usb_device_fs`, with their PLL wrappers. Each one is tested against a
model of what is on the other side of its pins, and those models are
where a word of caution belongs: every primitive the blocks rely on is
checked against the device database and the netlist checker, not
against silicon, and the simulator does not simulate a DDR register or
an IO delay — it sees the port the design sees. So each test models the
IO registers at the pins the way `docs/fpga.md` states the convention,
and a mismatch between that convention and a real part would pass them.
The same goes for the latency conventions of the HyperRAM, which are
taken from the Infineon datasheet's drawings and written down in the
block's header.

Each block's header lists what it does not do. The largest gaps are the
ones a user would meet first: `sdram_ctrl` has no bursts and serves one
word at a time; `hyperram_ctrl` has no bursts either and does not use
RWDS as a capture clock; `dvi_tx` runs everything at five times the
pixel rate, which leaves 1280 x 720 beyond both families' fabric;
`eth_mac_rgmii` is gigabit only; and every USB endpoint here now carries
**64-byte packets**, which is the largest a full-speed endpoint of any type
may have and leaves isochronous transfers — up to 1023 bytes — as the one
size these blocks cannot express. That was eight bytes and the four-bit
length both transmitters took, and what it cost is
[measured on a part](#what-the-packet-size-is-worth-measured-and-not-calculated)
below rather than reasoned about.

**The class layer has two blocks in it.** `usb_cdc_acm` is a serial port and
`usb_hub` is a hub, and both bind to a driver the operating system already
ships. **What `usb_hub` is half of is the thing that is not here**: it is a
hub's control endpoint, and a hub that forwards packets is a **transaction
proxy** rather than a repeater, for the reason its own README's §2 gives — a
ULPI transceiver's floor is about 24 bit times one way and a hub is allowed
about 4, so the two buses have to be decoupled and the host's side has to NAK
until the answer is there. That is the largest single piece of USB work left in
this library and it is what `ip/usb_host_ulpi` and `ip/usb_hub` were both built
towards.

**A human interface
device is what is not here either**, and it is now a smaller job than it was: a HID
needs the report descriptor, which is `GET_DESCRIPTOR` with a class
descriptor type — a request the hook already offers and a class may already
claim — plus an interrupt IN endpoint, which `usb_dev_core` already has as
`NOTIF_ENDP`, plus the report itself. What is genuinely missing for it is
nothing in the infrastructure; it is the block.

Two smaller gaps the serial port left. **No strings**, so a port has no
product name in `lsusb` and no `/dev/serial/by-id/` entry naming it; string
descriptors are the one thing `usb_ctrl_ep` stalls that the hook could now
answer, and answering them wants a second wide parameter and a language
identifier. And **nothing acts on the line coding**: `baud` comes out of
`usb_cdc_acm` because a host sets it, and following it means dividing a clock
by a run-time value, which every design would rather decide for itself.

The `rv32i` core is big: about 2400 LUT4s, which does not fit an iCE40
HX1K's 1280 and does fit an ECP5 45F many times over. That is a
straightforward multi-cycle machine rather than a squeezed one — one
33-bit adder shared by ADD, SUB, both comparisons and every address, two
barrel shifters, 64-bit `mcycle` and `minstret`, and word-wide muxes the
LUT mapper does not pack especially tightly. Making it smaller is worth
doing and is not worth doing before it is right.
