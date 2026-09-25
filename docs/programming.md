# Programming a board

`reticle program <bitstream>` loads a bitstream into an attached FPGA over
JTAG, with no vendor tool and no external program of any kind. It is the
one part of Reticle that talks to hardware. Three families, over two
transports:

| Family | File | Transport | Section |
|---|---|---|---|
| Xilinx 7 series | `.bit` | FTDI FT2232H | the rest of this document |
| Gowin GW2A | `.fs` | FTDI FT2232H | *Gowin* |
| Lattice ECP5 | `.bit` | a Cynthion's Apollo microcontroller | *A second transport*, and *Configuring an ECP5* |

Two of those spell a bitstream `.bit`, so which vendor a file belongs to is
decided by **the bytes and not the extension**.

It is behind the `program` Cargo feature, which is **off by default**:

```sh
cargo build --features cli,program
```

That is the only feature with a dependency. `rawusb` is Karpeles Lab's
own dependency-free USB crate (MIT, edition 2024, MSRV 1.89), and
keeping it optional is what lets `cargo add reticle` still resolve to
zero dependencies for everyone who compiles a design and never touches a
cable. `cli` deliberately does not imply `program`, so the stock binary
is dependency-free too; it still recognises the `program` command and
says what to rebuild.

## What it does, and what it will not do

It writes the part's **volatile configuration memory**. A power cycle
undoes it, so every attempt is reversible.

It does **not** program the board's QSPI flash, and there is no code in
the crate that could: `src/program` has no flash commands at all, and for
the ECP5, which reaches its configuration flash through a JTAG instruction
rather than through a command of its own, `program::lattice::NOT_SHIFTED`
names that instruction and a test asserts no plan carries it. A bad
flash write is not reversible and is not a risk worth taking to save a
power cycle. It does not touch the mode pins either — JTAG configuration
works whatever a board's mode jumper is set to, which is what makes this
the safe way in — though it does read the mode pins back and report them.

The part's `IDCODE` is read and checked **before anything is written**,
and a mismatch stops the command with nothing done. The `.bit` header's
part field is checked against it first, before the cable is even opened.

## Using it

```console
$ reticle program --list
210183BD4B37  FTDI cable  [USB serial number]

$ reticle program --probe
adapter 210183BD4B37 (FTDI cable)
FT2232H, TCK 1000000 Hz
IDCODE 0x0362d093: Xilinx, revision 0
status: 0x70001d0c [INIT_B INIT_COMPLETE MMCM_LOCK, MODE 101]

$ reticle program design.bit
design.bit: top for 7a35tcpg236, built 2019/09/11 17:23:18
adapter 210183BD4B37 (FTDI cable)
FT2232H, TCK 1000000 Hz
IDCODE 0x0362d093 (revision 0), as expected
status before: 0x70001d0c [INIT_B INIT_COMPLETE MMCM_LOCK, MODE 101]
status after JPROGRAM: 0x50001d0c [INIT_B INIT_COMPLETE MMCM_LOCK, MODE 101]
  ...
2192012 bytes of configuration data shifted in 17.6 s
status after JSTART: 0x70107dfc [DONE RELEASE_DONE INIT_B INIT_COMPLETE EOS MMCM_LOCK, MODE 101]
DONE is high: the part accepted the bitstream and is running it.
```

`--probe` reads and reports and writes nothing at all, which is the
right first command on an unfamiliar board. `--device <serial>` picks
one adapter when several are attached; with none named and more than one
attached the command stops rather than guessing. `--clock <hz>` moves
TCK off its 1 MHz default, and `--expect <idcode>` names another
7-series part.

### Which adapter, and which transport

`--list` and `--device` cover **both** kinds of adapter, an FTDI cable and
a Cynthion, and the command works out which transport owns a serial number
*before it opens anything*:

