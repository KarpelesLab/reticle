# A real Lattice ECP5, and a real `.bit`

## A design produced by this has been loaded into a part

On 2026-09-25 a design built by this flow from Verilog was loaded into the
LFE5U-12F of a Great Scott Gadgets Cynthion r1.4 attached to the machine
this was written on, over the board's own Apollo debug microcontroller,
and **the part accepted it and asserted `DONE`** with no fault bit set:
status `0x00200100`, read back from the configuration status register
afterwards. Nothing Lattice had ever been configured by this project
before that.

The design is `testdata/fpga/cynthion/leds.v`: six output pads, each
driven by a constant zero, constrained to the board's six FPGA LEDs. The
LEDs are active low, so what it should produce is **all six lit**.

A second design was loaded the same way, ten minutes earlier:
`testdata/fpga/cynthion/leds_alternate.v`, which drives `6'b101010` and
should light LEDs 0, 2 and 4 and leave 1, 3 and 5 dark. It exists because
six lit LEDs are a weaker observation than three lit and three dark — the
second is not a state anything else on the board produces. It was also
accepted, with `DONE` high and the same status.

### Then somebody looked, and the LEDs were dark

On 2026-09-26 the board's owner reported that LEDs 0 to 5 were **dark**,
with that bitstream in the part and `DONE` high. So the first run above is
exactly the thing this file said it was: a part that accepted a
configuration and said it was running it, which is not the same claim as a
lit LED, and the difference turned out to be real.

**What was missing was the bank's rail.** An IO bank of an ECP5 has one
setting of its own — `BANK.VCCIO`, in a `BANKREF<n>` tile that is nowhere
near the pads it serves — and `configure_io` set every bit of a pad's own
three tiles and not that one. It is not a subtle omission once looked for:
all three of Great Scott Gadgets' own bitstreams for this board set
`BANK.VCCIO` to `3V3` on every bank of the part, and nextpnr writes it for
every bank a design puts an IO in (`init_io_banks` in its `bitstream.cc`).
A bitstream without it says nothing about what voltage the bank runs at.

That bit is now written, for the banks a design uses and no others, and
`tests/fpga_trellis.rs::the_bank_rail_is_the_bit_lattices_own_packer_sets_for_this_board`
checks it against all three reference bitstreams at absolute frame
positions rather than counting it. `leds.bit` went from 60 configuration
bits to 61.

The corrected bitstream was loaded into the same board on 2026-09-26 and
the part accepted it, status `0x00200100` — `DONE`, no fault — the same as
before, because `DONE` was never the thing in question.

### And then they were lit

On 2026-09-26, with the corrected bitstream in the part, the board's owner
reported **all six LEDs on**. So `BANK.VCCIO` was the whole of what was
missing, and the chain from Verilog to a lit LED on a Lattice part closes.

That is the third vendor this project has configured and had watched:
a Xilinx XC7A35T on a Digilent Basys 3, twice, and now an LFE5U-12F on a
Cynthion, in each case with no vendor tool at any step.

It is worth keeping what the sequence cost, because the lesson is cheap to
read and was expensive to learn. `DONE` went high on the *first* run, with
dark LEDs, and it went high on the corrected run too. `DONE` says the
configuration engine accepted a bitstream; it says nothing whatever about
whether the bitstream configures what the design asked for. Every
structural check this file describes passed on the broken bitstream. The
only thing that told the difference was a person looking at the board.

