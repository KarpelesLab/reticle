# The Reticle IP library

The first-party half of phase 8. [`docs/ip.md`](ip.md) describes the
machinery — the manifest formats, the resolver, the bus model, the black
boxes — and [`docs/writing-a-cpu.md`](writing-a-cpu.md) describes how to
package a processor, using this library's two as the worked examples.
This document describes the **blocks**: thirty-two pieces of HDL that
drop into a design the way a crate drops into a Rust program, each with a
manifest, a Rust co-simulation test, and a resource footprint that was
measured rather than guessed.

They live at the top of the repository, in `ip/`, one directory per
package, grouped into nine folders by what a block is *for*. The
grouping is a filing system and nothing else: a project finds a block by
name (`library ../../ip`, then `depends uart ^1.0.0`), the name is the
one the block's own `reticle.ip` declares, and no code anywhere reads a
category. So this listing is a fact about the tree and not an interface
anybody depends on:

```text
ip/
  bus/
    axil_gpio/   reticle.ip  rtl/axil_gpio.v
    i2c_master/  reticle.ip  rtl/i2c_master.v
    spi_display_rx/  reticle.ip  README.md  rtl/spi_display_rx.v
    spi_master/  reticle.ip  rtl/spi_master.v
    uart/        reticle.ip  README.md  rtl/uart_tx.v  rtl/uart_rx.v
                 rtl/uart.v  rtl/uart_baud_div.v
  compress/
    inflate/  reticle.ip  README.md  rtl/inflate_adler.v
              rtl/inflate_window.v  rtl/inflate.v
  cpu/
    mos6502/  reticle.ip  rtl/mos6502.v
    rv32i/    reticle.ip  rtl/rv32i.v
  crypto/
    chacha20/  reticle.ip  README.md  rtl/chacha20_qr.v
               rtl/chacha20_core.v  rtl/chacha20.v
    sha256/    reticle.ip  README.md  rtl/sha256_core.v  rtl/sha256.v
  memory/
    fifo_async/     reticle.ip  rtl/fifo_async.v
    fifo_sync/      reticle.ip  rtl/fifo_sync.v
    hyperram_ctrl/  reticle.ip  rtl/hyperram_ctrl.v
    ram_wrapper/    reticle.ip  rtl/ram_sp.v  rtl/ram_sdp.v
    sdram_ctrl/     reticle.ip  rtl/sdram_ctrl.v
    spiflash_xip/   reticle.ip  rtl/spiflash_xip.v
  net/
    eth_mac_rgmii/  reticle.ip  rtl/eth_mac_rgmii.v
    eth_mac_rmii/   reticle.ip  rtl/eth_mac_tx.v  rtl/eth_mac_rx.v
                    rtl/eth_mac_rmii.v
  usb/
    usb_cdc_acm/        reticle.ip  README.md  rtl/usb_cdc_req.v
                        rtl/usb_cdc_acm.v  rtl/usb_cdc_acm_fs.v
                        rtl/usb_cdc_acm_ulpi.v
    usb_device_fs/      reticle.ip  rtl/usb_fs_rx.v  rtl/usb_fs_tx.v
                        rtl/usb_ctrl_ep.v  rtl/usb_device_fs.v
                        (usb_ctrl_ep.v holds four modules; see below)
    usb_device_fs_pll/  reticle.ip  rtl/usb_device_fs_pll.v
    usb_device_ulpi/    reticle.ip  README.md  rtl/usb_ulpi_link.v
                        rtl/usb_device_ulpi.v
    usb_host_ulpi/      reticle.ip  README.md  rtl/usb_ulpi_host_link.v
                        rtl/usb_host_sie.v  rtl/usb_host_enum.v
                        rtl/usb_host_ulpi.v
    usb_hub/            reticle.ip  README.md  rtl/usb_hub_req.v
                        rtl/usb_hub.v  rtl/usb_hub_fs.v  rtl/usb_hub_ulpi.v
    usb_proxy/          reticle.ip  README.md  rtl/usb_proxy_relay.v
                        rtl/usb_proxy_dn.v  rtl/usb_hub_proxy_ulpi.v
  util/
    cdc_pulse/  reticle.ip  rtl/cdc_pulse.v
    cdc_sync/   reticle.ip  rtl/cdc_sync.v
    pwm/        reticle.ip  rtl/pwm.v
    timer/      reticle.ip  rtl/timer.v
  video/
    dvi_tx/      reticle.ip  rtl/tmds_encoder.v  rtl/video_timing.v
                 rtl/dvi_tx.v
    dvi_tx_pll/  reticle.ip  rtl/dvi_tx_pll.v
    ppu2c02/     reticle.ip  README.md  rtl/ppu_palette.v  rtl/ppu2c02.v
    vga_out/     reticle.ip  README.md  rtl/vga_out.v
```

**A folder is named after the package it holds, and the name is not
shortened for the path it sits in**: `ip/usb/usb_device_fs/` declares
`name usb_device_fs`, not `device_fs`. The path is a little redundant for
it. Renaming the packages would have changed their identity — every
`depends` line, every lock file entry and the meaning of every version
number — to tidy a path, which is not a trade worth making.

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
| `uart` | `uart`, `uart_tx`, `uart_rx` | UART, 8N1 by default, framing from parameters, run-time or parameterised baud divisor, ready / valid, `rx_error` plus the three it is made of | — |
| `uart` | `uart_frame`, `uart_frame_tx`, `uart_frame_rx` | the same two halves with the **character format on ports** — 5 to 8 data bits, five parity modes, one or two stop bits — a receive handshake, and framing, parity, break and overrun reported separately | — |
| `uart` | `uart_baud_div` | clocks per bit from a bit rate, by restoring long division, with the rates it refuses | — |
| `uart` | `uart_line_coding` | a USB host's `bCharFormat`, `bParityType` and `bDataBits` turned into those format ports, with an `ok` for the values it cannot give exactly | — |
| `spi_master` | `spi_master` | byte-level SPI master, any CPOL / CPHA | — |
| `i2c_master` | `i2c_master` | byte-level I²C master, 7-bit addressing, clock stretching tolerated | — |
| `spi_display_rx` | `spi_display_rx` | the **receiving** end of a display's four-wire SPI: one data wire oversampled against an external clock, bytes framed by the chip select and tagged by D/C, with every uncertainty a parameter, a rate limit stated as a ratio of clocks, and six counters that are the only way the link will ever be characterised | `cdc_sync` |
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
| `sha256` | `sha256`, `sha256_core` | FIPS 180-4 SHA-256 over a byte stream, **padding included**: one round per cycle, 129 cycles a block, fixed latency for a given message length | — |
| `chacha20` | `chacha20`, `chacha20_core`, `chacha20_qr` | RFC 8439 ChaCha20: the quarter round, the block function at one round per cycle (22 cycles a block), and the stream cipher over a 32-bit port, with a stop when the block counter runs out | — |
| `inflate` | `inflate`, `inflate_window`, `inflate_adler` | RFC 1951 DEFLATE **decompressing**, with RFC 1950's zlib framing: all three block types, a 32 KiB sliding window in block RAM, a copy engine that stalls mid-match, and every malformed stream reported on a code | — |

`ppu2c02` is the block whose *subject* needs a statement rather than only
its behaviour, so it has a page of its own,
[`ip/video/ppu2c02/README.md`](../ip/video/ppu2c02/README.md): it is
implemented from the published description of a machine, and no game
data, character data or lockout logic is in this repository. It is used
by [`examples/nes`](../examples/nes), which runs a demo written for that
example and nothing else. It is not in the footprint table below, which
measures the blocks `tests/ip_library.rs` takes through the flow; its
numbers are on its own page and in `tests/nes.rs`.

Three other blocks carry a page.
[`ip/bus/spi_display_rx/README.md`](../ip/bus/spi_display_rx/README.md) is
the one written from **no specification at all** — the link it receives is
an observation on a user's own screen rather than a document — so it
records what was observed, in the user's words, across the three passes
that corrected each other, and then which of its parameters a measurement
still has to choose.
[`ip/video/vga_out/README.md`](../ip/video/vga_out/README.md) says what
truncating colour to a board's bits per channel costs a picture.
[`ip/usb/usb_device_ulpi/README.md`](../ip/usb/usb_device_ulpi/README.md) is
a different kind of page again: it is **the protocol, written down before
the block was**, the way [`docs/apollo-protocol.md`](apollo-protocol.md) was
written before the Apollo transport — every fact of ULPI 1.1 the block
relies on, with the section it came from and how sure of it this project is,
then what the block leaves out and why, then what simulation established and
what it cannot. A link layer written from a reading nobody wrote down is a
link layer nobody can check.

[`ip/usb/usb_cdc_acm/README.md`](../ip/usb/usb_cdc_acm/README.md) is the
third of that kind and is about a different sort of fact again. ULPI's
document is a bus read out of a specification; a class layer's question is
not "is this descriptor legal" but **"does the driver bind"**, which no
specification answers and only a host can. So that page has a fourth
confidence level beside HIGH, MEDIUM and LOW — **CHECKED**, meaning measured
on a host with the output quoted — and it is careful about the difference.
**CHECKED** is that `cdc_acm` binds to this descriptor set, that the port
opens and carries bytes, that the **SERIAL_STATE notification arrives and
the driver acts on it** — the ten bytes read off endpoint `82h` and
`TIOCMGET` reporting DCD and DSR, which `cdc_acm` can only get from that
notification — and that SET_LINE_CODING's seven bytes reach the device:
115200 read back out of its own registers, which is the only measurement
anywhere of the class hook's host-to-device data stage on silicon.
**MEDIUM** is the *why* of any of it: that the driver reads the union
functional descriptor to tell the interfaces apart, that it wants an
interrupt IN endpoint on the communications interface, that it gates
SET_LINE_CODING on `bmCapabilities` D1. Nothing here read that driver's
source or watched the bus, and the devices that would settle those — one
without a union descriptor, one without the endpoint — have not been on a
board. The page also records where the specification and the driver pull in
opposite directions and which way the block went: D1 is one bit over four
things, the block does three of them, and it is set anyway.

[`ip/usb/usb_host_ulpi/README.md`](../ip/usb/usb_host_ulpi/README.md) is the
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

[`ip/usb/usb_hub/README.md`](../ip/usb/usb_hub/README.md) is the fifth, and
it is the page whose **first** section is an argument about why the block is
not the thing its name suggests. A hub is a repeater — USB 2.0 §11.1.1 — and
through a ULPI transceiver the floor for a byte in and a byte out is about
24 bit times against the 4 a hub is allowed, because the transceiver does
not report a byte until the byte is complete and then prepends a fresh SYNC
on the way out. So `usb_hub` is a hub's **control endpoint**: the
descriptors, the class requests, the port state and the status-change
endpoint, with a second USB controller behind the port and nothing joining
the two conversations. That page's §2 is the timing, §8 is the kernel log of
a host finding the hub, finding something on its port and failing to
enumerate it, and both are written as the correct outcome of that round
rather than as a defect. It also carries one deliberate departure from the
other four: it cites chapter 11 by **section** and never by table number,
because a section number misquoted is findable and a table number misquoted
sends a reader somewhere else and looks authoritative doing it.

[`ip/usb/usb_proxy/README.md`](../ip/usb/usb_proxy/README.md) is the sixth,
and it is the sequel to that kernel log: the half that forwards. Its **§2**
is the one section to read if only one gets read, because it is an
architectural decision written down as one — **pass-through addressing**,
which makes the PC the only authority for the downstream device's address
and so needs no translation table, no descriptor cache and no way for the
two buses to disagree about a packet size. The price of it is a port reset
that really reaches the device, which is what `ip/usb/usb_hub` gained a
handshake for, and the consequence of it is that `ip/usb/usb_host_ulpi`'s
own enumerator is not instantiated in a proxy at all.