```console
$ reticle program --list
210183BD4B37                FTDI cable  [USB serial number]
35L6H2CMGJJVCIBAEA3GCLAN74  Cynthion, Apollo debugger  [the microcontroller's own USB serial number]
```

That order — decide, then open — is deliberate, and it is not just
tidiness. The two transports have no byte-level protocol in common, so
"try the FTDI path and fall back to Apollo" would be a retry loop whose
first attempt lands on whatever device happens to match, and on a machine
with several boards attached that is how the wrong one gets opened.
Opening a Cynthion is not free either: reaching Apollo may take the USB
port away from the board's FPGA, and that is not undone in software. So
`reticle::program::choose_adapter` is a pure function over a single
enumeration of the bus, it runs first, and a request the Apollo transport
cannot serve is refused *before* any handover. It is unit-tested with
nothing attached, which is where the interesting cases are: two adapters
and no `--device`, the name of the mode a board is not in, and a flash UID
that cannot be matched.

The third column says what kind of name each string is, because on a
Cynthion they are not all the same kind of thing:

- an **FTDI cable**'s serial is its USB serial number, and that is all it
  is;
- a **Cynthion in gateware mode** advertises the board's **flash UID** as
  its USB serial number — sixteen hex digits, the board's stable identity;
- the **same board in debugger mode** reports the *microcontroller's* own
  serial number, which is not the flash UID and not derived from it.

Either of a board's two names selects it, as long as it is the name of the
mode the board is in now. A flash UID aimed at a board that is already a
debugger is an error that says why rather than a bare "no such adapter":
Reticle will not read a debugger's flash UID, because Apollo reads one by
forcing the FPGA offline and driving the board's configuration flash over
JTAG, and [`docs/apollo-protocol.md`](apollo-protocol.md) §7 is where that
line is drawn.

A serial that matches nothing is an error listing what *is* attached, with
what each one is. A serial nobody can see is a serial nobody can type, and
a serial nobody can identify is one they type into the wrong board.

### Permissions

The adapter is reached through usbfs, so the device node under
`/dev/bus/usb/` has to be writable — on most distributions that means
being in the group a udev rule assigns it to (`plugdev` or `usb`). The
command says so when it cannot open the device.

On Linux the kernel's `ftdi_sio` driver claims **both** channels of an
FT2232H, so the JTAG channel turns up as a `/dev/ttyUSB*` even though it
is not a serial port. The transport detaches it before claiming;
replugging the board or a power cycle gives it back.

## The shape of the code

The library rule is that nothing in `src/` performs I/O and the CLI owns
all of it. `src/program` keeps that rule, which takes some doing for
something whose entire purpose is to drive a cable:

| Module | What it is | Touches a device |
|---|---|---|
| `program::ftdi` | the MPSSE command encoding (FTDI AN_108, AN_135) | no |
| `program::apollo` | the Cynthion debugger's request set | no |
| `program::jtag` | the IEEE 1149.1 TAP state machine and scans | no |
| `program::xilinx` | the UG470 configuration sequence | no |
| `program::gowin` | the Gowin `.fs` container and SRAM sequence | no |
| `program::lattice` | naming a Lattice part, and the ECP5 SRAM sequence | no |
| `program::choose_adapter` | which transport a `--device` serial belongs to | no |
| `program::usb` | opening, detaching, claiming, transfers | yes |

Everything but `usb` is a *sequencer* or a decision. The sequencers each
produce a `jtag::Job` or an `apollo::Program` — a buffer of bytes or a
list of control requests to send, plus a description of what the reply
means — and decode a reply back into a value. None of them knows what USB
is. `choose_adapter` is a pure function from "what is attached" to "which
transport", so the one decision with a board on the other end of it is
testable with no board at all. `usb` writes the bytes and reads the
bytes, and that is all it does.

`unsafe_code = "deny"` holds throughout: every ioctl is inside `rawusb`'s
own `src/sys/`.