One difference from Lattice's own header was **checked and ruled out**:
`ecppack` wrote control register 0 as `0x40000038` for these three files
and Reticle writes `0x40000000`. The `0x38` is `ecppack --freq 38.8`, the
master SPI clock the part uses when it loads *itself* from flash
(`frequencies` in prjtrellis' `Bitstream.cpp`), and it reaches no IO
buffer. `CTRL0_DEFAULT` is what `ecppack` writes with no options.

Both runs are reproducible:

```sh
reticle fetch prjtrellis-db
reticle fpga testdata/fpga/cynthion/leds.v \
    --device ecp5-12f-CABGA256 \
    --constraints testdata/fpga/cynthion/leds.rcf \
    --bitstream /tmp/leds.bit
reticle program --list                    # find the board's serial
reticle program --device <serial> /tmp/leds.bit
```

What that writes is **only the volatile configuration SRAM**. A power
cycle reloads the part from the board's flash, which nothing in this
project touches; `docs/programming.md` and `src/program/lattice.rs`'s
`NOT_SHIFTED` are where that is made checkable rather than promised.

### What this settles, and what it does not

It settles that the chain from Verilog to a configuration a real ECP5
accepts closes: the frontend, synthesis, IO primitive mapping, the
placer's pin constraints, the frame map, the `.bit` container, its 7562
check words, and the JTAG configuration sequence.

It settles **nothing about routing or about logic**, and that is not a
gap in the measurement, it is a gap in the backend. `src/fpga/trellis`
builds the part's geometry and its pads and **declares no interconnect at
all**: no pips, and no wires but the three each pad bel's pins name. A
design with anything to route is refused by name, before a file is
written, by `TrellisFabric::unroutable`. So:

- nothing clocked can be built, which is why the milestone is six lit
  LEDs and not a counter walking across them;
- no lookup table can be built either, because its inputs and its output
  are connections;
- the 60 MHz oscillator on ball A8 is declared in the device file and
  reaching it is not possible.

The table at the end says what each of those needs.

## What Project Trellis ships, and in what form

This decides everything after it, so it is written down in full.

### It is a separate repository, wired in as a submodule

`YosysHQ/prjtrellis` is the tooling — `ecppack`, `libtrellis`, the
fuzzers and the documentation. The **data** is `YosysHQ/prjtrellis-db`, a
separate repository that `prjtrellis` carries as a git submodule; its
`.gitmodules` is exactly:

```
[submodule "database"]
	path = database
	url = https://github.com/YosysHQ/prjtrellis-db
```

Reticle pins the commit that submodule currently points at,
`015e0330630d7c238c0e4f2cdd9c8157eb78c54a` (2025-09-15), so what is read
is what `ecppack` and nextpnr would read. `src/bin/reticle/datadir.rs`
holds the pin and `src/bin/reticle/prjtrellis-db.manifest` a SHA-256 and
a size for each of the 191 files taken; a file that does not match is
refused and nothing is installed. The licence is CC0.

**The database is not in this repository and must not be.** `reticle
fetch prjtrellis-db` downloads it into `~/.cache/reticle`, or
`RETICLE_TRELLISDB` names an existing checkout.

### The layout, and what is taken

Everything is plain text checked into git — **nothing is compressed**,
which is the one way this differs from Project Apicula's Gowin database
and the reason `src/fpga/trellis` needs no dependency at all where
`src/fpga/apicula` needs `compcol` for xz. The repository is 705 files
and 81 MB across five device families. What an LFE5U-12F needs is 191
files and 5.8 MB:

| Path | Format | What it is | Read by Reticle |
|---|---|---|---|
| `devices.json` | JSON | every part: IDCODE, frames, bits per frame, pad bits, `max_row`/`max_col`, packages | yes |
| `ECP5/<part>/tilegrid.json` | JSON | every tile: its type, its `R<row>C<col>`, and the rectangle of configuration memory it owns | yes |
| `ECP5/<part>/iodb.json` | JSON | each package: ball → `(row, col, pio)`, and beside them a `pio_metadata` array giving each PIO's **IO bank** | yes, both |
| `ECP5/<part>/globals.json` | JSON | the clock quadrants, spines and taps | **no** — nothing here routes a clock |
| `ECP5/tiledata/<type>/bits.db` | line-oriented text | one tile type's configuration bits: `.mux`, `.config`, `.config_enum`, `.fixed_conn` | yes, all 185 |
| `ECP5/timing/speed_<n>/*.json` | JSON | cell and interconnect delays | **no** — nothing here does timing |

The `bits.db` files are **shared by the whole family**, one per tile type,
not one per part — which is why the pinned subset has all 185 of them and
only one part's three per-part files.

### `bits.db`, which is the interesting one

Four record kinds, and `libtrellis`' own loader
(`libtrellis/src/BitDatabase.cpp`) accepts exactly these four and throws
on anything else:

```
.mux <sink>                 a programmable connection; then one line per
<source> F1B2 F6B3          source with the bits that select it
                            (a source with no bits is not a connection:
                            it is the state when nothing drives the mux)

.config <name> <default>    a multi-bit field, MSB first in the default
!F25B10                     string; then one bit group per bit, **bit 0
!F24B10                     first**, so the first line is the *last*
                            character of the default

.config_enum <name> [dflt]  a field with named settings
0 F25B10
1 F2B3 F4B2
JA0 -                       `-` is the empty group: this value is
                            "none of the other patterns"

.fixed_conn <sink> <source> a connection that is simply there
```

A bit is `F<frame>B<bit>` **within its own tile**, and a leading `!` means
the feature wants it **clear**. Reticle drops the inverted ones rather
than recording them, because a bitstream is assembled by setting bits in a
zeroed bitmap; `TrellisDatabase::locate_enum` says why that is only safe
while nothing else writes to the same tile.

Two names in `tilegrid.json` are the other way round from what they look
like, and getting them backwards would move every bit of the part:
**`cols` counts frames and `rows` counts bits within a frame.**
`libtrellis`' `Database.cpp` reads them as `num_frames` and
`bits_per_frame`, and `src/fpga/trellis/parse.rs` follows it.

### The bitstream container

Documented, unusually well, in `docs/architecture/bitstream_format.rst`
in `prjtrellis` itself, and implemented in `libtrellis/src/Bitstream.cpp`.
`src/fpga/ecp5.rs` is Reticle's reader and writer and its header has the
layout; the parts worth naming here are the ones that are not guessable:

- the check word is **CRC-16/BUYPASS** — width 16, polynomial `0x8005`,
  initial value zero, no reflection, no final exclusive-or — written high
  byte first, and a `0xFF` dummy command does not enter it;
- for an ECP5 there is a fresh check word **after every frame**, which is
  what the `0x91` operand byte of the payload command means: check words
  on, per frame rather than at the end, one dummy byte after each;
- **frames are written from the last to the first**;
- **a frame's bits are packed from its last byte upwards**, so a frame is
  a big-endian integer whose least significant bit is bit 0. Getting this
  backwards mirrors every frame and no structural check would notice.

Reticle's reader and writer are checked against three bitstreams Great
Scott Gadgets built for this very board with `ecppack`, and each comes
back **byte for byte identical**:

| File | Bytes | Set bits | Block RAM blocks |
|---|---|---|---|
| `analyzer.bit` | 238 282 | 250 001 | 9 |
| `selftest.bit` | 112 303 | 27 006 | 0 |
| `facedancer.bit` | 399 402 | 424 160 | 44 |

They are not in this repository — they are 750 kB of somebody else's build
output, and what they are for here is a cross-check, not an input. They
ship in the `cynthion` Python package under
`cynthion/assets/CynthionPlatformRev1D4/`; point `RETICLE_ECP5_REF` at a
directory holding them and `tests/fpga_trellis.rs` reads them, or skips
with that sentence if it cannot.

## The part, measured

| | |
|---|---|
| Part | LFE5U-12F, IDCODE `0x21111043` |
| Package | caBGA-256 (`CABGA256` in `iodb.json`) |
| Tile grid | 73 columns × 51 rows, every position occupied |
| Tiles | 4312, of 134 distinct Lattice types |
| Positions holding more than one tile | 480 |
| Most tiles at one position | 6 |
| `Arch` tile types | 129, one per composition of a position |
| Configuration memory | 7562 frames × 592 bits = **4 476 704 bits** |
| Bytes per frame | 74 |
| Balls in the caBGA-256 map | 197 |
| Pads this backend declares | 56, all on the top edge |

`tests/fpga_trellis.rs::the_database_describes_one_part_of_the_ecp5_family`
asserts every one of those, so the table cannot drift from the database.

**The LFE5U-12F and the LFE5U-25F are the same die.** Their three
per-part files in `prjtrellis-db` are byte-identical, their `devices.json`
entries differ in the `idcode` field and in nothing else, nextpnr loads
one `chipdb-25k.bin` for both, and Lattice's own TN-02039 Table B.4 gives
both 7562 frames of 592 bits. Only the identifier tells them apart, which
is why `TrellisFabric::check_idcode` is not a nicety: a bitstream from the
wrong half of the database would configure the part and assert `DONE`.

## Where a pad's bits are, and how that was established

This is the part of an ECP5 that no amount of reading a database settles,
and it is where a wrong answer is least visible: a bitstream with a pad
configured one column over still loads and still asserts `DONE`.

A PIO on the top edge has its configuration in **three tiles at two grid
positions**, plus one bit in a fourth tile that is not near it at all:

| What | Tile type | Row |
|---|---|---|
| the pad: IO standard, drive, pull, clamp | `PIOT0` (side A) / `PIOT1` (side B) | 0 |
| a second copy of the IO standard, and the output data mux | `PICT0` / `PICT1` | 1 |
| the constant that can tie the data wire | `CIB` | 1 |
| **the bank's VCCIO rail** | `BANKREF<bank>` | 0, at the end of the edge |

and the **column** differs by side: side A's three tiles are at the column
`iodb.json` gives the ball, side B's are one column east of it. That is
nextpnr's own rule (`get_pio_tile` and `get_pic_tile` in its ECP5
backend).