[`ip/crypto/sha256/README.md`](../ip/crypto/sha256/README.md) and
[`ip/crypto/chacha20/README.md`](../ip/crypto/chacha20/README.md) are the
seventh and eighth, and they are the first pages here that have to be
careful about a **negative** claim rather than a positive one. A USB page
says what a host was observed to do; a crypto page has to say what an
*attacker* cannot do, and the shape of that is a section listing exactly
what is **not** defended against. Both have a §4 for the constant-time
property that is measured — fixed latency, no addressable storage — and a
§5 for what that property is not, which is anything at all about power
consumption or electromagnetic emission. The category's own section,
[below](#the-crypto-category-and-what-a-constant-time-claim-is-worth), is
where the doctrine and the provenance live, including which facts came out
of a document and which were confirmed against a second running
implementation.

[`ip/bus/uart/README.md`](../ip/bus/uart/README.md) is the ninth, and it is
the only one so far whose subject is a **shape** rather than a protocol.
The block is eight modules in two layers because Verilog-2005 has no
default for a port and five designs in this repository instantiate the
three that were there first: a character format a USB host chooses cannot
be a parameter, a parameter is the only thing that can keep an existing
instantiation meaning what it meant, so the package has both and one
implementation underneath. That page is also where this library first had
to write down **what a loopback cannot see** — a transmitter sending one
stop bit where two were asked for is invisible to any receiver, because a
receiver samples the first stop bit and nothing after it — and it says
which three mutations were run to establish that rather than assert it.

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

`class_claim` and `class_len` are read **combinationally**, in the same
cycle `class_req` is high. That is not a shortcut: endpoint 0 chooses the
transfer's stage in the very cycle the SETUP's data packet ends, so a class
that answered a cycle later would need a fifth stage there, a handshake
back, and a rule about what happens if the host's next token arrives first.
A request decoder is a comparison of eight bytes against constants — there
is nothing in it to sequence — so asking for it combinationally asks for
nothing a class cannot give. `ip/usb/usb_cdc_acm/rtl/usb_cdc_req.v` is that
decoder and it is fifteen lines of `assign`.

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
`ip/usb/usb_cdc_acm/README.md` §4 writes up as the defect it was.

`serial_state` is a **port** of `usb_cdc_acm` and not a constant, seven bits
wide because §6.5.4 defines seven and reserves the other nine: DCD, DSR,
break, ring, framing, parity and overrun. A device with no modem lines ties it
to `7'b000_0011` — both carriers, no errors — and the argument is the
specification's own words for those two bits, since a port whose far end is in
the same die has its carrier present and its data set ready from the moment it
exists. `ip/usb/usb_cdc_acm/README.md` §4 and §5 say what a host was observed to
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

A **distributed RAM** — `reg [7:0] buf [0:63]` — needs neither a write
decoder nor a read multiplexer, and was the obvious answer. It was not
available: when this was written `src/fpga/devices/ecp5.dev` declared that
bel with no site count, so `fpga::place` counted zero of them and refused
any design that needed one, which is the same gap
`testdata/fpga/cynthion/usb_cdc_uart.v` records about `ip/memory/fifo_sync`.
It is available now — 3036 sites on the Cynthion's part — and taking it gave
back 977 of those 937 lookup tables and 1024 of the flip-flops as well.

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
`testdata/fpga/cynthion/usb_cdc_uart.v` carries **one byte at a time**
through its UART — that file's header says why, and the short version is
that it was written while `ip/memory/fifo_sync` could not be placed on this
part — so it sends one-byte packets whatever `wMaxPacketSize` says, and a
wider packet does nothing for it at all. The placement gap is closed now, so
that is a design this round did not revisit rather than a constraint it was
under. The bulk loopback is the design that measures the endpoint rather
than the bridge above it.

**What these numbers are and are not.** They are two runs of one host on one
machine against one part, repeatable to a tenth of a percent over four runs
each, and they are **printed by a test and never asserted**: no number here is
compared against a clock, and `tools/check.sh` does not run that test. What
they do not measure is a host that pipelines transfers instead of waiting for
each one — a queue of URBs would overlap the round trips and go faster at both
sizes — so they are the floor of what the endpoint can do and not the ceiling.

**What it found in this compiler, and it is fixed now.**
`ip/memory/fifo_sync` could not be placed on an ECP5 at all. Its storage is
an array indexed by a variable, which becomes a distributed RAM, and nothing
in the ECP5 fabric model had a site for one to go in, so `fpga::place`
counted zero of them and refused at any depth and with `FWFT` either way:

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
`TRELLIS_DPR16X4` is **slices A, B and C of one logic tile**, held together
by one bit of that tile (`F50B11`), so a tile holds exactly one and it costs
six of the tile's eight lookup tables. `ip/memory/fifo_sync` now places,
routes and comes out as a bitstream at depths 16, 32 and 64, with every bit
of the image decoding back through the database into a feature it names — 97
bits per RAM, which is `ecppack`'s own number for the 111 distributed RAMs
in this board's reference bitstreams.

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
came out as 1383 lookup tables, 604 flip-flops and **18** distributed RAMs —
the bulk pair's sixteen and the notification endpoint's two — with all 59 995 of
its bits decoding and nothing unexplained. On the part the kernel's own
`cdc_acm` binds it on `/dev/ttyACM1`, the ten bytes of SERIAL_STATE arrive off
endpoint `82h` with both carriers set, 48 bytes go out and come back byte for
byte through the UART, and GET_LINE_CODING answers 115200 8N1. That is
`tests/usb_cdc_acm.rs`, and it is `#[ignore]`d like the other two.

Those three numbers are of **that** design and no longer of this one: the
design has grown twice since, and the second time is
[`ip/bus/uart/README.md`](../ip/bus/uart/README.md) §9. It is 1786 lookup
tables and 837 flip-flops with the 8N1 UART it had, and **1877 and 845**
with the configurable one and the line-coding decode — 91 lookup tables and
8 flip-flops for a host's framing reaching the wire. Both were built,
loaded and measured through the device file in the same session, which is
what makes the difference a measurement rather than a subtraction: five
different line codings produced one waveform before and five different
waveforms after.

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

**So a hub costs less than the vendor-specific device it is built on.** On
the ECP5, `usb_device_fs` is 886 LUT4, 364 flip-flops and 16
`TRELLIS_DPR16X4`, and `usb_hub_fs` is **847, 318 and 2** — thirty-nine
fewer lookup tables, forty-six fewer flip-flops and fourteen fewer
distributed RAMs than the block it is a class layer on top of.
`usb_device_ulpi` to `usb_hub_ulpi` is the same subtraction twice: 979 and
370 and 16 against 942 and 324 and 2. Against the other class,
`usb_cdc_acm_ulpi`'s 1229 and 481 and 18, the hub is 287 lookup tables and
157 flip-flops smaller. A class layer is not necessarily an addition.

(The two distributed RAMs hold **sixteen bits** — the status-change
endpoint's two-byte packet buffer — because one `TRELLIS_DPR16X4` is four
bits of width and the width is eight. `BUF_RAM = 0` would put those sixteen
bits in flip-flops and is very likely the better choice at that size on this
family; the footprint table does not measure it and the parameter is there
so a design can.)

**And two things the hub found that the serial port had not.**

The first is that **the class hook was reached for a standard request, and
should not have been**. `usb_ctrl_ep` implemented five and offered the rest,
and its header said so in as many words — "string descriptors and GET_STATUS
included, so a class that wants those can have them without this file
changing again". A hub was the first class to need one: Linux's
`hub_configure` sends the standard GET_STATUS of USB 2.0 §9.4.5 during hub
probe, with the comment "power budgeting mostly matters with bus-powered
hubs", and takes its failure path if the transfer does not complete. So
`usb_hub_req` claimed it and answered two zero bytes.

That was a layering smudge, it was written up as one in that block's §7, and
**the round that built the proxy made the change** — because it had the
board and the CDC tests in front of it, which is exactly what the report
asked for. GET_STATUS to the **device** is endpoint 0's now: two bytes whose
bit 0 is bit 6 of `CFG_ATTR`, the same byte the configuration descriptor's
`bmAttributes` is written from, and whose bit 1 is a Remote Wakeup nothing
in this library can enable. **What it costs is one flip-flop and about
thirty lookup tables** in the technology-independent netlist, on every
device in the library: `usb_device_fs` went from 88 flip-flops and 804 LUT4
cells to 89 and 835, which is the `stat_sel` register and the multiplexer
the two bytes come out of. After ECP5 mapping the same change reads 853 LUT4
and 355 flip-flops against 886 and 364 — **thirty-three lookup tables and
nine flip-flops**, and the iCE40 flow reads the same nine. Why a mapper
turns one more generic flip-flop into nine was not chased down; both numbers
are in the table below and this sentence is the measurement rather than the
arithmetic. And no class has to think about the request again.

GET_STATUS to an **interface** or an **endpoint** is still on the hook, and that
is deliberate rather than unfinished: §9.4.5 makes an interface's two bytes
reserved and zero and an endpoint's bit 0 the Halt feature, and both are
answerable only by something that knows which interfaces and endpoints exist,
which endpoint 0 does not — `IFACE_DESC` is a blob it indexes and `usb_bulk_ep`
is a module beside it. A device that answered zero for *any* endpoint number
would be claiming endpoints it has not got, where §9.4.5 asks for a STALL.
`usb_device_fs_ignores_bad_packets_and_stalls_what_it_cannot_do` asserts both
halves of that division.

The second is **how not to report a change**. `ip/usb/usb_cdc_acm`'s
notification endpoint had a register meaning "the host has been told", it
was set once per configuration, and a host that was not listening at that
moment never heard again — measured on a part as no DCD and no DSR on three
consecutive opens. The hub has no such register at all: what it reports is
**sticky state the host must clear**, the host's own
ClearPortFeature(C_PORT_*) is the acknowledgement, and the sender is one
wire —

```verilog
wire owed = configured & (change_map != 8'h00);
```

— so the defect is not avoided, it is unrepresentable. What that costs is
one extra one-byte packet per poll while a change is outstanding, plus
exactly one stale bitmap after a host clears a change that a packet had
already been armed with, and `StatusPipe::settles` in `tests/ip_library.rs`
**asserts** the stale one rather than tolerating it. The alternative is a
latch saying "this bitmap has already gone", which is the same defect with a
different name, because nothing in a device can know whether the host that
received a bitmap is the host that will act on it.

The same shape handles re-enumeration for free, which the serial port needed an
extra trigger for: a hub that is not configured has powered-off ports, a
powered-off port's connection is meaningless (§11.5.1's Powered-off state), so
the port's
connection rises when the **host** powers it — which is the moment the host is
listening — and a device already plugged in before the host ever looked is
reported with no edge detector and no one-shot anywhere.

## The third USB block on one die, and the one it forwards for

`usb_proxy` is the biggest block in this library by some way, and the reason
is that it is **two USB controllers and the thing between them**: a
peripheral Link and a device core on one ULPI transceiver, a host Link and a
transaction engine on another, a second packet decoder, and a 64-byte relay
buffer. On the ECP5 that is 3090 LUT4, 999 flip-flops and 10
`TRELLIS_DPR16X4` against `usb_hub_ulpi`'s 942, 324 and 2 — so **forwarding
costs about 2150 lookup tables more than reporting a port**, and a design
that only wants to be a hub should be one.

Two of those numbers are worth reading as a comparison rather than a cost.

**It does not instantiate `usb_host_enum`.** `usb_host_ulpi` whole is 1299
LUT4 and 542 flip-flops; the proxy takes `usb_ulpi_host_link` and
`usb_host_sie` out of it and leaves the enumerator behind, because in a
proxy the **PC** must be the thing that enumerates the device or there are
two authorities assigning addresses. `ip/usb/usb_proxy/README.md` §2 is that
decision and what was weighed against it; the short of it is that
pass-through addressing has no translation table, no descriptor cache and no
way for the two sides to disagree about `bMaxPacketSize0`, and the
enumerator would have been the thing it had to disagree with.

**The ten distributed RAMs** are the two of the hub's status-change endpoint
plus eight for the relay's one 64-byte packet buffer. One buffer and not
two, because a transaction moves bytes one way at a time: an IN fills it
from the device and drains it to the PC, an OUT the other way round, and the
two can never be active in the same cycle. It is written with **one write
port** fed by a decode rather than two write statements, for the same reason
`usb_bulk_ep` keeps its two buffers separate — a memory written from two
places is one a backend has to take apart again.

It is also the first block here to need something *back* from a block below
it. `ip/usb/usb_hub`'s port reset used to complete in the cycle it was asked
for, with PORT_RESET in `wPortStatus` a constant zero, because there was
nothing downstream to reset. A proxy has to reset the real device —
pass-through addressing depends on the device forgetting its address exactly
when the PC thinks it has — so that reset became a handshake: `port_reset`
is a level the hub holds while resetting and `port_reset_done` is one cycle
from whatever drove it. **Tying `port_reset_done` high is the old behaviour
exactly**, which is what a design with nothing downstream wants and what
`testdata/fpga/cynthion/usb_hub_target.v` does.

## The crypto category, and what a constant-time claim is worth

`ip/crypto/` is the eighth category and the first in this library whose
subject makes a claim about an *attacker* rather than about a protocol.
Two blocks are in it, `sha256` and `chacha20`, and what makes them a
category rather than two more blocks is a doctrine they are built to and
must be read with.

The doctrine comes from `purecrypto`, the user's from-scratch Rust
cryptography library, whose foundation is stated in its `ct` module:
"every operation here runs in time independent of the secret values it
touches, so higher layers can be built without secret-dependent branches
or memory accesses." In hardware that becomes three obligations, and the
first two are met here.

**Fixed latency.** The cycle count depends on the message *length* and on
nothing in the message or the key. `sha256` is 129 cycles a block whatever
the bytes are; `chacha20_core` is 22 cycles a block whatever the key is.
Both are **measured** rather than argued:
`sha256_takes_the_same_cycles_whatever_the_message_says` runs nine
maximally different bodies at each of six lengths and asserts one cycle
count per length, and `chacha20_takes_the_same_cycles_whatever_the_key_is`
runs nine keys against three nonces and three counters — eighty-one
blocks — and asserts 22 for every one of them. Both also assert that the
outputs were all *different*, because nine equal cycle counts from nine
identical runs would prove nothing.

A cycle count that depends on a *length* is not a leak worth chasing: a
caller streams the bytes in through a handshake, so the byte count is on
the interface whatever the block does, and no block could hide it.

**No secret-dependent addressing.** Neither block contains a memory array
at all, which `crypto_blocks_hold_no_memory_to_index` asserts
structurally, after synthesis, on every variant in the footprint table.
SHA-256's one table — the sixty-four K constants of FIPS 180-4 §4.2.2 — is
a `case` over the round counter and becomes logic; ChaCha20 has no table
of any kind. There is therefore nothing in either block that a secret byte
*could* index.

What that test does not establish is that no *multiplexer* is selected by
a secret, which would be a timing channel in a netlist without being a
memory. Nothing in either block does it — every select here is a
counter — but proving it would take a taint analysis from the key ports
forward, and Reticle has none. That is the gap, named.