The split is not tidiness for its own sake. The two mistakes this code
can make are both invisible at the byte level, and both are unit-testable
only because the encoding is a pure function:

- **an off-by-one in a shift length**, which leaves the TAP one bit out
  of step and every later scan reading plausible nonsense;
- **a bit order backwards**, which produces a stream of exactly the right
  length and shape that the part quietly ignores.

`tests/program_jtag.rs` is where that is cashed in: it contains a small
independent interpreter for the MPSSE command stream driving a model TAP,
written from the standard rather than from `program::jtag`, and judges
the crate's output by what a part would do with it. Nine tests run with
no hardware; one more is `#[ignore]`d and needs a board.

## Bit order

Two opposite conventions meet, and getting them the wrong way round is
the classic way for everything to look right and nothing to work.

- **JTAG shifts least-significant bit first.** Bit 0 of a register is the
  first bit through TDI, and the first bit out of TDO is bit 0 of what
  was captured. So `IDCODE`, a JTAG register, is read with
  `Job::capture_u32` and no correction.
- **A configuration bitstream is most-significant bit first.** Bit 7 of
  the payload's byte 0 is the first bit the configuration engine must
  see. So every payload byte is bit-reversed — the *bits inside each
  byte*, never the byte order — by `xilinx::reverse_all`, once, inside
  `configure_job`, where no caller can forget it.
- **A configuration register read through `CFG_OUT` is also
  most-significant bit first**, the opposite of `IDCODE` through the same
  cable, so it is decoded by `xilinx::word_msb_first`.

## The sequence

From UG470, *7 Series FPGAs Configuration User Guide*, which is the same
document `fpga::xc7` writes the bitstream container from.

1. Five TMS-high clocks to `Test-Logic-Reset`, which works from any state
   including an unknown one, then rest in `Run-Test/Idle`.
2. `IDCODE` (IR `0b001001`), read 32 bits of DR, and **stop here** unless
   it matches. The top four bits are the silicon revision and are
   ignored; the other 28 must agree.
3. `CFG_IN` (`0b000101`) with a type 1 read packet for the status
   register, then `CFG_OUT` (`0b000100`) and 32 bits of DR — the state
   before anything is written, on the record.
4. `JPROGRAM` (`0b001011`), then `ISC_NOOP` (`0b010100`) so the
   instruction register is not left holding `JPROGRAM`, then ten thousand
   clocks in `Run-Test/Idle` while the part clears its configuration
   memory. `DONE` must go low here; it is an error if it does not.
5. `CFG_IN` and the whole payload shifted through DR, bit-reversed.
6. `JSTART` (`0b001100`) and the startup clocks, then `BYPASS`.
7. Read the status register again: `DONE` must now be high, and
   `CRC_ERROR` and `ID_ERROR` must not be set.

### The status register

`xilinx::Status` decodes UG470's *Status Register Description* table
(numbered 5-25 in some revisions, 5-29 in others). The bit that matters
is **`DONE`, bit 14** — the level on the `DONE` pin. The raw word is
always printed beside the decoded names, so a field this crate placed
wrongly can be checked against any other tool.

## The board this was verified on

A Digilent Basys 3 (XC7A35T, `7a35tcpg236`) with the FT2232H it carries
on board, `0403:6010`.

The low byte of the FTDI data bus is set to value `0x88`, direction
`0x8b`. `ADBUS0..3` are TCK, TDI, TDO and TMS, which is the MPSSE's
fixed assignment; `ADBUS7` is an output held high because on Digilent's
JTAG-SMT2 circuit it enables the buffer between the FTDI part and the
FPGA's JTAG pins, and leaving it low leaves the chain disconnected.
Those two numbers describe the *board*, not the FPGA, no data sheet
states them, and they were taken from the published OpenOCD
configuration for a Basys 3 (`ftdi layout_init 0x0088 0x008b`). They are
`program::BASYS3_PINS`, and a board that wires `ADBUS7` differently
needs its own pair.