It was **checked, not taken on trust.** All six of this board's FPGA LEDs
are outputs in its own analyzer gateware, so every bit Reticle sets to
make one of those balls an output can be compared, at absolute frame
positions, with the bits `ecppack` set for the same ball. All 48 agree:

| LED | Ball | Side | Column | Pad tile bits | Tile-south bits |
|---|---|---|---|---|---|
| 0 | E13 | B | 63 | F6610B0 F6615B0 F6617B0 F6623B0 F6624B0 F6625B0 | F6662B12 F6663B12 |
| 1 | C13 | B | 61 | F6398B0 F6403B0 F6405B0 F6411B0 F6412B0 F6413B0 | F6450B12 F6451B12 |
| 2 | B14 | A | 67 | F7034B0 F7039B0 F7041B0 F7047B0 F7048B0 F7049B0 | F7086B12 F7087B12 |
| 3 | A15 | B | 68 | F7140B0 F7145B0 F7147B0 F7153B0 F7154B0 F7155B0 | F7192B12 F7193B12 |
| 4 | D12 | A | 58 | F6080B0 F6085B0 F6087B0 F6093B0 F6094B0 F6095B0 | F6132B12 F6133B12 |
| 5 | C11 | B | 50 | F5232B0 F5237B0 F5239B0 F5245B0 F5246B0 F5247B0 | F5284B12 F5285B12 |

