# Programming a board

`reticle program <design.bit>` loads a bitstream into an attached Xilinx
7-series FPGA over JTAG, through an FTDI FT2232H, with no vendor tool and
no external program of any kind; `reticle program <design.fs>` does the
same for a Gowin GW2A (see *Gowin* below). It is the one part of Reticle
that talks to hardware.

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
the crate that could: `src/program` has no flash commands at all. A bad
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
210183BD4B37

$ reticle program --probe
adapter 210183BD4B37, TCK 1000000 Hz
IDCODE 0x0362d093 (revision 0), as expected for 0x0362d093
status before: 0x70001d0c [INIT_B INIT_COMPLETE MMCM_LOCK, MODE 101]

$ reticle program design.bit
design.bit: top for 7a35tcpg236, built 2019/09/11 17:23:18
adapter 210183BD4B37, TCK 1000000 Hz
IDCODE 0x0362d093 (revision 0), as expected for 0x0362d093
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
| `program::jtag` | the IEEE 1149.1 TAP state machine and scans | no |
| `program::xilinx` | the UG470 configuration sequence | no |
| `program::usb` | opening, detaching, claiming, bulk transfers | yes |

The first three are *sequencers*. Each produces a `jtag::Job` — a buffer
of bytes to send, plus a description of what the reply means — and
decodes a reply back into a value. None of them knows what USB is. The
fourth writes the bytes and reads the bytes, and that is all it does.

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