### What was established on hardware

Project X-Ray's `artix7/harness/basys3/swbut/design.bit` — a Vivado
bitstream for this exact board, CC0, not committed here — was loaded over
this path. Observed:

- `IDCODE` read back as `0x0362d093`, exactly the XC7A35T;
- the configuration `IDCODE` register (register 12) read back as
  `0x0362d093` too, which is an independent check that the `CFG_OUT`
  readback path and the most-significant-bit-first decoding are right,
  against a value that is known in advance;
- the mode pins read `0b101`, JTAG, unchanged by anything this does;
- `DONE` **high after `JSTART`** and not before, with `EOS`,
  `RELEASE_DONE`, `INIT_COMPLETE`, `GHIGH_B`, `GWE` and `GTS_CFG_B` all
  set and no error bit — status `0x70107dfc`, against `0x70001d0c` on the
  unconfigured part;
- and on a second run, with the part already configured by the first,
  `DONE` going **high → low → high** across `JPROGRAM` and `JSTART`, so
  the bit is being driven by what this command does rather than merely
  read;
- and, as a negative control, the same bitstream with 64 bytes flipped in
  the middle of its frame data produced `CRC_ERROR` and no `DONE`. The
  part is really parsing what is sent to it, not merely being clocked at.

What that does **not** establish is that the LEDs follow the switches:
that is something only a person looking at the board can confirm. `DONE`
asserting and the status word coming back clean is the strongest
statement this path can make from software, and it is the statement it
makes.

The programmer was proved first, against a file known to be good, so
that it could not be confused with the bitstream side. On 2026-09-24 it
then loaded two of Reticle's own: a lookup table and three pins, which a
person watched drive an LED from two switches, and `blink`, a clocked
counter, which a person watched blink. `docs/fpga-xray.md` says what each does and does not
establish about the bitstream side.

## Gowin

A `.fs` file is loaded with Gowin's own SRAM sequence (UG290, TN653), in
`src/program/gowin.rs`: `CONFIG_ENABLE` and a wait for edit mode;
`ERASE_SRAM` and a wait for the erase; `XFER_DONE` and `CONFIG_DISABLE`;
then `CONFIG_ENABLE`, `ADDRESS_INITIALIZE`, `TRANSFER`, the whole file
through DR, the checksum step, `CONFIG_DISABLE`, and a read of the status
register for `DONE` and no error.

- **The instruction register is eight bits**, not six, which is why each
  vendor's sequence runs only on a part its identifier names: a Xilinx
  instruction shifted at a Gowin part executes whatever the bits land on.
  The identifier is read first with no instruction at all.
- **SRAM only, by construction.** The opcodes are an enum holding none
  that reaches flash, and `gowin::FORBIDDEN` lists the ones that do —
  `0x16` among them, a pass-through to the board's SPI flash — with a test
  keeping the two apart.
- **The checksum step (`0x0a`, 32 bits, `0x08`) is not in Gowin's
  documents**, and without it a GW2A-18 took the file and left `DONE`
  clear with no error bit. It is what openFPGALoader sends, and the value
  is the one the file's footer carries.
- **The file goes through first character first.** Eight characters pack
  into a byte highest-first, as the file reads, and each byte is
  bit-reversed for the least-significant-first shift, the correction a
  `.bit` gets too.