That is
`tests/fpga_trellis.rs::the_led_pads_are_where_this_boards_own_gateware_has_them`,
and it is a green test rather than a paragraph.

The **ties** are the one thing that comparison cannot check, because the
gateware routes a signal into the data wire where this backend ties a
constant, so both of its constant-mux bits are clear there. The test
asserts that too, so the comparison stays honest about what it covers.

The constants are `CIB.JA0MUX` for the output data and `CIB.JB0MUX` for
the output enable, each with the values `0`, `1` and the wire's own name.
Which wire that is comes from the pad tile's own fixed connections:
`JPADDOA <- S1_JA0` and `JPADDOB <- S1E1_JA0`, so each side reads the
`JA0` of the tile one row south of its pad tile — which is the position
this backend puts the rest of the side's bits at. The output enable is
tied low so the buffer drives; an `OUTPUT_*` base type is believed to
ignore it, and tying it costs one bit and removes the question.

### The bank's rail, which is nowhere near the pad

This is the bit that was missing when the LEDs were dark, so it is worth
being exact about where it is and how its place was established.

`BANK.VCCIO` is a `.config_enum` of the `BANKREF<n>` tile types, and there
are seven such tiles on this die: one for each of banks 0, 1, 2, 3, 6 and
7, and `BANKREF8`, which is also where the part's sysconfig settings live.
`BANKREF1` owns all six LEDs and is the single tile at grid **(69, 0)**;
bank 1's top-edge pads have their tiles at columns 33 to 68, so it is
between one and thirty-six columns east of the pad it is configuring. Its `3V3` setting is
one bit, `F18B0` within the tile.

Three things had to be right and each has a source:

1. **Which bank a pad is in.** `iodb.json` has a `pio_metadata` array
   beside `packages`, one entry per `(row, col, pio)` of the die with its
   `bank`. All six LED balls come back bank 1, which is what
   `testdata/fpga/cynthion/leds.rcf` says and what the board's
   `bank0_1.kicad_sch` implies. `parse::pio_banks` reads it; nothing
   infers a bank from a column, and a pad whose bank the database does not
   state is left out of the ball map rather than placed and configured
   incompletely.
2. **Which value.** `bank_voltage` is nextpnr's `get_vccio` table
   (`ecp5/pio.cc`), so `LVCMOS33` is `3V3`. The one oddity is Lattice's:
   a 1.35 V bank is programmed as `1V2`.
3. **That those really are the bits.** Checked the same way the pad tiles
   were: against Lattice's own packer. All three of Great Scott Gadgets'
   bitstreams set exactly that absolute frame position, and the test
   asserts it for each of them. It also asserts that bank 0 — the other
   half of the top edge, with no pad in this design — is **not** written,
   because writing a bank a design does not use is not what `ecppack`
   does either.