**And what is not defended against.** Everything above is about *logical*
time. An FPGA with a perfectly fixed cycle count still leaks through power
consumption and electromagnetic emission, and differential power analysis
is a real, practised attack on exactly these two primitives — the SHA-256
round's additions and ChaCha20's both consume key-dependent power whatever
cycle they happen in. **Nothing in this repository has measured a power
trace or an emission, and nothing here is masked, randomised or
duplicated against one.** No block in `ip/crypto/` should be relied on
where an attacker has physical access to the part, and neither block's
README claims otherwise; each one's §5 says it again at length.

There is one thing a crypto block can do that neither of those is about,
and `chacha20` does it: RFC 8439's block counter is 32 bits, so a (key,
nonce) pair is good for 2^32 blocks. Wrapping it would hand out the same
keystream twice, which ends the confidentiality of both messages, so the
block latches `exhausted`, **stops accepting data**, and makes the caller
supply a new nonce. `chacha20_stops_rather_than_repeat_its_keystream`
starts a stream at counter 0xFFFFFFFE, checks that the two blocks it is
entitled to come out, that the next word is refused, and that the two
blocks differ.

### Where each fact came from

This library's documents separate what was checked from what was quoted,
and a crypto block has an unusually clean separation available: there are
published vectors, and there is a second implementation on this machine.

**Read from a document.** The algorithms, the constants and every
expectation but one. SHA-256 is FIPS 180-4: §4.2.2's sixty-four K
constants, §5.3.3's eight initial values, §4.4 to §4.7's sigma functions,
§5.1.1's padding and §6.2.2's round. The vectors are Appendix B.1
(`"abc"`), B.2 (the 56-byte string — which is **the padding edge**, and
FIPS choosing a 56-byte example is not a coincidence) and B.3 (one million
`'a'`, run as an `#[ignore]`d test). ChaCha20 is RFC 8439, with §2.1.1's
quarter-round vector, §2.2.1's quarter round on a state, §2.3.2's block
function **including the sixteen-word state after twenty rounds and before
the feed-forward addition**, §2.4.2's 114-byte encryption with its
keystream, and Appendix A.1's five blocks and A.2's two encryptions.

§2.3.2's intermediate state is the valuable one, and it is why one test
reads an internal register rather than a port: if the keystream is wrong
and the after-twenty-rounds state is right, the fault is in the addition
or the serialisation; if the intermediate state is wrong, it is in a
quarter round or in how four of them are wired into a column or a
diagonal, and `chacha20_qr`'s own two vectors say which. That is the
difference between localising a fault and knowing that the output is
wrong.

**Confirmed against a running implementation.** `purecrypto` was driven
with the same inputs, out of tree, and compared. It reproduced all four
published SHA-256 digests and all nine published ChaCha20 vectors byte for
byte — so by the time it is used as an oracle it has already agreed with
both authorities everywhere both of them speak. What rests on it alone is
two things:

- the **empty message's** digest, which FIPS 180-4 Appendix B does not
  give. `e3b0c442...` is published widely and derivable from §5.1.1 by
  hand; here it is a value a second implementation computed.
- the nineteen-length padding table in
  `sha256_pads_every_length_purecrypto_was_asked_about`, which is how
  every branch of the padding is covered at all: 55 bytes (no zeros at
  all), 56 (the spill into an extra block), 57 to 63 (the spill with the
  zeros wrapping a block boundary), 0 mod 64 (a whole block of message
  and then a whole block of nothing but padding), and three lengths past
  two blocks. No published document gives nineteen lengths of an
  arbitrary message.

`purecrypto` is **not** a dependency of anything committed: this
repository ships no third-party crates, and the comparison was run in a
throwaway crate outside the tree. What is committed is the numbers it
produced, as a table, which is the same shape as every other golden here.

**Found by a test no document asks for.** Encryption and decryption are
the same operation for a keystream cipher, so putting a ciphertext back
through must give the plaintext. RFC 8439 does not state that as a vector
and does not have to. That round trip found a real defect: `chacha20` asks
for the next keystream block as soon as the current one is drained, so a
stream whose last word fell on a block boundary leaves a request in the
air — and a new `start` arriving then was ignored by `chacha20_core`,
which took `start` only from idle, leaving the wrapper to use a block
computed for the *old* counter as the new stream's first. Every one of the
nine published vectors passed with that bug in place, because every one of
them starts from reset. `chacha20_core` now takes `start` from any state,
and `chacha20` ignores a `valid` that arrives while its own request is
still up.

### And then it went on a part, and said no

**CHECKED.** Everything above was true of a simulator. Both blocks have now run
on a Great Scott Gadgets Cynthion — an ECP5 `LFE5U-12F-8CABGA256` — at 60 MHz,
in `testdata/fpga/cynthion/usb_crypto_console.v`: the two of them behind
`ip/usb/usb_cdc_acm`, so a `/dev/ttyACM*` the kernel's own driver binds answers
typed lines. `ip/crypto/sha256/README.md` §8 is the long account with the
sessions, and `tests/usb_crypto_console.rs` is the test.

**Both cores give wrong answers on it**, and the three things that only a part
could say are these.

**One: a design with both cores does not route.** 11 955 lookup tables, 40 ripup
iterations, an hour and fifty minutes, and 886 nodes still oversubscribed — with
the worst of them a *control wire* the router's own message says the placer
rejects before it is made, so the architecture description is missing a rule.
`WITH_HASH` and `WITH_CIPHER` make each core optional and the single-core builds
route in minutes: 5 891 lookup tables and 8 353 of 8 353 signals for SHA-256 and
8 375 and 10 111 of 10 111 for ChaCha20, both with every set bit decoding back
through the database and nothing unexplained. The bitstreams are not in question.

**Two: the cycle counts are right and the data paths are not.** Over forty runs
of each line on the part, the two commands with no adder in them — a string out
of a 64-entry table and a 256-bit register rotated four bits at a time — were
right **forty times out of forty**. The two with chained 32-bit additions were
wrong every time, and *differently* between identical runs: seven distinct
digests in sixteen runs of the empty message, and two distinct keystreams in
forty runs of `e 10`. One line, one answer, is a property no standard has to
state, and which of these commands stops having it is what separates a
mis-mapped lookup table from a path that does not settle inside 16.67 ns.

Sixty megahertz is the ULPI interface rate, so there was no slower clock to
retreat to. The cause is the paragraph both READMEs' §7 and §8 have always had,
and it has been promoted from a footnote: `src/fpga/trellis` describes `CCU2C`
without a (ci, i0, i1, co) port map, a 32-bit addition becomes a ripple of
LUT4 about twenty-one levels deep, SHA-256's T1 chain is **39** levels where an
iCE40's is 9 and ChaCha20's quarter round is **87** where an iCE40's is 5.
**Inferring the carry cell is not a change that would improve a clock; it is
what stands between every arithmetic block in this library and an ECP5.** It is
a `src/` change and this round reports it rather than making it.

**It has been made since.** `src/fpga/devices/ecp5.dev` gives `CCU2C` a port
map now and `src/fpga/trellis` has 12 144 `carry` sites, so the two numbers
this paragraph turns on are **7 and 4** rather than 39 and 87, and forty-four
ECP5 rows of the footprint table moved with them.
`docs/fpga-trellis.md`'s first section is the account, including what
`ecppack` writes for a `CCU2C` over 2131 instances and which way its chain
runs. The paragraph above is left as it was written because the diagnosis it
makes — two populations of command, one with adders in it and one without —
is the reasoning that identified the cause, and it is the useful part.

**Three: a real throughput figure, and the bottleneck.** These are good although
the answers are not, because a rate is a cycle count and the cycle count is the
half that works. SHA-256 hashing sixteen mebibytes of zeros it makes itself:
**29.63 MB/s, 0.494 bytes per clock** against the **0.496** a four-state
zero-delay simulator computes — four tenths of a per cent over 262 144 blocks,
and the one claim on either page the part *confirmed*. The same block fed over
USB: **132.0 kB/s** of message and **264.0 kB/s** on the wire, which is a factor
of **225** below the core. Half of that factor is the protocol's hexadecimal
framing and the other half is the endpoint — `tests/usb_loopback.rs` times the
bulk loopback of `usb_ulpi_device.v` at 255 500 bytes/s each way at a 64-byte
packet, and 264 kB/s is that ceiling. The two directions differ by 3.8:
1 005 kB/s inbound against 264 outbound, because an OUT endpoint that NAKs while
a core is busy costs the host a transaction per NAK and an IN endpoint does not.

**And one thing it is tempting to read into this and must not be.** None of it
says either block is wrong. Every vector above still passes, the decompositions
are still checked, and the fault is in the path from the blocks' logic to this
part's flip-flops. Nor is any of it a statement about what the blocks **leak**:
a cycle count measured on a part is still a cycle count, nothing has recorded a
power trace or an emission, and the sentence in each README's §5 is untouched —
neither block is for secrecy against an attacker holding the board. What the
round does add to that section is one sharp thing: §5 called a data-path delay
"real and invisible here", and it is now real and *visible*, as a wrong digest.

**What AES should inherit from this half of the round** is an **eighth** item
for the list below. A crypto block in this library gets on a part, behind a host
interface that needs no host software, with its answers compared against
somebody else's implementation rather than a second one of our own — and the
design that does it is three modules, so the parser can be driven without the
USB stack, the USB stack without the board, and the board is a top level with
pads and LEDs. The ninth item is shorter: **ask whether one line gets one
answer before asking whether it is the right one.** It needs no oracle, it is
the first thing a terminal can measure, and on this round it is what turned "the
cores are wrong" into "the adders do not arrive".

### What AES should inherit

AES is deliberately **not** in this round. It is the biggest of the three —
a key schedule, a choice of S-box representation, and modes — and it is
much easier once there is a settled answer to what a crypto block in this
library looks like. This round is that answer, and these are the parts of
it AES should take.

1. **Three levels, smallest first, each a module because a published
   vector addresses it.** `chacha20_qr` exists because RFC 8439 §2.1.1 is
   a test vector for a quarter round; `sha256_core` exists because a
   caller with padded blocks should not pay for a padder. AES's
   equivalents are the S-box, one round, the key schedule and a mode, and
   FIPS 197 publishes intermediate state per round — so the round should
   be reachable from a testbench and those tables asserted against it, the
   way RFC 8439 §2.3.2's are here.

2. **The port width is the width at which the port stops being the
   limit.** `sha256` is 8 bits wide because its compression takes 64
   cycles for 64 bytes, so one byte per cycle is already as fast as the
   core; `chacha20` is 32 bits wide because its core makes 64 bytes in 22
   cycles and a 32-bit port drains them in 16. The rule, not the number,
   is what carries over: AES-128 at one round per cycle is about eleven
   cycles for sixteen bytes, so a 32-bit port (four cycles) is free and a
   128-bit flat port buys nothing but 128 IO buffers in this table.

3. **A byte string has its first byte at the most significant end**, on
   every flat multi-byte port, in both blocks, with no exception — so a
   key copied out of a specification goes straight in. The one port that
   is not a byte string says so: ChaCha20's `counter` is a number, because
   RFC 8439 calls word 12 a block counter and prints it as one.

4. **Never a register wider than the values it holds.** `sha256`'s byte
   counter is 61 bits and not 64, because the length field FIPS 180-4
   appends is a *bit* count whose low three bits are necessarily zero on a
   byte interface — three flip-flops that could only hold zero are three
   flip-flops not declared. The state registers hold three to five values
   and are two or three bits wide. AES's round counter holds 0 to 13.

5. **Fixed latency proved by measurement, in the same shape.** Two tests,
   one per block, each running many maximally different secrets at one
   length and asserting a single cycle count — and asserting the outputs
   differed, so the equality is not the equality of identical runs. AES is
   where this gets *hard* rather than easy: a table-driven S-box in a
   block RAM is exactly the secret-dependent memory access
   `crypto_blocks_hold_no_memory_to_index` forbids, so AES's S-box has to
   be combinational — 256 entries of logic, or the composite-field
   inversion — and that test should be **extended** to cover AES rather
   than relaxed for it. If AES cannot pass it, AES is not ready.

6. **An area/throughput decision stated with numbers, and a trade named
   and declined.** Both blocks here refuse a second 512-bit buffer, and
   each header says what it would have bought (1.98x a block for
   `sha256_core`, 1.77x for `chacha20`) and what it would have cost (512
   flip-flops, more than the rest of the block holds). AES's equivalent is
   one round per cycle against a fully unrolled pipeline, and the numbers
   belong in its header before the code does.

7. **Say what is not defended against, in the README, in a section of its
   own.** AES is the primitive differential power analysis was *developed*
   on. A block that does not say so is worse than one that does not exist.

## The compress category, and a block whose input is somebody else's

`ip/compress/` is the ninth category and it holds one block, `inflate`:
RFC 1951 DEFLATE decompressing, with RFC 1950's zlib framing around it.
[`ip/compress/inflate/README.md`](../ip/compress/inflate/README.md) is
the full account — the five design decisions, the corpus, the error
table, the footprints and the measured throughput. What belongs here is
the part that is about the **library** rather than about DEFLATE.

### Why decompression first

Compression and decompression are not two halves of one block, and the
asymmetry is why this round is only the second half. A decompressor is a
bit reader, a canonical-Huffman walk, a counting sort and a copy engine —
1894 LUT4, 390 flip-flops and eighteen `DP16KD`, sixteen of which are
the window. A compressor needs hash-chain match
finding over the same 32 KiB window, two passes over every block to
choose between the three block types by exact bit cost, and a Huffman
code *builder*, which is the length-limited package-merge problem and a
different and larger piece of work than the sort the decoder needs. It
comes later, and it will go better for this round having settled what the
interface and the window look like.