Verified on a Sipeed Tang Primer 20K, whose dock's JTAG adapter is a
Sipeed "JTAG Debugger" presenting as an FT2232D (`0403:6010`,
bcdDevice `0x0500`), driven with `program::SIPEED_PINS` (value `0x08`,
direction `0x0b`: openFPGALoader's for this board). The status read
`0x6020` (`DONE`, security) with the board's own design running,
`0x00a0` (edit mode, erased) after the erase, and `0x2020` (`DONE`) after
loading `examples/primer20k/key_led.v`, which a person then watched
work. `docs/fpga-gowin.md` has the rest.

## A second transport: the Cynthion's Apollo debugger

Everything above goes through an FTDI part. A Great Scott Gadgets
**Cynthion** has none: its Lattice ECP5 has no JTAG header, and the only
way to its TAP is to ask the board's debug microcontroller — running
**Apollo** firmware — to perform scans over USB.

Reticle speaks that protocol too. The wire protocol is written down in
[`docs/apollo-protocol.md`](apollo-protocol.md), which was written
**before** the implementation and which the implementation was written
from; it says for every request where the number came from and whether a
board has confirmed it.

### What has been done with it

On 2026-09-25 a Cynthion on the author's machine was taken from its
analyzer gateware to the Apollo debugger and the ECP5's identifier was
read:

```
IDCODE 0x21111043
IDCODE 0x21111043: manufacturer 0x021 (Lattice), part number 0x1111, version 2 — LFE5U-12F (or LAE5U-12F)
```

The raw value is `0x21111043`. Reading it: bit 0 is 1, as IEEE 1149.1
requires of an identification register; bits 11..1 are `0x021`, which is
JEDEC JEP106 bank 1 code `0x21`, Lattice Semiconductor; the part number
field is `0x1111` and the top nibble is `2`, and that combination is the
**LFE5U-12F** — the smallest ECP5, 12 kLUT. (`0x4111_1043`, the same
part number with a different top nibble, is the LFE5U-25F, which is why
`program::lattice` matches on all thirty-two bits and not on twenty-eight
the way the Xilinx path does.)

Reading was as far as it went for a day. It is not any more: see
*Configuring an ECP5* below.

### Running it

From the command line, which is how a person reaches it:

```console
$ reticle program --probe --device 35L6H2CMGJJVCIBAEA3GCLAN74
adapter 35L6H2CMGJJVCIBAEA3GCLAN74 (Cynthion, Apollo debugger)
Apollo on 35L6H2CMGJJVCIBAEA3GCLAN74
  identifier: Apollo Debug Module
  firmware: v1.1.1
  USB API: 1.2
  max scan 2048 bits, quirks 0x00000000 (assumed; this firmware has no case for that request)
IDCODE 0x21111043: manufacturer 0x021 (Lattice), part number 0x1111, version 2 — LFE5U-12F (or LAE5U-12F)
nothing further was read and nothing was written: reading a status register means shifting some vendor's instruction, and there is no ECP5 configuration sequence in this crate
```

`--probe` is the whole of what a Cynthion can be asked from here.
Handing it a `.bit` or a `.fs` is refused, **before the board is
opened**, because there is no ECP5 configuration sequence in the crate
and no ECP5 fabric database to build a bitstream with; refusing before
rather than after matters, since opening the board might have taken its
USB port away from its FPGA for the rest of the session.

`--clock` does not apply: Apollo owns the TAP and is never told a TCK
rate. `--expect` does, and it compares all thirty-two bits rather than
masking the top nibble off as a revision, because on an ECP5 that nibble
is part of which part it is.

The same read from a test, which is where the transport is checked
against a model of the debugger rather than against a board:

```console
$ cargo test --features program --test program_apollo -- --ignored --nocapture read_the_ecp5_idcode
```

The test skips with a message when no Cynthion is attached, so it is safe
anywhere; `RETICLE_CYNTHION` picks a board by serial number when several
are.

### Three things to know before pointing it at a board

- **Getting to Apollo takes the USB port away from the FPGA.** A
  Cynthion running gateware enumerates as the gateware's device; the
  debugger is behind a handover request that makes the board
  re-enumerate. Nothing persistent happens — no flash is touched and the
  FPGA stays configured — but **the board stays in debugger mode until it
  is replugged or power cycled.** The request that asks for the port back
  is sent and is accepted, and it is not enough on its own; §6 of the
  protocol document says why. The command says all of this on its way
  through rather than afterwards.
- **The two modes answer to different names, and only one of them names
  the board.** In gateware mode the USB serial string is the board's
  **flash UID**; in debugger mode it is the microcontroller's own serial
  number. The flash UID is the board's stable identity and the same in
  both modes, but a debugger does not put it on the bus and no request
  returns it — so `--device` matches whichever name the board is wearing
  now, and a handover is followed by watching for the debugger that
  appeared rather than by matching a name.
- **The requests this project will not send are a list, not a promise.**
  `docs/apollo-protocol.md` §7 has it — reconfiguring the FPGA, forcing it
  offline, the LED pattern, an ADC channel, the flash bridge, DFU — and
  `apollo::NOT_SENT` is the same list in the code, with a test asserting
  that nothing the crate compiles ever appears in it. Configuring an ECP5
  does **not** need any of them, including the one that sounds as though it
  would: §7 explains that `0xC1` "force the FPGA offline" is itself a JTAG
  scan of `ISC_ENABLE`, which the configuration sequence shifts anyway.

### Configuring an ECP5

On **2026-09-25**, over the same transport, the LFE5U-12F of the same
Cynthion r1.4 **took a bitstream into its configuration SRAM and asserted
`DONE`**. Nothing Lattice had been configured by this project before that.

Four files have gone in, in this order:

| File | Bytes | Built by | Status after |
|---|---|---|---|
| `analyzer.bit` | 238 282 | Great Scott Gadgets, for this board | `0x00200100` (`DONE`, no fault) |
| `leds_alternate.bit` | 98 473 | `reticle fpga` | `0x00200100` |
| `leds.bit` | 98 473 | `reticle fpga` | `0x00200100` |
| `leds.bit`, rebuilt (2026-09-26) | 98 474 | `reticle fpga` | `0x00200100` |

The first was deliberately not ours. It is the bitstream the board is
configured with every time it is plugged in, so if the *sequence* were
wrong it would have failed there, before any question about the compiler
arose. `docs/fpga-trellis.md` is the story of the other three, including
why the third and the fourth differ by one byte: the third lit nothing,
and the bank setting that was missing from it is a bit.

**What that table does not say is whether anything lit.** `DONE` was high
after all four, and after the third the board's owner looked and the LEDs
were dark. The status register is the part confirming it took a
configuration, and that is all it is; the compiler is judged somewhere
else.

What a run looks like:

```console
$ reticle program --device 35L6H2CMGJJVCIBAEA3GCLAN74 /tmp/leds.bit
/tmp/leds.bit: Lattice ECP5, 98474 bytes, compressed, for IDCODE 0x21111043 (LFE5U-12F (or LAE5U-12F)) — Part: LFE5U-12F-8CABGA256
adapter 35L6H2CMGJJVCIBAEA3GCLAN74 (Cynthion, Apollo debugger)
Apollo on 35L6H2CMGJJVCIBAEA3GCLAN74
  identifier: Apollo Debug Module
  firmware: v1.1.1
  USB API: 1.2
  max scan 2048 bits, quirks 0x00000000 (assumed; this firmware has no case for that request)
IDCODE 0x21111043: manufacturer 0x021 (Lattice), part number 0x1111, version 2 — LFE5U-12F (or LAE5U-12F)
status before: 0x00200100 (DONE)
after LSC_REFRESH, IDCODE 0x21111043
status after ISC_ENABLE: 0x00200f10 (DONE, ISC_ENABLE)
status after ISC_ERASE: 0x00200e10 (ISC_ENABLE)
  10% (9987 of 98478 bytes on the wire)
  ...
98474 bytes of bitstream shifted in 0.5 s
status after ISC_DISABLE: 0x00200100 (DONE)
DONE is high: the part accepted the bitstream and is running it. Only the volatile configuration SRAM was written; a power cycle reloads the board's flash.
```

Every line of that is the part answering, not the host assuming, and each
one is a check that is acted on:

- `status before` puts the `DONE` bit **before** anything on the record.
- `LSC_REFRESH` restarts configuration, which is what strobing `PROGRAMN`
  would do, and the identifier is read **again** afterwards. If the part
  stops answering there, nothing has been erased yet.
- `ISC_ENABLE` set in the status register is the part confirming it is in
  configuration mode. A run that does not see it stops.
- `DONE` **clear** after `ISC_ERASE` is the part confirming it really did
  throw its configuration away. A run that still sees `DONE` stops, because
  a part that did not erase will not take a new bitstream either.
- `DONE` set at the end, with no fault bit, is the whole result.

Between the steps the host **polls** — the ECP5's own `LSC_CHECK_BUSY`,
counted in reads rather than in seconds. Nothing here sleeps for a fixed
time, which is also why no test asserts on a clock.

#### What it cannot do

- **Only the volatile configuration SRAM.** A power cycle reloads the part
  from the board's flash. `program::lattice::NOT_SHIFTED` names the JTAG
  instructions that would reach something a power cycle does not undo —
  `LSC_ENTER_BACKGROUND_SPI` above all, which turns the part into a
  pass-through to the board's configuration flash — and a test walks every
  plan the module builds to assert none carries one. That is the same guard
  `gowin::FORBIDDEN` and `apollo::NOT_SENT` get.
- **Over Apollo only.** The five plans are transport-neutral and
  `jtag::Scan` would encode them for an FTDI cable, but that pairing has
  never been run on a part, so `reticle program` refuses an ECP5 `.bit`
  over a cable rather than trying it.
- **The part, exactly.** An LFE5U-12F and an LFE5U-25F are the same die
  with different identifiers, so a bitstream for the wrong one would
  configure the part and assert `DONE` while doing something else. The
  identifier is checked against the file's `VERIFY_ID` operand before
  anything is written, and a file with no `VERIFY_ID` is refused rather
  than loaded hopefully.

#### Where the sequence came from

`ECP5CommandBasedProgrammer.configure` in Apollo's own host package
(`apollo_fpga/ecp5.py`, BSD-3-Clause) — the sequence a Cynthion is
configured with by its vendor's tool — cross-checked against `ecpprog`'s
`ecp5_program` and against Lattice's TN-02039 for the instruction opcodes
and the status register's bit positions. The three agree except that
`ecpprog` omits the undocumented `0x1C` preamble, which Apollo's own source
comments `# ???`; Reticle keeps it, because reproducing the sequence with
the fewest unknowns was worth more than dropping a step nobody can
explain.

One detail is not in any of them in a form that can be copied: the **bit
order** of the payload. The configuration logic takes the most significant
bit of the first byte first and a JTAG data register shifts the least
significant bit of the first byte first, so every byte is reversed and the
byte order is left alone. `lattice::burst_order` is that, with the argument
for why it is that and not the other plausible arrangement written next to
it — and 238 282 bytes of somebody else's bitstream asserting `DONE` is
the check.

### How it fits the rest of `src/program`

Apollo is not an FTDI part with different numbers; it is a different
shape of device. An MPSSE is a shift engine that the host tells about
TMS. Apollo owns a TAP controller and takes **state numbers**, and has
no way to be sent a TMS sequence at all.

So the two transports share nothing at the byte level, and what they do
share is one level up: `jtag::Plan`, a list of named JTAG operations.
`jtag::Scan::apply` compiles a plan into MPSSE bytes and
`apollo::compile` compiles the same plan into control requests, so
`jtag::idcode_plan` — reset, then thirty-two bits out of DR, with no
instruction shifted — is written once and both transports perform it.
`tests/program_jtag.rs` drives the FTDI encoding through a model TAP and
`tests/program_apollo.rs` does the same for the Apollo one, both with
nothing attached.