### Everything Lattice's own packer writes for one of these pads

Finding `BANK.VCCIO` by comparing against three bitstreams is how the gap
was found; it is not a reason to think there is not another. So the
question was asked the other way round as well: **what does nextpnr write
for a plain `OUTPUT_LVCMOS33` pad whose data comes from a constant, in
full?** `write_io`, `tie_cib_signal` and `init_io_banks` in its
`ecp5/bitstream.cc` are the whole answer, and it is six settings:

| Setting | Tile | Written here |
|---|---|---|
| `PIO<s>.BASE_TYPE = OUTPUT_LVCMOS33` | the pad tile | yes, `pad_bits` |
| `PIO<s>.BASE_TYPE = OUTPUT_LVCMOS33` again | the tile one row south | yes, `pic_bits` |
| `CIB.JB0MUX = 0` — the tristate tied low | the `CIB` | yes, `enable_bits` |
| `CIB.JA0MUX = 0` or `1` — the constant | the `CIB` | yes, `low_bits` / `high_bits` |
| `BANK.VCCIO = 3V3` | `BANKREF<bank>` | yes, since 2026-09-26 |
| `PIO<s>.PULLMODE`, `HYSTERESIS`, `SLEWRATE`, `DRIVE`, `OPENDRAIN`, `CLAMP`, `TERMINATION`, `DATAMUX_*`, `TRIMUX_TSREG` | various | **no, and neither does nextpnr**: each is behind an attribute or a direction this design does not have, or defaults to the value it would be given |

So after the bank rail there is nothing left that Lattice's own packer
writes for this cell and this does not. That is not the same as a
guarantee — two things it does not cover are named below — but it means
the next doubt is not "another missing pad bit".

The two things it does not cover:

- **nextpnr's base configuration.** `ecp5/baseconfigs.cc` starts every
  bitstream from a non-empty `ChipConfig`: for this die, 166 settings in
  twenty-one tiles, all of them `VCIB_DCU*`, `CIB_DCU*`, `CIB_PLL3`,
  `CIB_EFB*`, `CMUX_*`, `EFB0_PICB0` or `DSP_SPINE_UL1`. **None of it is in
  a `PIO`, `PICT` or `BANKREF` tile** — it is the SERDES, the PLLs, the
  embedded function block and the clock multiplexers — and this backend
  writes none of it. The part accepts a bitstream without it and asserts
  `DONE`, so nothing there is required to finish configuration; whether any
  of it matters to a pad is unknown and was not assumed either way. (The
  12F has no entry of its own there, because nextpnr treats it as the 25F
  it shares a die with.)
- **The `.config_enum` bit values themselves**, which come from Project
  Trellis' fuzzing rather than from Lattice. Those are as trustworthy as
  `ecppack` is, and the two settings that could be checked against a real
  `ecppack` output — the base type and the bank rail — were.

### Where the pin numbers came from

Three independent sources, and they agree:

1. **Great Scott Gadgets' own Amaranth platform file**,
   `cynthion/python/src/gateware/platform/cynthion_r1_4.py` in
   `greatscottgadgets/cynthion`:

   ```python
   # FPGA LEDs
   *LEDResources(pins="E13 C13 B14 A15 D12 C11",
                 attrs=Attrs(IO_TYPE="LVCMOS33"), invert=True),

   # Primary, discrete 60MHz oscillator.
   Resource("clk_60MHz", 0, Pins("A8", dir="i"),
       Clock(60e6), Attrs(IO_TYPE="LVCMOS33")),
   ```

   in that order, so `led[0]` is E13. `invert=True` is the polarity:
   Amaranth's `PinsN` is `Pins(..., invert=True)`, so driving one means
   pulling the pad low.
2. **The r1.4.0 KiCad schematics** in
   `greatscottgadgets/cynthion-hardware`, whose `main` is that tag. In
   `indicators_buttons.kicad_sch` all six anodes sit on one wire up to the
   `+3V3` symbol and each cathode goes through a series resistor to nets
   `LED0`..`LED5` (refdes D7 D6 D5 D4 D3 D2) — **anode to the supply,
   cathode to the FPGA, so active low**, confirming `invert=True`
   independently. `clock_misc.kicad_sch` has the oscillator: Y1, a SiTime
   `SIT1602BC-23-33E-60.000000E`, 3.3 V, 60.000000 MHz, to A8.