The second reason is that **its test vectors are infinite and free**.
There is no published table of DEFLATE vectors and there does not need to
be one: compress anything with a compressor, decompress it in hardware,
compare every byte. That is a far better position than `ip/crypto/` is
in, where the published vectors are a dozen fixed strings, and it is
worth saying out loud because the next block in this category inherits
it.

### The oracle, and what it is not

`testdata/ip/inflate_corpus.txt` holds 132 streams that
[`compcol`](https://github.com/KarpelesLab/compcol) — the user's own
from-scratch Rust compression library — produced, with 123 of them in
the tier the gate runs. This is the same method `ip/crypto/sha256` used
with `purecrypto` and for the same reason: **two independent things
agreeing is a measurement where one thing asserting is not.**

Three things about how it was done, because the shape is reusable:

- **Out of tree, read only.** The generator is a program outside this
  repository that takes `compcol` as a path dependency and calls its
  public API. `compcol` *is* already an optional dependency here, behind
  `apicula`, for the xz inside a Gowin chip database — and this round did
  not touch that, did not add a feature to it and did not add a
  dev-dependency. What is committed is what it produced, which is the
  rule `Cargo.toml`'s dependency note and `sha256`'s README both state.
- **The plaintext is a rule, not a file.** Each record names a generator
  rule and a length rather than carrying the bytes, so one case is
  120 000 bytes of text for a kilobyte of committed hex and the whole
  corpus is 188 kB. The cost is that the rule exists twice, and the
  mitigation is the third field: each record carries the Adler-32 of its
  plaintext, and `inflate_plaintext_rules_match_the_corpus` checks all
  132 before any simulation, so a rule that has drifted is reported as a
  rule that has drifted rather than as a broken decompressor.
- **The corpus records what it covers.** Each record also carries the
  BFINAL and BTYPE of its stream's first block, which is as much as can
  be read off a DEFLATE stream without an inflater. That is what lets the
  test *assert* that all three of RFC 1951 §3.2.3's block types are
  present — 24 stored, 84 fixed and 15 dynamic among the 123 — rather
  than assume it. A corpus that quietly lost its stored cases would
  otherwise still pass.

### What the breadth bought, immediately

**MEASURED.** The round's first full corpus run failed, on the fourth
byte of a four-byte stream, with a checksum mismatch — and only under
one of the four consumers.

Each case is run with up to four different `out_ready` patterns, because
a consumer is the only thing that can stall a copy and a copy that
resumes wrongly is the defect a decompressor is most likely to have. The
pattern that drops `out_ready` on every other cycle found that
`inflate_window`'s `rd_valid` was `rd_en` delayed by one cycle, so a copy
stalled for a *single* cycle lost the byte it had already fetched out of
the block RAM. It is a one-deep valid now, cleared by an explicit
`rd_take` input, and the window's own test grew the section that would
have caught it.

That is the same lesson the crypto round learned from the other
direction, where a real defect in `chacha20` passed all nine published
RFC 8439 vectors because every one of them starts from reset:
**published vectors test the function and not the sequencing around it.**
Here there were no published vectors to be lulled by, and what found the
fault was 123 streams times four consumers rather than a better vector.

### A defect found in the oracle

Worth recording because the next round that wants multi-block vectors
will meet it. `compcol`'s `Flush::Sync` is the API for forcing a block
boundary — RFC 1951 §3.2.4's empty stored block — and after one,
`finish()` reports `StreamEnd` having written two bytes, and the
resulting stream decodes to the right bytes but never reaches
`StreamEnd` coming back in: `decompress_to_vec` returns
`UnexpectedEnd`. It reproduces in twenty lines with a 440-byte input and
has nothing to do with this block. Nothing here was changed for it and
nothing in `compcol` was touched; the multi-block coverage `inflate` has
instead comes from the bulk cases, which are several 16 KiB blocks each.

### What the next block in this category should inherit

1. **Decide the framing explicitly and support both.** `inflate` takes
   `WRAPPER`, parses RFC 1950 and does not sniff, because a raw DEFLATE
   stream can begin with bytes that pass RFC 1950 §2.2's check and a
   wrong guess is a wrong decode. The container's own checksum is the
   reason raw has to exist: gzip, zip and PNG all carry a CRC-32 and
   would never want the Adler-32.
2. **A checksum earns its area several times over.** RFC 1950's framing
   is 88 LUT4 and 33 flip-flops, and of 482 single-byte corruptions of a
   real stream, **361 are caught by the checksum and nothing else**. For
   a block whose input is somebody else's bytes, that ratio is the
   argument.
3. **Report, never hang, and make the test bound a cycle count.** Every
   malformed input has a code and a hand-built vector derived from the
   RFC. Every test runs with a *loop bound* in simulated clock edges —
   not a time-out — so reaching it is a failure of the design rather than
   of the host, and 1196 runs across the round reached none.
4. **A module boundary is how a check gets tested.** `inflate_window`
   is a module mostly because no *stream* can ask for distance zero, so
   that arm of its distance check is unreachable from the decoder and
   only a direct testbench can drive it.
5. **Stall rather than buffer, when the asymmetry is large.** A few
   input bits can owe 258 output bytes; a counter and a pointer cost
   nothing against 258 bytes of block RAM, and the buffer would not even
   remove the stall.
6. **Sweep the consumer, not just the input.** Four `out_ready`
   patterns per case found what no vector would have.
7. **Say which rule came from a reference implementation.** Exactly one
   decision in `inflate` is not in either RFC — whether an incomplete
   Huffman code is legal — and it follows `zlib`. The README marks it
   **QUOTED** and says so in one sentence, which is what keeps the rest
   of the page's **HIGH** worth something.

## A block written from nobody's specification

Every other block in this library implements a document: a bus standard,
an ISA, a datasheet, an RFC. **`spi_display_rx` implements an
observation** — four wires on a screen that a user looked at with a logic
analyser — and that makes it a different kind of engineering problem,
worth a section because the method generalises.

The link is conventional once you know what it is: `sclk` in bursts of
exactly eight edges, `mosi` carrying commands and pixels, `dc` saying
which, `cs_n` framing each byte. Receive only. But *knowing what it is*
took three passes, and two of the readings along the way were wrong —
the first had two separate data wires with no D/C at all, the second had
`dc` toggling once per bit.
[`ip/bus/spi_display_rx/README.md`](../ip/bus/spi_display_rx/README.md)
§1 records all three with the user's own words, because the wrong
readings are the reason two of the parameters exist, and a reader who
only sees the final answer cannot tell which parts of it are load
bearing.

**What the block does about uncertainty is the part worth copying.**
Three moves:

1. **Every unknown is a parameter whose right value a measurement will
   choose**, not a guess compiled in. Which `sclk` edge samples, which
   bit arrives first, whether `cs_n` is a level or a pulse, which
   polarity it is, which bit of a byte `dc` tags it with, and whether
   `dc` is required to agree with itself across the byte. The defaults
   are what the observations imply and nothing more.
2. **Every uncertainty that a counter could settle gets a counter.** Six
   of them, and two have an expected reading of **exactly zero** —
   `bit_error_count`, because the eight-edge bursts were confirmed, and
   `dc_change_count`, because `dc` holding for a whole byte was
   confirmed. An instrument expected to read zero is worth far more than
   one expected to read "small": the user's first description said the
   bursts were eight edges "give or take", and a counter built for a
   tolerance would have measured nothing. The third pair,
   `cmd_byte_count` against `data_byte_count`, settles a reading rather
   than a fault — the user saw `dc` looking like the inverse of `cs_n`,
   which is exactly what a pixel-only capture looks like, and that page's
   §3 writes it down so nobody re-derives it.
3. **Where no counter can help, the test says so.** `SAMPLE_EDGE` cannot
   be chosen from inside the block, and
   `spi_display_rx_samples_the_edge_and_the_bit_order_it_is_told_to`
   *proves* it: a falling-edge master read on the rising edge delivers
   every frame at eight bits, with no mismatch, no overrun and a
   perfectly held `dc` — and the wrong bytes. A test that pins a
   limitation is how a limitation stops being a surprise.

**And it does not clock on `sclk`.** An external pin driving a clock
network needs a buffer the pad can reach, which `place::confine_to_reachable`
now enforces, and a block clocked on `sclk` crosses a domain on the way
out anyway. So all four pins go through their own `cdc_sync` and the
whole block is oversampled in the system clock domain —
`spi_display_rx_is_one_clock_domain` asks `timing::analyze_cdc` and gets
one domain and no crossing, which is the claim stated as a test rather
than as a sentence. The price is a rate limit, and the block states it
**as a ratio of clocks rather than as a frequency**, because the screen
will be connected on hardware nobody here has: an `sclk` period must last
at least `2 * PHASE_MARGIN` system clocks — four at the default, so
`sclk` may be up to a quarter of the system clock — and the fourth wire
must be deasserted for `PHASE_MARGIN` of them, since the frame is
edge-detected the same way. A frequency is a fact about our board; a
ratio is a fact anyone can check against theirs, and the 60 MHz figure
(15 MHz) is one row of a table rather than the statement. Both halves are
pinned by tests: at the limit, a shade past it, and with the parameter
moved against a fixed waveform, which is the same experiment as moving
the clock against a fixed link. Past the limit the failure is a
**reported** overrun and then a bit count below eight, never a quiet
byte.

**Nothing in it has been near a part, and that is deliberate rather than
pending.** The screen is on another machine and will be connected by
someone else, so there is no top level under `testdata/fpga/` and no
constraints file: a pin assignment made here would be a guess about
hardware nobody here can see. What that leaves unknown is §10 of the
block's README, written in the terms
[`ip/crypto/sha256/README.md`](../ip/crypto/sha256/README.md) §8 earned —
a block that passed published vectors, a second implementation, mapped
equivalence at two widths and a constant-time measurement, and then
computed wrong digests on silicon. A block verified only in simulation
should say so where a reader cannot miss it, and the counters are the
reason that is survivable here: whoever connects the screen can run the
measurement themselves, and §4 of that README is written for them.

## Using one

A block is an ordinary IP package, so a project reaches it with a
`depends` line and nothing else:

```text
# reticle.proj
name blinky
top top
device ice40-hx1k-tq144

source rtl/top.v

library ../reticle/ip

depends uart      ^1.0.0
depends fifo_sync ^1.0.0
```

The `library` line says where this tree is, once. The two `depends` lines
then name only what they want, and a block is found by the **name its own
`reticle.ip` declares** rather than by its directory, so where a block
sits under `ip/` is `ip/`'s business and not the project's. A package from
somewhere else is still named with `path <dir>`, and
`depends uart ^1.0.0 path ../reticle/ip/bus/uart` still works unchanged for a
project that wants to pin one directory.

`cdc_sync` is not named there and does not have to be: `axil_gpio`,
`cdc_pulse` and `fifo_async` declare it themselves, and the same library
search places it. `docs/ip.md` has the whole resolution story, including
what the lock file records (`package uart 1.1.0 library ../reticle/ip/bus/uart`
— the answer the search gave, so that a block which has moved is
something `reticle build --locked` reports rather than something a build
silently follows).

`tests/ip_library.rs` indexes this tree and checks it is indexable: no two
packages claim one name, every manifest declares one, and every block's
`depends` names a package the library has. The same test
(`every_block_is_findable_by_the_name_it_declares`) holds the layout
above — three path components, a known category in the middle, and the
last one the package's own name — because nothing in the resolver
requires any of it and a document is not a check.

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
| `manifests_parse` | every `reticle.ip` parses, is named for the folder it sits in, lists files that exist and round-trips through `IpManifest::to_text` |
| `packages_resolve_and_elaborate` | every block builds through `ip::resolve` and `ip::elaborate` from a generated project that declares a `library` root and names no path, dependencies included |
| `blocks_synthesise_cleanly` | `synth::run` reports nothing at all — no error, no warning, and no inferred latch |
| `footprints_match_the_documentation` | the table below is the one this run measured |
| `axil_gpio_matches_the_axi4lite_definition` | `bus::match_ports` finds all nineteen AXI4-Lite signals on the GPIO at the widths its parameters imply |
| `crypto_blocks_hold_no_memory_to_index` | neither `sha256` nor `chacha20` contains a memory array at any level, so no table in either can be addressed by a secret |
| `inflate_plaintext_rules_match_the_corpus` | the eight plaintext rules `testdata/ip/inflate_corpus.txt` names still produce what the corpus was generated from, checked by Adler-32 before anything simulates |

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
  CLK_DIV rather than stopping the port. All five of those tests predate
  the character format and **none was edited** when it arrived, which is
  the sharpest statement that 8N1 still means what it meant. Beside them,
  `rx_error` through the 8N1 wrapper is checked to be the disjunction of
  the three signals that now say *which*.
- **`uart_frame`, against something that is not `uart_rx`** — the forty
  character formats (four data widths x five parity modes x one or two
  stop bits), three times over: the transmitter's line decoded **by hand
  in Rust**, the receiver fed from levels a **hand-written encoder**
  produced, and the two wired to each other. Then each of the four
  receive errors deliberately: a stop bit held low, a data bit inverted
  under a parity that depends on the data and the parity bit inverted
  under one that does not, the line held low for three frame periods, and
  `rx_ready` held low across two characters. Each raises its own signal,
  and each is followed by a good character that the receiver still takes.

  **Why a hand decoder, and not `uart_rx`.** Three mutations were run
  against the finished RTL to find out. Swapping odd and even parity in
  the transmitter is caught by the hand decoder *and* by the loopback,
  because a receiver checks parity. Forcing `two_stop` low, so that every
  format sends one stop bit, is caught by the hand decoder and **the
  loopback passes it** — and not by luck: a receiver samples the first
  stop bit and nothing after it, so no receiver anywhere could catch it.
  Only something that counts bit periods between start edges can. That
  one row is the whole case for `ip/bus/uart/README.md` §7's approach, and
  it is the same reason `examples/mos6502_computer/tb/computer_tb.v` and
  `examples/soc/tb/soc_tb.v` decode their lines by hand in Verilog.
- **`uart_line_coding` and `uart_frame` together** — the second of the two
  loops that join a USB host to a waveform. Every value PSTN 1.2 defines
  in `bCharFormat`, `bParityType` and `bDataBits`, plus an undefined one
  in each, against the mapping tables in the block's README; and then the
  three numbers the decode produced are driven onto `uart_frame`'s format
  ports and the frame on `tx` is decoded by hand against what the host
  asked for. Nothing is spelled twice — the format the decoder expects is
  derived from what the hardware produced — so a decode and a transmitter
  that disagreed would fail. The substitutions are in it: a host asking
  for one and a half stop bits gets a frame of two, and one asking for
  sixteen data bits gets eight, each with `ok` low.
- **`uart`'s two handshakes, in
  `a_streams_ready_is_a_function_of_registers`** — the rule
  `ip/crypto/chacha20` established and `ip/compress/inflate` turned into a
  test, applied to a UART because a UART is exactly the block somebody
  puts two of back to back. `tx_ready`, `rx_valid`, `rx_error` and the
  four separate error flags are each walked backwards through the timing
  graph, and no input port is reachable from any of them. The one that had
  to be *designed* for it is `rx_overrun`: the obvious spelling is
  `assign rx_overrun = rx_valid & ~rx_ready`, and writing it that way
  fails the test with ``uart.uart_frame: `rx_overrun` depends
  combinationally on {"rx_ready"}``, which is how the assertion was
  checked rather than assumed. `uart_line_coding` is in the same test with
  the assertion **inverted**: it is a wholly combinational decode, so an
  empty set there would mean it had stopped decoding anything.
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
  against a register: 215 `LUT4` at depth **8**. So the run-time divisor
  costs about 95 lookup tables and two levels of depth, and the magnitude
  comparison would have bought eleven more levels and nothing else — a
  rate that changes mid-character costs that character either way.
  Latching also gives the better semantics: a character in flight keeps
  the rate it started at, so a rate change can never corrupt a byte.

  Those three numbers are from **before** the character format arrived
  and are kept because they are the account of why `div` is latched. The
  table below now reads **237** `LUT4` for 8N1 at the same depth of 8,
  and 303 for `uart_frame` with the format on ports;
  `ip/bus/uart/README.md` §4 says where the 24 went and reports the one
  place the same "a constant does not fold through a flip-flop" effect
  cost 38 more until the receive format was *un*latched.
- **`spi_master`** — modes 0 and 3, with a slave model that samples
  `mosi` on the rising edge and presents `miso` on the falling one, so
  the bits are checked where a real slave would look at them; `cs_n` is
  checked to fall before the first edge and rise after the last.
- **`spi_display_rx`** — fifteen testbenches against a model of the
  display's own master, driven in **absolute simulation time** with every
  far-side event on an odd tick, so no pin ever changes in the same
  instant as the system clock edge that samples it. A plausible session
  — six commands then sixty-four pixels — with every byte and every D/C
  tag checked; a pixel-only run, which is what the user's capture
  actually was; frames of **seven and nine** bits and no chip select at
  all, each in all three framing modes; a reset landing in the middle of
  a byte; `mosi` and `dc` driven three system clocks apart; a `dc` that
  carries the data rather than the tag, in each of the three D/C
  settings; an `sclk` at the rate limit, a shade over it and well past
  it; `PHASE_MARGIN` moved against a fixed waveform, which is the same
  experiment as moving the system clock against a fixed link; a gap of
  two system clocks and of one; a gap of **half** a system clock, which
  falls entirely between two sampling edges and so is the one the overrun
  counter cannot see at all; a boundary pulse instead of a level; and
  both chip select polarities. Its outputs are also walked backwards
  through the timing graph by
  `a_streams_ready_is_a_function_of_registers`, because a block whose
  inputs are asynchronous pins must not put one of them into a consumer's
  combinational logic. Three of those tests exist to pin what the
  block **cannot** tell you — the sampling edge, the bit order, and
  whether a pulse and a level can be told apart at all — and one asks
  `timing::analyze_cdc` to confirm there is only one clock in it.
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
  truncates rather than rounds. `ip/video/vga_out/README.md` says why, and what
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
  against the full-speed core, the full-speed core with the host's clock 0.4
  % slow and 0.4 % fast, the ULPI core, and the ULPI core behind the
  transceiver that reports LineState a clock late. A full **64-byte**
  packet, one a byte short of full, an eight-byte one, a five-byte one and a
  one-byte one go out and come back, each read before the next is sent, with
  `FF` and `07` in the payload so the bit stuffing is exercised inside a
  data packet and not only inside a descriptor. The 63-byte one is there
  because it is the only length at which a packet fills the buffer and does
  not start at the bottom of it, which is what a base counter off by one
  gets wrong. The bytes are checked at the byte interface as well as at the
  host, packet by packet, with `out_last` where the host put the end of each
  one, and the pair is in **loopback** — `out_*` wired into `in_*` — which
  is the wiring the board has rather than a testbench's private arrangement.

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
  length that fills a buffer and, in the shift-register shape, does not
  start at the bottom of it. What **none** of them reaches is the shape as
  it is built for a device: the array becomes a memory cell, and a memory
  cell is the backend's to lower —
  `the_logic_fallback_answers_like_the_memory_it_replaced` in
  `tests/fpga_flow.rs` is what answers for that, and a board is what answers
  for the silicon. `BUF_RAM = 0` gets `bulk_loopback` too, in
  `usb_device_fs_loops_bytes_with_the_buffers_as_shift_registers`, because
  the two shapes share all four of their counters and differ in a
  subtraction — the shift register reads at `index - written` where the
  array reads at `index`, and an off-by-one there is what the 63-byte packet
  is for. Its other rules, the toggle and the NAKs and the zero-length
  packet, are not re-run on that shape: none of them touches a buffer's
  addressing.

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
  [`ip/usb/usb_device_ulpi/README.md`](../ip/usb/usb_device_ulpi/README.md) §11
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

  **It has run on a board**, and this paragraph used to say it had not. The
  reason it could not was that the Cynthion's TARGET transceiver is on the
  **left** edge of the die and this backend described the top and right
  edges only — so a design on those balls was refused by name, and the three
  bidirectional VBUS switches onto the TARGET A node are on that same edge,
  which meant no design this flow could build could put power on that
  socket. All four edges are described now;
  [`docs/fpga-trellis.md`](fpga-trellis.md) has how the left one was
  established and why a mirror of the right edge would have been wrong.
  [`ip/usb/usb_host_ulpi/README.md`](../ip/usb/usb_host_ulpi/README.md) §9
  is what the part then said: nine registers of a real USB3343 read over
  sixteen left-edge balls, with `0424` — Microchip's vendor ID — among them.
  A device in that socket has since been enumerated through that same
  transceiver, which is `usb_proxy` below.

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

  What none of it could reach is whether a **real driver binds**, and that
  is the interesting half of a class layer. `tests/usb_cdc_acm.rs` is that
  half: it asks the kernel, through sysfs, whether `cdc_acm` claimed the
  device and made a terminal, and then writes bytes to that terminal and
  reads them back.
  [`ip/usb/usb_cdc_acm/README.md`](../ip/usb/usb_cdc_acm/README.md) §5 is
  what a host said, quoted — including the two faults that were invisible in
  simulation and what each of them was.

- **`usb_proxy`** — **three** ends on **two** pairs, which no other test
  here needs: a host model driving the proxy's upstream transceiver, the
  proxy's own downstream transceiver, and a whole second design —
  `usb_device_ulpi` — behind a third. The downstream pair is resolved the
  way `usb_host_ulpi`'s one is and the upstream pair by the host model, and
  `ProxyRig` in `tests/ip_library.rs` is the harness for it. Eight tests,
  and the shape of each is a **PC enumerating a device it can only reach
  through our hub**: the hub enumerated and its port powered and reset, and
  then the device's own eighteen-byte device descriptor and thirty-two-byte
  configuration descriptor read **through** the proxy and compared with the
  same `expected_*` functions the device's own tests use, the address the
  host model chose read back off the **device's** own `address` output, and
  its `configured` output after the host model's SET_CONFIGURATION.

  Three of the eight reach what the first cannot. One runs the whole of it
  through three transceiver models that each hear their own transmission and
  each report `LineState` a clock late, which matters more here than
  anywhere else because `LineState` is what decides when the downstream port
  is a port at all. One runs it against a device built with `MAXPKT0 = 8` —
  the smallest USB 2.0 §5.5.3 allows — so a data stage is three and four
  packets instead of one, and the four-packet one is **exactly** the length
  asked for, with no short packet to stop on. And one moves bytes: four bulk
  packets of four lengths, including a one-byte short one and a full
  sixty-four, through the device's own loopback and back, with **eight
  downstream transactions counted** — one per packet each way, which is the
  number that distinguishes bytes the device sent from bytes a relay held
  and handed over twice.

  The other four are the properties that are easy to get silently wrong: a
  STALL the device really sent — a string descriptor it has not got —
  arriving at the host as a STALL and staying sticky until the next SETUP; a
  port the host has not reset forwarding **nothing at all**, not even a NAK,
  and a second reset putting the device's own address back to 0; a SETUP
  preempting a transaction that is still running, which USB 2.0 §8.5.3 makes
  compulsory and which that test's own comment says it would **not** have
  caught the defect it accompanies; and one clock domain across a design
  with two ULPI buses in it.

  What none of it could reach is whether a **kernel** enumerates the device,
  and [`ip/usb/usb_proxy/README.md`](../ip/usb/usb_proxy/README.md) §8 is
  that: the same `dmesg` buffer with `unable to enumerate USB device` before
  and `idVendor=1d50, idProduct=60e6` after. `tests/usb_proxy.rs` is the
  test, and its load-bearing assertion is one boolean — a child of our hub
  exists in sysfs.

- **`sha256`** — the three messages FIPS 180-4 Appendix B works through,
  and the empty one it does not. B.1 is `"abc"`; B.2 is the 56-byte string
  and so is the **padding edge**, where the eight-byte length field does
  not fit and the padding spills into a second block; B.3's million
  characters are an `#[ignore]`d test because they are two million clock
  edges. Then `sha256_core` on its own, fed a block the test padded by
  hand from §5.1.1, which is what makes "the padding is the wrapper's job"
  a decomposition rather than a claim. Then nineteen message lengths
  around every boundary the padding has, with the digests from
  `purecrypto`. And then the cycle count: nine maximally different bodies
  at each of six lengths, one cycle count per length.
- **`chacha20`** — four of RFC 8439's sections and both of its appendices.
  §2.1.1 and §2.2.1 go straight into `chacha20_qr`'s ports, because a
  quarter round is a module here precisely so that a vector can address
  it. §2.3.2 is checked **twice over**: the serialised block on the port,
  and the sixteen-word state after twenty rounds and before the
  feed-forward addition, read off the working register — which is the
  difference between knowing a keystream is wrong and knowing which
  quarter round is. Appendix A.1's five blocks cover the counter and nonce
  positions §2.3.2 cannot. §2.4.2's 114 bytes and Appendix A.2's 375 and
  127 exercise the stream across block boundaries, with the keystream
  checked as well as the ciphertext. Eighty-one key, nonce and counter
  combinations give eighty-one different blocks at 22 cycles each. And a
  stream started at counter 0xFFFFFFFE gets the two blocks it is owed and
  is then refused, with `exhausted` up.
- **`inflate`** — 132 streams a second, independent compression library
  produced, and no published vectors at all, because DEFLATE has none and
  does not need any. 123 of them run in the gate, each under up to four
  different `out_ready` patterns, and every byte of all 228 272 is
  compared; the other nine are hundreds of thousands of cycles each and
  are the only ones long enough to make the 32 KiB window wrap, so they
  are `#[ignore]`d. Then the half no compressor can produce: 21 streams
  hand built from RFC 1951 and RFC 1950 with a bit writer, one for each
  way a stream can be malformed and all seven error codes covered; 482
  single-byte corruptions of three real streams, of which 479 are
  reported and 3 are harmless padding flips and **none** decodes to the
  wrong bytes and says `done`; and 235 truncations, every proper prefix
  of three streams, all of them reported. `inflate_window` and
  `inflate_adler` are driven on their own as well, the first because no
  stream can ask it for distance zero and only a direct testbench can.
  Every one of the 1196 runs carries a **loop bound in simulated clock
  edges**, so a stream that could make this block wait forever fails the
  test rather than hanging it, and none of them reached it.

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

**The two crypto blocks are where `LUT depth` and the carry chain part
company, and the numbers say which backend infers one.** A ChaCha20
quarter round is four 32-bit additions in series and `chacha20_core`
computes a whole round in a cycle, so its critical path is those four
adders. On the generic LUT4 and LUT6 mappings that is **depth 87**,
because neither emits a carry cell and a 32-bit ripple-carry add is
twenty-odd levels of logic; on the iCE40 it is **depth 8**, because
`SB_CARRY` is inferred and a 32-bit add is one carry chain.

**The ECP5 was 87 too, and it is 4.** That row used to say what this
paragraph said next — that `src/fpga/trellis` inferred no `CCU2C` and
every arithmetic block in the table paid for it — and the gap is closed.
**Forty-four ECP5 rows moved**:

| Block | Before | After |
|---|---|---|
| `chacha20_core` | 5834 LUT4, depth **87** | 547 CCU2C + 3033 LUT4, depth **4** |
| `sha256_core` | 3041 LUT4, depth **39** | 314 CCU2C + 1688 LUT4, depth **7** |
| `inflate_adler` | 152 LUT4, depth **26** | 18 CCU2C + 91 LUT4, depth **6** |
| `inflate` (`WRAPPER=1`) | 1894 LUT4, depth **33** | 143 CCU2C + 1618 LUT4, depth **19** |
| `rv32i` (`REGFILE_BRAM=0`) | 2487 LUT4, depth **34** | 117 CCU2C + 2080 LUT4, depth **33** |
| `mos6502` (`DECIMAL_MODE=1`) | 1720 LUT4, depth **16** | 119 CCU2C + 1596 LUT4, depth **16** |

**And thirty-five iCE40 rows moved by one or two cells each, with no
depth anywhere.** That is a second thing the same round found, and it is
worth a paragraph because it is a *correctness* fix rather than a cost.
`cnt + 1` is an adder whose second operand is a constant, so most of its
`b` pins are a constant **zero** — and a constant on a carry element's
operand pin is not a wire anything routes. On an ECP5 an unrouted slice
input reads as a **one**, so the chain computed `cnt - 1`: the design
placed, routed and decoded perfectly and
`testdata/fpga/cynthion/usb_ulpi_device.v` stopped enumerating on a
Cynthion. `techcells::drive_constant_data` gives those pins a real
driver now, on every family that has a carry element, which costs the
one or two shared constant lookup tables these rows gained — and on the
iCE40 it is the same latent defect being closed before anything runs
`SB_CARRY` on a part. `docs/fpga-trellis.md` has how the board found it.

The first four are the point and the last two are the honest limit. The
adder-bound blocks' depth collapses, and the ECP5 now comes out
**shallower than the iCE40** on every one of them — 4 against 8, 7
against 9, 6 against 20, 19 against 31 — because a `CCU2C` computes its
own sums where an `SB_CARRY` needs an XOR pair per bit outside it, and
those XORs are LUT cells the mapper covers and counts.

`inflate_adler` is the smallest clean measurement of the whole change,
which is why it is in this list although it is six lines of RTL: two
chained 17-bit modular sums, nothing else, **26 levels before and 6
after**. It was the one block in the library where the ECP5 was deeper
than the *generic LUT6 mapping* as well as deeper than the iCE40, and it
is now shallower than both.

`rv32i` and `mos6502` are not adder-bound: their critical paths are
instruction decode and addressing, so they buy area (407 and 124 fewer
LUT4) and a level at most. A depth figure is not a timing figure, and the
blocks whose depth did not move are the ones that say so.

Both blocks are also near the top of this table by area, and the reason is
the same arithmetic. `chacha20_core` is 5834 LUT4 on the generic
mapping — the largest single module in the library — of which sixteen
32-bit adders in the quarter rounds and sixteen more in the feed-forward
addition are most of it. That is the price of one round per cycle, and
`chacha20_qr`'s own row is what makes it checkable: 573 LUT4 for one
quarter round, four of them in a round, and the rest is the
column/diagonal muxing and the final add. On the ECP5 that quarter round
is 68 `CCU2C` and 128 `LUT4` at **depth 1**: four 32-bit adds, the
rotations that are wiring, and the XORs — and nothing else, because the
adds cost no lookup table at all. (Its operands are all ports, so it is
also the one crypto row that gained no constant driver.)

Every port of these blocks takes an IO buffer as usual, which is why
`chacha20_core` shows 902 `SB_IO` — a 256-bit key, a 96-bit nonce, a
32-bit counter and a 512-bit block output are 896 of them. Dropped into a
design the buffers disappear and the adders do not.

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

**`spi_display_rx` is the row where the instrumentation is most of the
block**, and the two rows say so. On the ECP5 it is 209 LUT4 and 141
flip-flops, of which **96 are the six counters** — sixteen bits each at
the default `COUNT_WIDTH`, about seventy per cent of the block's storage
— and 93 of the iCE40's `SB_CARRY` are their incrementers and saturation
compares. The receiver proper is an eight-bit shift register, a four-bit
bit counter, four two-flop synchronisers and the edge detection: under
forty-five flip-flops. So `COUNT_WIDTH = 8` roughly halves the block, and
`COUNT_WIDTH = 1` turns every counter into a sticky flag for a design
that only wants the three error wires. `PHASE_MARGIN` costs almost
nothing by comparison: its two hold counters are
`$clog2(PHASE_MARGIN + 1)` bits each, two at the default. The pair of rows also prices the
framing: `FRAME_MODE = 2`, which ignores the chip select and counts eight
bits for itself, is 11 LUT4 and 2 flip-flops smaller — which is what
"keep the other two modes, they are cheap" meant as a number.

Its 124 IO buffers are, as everywhere in this table, an artefact of
measuring a block as its own top: six inputs, twenty-two bits of byte,
tag and status, and 96 bits of counter all take a pad. Dropped into a
design they disappear and the counters do not.

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
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 18 x SB_DFFER, 1 x SB_GB, 27 x SB_IO, 292 x SB_LUT4 | 4 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | ECP5 45F | 6 x CCU2C, 1 x DCCA, 17 x LUT4, 2 x TRELLIS_DPR16X4, 18 x TRELLIS_FF, 27 x TRELLIS_IO | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT4 | 2 x dff, 27 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT6 | 2 x dff, 22 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 2 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 10 x SB_DFFER, 1 x SB_GB, 27 x SB_IO, 292 x SB_LUT4 | 4 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | ECP5 45F | 6 x CCU2C, 1 x DCCA, 17 x LUT4, 2 x TRELLIS_DPR16X4, 10 x TRELLIS_FF, 27 x TRELLIS_IO | 3 |
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
| `uart` | `uart` | CLK_DIV=104 | LUT4 | 18 x dff, 237 x lut | 8 |
| `uart` | `uart` | CLK_DIV=104 | LUT6 | 18 x dff, 216 x lut | 6 |
| `uart` | `uart` | CLK_DIV=104 | iCE40 HX1K | 33 x SB_CARRY, 80 x SB_DFFER, 26 x SB_DFFES, 17 x SB_DFFR, 2 x SB_DFFS, 1 x SB_GB, 43 x SB_IO, 274 x SB_LUT4 | 8 |
| `uart` | `uart` | CLK_DIV=104 | ECP5 45F | 1 x DCCA, 240 x LUT4, 125 x TRELLIS_FF, 43 x TRELLIS_IO | 8 |
| `uart` | `uart_baud_div` | (defaults) | LUT4 | 9 x dff, 282 x lut | 23 |
| `uart` | `uart_baud_div` | (defaults) | LUT6 | 9 x dff, 235 x lut | 17 |
| `uart` | `uart_baud_div` | (defaults) | iCE40 HX1K | 35 x SB_CARRY, 181 x SB_DFFER, 3 x SB_DFFES, 1 x SB_GB, 52 x SB_IO, 263 x SB_LUT4 | 23 |
| `uart` | `uart_baud_div` | (defaults) | ECP5 45F | 1 x DCCA, 282 x LUT4, 184 x TRELLIS_FF, 52 x TRELLIS_IO | 23 |
| `uart` | `uart_frame` | CLK_DIV=104 | LUT4 | 21 x dff, 303 x lut | 8 |
| `uart` | `uart_frame` | CLK_DIV=104 | LUT6 | 21 x dff, 254 x lut | 6 |
| `uart` | `uart_frame` | CLK_DIV=104 | iCE40 HX1K | 42 x SB_CARRY, 83 x SB_DFFER, 28 x SB_DFFES, 18 x SB_DFFR, 2 x SB_DFFS, 1 x SB_GB, 53 x SB_IO, 346 x SB_LUT4 | 8 |
| `uart` | `uart_frame` | CLK_DIV=104 | ECP5 45F | 1 x DCCA, 305 x LUT4, 131 x TRELLIS_FF, 53 x TRELLIS_IO | 8 |
| `uart` | `uart_line_coding` | (defaults) | LUT4 | 18 x lut | 3 |
| `uart` | `uart_line_coding` | (defaults) | LUT6 | 15 x lut | 3 |
| `uart` | `uart_line_coding` | (defaults) | iCE40 HX1K | 34 x SB_IO, 18 x SB_LUT4 | 3 |
| `uart` | `uart_line_coding` | (defaults) | ECP5 45F | 18 x LUT4, 34 x TRELLIS_IO | 3 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT4 | 10 x dff, 87 x lut | 6 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT6 | 10 x dff, 80 x lut | 4 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | iCE40 HX1K | 22 x SB_CARRY, 35 x SB_DFFER, 1 x SB_DFFES, 17 x SB_DFFR, 1 x SB_GB, 25 x SB_IO, 74 x SB_LUT4 | 3 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | ECP5 45F | 14 x CCU2C, 1 x DCCA, 74 x LUT4, 53 x TRELLIS_FF, 25 x TRELLIS_IO | 3 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT4 | 14 x dff, 116 x lut | 7 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT6 | 14 x dff, 90 x lut | 4 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | iCE40 HX1K | 18 x SB_CARRY, 20 x SB_DFFER, 4 x SB_DFFES, 17 x SB_DFFR, 1 x SB_GB, 30 x SB_IO, 117 x SB_LUT4 | 3 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | ECP5 45F | 1 x DCCA, 116 x LUT4, 41 x TRELLIS_FF, 30 x TRELLIS_IO | 7 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=0 | LUT4 | 29 x dff, 208 x lut | 5 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=0 | LUT6 | 29 x dff, 180 x lut | 4 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=0 | iCE40 HX1K | 93 x SB_CARRY, 113 x SB_DFFER, 2 x SB_DFFES, 26 x SB_DFFR, 1 x SB_GB, 124 x SB_IO, 194 x SB_LUT4 | 4 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=0 | ECP5 45F | 1 x DCCA, 209 x LUT4, 141 x TRELLIS_FF, 124 x TRELLIS_IO | 5 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=2 | LUT4 | 27 x dff, 198 x lut | 5 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=2 | LUT6 | 27 x dff, 174 x lut | 3 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=2 | iCE40 HX1K | 93 x SB_CARRY, 111 x SB_DFFER, 2 x SB_DFFES, 26 x SB_DFFR, 1 x SB_GB, 124 x SB_IO, 192 x SB_LUT4 | 5 |
| `spi_display_rx` | `spi_display_rx` | FRAME_MODE=2 | ECP5 45F | 1 x DCCA, 198 x LUT4, 139 x TRELLIS_FF, 124 x TRELLIS_IO | 5 |
| `pwm` | `pwm` | WIDTH=8 | LUT4 | 2 x dff, 23 x lut | 6 |
| `pwm` | `pwm` | WIDTH=8 | LUT6 | 2 x dff, 17 x lut | 4 |
| `pwm` | `pwm` | WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 8 x SB_DFFER, 8 x SB_DFFR, 1 x SB_GB, 21 x SB_IO, 23 x SB_LUT4 | 6 |
| `pwm` | `pwm` | WIDTH=8 | ECP5 45F | 5 x CCU2C, 1 x DCCA, 22 x LUT4, 16 x TRELLIS_FF, 21 x TRELLIS_IO | 6 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT4 | 4 x dff, 61 x lut | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT6 | 4 x dff, 51 x lut | 4 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 17 x SB_DFFER, 9 x SB_DFFR, 1 x SB_GB, 47 x SB_IO, 58 x SB_LUT4 | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | ECP5 45F | 5 x CCU2C, 1 x DCCA, 57 x LUT4, 26 x TRELLIS_FF, 47 x TRELLIS_IO | 5 |
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
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | iCE40 HX1K | 220 x SB_CARRY, 1024 x SB_DFFE, 292 x SB_DFFER, 66 x SB_DFFR, 1 x SB_GB, 208 x SB_IO, 5049 x SB_LUT4 | 36 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | ECP5 45F | 117 x CCU2C, 1 x DCCA, 2080 x LUT4, 32 x TRELLIS_DPR16X4, 358 x TRELLIS_FF, 208 x TRELLIS_IO | 33 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT4 | 16 x dff, 2445 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT6 | 16 x dff, 2182 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 28 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | iCE40 HX1K | 220 x SB_CARRY, 2 x SB_DFFE, 292 x SB_DFFER, 66 x SB_DFFR, 1 x SB_GB, 208 x SB_IO, 2357 x SB_LUT4, 4 x SB_RAM40_4K | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | ECP5 45F | 117 x CCU2C, 1 x DCCA, 4 x DP16KD, 2048 x LUT4, 360 x TRELLIS_FF, 208 x TRELLIS_IO | 33 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | LUT4 | 25 x dff, 1710 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | LUT6 | 25 x dff, 1320 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | iCE40 HX1K | 179 x SB_CARRY, 132 x SB_DFFER, 4 x SB_DFFES, 3 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 1735 x SB_LUT4 | 16 |
| `mos6502` | `mos6502` | DECIMAL_MODE=1 | ECP5 45F | 119 x CCU2C, 1 x DCCA, 1596 x LUT4, 139 x TRELLIS_FF, 57 x TRELLIS_IO | 16 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | LUT4 | 25 x dff, 1646 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | LUT6 | 25 x dff, 1286 x lut | 18 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | iCE40 HX1K | 125 x SB_CARRY, 132 x SB_DFFER, 4 x SB_DFFES, 3 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 1643 x SB_LUT4 | 16 |
| `mos6502` | `mos6502` | DECIMAL_MODE=0 | ECP5 45F | 83 x CCU2C, 1 x DCCA, 1543 x LUT4, 139 x TRELLIS_FF, 57 x TRELLIS_IO | 16 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT4 | 26 x dff, 305 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT6 | 26 x dff, 283 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | iCE40 HX1K | 10 x SB_CARRY, 127 x SB_DFFER, 64 x SB_DFFES, 3 x SB_DFFR, 1 x SB_GB, 34 x SB_IO, 303 x SB_LUT4 | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | ECP5 45F | 8 x CCU2C, 1 x DCCA, 302 x LUT4, 194 x TRELLIS_FF, 34 x TRELLIS_IO | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT4 | 9 x dff, 177 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT6 | 9 x dff, 166 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | iCE40 HX1K | 7 x SB_CARRY, 97 x SB_DFFER, 3 x SB_DFFES, 8 x SB_DFFR, 1 x SB_GB, 140 x SB_IO, 174 x SB_LUT4 | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | ECP5 45F | 5 x CCU2C, 1 x DCCA, 173 x LUT4, 108 x TRELLIS_FF, 140 x TRELLIS_IO | 4 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | LUT4 | 36 x dff, 429 x lut | 10 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | LUT6 | 36 x dff, 348 x lut | 9 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | iCE40 HX1K | 24 x SB_CARRY, 180 x SB_DFFER, 3 x SB_DFFR, 36 x SB_DFFS, 1 x SB_GB, 121 x SB_IO, 420 x SB_LUT4 | 10 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | ECP5 45F | 16 x CCU2C, 1 x DCCA, 418 x LUT4, 1 x ODDRX1F, 219 x TRELLIS_FF, 121 x TRELLIS_IO | 10 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | LUT4 | 26 x dff, 214 x lut | 7 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | LUT6 | 26 x dff, 187 x lut | 7 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | iCE40 HX1K | 13 x SB_CARRY, 102 x SB_DFFER, 4 x SB_DFFES, 11 x SB_DFFR, 1 x SB_DFFS, 1 x SB_GB, 91 x SB_IO, 215 x SB_LUT4 | 8 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | ECP5 45F | 11 x CCU2C, 1 x DCCA, 1 x DELAYG, 9 x IDDRX1F, 209 x LUT4, 10 x ODDRX1F, 118 x TRELLIS_FF, 91 x TRELLIS_IO | 6 |
| `dvi_tx` | `dvi_tx` | MODE=0 | LUT4 | 18 x dff, 480 x lut | 9 |
| `dvi_tx` | `dvi_tx` | MODE=0 | LUT6 | 18 x dff, 357 x lut | 7 |
| `dvi_tx` | `dvi_tx` | MODE=0 | iCE40 HX1K | 187 x SB_CARRY, 72 x SB_DFFER, 52 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 571 x SB_LUT4 | 8 |
| `dvi_tx` | `dvi_tx` | MODE=0 | ECP5 45F | 164 x CCU2C, 1 x DCCA, 349 x LUT4, 4 x ODDRX1F, 124 x TRELLIS_FF, 57 x TRELLIS_IO | 5 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | LUT4 | 18 x dff, 480 x lut | 9 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | LUT6 | 18 x dff, 357 x lut | 7 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | iCE40 HX1K | 187 x SB_CARRY, 72 x SB_DFFER, 52 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 571 x SB_LUT4, 1 x SB_PLL40_CORE | 8 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | ECP5 45F | 164 x CCU2C, 1 x DCCA, 1 x EHXPLLL, 349 x LUT4, 4 x ODDRX1F, 124 x TRELLIS_FF, 57 x TRELLIS_IO | 5 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | LUT4 | 7 x dff, 80 x lut | 4 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | LUT6 | 7 x dff, 68 x lut | 3 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | iCE40 HX1K | 22 x SB_CARRY, 36 x SB_DFFER, 2 x SB_DFFES, 1 x SB_GB, 67 x SB_IO, 78 x SB_LUT4 | 3 |
| `vga_out` | `vga_out` | MODE=0, BPC=4 | ECP5 45F | 14 x CCU2C, 1 x DCCA, 66 x LUT4, 38 x TRELLIS_FF, 67 x TRELLIS_IO | 3 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | LUT4 | 28 x dff, 396 x lut | 5 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | LUT6 | 28 x dff, 349 x lut | 4 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | iCE40 HX1K | 10 x SB_CARRY, 119 x SB_DFFER, 64 x SB_DFFES, 7 x SB_DFFR, 2 x SB_GB, 39 x SB_IO, 395 x SB_LUT4 | 5 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | ECP5 45F | 8 x CCU2C, 2 x DCCA, 6 x DELAYG, 5 x IDDRX1F, 391 x LUT4, 6 x ODDRX1F, 190 x TRELLIS_FF, 39 x TRELLIS_IO | 5 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | LUT4 | 89 x dff, 835 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | LUT6 | 89 x dff, 719 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 9 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 64 x SB_CARRY, 1024 x SB_DFFE, 309 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 2901 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 45 x CCU2C, 1 x DCCA, 808 x LUT4, 16 x TRELLIS_DPR16X4, 364 x TRELLIS_FF, 39 x TRELLIS_IO | 9 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | LUT4 | 89 x dff, 810 x lut, 2 x memory 8x8, 2 x memrd, 2 x memwr | 11 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | LUT6 | 89 x dff, 684 x lut, 2 x memory 8x8, 2 x memrd, 2 x memwr | 8 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | iCE40 HX1K | 51 x SB_CARRY, 128 x SB_DFFE, 285 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 1018 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8 | ECP5 45F | 38 x CCU2C, 1 x DCCA, 740 x LUT4, 4 x TRELLIS_DPR16X4, 340 x TRELLIS_FF, 39 x TRELLIS_IO | 9 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | LUT4 | 91 x dff, 1860 x lut | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | LUT6 | 91 x dff, 1602 x lut | 9 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | iCE40 HX1K | 64 x SB_CARRY, 1333 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 1834 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, BUF_RAM=0 | ECP5 45F | 45 x CCU2C, 1 x DCCA, 1785 x LUT4, 1388 x TRELLIS_FF, 39 x TRELLIS_IO | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | LUT4 | 91 x dff, 919 x lut | 11 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | LUT6 | 91 x dff, 779 x lut | 8 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | iCE40 HX1K | 51 x SB_CARRY, 413 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 881 x SB_LUT4 | 10 |
| `usb_device_fs` | `usb_device_fs` | VID=16'h1209, PID=16'h0001, MAXPKT=7'd8, MAXPKT0=7'd8, BUF_RAM=0 | ECP5 45F | 38 x CCU2C, 1 x DCCA, 854 x LUT4, 468 x TRELLIS_FF, 39 x TRELLIS_IO | 9 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | LUT4 | 89 x dff, 835 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 10 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | LUT6 | 89 x dff, 719 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 9 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 64 x SB_CARRY, 1024 x SB_DFFE, 309 x SB_DFFER, 39 x SB_DFFES, 13 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 39 x SB_IO, 2901 x SB_LUT4, 1 x SB_PLL40_CORE | 10 |
| `usb_device_fs_pll` | `usb_device_fs_pll` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 45 x CCU2C, 1 x DCCA, 1 x EHXPLLL, 808 x LUT4, 16 x TRELLIS_DPR16X4, 364 x TRELLIS_FF, 39 x TRELLIS_IO | 9 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | LUT4 | 83 x dff, 932 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 10 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | LUT6 | 83 x dff, 817 x lut, 2 x memory 64x8, 2 x memrd, 2 x memwr | 11 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 97 x SB_CARRY, 1024 x SB_DFFE, 325 x SB_DFFER, 37 x SB_DFFES, 8 x SB_DFFR, 1 x SB_GB, 55 x SB_IO, 2984 x SB_LUT4 | 10 |
| `usb_device_ulpi` | `usb_device_ulpi` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 66 x CCU2C, 1 x DCCA, 876 x LUT4, 16 x TRELLIS_DPR16X4, 370 x TRELLIS_FF, 55 x TRELLIS_IO | 10 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | LUT4 | 96 x dff, 1295 x lut | 10 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | LUT6 | 96 x dff, 1158 x lut | 11 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | iCE40 HX1K | 132 x SB_CARRY, 482 x SB_DFFER, 35 x SB_DFFES, 25 x SB_DFFR, 1 x SB_GB, 159 x SB_IO, 1229 x SB_LUT4 | 10 |
| `usb_host_ulpi` | `usb_host_ulpi` | (defaults) | ECP5 45F | 81 x CCU2C, 1 x DCCA, 1183 x LUT4, 542 x TRELLIS_FF, 159 x TRELLIS_IO | 10 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | LUT4 | 114 x dff, 1112 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 12 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | LUT6 | 114 x dff, 918 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 11 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 84 x SB_CARRY, 1152 x SB_DFFE, 414 x SB_DFFER, 44 x SB_DFFES, 14 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 104 x SB_IO, 3438 x SB_LUT4 | 12 |
| `usb_cdc_acm` | `usb_cdc_acm_fs` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 60 x CCU2C, 1 x DCCA, 1050 x LUT4, 18 x TRELLIS_DPR16X4, 475 x TRELLIS_FF, 104 x TRELLIS_IO | 9 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | LUT4 | 108 x dff, 1183 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 11 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | LUT6 | 108 x dff, 1029 x lut, 1 x memory 16x8, 2 x memory 64x8, 3 x memrd, 3 x memwr | 11 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 117 x SB_CARRY, 1152 x SB_DFFE, 430 x SB_DFFER, 42 x SB_DFFES, 9 x SB_DFFR, 1 x SB_GB, 120 x SB_IO, 3490 x SB_LUT4 | 11 |
| `usb_cdc_acm` | `usb_cdc_acm_ulpi` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 81 x CCU2C, 1 x DCCA, 1114 x LUT4, 18 x TRELLIS_DPR16X4, 481 x TRELLIS_FF, 120 x TRELLIS_IO | 10 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | LUT4 | 93 x dff, 844 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 11 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | LUT6 | 93 x dff, 728 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 9 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 41 x SB_CARRY, 16 x SB_DFFE, 262 x SB_DFFER, 39 x SB_DFFES, 14 x SB_DFFR, 3 x SB_DFFS, 1 x SB_GB, 24 x SB_IO, 848 x SB_LUT4 | 10 |
| `usb_hub` | `usb_hub_fs` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 28 x CCU2C, 1 x DCCA, 783 x LUT4, 2 x TRELLIS_DPR16X4, 318 x TRELLIS_FF, 24 x TRELLIS_IO | 10 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | LUT4 | 87 x dff, 939 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 10 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | LUT6 | 87 x dff, 829 x lut, 1 x memory 2x8, 1 x memrd, 1 x memwr | 11 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 74 x SB_CARRY, 16 x SB_DFFE, 278 x SB_DFFER, 37 x SB_DFFES, 9 x SB_DFFR, 1 x SB_GB, 40 x SB_IO, 939 x SB_LUT4 | 10 |
| `usb_hub` | `usb_hub_ulpi` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 49 x CCU2C, 1 x DCCA, 857 x LUT4, 2 x TRELLIS_DPR16X4, 324 x TRELLIS_FF, 40 x TRELLIS_IO | 10 |
| `usb_proxy` | `usb_hub_proxy_ulpi` | VID=16'h1209, PID=16'h0001 | LUT4 | 212 x dff, 3056 x lut, 1 x memory 2x8, 1 x memory 64x8, 2 x memrd, 2 x memwr | 11 |
| `usb_proxy` | `usb_hub_proxy_ulpi` | VID=16'h1209, PID=16'h0001 | LUT6 | 212 x dff, 2613 x lut, 1 x memory 2x8, 1 x memory 64x8, 2 x memrd, 2 x memwr | 11 |
| `usb_proxy` | `usb_hub_proxy_ulpi` | VID=16'h1209, PID=16'h0001 | iCE40 HX1K | 190 x SB_CARRY, 528 x SB_DFFE, 870 x SB_DFFER, 91 x SB_DFFES, 38 x SB_DFFR, 1 x SB_GB, 87 x SB_IO, 4058 x SB_LUT4 | 10 |
| `usb_proxy` | `usb_hub_proxy_ulpi` | VID=16'h1209, PID=16'h0001 | ECP5 45F | 122 x CCU2C, 1 x DCCA, 2917 x LUT4, 10 x TRELLIS_DPR16X4, 999 x TRELLIS_FF, 87 x TRELLIS_IO | 10 |
| `sha256` | `sha256_core` | (defaults) | LUT4 | 7 x dff, 3041 x lut | 39 |
| `sha256` | `sha256_core` | (defaults) | LUT6 | 7 x dff, 2013 x lut | 33 |
| `sha256` | `sha256_core` | (defaults) | iCE40 HX1K | 568 x SB_CARRY, 902 x SB_DFFER, 136 x SB_DFFES, 1 x SB_DFFR, 1 x SB_GB, 270 x SB_IO, 2404 x SB_LUT4 | 9 |
| `sha256` | `sha256_core` | (defaults) | ECP5 45F | 314 x CCU2C, 1 x DCCA, 1688 x LUT4, 1039 x TRELLIS_FF, 270 x TRELLIS_IO | 7 |
| `sha256` | `sha256` | (defaults) | LUT4 | 13 x dff, 3205 x lut | 39 |
| `sha256` | `sha256` | (defaults) | LUT6 | 13 x dff, 2270 x lut | 33 |
| `sha256` | `sha256` | (defaults) | iCE40 HX1K | 633 x SB_CARRY, 1231 x SB_DFFER, 136 x SB_DFFES, 2 x SB_DFFR, 1 x SB_GB, 272 x SB_IO, 2559 x SB_LUT4 | 9 |
| `sha256` | `sha256` | (defaults) | ECP5 45F | 349 x CCU2C, 1 x DCCA, 1840 x LUT4, 1369 x TRELLIS_FF, 272 x TRELLIS_IO | 7 |
| `sha256` | `sha256` | LEN_BITS=32 | LUT4 | 13 x dff, 3135 x lut | 39 |
| `sha256` | `sha256` | LEN_BITS=32 | LUT6 | 13 x dff, 2219 x lut | 33 |
| `sha256` | `sha256` | LEN_BITS=32 | iCE40 HX1K | 604 x SB_CARRY, 1202 x SB_DFFER, 136 x SB_DFFES, 2 x SB_DFFR, 1 x SB_GB, 272 x SB_IO, 2503 x SB_LUT4 | 9 |
| `sha256` | `sha256` | LEN_BITS=32 | ECP5 45F | 335 x CCU2C, 1 x DCCA, 1782 x LUT4, 1340 x TRELLIS_FF, 272 x TRELLIS_IO | 7 |
| `chacha20` | `chacha20_qr` | (defaults) | LUT4 | 573 x lut | 87 |
| `chacha20` | `chacha20_qr` | (defaults) | LUT6 | 441 x lut | 75 |
| `chacha20` | `chacha20_qr` | (defaults) | iCE40 HX1K | 124 x SB_CARRY, 256 x SB_IO, 447 x SB_LUT4 | 5 |
| `chacha20` | `chacha20_qr` | (defaults) | ECP5 45F | 68 x CCU2C, 128 x LUT4, 256 x TRELLIS_IO | 1 |
| `chacha20` | `chacha20_core` | (defaults) | LUT4 | 4 x dff, 5834 x lut | 87 |
| `chacha20` | `chacha20_core` | (defaults) | LUT6 | 4 x dff, 4189 x lut | 76 |
| `chacha20` | `chacha20_core` | (defaults) | iCE40 HX1K | 996 x SB_CARRY, 519 x SB_DFFER, 1 x SB_DFFR, 1 x SB_GB, 902 x SB_IO, 4643 x SB_LUT4 | 8 |
| `chacha20` | `chacha20_core` | (defaults) | ECP5 45F | 547 x CCU2C, 1 x DCCA, 3033 x LUT4, 520 x TRELLIS_FF, 902 x TRELLIS_IO | 4 |
| `chacha20` | `chacha20` | (defaults) | LUT4 | 11 x dff, 5978 x lut | 87 |
| `chacha20` | `chacha20` | (defaults) | LUT6 | 11 x dff, 3810 x lut | 76 |
| `chacha20` | `chacha20` | (defaults) | iCE40 HX1K | 1030 x SB_CARRY, 590 x SB_DFFER, 3 x SB_DFFR, 1 x SB_GB, 456 x SB_IO, 4766 x SB_LUT4 | 8 |
| `chacha20` | `chacha20` | (defaults) | ECP5 45F | 567 x CCU2C, 1 x DCCA, 3125 x LUT4, 593 x TRELLIS_FF, 456 x TRELLIS_IO | 4 |
| `inflate` | `inflate_adler` | (defaults) | LUT4 | 2 x dff, 152 x lut | 26 |
| `inflate` | `inflate_adler` | (defaults) | LUT6 | 2 x dff, 130 x lut | 20 |
| `inflate` | `inflate_adler` | (defaults) | iCE40 HX1K | 32 x SB_CARRY, 31 x SB_DFFER, 1 x SB_DFFES, 1 x SB_GB, 44 x SB_IO, 124 x SB_LUT4 | 20 |
| `inflate` | `inflate_adler` | (defaults) | ECP5 45F | 18 x CCU2C, 1 x DCCA, 91 x LUT4, 32 x TRELLIS_FF, 44 x TRELLIS_IO | 6 |
| `inflate` | `inflate_window` | WINDOW_BITS=15 | LUT4 | 6 x dff, 157 x lut, 1 x memory 32768x8, 1 x memrd, 1 x memwr | 12 |
| `inflate` | `inflate_window` | WINDOW_BITS=15 | LUT6 | 6 x dff, 114 x lut, 1 x memory 32768x8, 1 x memrd, 1 x memwr | 8 |
| `inflate` | `inflate_window` | WINDOW_BITS=15 | iCE40 HX1K | 28 x SB_CARRY, 6 x SB_DFFE, 40 x SB_DFFER, 1 x SB_DFFR, 1 x SB_GB, 41 x SB_IO, 724 x SB_LUT4, 64 x SB_RAM40_4K | 12 |
| `inflate` | `inflate_window` | WINDOW_BITS=15 | ECP5 45F | 16 x CCU2C, 1 x DCCA, 16 x DP16KD, 287 x LUT4, 45 x TRELLIS_FF, 41 x TRELLIS_IO | 12 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=1 | LUT4 | 57 x dff, 1749 x lut, 1 x memory 16x6, 1 x memory 16x9, 1 x memory 288x9, 1 x memory 320x4, 1 x memory 32768x8, 1 x memory 32x5, 6 x memrd, 6 x memwr | 31 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=1 | LUT6 | 57 x dff, 1455 x lut, 1 x memory 16x6, 1 x memory 16x9, 1 x memory 288x9, 1 x memory 320x4, 1 x memory 32768x8, 1 x memory 32x5, 6 x memrd, 6 x memwr | 26 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=1 | iCE40 HX1K | 223 x SB_CARRY, 412 x SB_DFFE, 369 x SB_DFFER, 7 x SB_DFFES, 5 x SB_DFFR, 1 x SB_GB, 30 x SB_IO, 3272 x SB_LUT4, 67 x SB_RAM40_4K | 31 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=1 | ECP5 45F | 143 x CCU2C, 1 x DCCA, 18 x DP16KD, 1618 x LUT4, 9 x TRELLIS_DPR16X4, 390 x TRELLIS_FF, 30 x TRELLIS_IO | 19 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=0 | LUT4 | 55 x dff, 1576 x lut, 1 x memory 16x6, 1 x memory 16x9, 1 x memory 288x9, 1 x memory 320x4, 1 x memory 32768x8, 1 x memory 32x5, 6 x memrd, 6 x memwr | 27 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=0 | LUT6 | 55 x dff, 1303 x lut, 1 x memory 16x6, 1 x memory 16x9, 1 x memory 288x9, 1 x memory 320x4, 1 x memory 32768x8, 1 x memory 32x5, 6 x memrd, 6 x memwr | 24 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=0 | iCE40 HX1K | 191 x SB_CARRY, 412 x SB_DFFE, 337 x SB_DFFER, 7 x SB_DFFES, 5 x SB_DFFR, 1 x SB_GB, 30 x SB_IO, 3102 x SB_LUT4, 67 x SB_RAM40_4K | 31 |
| `inflate` | `inflate` | WINDOW_BITS=15, WRAPPER=0 | ECP5 45F | 125 x CCU2C, 1 x DCCA, 18 x DP16KD, 1515 x LUT4, 9 x TRELLIS_DPR16X4, 358 x TRELLIS_FF, 30 x TRELLIS_IO | 19 |
| `inflate` | `inflate` | WINDOW_BITS=10, WRAPPER=1 | LUT4 | 57 x dff, 1691 x lut, 1 x memory 1024x8, 1 x memory 16x6, 1 x memory 16x9, 1 x memory 288x9, 1 x memory 320x4, 1 x memory 32x5, 6 x memrd, 6 x memwr | 31 |
| `inflate` | `inflate` | WINDOW_BITS=10, WRAPPER=1 | LUT6 | 57 x dff, 1423 x lut, 1 x memory 1024x8, 1 x memory 16x6, 1 x memory 16x9, 1 x memory 288x9, 1 x memory 320x4, 1 x memory 32x5, 6 x memrd, 6 x memwr | 26 |
| `inflate` | `inflate` | WINDOW_BITS=10, WRAPPER=1 | iCE40 HX1K | 213 x SB_CARRY, 407 x SB_DFFE, 359 x SB_DFFER, 7 x SB_DFFES, 5 x SB_DFFR, 1 x SB_GB, 30 x SB_IO, 2531 x SB_LUT4, 5 x SB_RAM40_4K | 31 |
| `inflate` | `inflate` | WINDOW_BITS=10, WRAPPER=1 | ECP5 45F | 139 x CCU2C, 1 x DCCA, 3 x DP16KD, 1421 x LUT4, 9 x TRELLIS_DPR16X4, 376 x TRELLIS_FF, 30 x TRELLIS_IO | 19 |
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

**The class layer has two blocks in it and the proxy is above them.**
`usb_cdc_acm` is a serial port and `usb_hub` is a hub, and both bind to a driver
the operating system already ships. `usb_hub` is a hub's **control endpoint**,
and the half that forwards packets is `usb_proxy` — a **transaction proxy**
rather than a repeater, for the reason that block's README §3 gives: a ULPI
transceiver's floor is about 24 bit times one way and a hub is allowed about 4,
so the two buses are decoupled and the upstream side NAKs until the answer is
there. That was the largest single piece of USB work left in this library and it
is what `ip/usb/usb_host_ulpi` and `ip/usb/usb_hub` were both built towards.

What is **still** not here, above the proxy, is isochronous transport
through it: the downstream SOF comes from `usb_host_sie`'s own free-running
counter and the PC's frame number is not carried across, so a device that
times anything from the frame sees a different one on each side. Control,
bulk and interrupt transfers do not depend on it, which is what USB 2.0 §5.6
to §5.8 make the difference.

**A human interface device is what is not here either**, and it is now a
smaller job than it was: a HID needs the report descriptor, which is
`GET_DESCRIPTOR` with a class descriptor type — a request the hook already
offers and a class may already claim — plus an interrupt IN endpoint, which
`usb_dev_core` already has as `NOTIF_ENDP`, plus the report itself. What is
genuinely missing for it is nothing in the infrastructure; it is the block.

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

**AES is what the `crypto` category does not have**, and it was left out on
purpose rather than forgotten. It is the biggest of the three symmetric
primitives a library like this wants — a key schedule, a choice of S-box
representation, and then the modes, which are where most of the usable
surface is — and every one of those three choices is easier against a
settled answer to what a crypto block here looks like. That answer is now
written down in
[the category's own section](#the-crypto-category-and-what-a-constant-time-claim-is-worth),
and its last part is the seven things AES should inherit. One of them is
not negotiable and is the reason this order is the right one: a
table-driven S-box in a block RAM is exactly the secret-dependent memory
access `crypto_blocks_hold_no_memory_to_index` forbids, so AES's S-box has
to be combinational, and the test that forbids it should be **extended**
to AES rather than relaxed for it.

**`spi_display_rx` has not been near a part, and will not be from
here.** The screen it receives is on another machine and will be
connected by someone else, so there is no top level and no pin
assignment for it — a board design made here would be a guess about
hardware nobody here can see. That makes it the one block in this library
whose *entire* deliverable is the HDL, the tests and the page, and it is
why the page is shaped the way it is: §4 of its README is a bring-up
procedure written for a person this project will never talk to, naming
every counter, what it should read, and what a wrong value means, and §6
is the order to suspect things in — the sampling edge, then the
oversampling ratio against their real `sclk`, then the fourth wire's
polarity, then whether that wire is a chip select at all. The two
counters that should read **zero**, `bit_error_count` and
`dc_change_count`, are the only evidence that could ever exist that the
link is what the capture suggested, and only whoever has the screen can
produce it.

**And neither crypto block has been near a board**, which is a different
sort of gap from the ones above because it is the only one a measurement
would close rather than code. Both are pure logic with no device
primitive, no PLL and no pin timing, so simulation sees everything the
part would about *function*. What a board would add is two things
simulation cannot produce: a **real throughput figure**, which is this
table's cycle counts multiplied by a clock that place and route actually
closed — and on the ECP5 that clock is the thing the missing carry
inference above would move most — and a **power trace**, which is the only
way the side channel §5 of both READMEs disclaims could ever be
characterised. The cheapest experiment for the first is `sha256` with its
byte port fed from `ip/bus/uart`'s receiver and its digest clocked back out
of the transmitter: no new HDL but a top level, one serial port, and a
digest a host can compare against `sha256sum`. The second needs a shunt
resistor and an oscilloscope and is not an afternoon.

**Compression is what the `compress` category does not have**, and it was
left out for the same kind of reason AES was: it is the larger half and it
is easier against a settled answer. A compressor needs hash-chain match
finding over the same 32 KiB window `inflate` already has, two passes over
every block to choose between RFC 1951 §3.2.3's three types by exact bit
cost, and a length-limited Huffman code *builder* — which is a different
and larger piece of work than the counting sort the decoder needs. What
this round settles for it is the window, the handshake, the stall-rather-
than-buffer answer to the output asymmetry, and the oracle: `compcol`
round-trips in both directions, so a compressor's output can be checked by
*its* decompressor as well as by this one, which is a stronger position
than decompression was in.

**And `inflate` has not been near a board either**, for the same reason
and with one addition. Besides the clock, a board would say whether
**sixteen `DP16KD` of one memory place and route at a useful frequency**,
which is the only part of this block simulation cannot speak to at all.
The cheapest experiment is the same shape as `sha256`'s: a top level that
takes a zlib stream in on `ip/bus/uart`'s receiver and sends the plaintext
back out of its transmitter, with an idle timeout standing in for
`in_last`, and a host that pipes a file through `compcol` and compares.
`ip/compress/inflate/README.md` §10 has it in full.