3. **Project Trellis' `iodb.json`** for the caBGA-256, which puts all six
   balls on row 0 — the top edge — and in bank 1, and puts A8 on row 0
   bank 0 with the dedicated clock function `PCLKC0_0`. This is also where
   the grid columns in the table above come from.

Bank 1's VCCIO is 3.3 V, hence LVCMOS33: `bank0_1.kicad_sch` has only
`+3V3` and `GND` power symbols.

What is **not** verified: that the net named `LED0` in the schematic is
the one silkscreened `0` on the board. The platform file's order, the net
names and Great Scott Gadgets' own blinky tutorial
(`platform.request("led", n) for n in range(0, 6)`) all line up, and the
PCB silkscreen layer was not opened to prove it. If a person looking at
the board sees the alternating design light 1, 3 and 5 rather than 0, 2
and 4, that is what was wrong.

The device file says all of this in `src/fpga/devices/ecp5.dev`, under
`device ecp5-12f-CABGA256`, with `pins partial` set because nine of 256
balls are named. **A device file cannot express polarity**, so the designs
drive zero and say in their headers that zero is lit.

## What was not needed, and why that is the news

The placer, the router and `src/fpga/bitstream.rs` needed **no edits**,
which is the same thing `docs/fpga-xray.md` and `docs/fpga-gowin.md` each
report for their vendor. One model change was needed, and exactly one:

**`NetPin::constant`.** Before it, `src/fpga/place.rs` recorded that a pin
was driven by a constant rather than by a signal, and did not record
*which* constant. Nothing had needed to know: neither Gowin nor the 7
series had ever compiled a design with a constant-driven pin, because both
have interconnect and their designs route something. An ECP5 pad tied to a
fixed zero or one is the first case where the fabric has to be told which,
so `NetPin` gained `constant: Option<Bit>`. `src/fpga/route.rs`'s share of
that change is three `constant: None` lines in its own test fixtures and
nothing else — a constant is still not a signal and the router still never
sees one.

Where the tie is *materialised* is not the router and not
`bitstream::generate`: it is `TrellisFabric::configure_io`, which walks the
placement and sets the bits. That is the same division
`apicula::Periphery` makes, for the same reason — the bits are in tiles the
bel does not own — and it is what keeps `src/fpga/bitstream.rs` at zero
edits for a third vendor.

## What remains

| What | What it needs |
|---|---|
| Anything routed at all | the routing graph: `bits.db`'s `.mux` records are read already (`TileDatabase::arcs`), and what is missing is the *rest* of being right — the wire globalisation rule (`parse::globalise`, written and unexercised), spans, and which of six tiles at a position owns a name |
| A lookup table | the above, plus `PLC2`'s 8 LUT bels and their `INIT` word, which `trellis/sites.rs` describes and nothing uses |
| Anything clocked | the above, plus the global clock network: `globals.json`'s quadrants, spines and taps, the `DCCA` buffers, and the path from a pad to a spine |
| The other three edges' pads | `PICL*`/`PICR*` put four PIOs at a position and `PICB*` two; none of it has been checked against a part, and `TrellisDatabase::load` leaves those balls out of the ball map rather than placing something it would configure nowhere |
| Block RAM | `Ecp5Stream` reads and writes the initialisation blocks — the reference files' 44 blocks round trip — and nothing generates one |
| An IO standard other than LVCMOS33 | the bits are in the database and the code takes the standard from the constraints; no other standard has been on a part |
| An ECP5 over an FTDI cable | nothing, in principle: the configuration plans are transport-neutral and `jtag::Scan` encodes them for MPSSE. It is refused because that pairing has never been run |

## The two things worth knowing if you change this

**The order of a position's windows is the order Project Trellis' tile
names sort in.** `parse::tilegrid` sorts by key and
`TrellisDatabase::load` pushes in that order, and a `ConfigBit`'s `row`
addresses them end to end. Change the sort and every bit of a shared
position moves. `CIB_R1C63:CIB` sorts before `MIB_R1C63:PICT1`, which is
why a `PICT1` bit `F54B0` becomes row 106 + 54.

**A pad bel must declare its pins.** It looks as though it need not — the
bits do not come from a bel — but `Netlist::build` records no `NetPin` for
a pin whose role names no wire this `Arch` declares, so the constant behind
an output pad would never reach `configure_io`. That is how it was found:
two designs driving different constants produced the same 60 bits. The
three wires are real ones from the pad tile's fixed connections, and
declaring them adds three nodes and no pips, so a design with something to
route still cannot be built.
