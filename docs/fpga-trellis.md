# A real Lattice ECP5, and a real `.bit`

## A design that routes has been built, and loaded into a part

On 2026-09-26 `testdata/fpga/cynthion/button_led.v` — the Cynthion's USER
button, through about thirty rows of the die and a lookup table, to two of
its LEDs — was compiled by this flow and loaded into the LFE5U-12F of the
Great Scott Gadgets Cynthion r1.4 attached to the machine this was written
on. **The part accepted it and asserted `DONE`** with no fault bit set:
status `0x00200100`, the same as every run before it, which is exactly why
`DONE` is not the claim being made. The board went on enumerating at the
same USB address afterwards.

What the flow reports for it:

```
114 configuration bit(s) set, 7 pad(s) and 1 lookup table(s) configured,
7/120 io, 1/24288 lut
routed 2 of 2 signal(s) with 23 pip(s) over 25 wire(s), and every sink was
walked back to its driver
```

**Whether the LEDs do what the design says has not been watched.** It
needs a person, for the reason the rest of this file exists, and the
person has not been asked yet. What they should press and what they should
see is in `button_led.v`'s header and repeated at the end of this section.

### What a person should look for

Press and hold the button silkscreened **`USER`**. There are three
tactile buttons and the other two end the experiment instead of performing
it: `PROG` reloads the FPGA from flash and `RESET` resets the debug
microcontroller. On the r1.4.0 layout `USER` and `PROG` are on the same
edge of the board and `RESET` is on the opposite one.

Watch the two LEDs at the end of the `FPGA LEDs` row **farthest from that
button**:

| | far end | next to it | the other four |
|---|---|---|---|
| nothing touched | dark | **lit** | dark |
| button held | **lit** | dark | dark |

They swap back when it is let go, they are never both lit and never both
dark, and the other four never light.

**The board does not number its LEDs**, which is worth saying because two
designs in `testdata/fpga/cynthion/` used to claim it does. The r1.4.0
silkscreen has one legend, `FPGA LEDs`, over the row and no digits; the row
is `D2` to `D7` in `cynthion.kicad_pcb` and the schematic wires `led_n[0]`
to `D7`, which is the end away from the button. So "the far end" is
`led_n[0]`, derived from the layout rather than read off the board.

That also makes this design the first that can settle something the earlier
ones could not. `leds.v` lights all six and `leds_alternate.v` lights
alternate ones, and both look the same read from either end; here only two
LEDs move, so which two they are decides the order.

### Somebody looked, and it does what it says

On 2026-09-26 the board's owner reported: one LED lit with nothing
touched, and on holding `USER` **that one goes dark and its neighbour
lights**. They swap, and nothing else moves.

So **a routed design works on this part**. Two signals, twenty-three
programmable connections, a pad in through a lookup table to a pad out,
built by this flow from Verilog and configured over the board's own debug
microcontroller with no vendor tool at any step.

It also settles the numbering, in favour of the schematic: the LED that
lights when the button is held is the one at the end **away** from the
button, which the layout calls `led_n[0]`. Four agreeing files were right
and the reversed reading is ruled out.

Worth recording what this run did *not* need. The two bugs that made
earlier runs look fine and behave wrongly — a bank's `BANK.VCCIO`, and an
input pad's `PULLMODE`, whose database default fights this board's pull-up
and holds a released pin below `VIH` — were both found before the board
was asked, by reading what Lattice's own packer writes *in full* for a
cell rather than diffing against it. Neither would have been caught by any
check on this machine, and `DONE` was high either way.
Either answer is a result.

The observation is deliberately a strong one. One LED lit rather than six
cannot be confused with the previous milestone; the lit one *moves* when a
finger moves, which no configuration that does nothing can do; and it moves
both ways, so a stuck input shows up as one of the two never changing rather
than as nothing happening.

### What was checked instead, since `DONE` is not evidence

Three things, in order of how much they are worth.

**1. The bitstream was read back through Project Trellis' own database,
and it says what the router said.** `TrellisDatabase::decode` matches every
`.mux`, `.config` and `.config_enum` record of every tile against the bits,
and
`tests/fpga_trellis.rs::the_bitstream_decodes_back_to_the_arcs_the_router_chose`
asserts two things about the result.

*Every set bit belongs to something.* All 114, over 31 tiles, with
**nothing left over**. A leftover bit is a bit this crate set for a reason
the database does not know, which is what a wrong tile rule looks like from
the inside.

*The arcs come back the same.* The set of connections the bits select,
resolved back into the same wires and positions the router works in, is
**exactly** the set the router chose. That is the check worth having,
because the bits of one tile are shared: `CIB.JA0MUX` and the `.mux JA0`
that routes into the same wire are literally one mux, and the bits a
feature wants *clear* are dropped rather than written, so a second feature
written into a tile can silently change what a first one selects. Nothing
structural would notice; this does.

What the decoding says, in full: the button as an `INPUT_LVCMOS33` with
`HYSTERESIS=ON` and `PULLMODE=NONE`; the six LED pads as
`OUTPUT_LVCMOS33` with their tristates tied low, four tied high (dark) and
two left for the router; `BANK.VCCIO = 3V3` on `BANKREF1` and `BANKREF3`;
one `SLICED.K0.INIT = 1010101010101010` — `~A`, read bit 0 first — with
`SLICED.B0MUX`, `C0MUX` and `D0MUX` tied to 1 and `A0MUX` left for the
route; and the arcs that make one path from the button to the lookup table
and one from the table to a LED.

A pip with **no** bits cannot be checked this way, because it leaves no
trace: the fixed connections and the bitless mux sources are eight of the
twenty-three pips the design takes. `Routing::verify` is what covers those,
by walking each sink backwards through the pips it was given until it
reaches the driver.

**2. The pads were compared against bitstreams Lattice's own packer wrote
for this very board**, the same way the LEDs were for the previous
milestone, and the comparison is what made the right edge describable at
all. See "Where a pad's bits are".

**3. The question was asked the other way round as well** — not "what
differs" but "what does `ecppack` write, in full, for this kind of thing"
— for the pad, for the lookup table and for an arc. See "Everything
Lattice's own packer writes".

### What this settles, and what it does not

It settles that the chain from Verilog to a *routed* configuration a real
ECP5 accepts closes, and that every bit of it is one the database explains.
It settles nothing about whether the board's LEDs follow the button, and
the difference between those two sentences is the whole lesson of the
section below.

It also settles nothing about **anything clocked**, and that is still a
gap in the backend rather than in the measurement: `globals.json` is not
read, so there is no clock network, no `DCCA` and no path from a pad to a
clock spine. The table at the end says what each remaining thing needs.

## The first design, and the lesson it cost

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

Every run in this file is reproducible, and the current milestone is the
last of the three:

```sh
reticle fetch prjtrellis-db
for design in leds leds_alternate button_led; do
    reticle fpga testdata/fpga/cynthion/$design.v \
        --device ecp5-12f-CABGA256 \
        --constraints testdata/fpga/cynthion/$design.rcf \
        --bitstream /tmp/$design.bit
done
reticle program --list                         # find the board's serial
reticle program --device <serial> /tmp/button_led.bit
```

(`leds_alternate.v` shares `leds.rcf`; the loop is shorthand.)

What that writes is **only the volatile configuration SRAM**. A power
cycle reloads the part from the board's flash, which nothing in this
project touches; `docs/programming.md` and `src/program/lattice.rs`'s
`NOT_SHIFTED` are where that is made checkable rather than promised.

### What that first run settled

That the chain from Verilog to a configuration a real ECP5 accepts closes:
the frontend, synthesis, IO primitive mapping, the placer's pin
constraints, the frame map, the `.bit` container, its 7562 check words, and
the JTAG configuration sequence.

It settled **nothing about routing or about logic**, and at the time that
was a gap in the backend and not in the measurement: `src/fpga/trellis`
declared no interconnect at all, and a design with anything to route was
refused by name before a file was written. That is what the section at the
top of this file changes, and the interconnect is described under "What the
routing graph is".

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
| `ECP5/<part>/globals.json` | JSON | the clock quadrants, spines and taps | **no** — nothing here routes a clock, and this is the one file a clocked design needs |
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
<other> -                   (a source with **no** bits is the state the mux
                            is in with none of them set, and that is still
                            a connection — see "A `.mux` source with no
                            bits is a connection" below, which is where
                            this was got wrong)

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
zeroed bitmap; `TrellisDatabase::locate_field` says why that is only safe
while nothing else writes to the same tile, and "A bit a feature wants
clear is dropped" below says where the exception lives.

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

## What the routing graph is, and the three rules that decide it

Project Trellis hands over `.mux` and `.fixed_conn` records per tile
*type*. Turning them into a graph the router can use takes three rules the
files imply and do not state, and each of them is the kind of rule that
produces a bitstream which loads, asserts `DONE` and routes a net through
metal that is not there.

### A wire belongs to the position a prefix points at

`S1E1_JA0` in a `PIOT0`'s `bits.db` is not a wire of that tile. It is the
`JA0` of the tile one row **south** and one column **east**; `N` decreases
the row, `S` increases it, `W` decreases the column and `E` increases it.
`libtrellis`' `RoutingGraph::globalise_net_ecp5` is the rule and
`parse::globalise_ref` follows it, together with the two other cases it
has: a `25K_`/`45K_`/`85K_` prefix means a wire only some dice have, and a
`G_` name is one node for the whole part except for the clock spines and
branches, which are per tile after all.

Two consequences.

**There are no join pips.** `super::xray` spends a third of its graph
edges declaring that one tile's wire is the same metal as its neighbour's,
because a 7-series wire has a different name in every tile it crosses.
Here the mapping is arithmetic, so a wire is declared once by the position
that owns it and every reference resolves straight to it. Every wire has
span `0 0`.

**The loader cannot decide a tile type's wire list by reading that type.**
A `CIB+PICT1` has to declare `JA0` because a `PIOT0` one row north and one
column west says `S1E1_JA0`, and nothing in the `CIB`'s own `bits.db` need
mention the name at all. So the loader walks every reference of every tile
of the die first — about a million and a half of them — and gives each name
to the *composition* the reference lands on. Doing it the other way round
leaves thousands of references resolving to nothing, and a reference that
resolves to nothing is a pip that silently vanishes. 3840 references do
point off the grid, which is what happens at the four edges of the die and
is not an error.

### A `.mux` source with no bits is a connection

This one was got wrong, and it is worth writing down because the mistake
was reasonable and the consequence was total.

```text
.mux F0
F0_SLICE -
F5A_SLICE F8B10
```

`F0` is a routing wire of a logic tile and `F0_SLICE` is a lookup table's
output. The first source has no bits, which reads like "the state of the
mux when nothing drives it" — and that reading is what the loader carried
at first, on the strength of the four `…BOUNCE` sources, which really are
that. It is wrong. With `F8B10` clear the mux carries the lookup table;
`F8B10` set switches it to the carry chain's cascade output. So dropping
the bitless source disconnects **every lookup table in the part** from the
fabric, and a bitless source becomes a pip with an empty bit list, which is
what Reticle's `Arch` already means by a connection that is always there
and what `ecppack` writes for one (`ChipConfig::add_arc` of a bitless arc
sets no bits).

It was found before a line of Rust was written, by walking the graph in a
throwaway script from the button's pad forward and from a LED's pad
backward and intersecting: the backward walk reached `F0` and never
`F0_SLICE`, and no lookup table on the die was in both sets. The
`…BOUNCE` sources stay harmless under the corrected rule because no `.mux`
has a `…BOUNCE` as its *sink*, so nothing drives one and a router can never
route through one.

### A bit a feature wants **clear** is dropped, and here is where that is safe

A bitstream is assembled by setting bits in a zeroed bitmap, so `!F25B10`
is a bit already in the state the feature wants and there is nothing to
write. That is only sound while no two features written into one tile
disagree about a bit — and a `.mux` source with an inverted bit is exactly
a feature that could disagree, because taking such an arc leaves a bit set
that another source of the same mux wanted clear.

So it matters where those are, and the answer is clean: **every `.mux`
source with an inverted bit is in the clock network's own tiles.** Over the
family's 185 `bits.db` files, 4523 of 85 379 mux source lines have one, and
all 4523 are in `CMUX_LL_0`, `CMUX_LR_0`, `CMUX_UL_0`, `CMUX_UR_0`,
`LMID_0`, `RMID_0`, `ECLK_L`, `ECLK_R`, `BMID_0H`, `BMID_0V`, `BMID_2`,
`BMID_2V`, `TMID_0` or `TMID_1`, and every one of them drives a clock
global (`G_…PCLK…`, `G_…DCC…CLKI`, `ECLKI…`). The general interconnect —
the `CIB*` tiles, the `PLC2`s and the `PIC*`s that a pad or a lookup table
routes through — has none, so a combinational design cannot reach one.
`tests/fpga_trellis.rs::an_inverted_mux_bit_only_happens_in_the_clock_network`
asserts that from the database rather than from this paragraph. A clocked
design could reach one, and that is one of the things the clock network will
have to deal with.

### What it comes to

| | |
|---|---|
| Global wires (one node for the die) | 467 |
| Tile wires | 1 095 958 |
| Graph nodes | 1 096 425 |
| `.mux` sources declared, over 129 tile types | 171 632 |
| `.fixed_conn`s declared | 14 614 |
| Graph edges kept | 8 211 900 |
| Graph edges dropped at the edges of the die | 53 632 |
| Distinct bit patterns, interned | 3536 |
| `RoutingGraph::heap_bytes` | 344 MiB |
| Time to build, release | under a second |
| `lut` sites | 24 288 |
| `io` sites | 120 |

Those are `tests/fpga_trellis.rs::the_database_describes_one_part_of_the_ecp5_family`'s
assertions, so the table cannot drift from the database.

**The whole die fits, and that is the surprise.** `super::xray` needs a
region option and a pip limit because an `xc7a50t` is 30.9 million edges
and 1386 MiB; this is a quarter of the edges and a quarter of the memory,
so there is no region option here and `reticle fpga --bitstream` loads
everything. The reason is the rule above: no join pips, and a wire is one
node however far it reaches.

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
| Pads this backend declares | 120: 56 on the top edge and 64 on the right |
| Balls it leaves out | 77, on the left and bottom edges |
| Graph nodes | 1 096 425 |
| Graph edges | 8 211 900 |
| `lut` sites | 24 288 |

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

### The buffer is not where the bits are, and both edges say so differently

Before anything routed, "a pad" meant a set of bits, and the bel was put
wherever they were. A routed design makes the distinction unavoidable,
because a bel's pins resolve in the tile the bel sits in and every
top-edge position has a `PADDOB_PIO` of its own. So:

- **the buffer** — the three wires a PIO presents to the fabric,
  `PADDO<L>_PIO`, `PADDT<L>_PIO` and `JPADDI<L>_PIO` — is at the position
  `iodb.json` gives the ball, and that is where the `io` bel goes;
- **the bits** are wherever the edge's rule puts them, which on the top
  edge is a column east for side B and on the right edge is a row south for
  every side.

| | Top edge | Right edge |
|---|---|---|
| Sides per position | A, B | A, B, C, D |
| Buffer | the ball's `(col, row)`, `PIOT0` | the ball's `(col, row)`, `PICR0*` |
| Pad tile: standard, hysteresis, pull | `(col + [B], 0)`, `PIOT0` / `PIOT1` | `(col, row + 1)`, `PICR1*` |
| Second copy of the standard | `(col + [B], 1)`, `PICT0` / `PICT1` | A, B: `(col, row)`, `PICR0*`; C, D: `(col, row + 2)`, `PICR2*` |
| `CIB` that ties the data and enable | `(col + 1, 1)`, wire `JA0` / `JB0` | `(col - 1, row)` for A, B and `(col - 1, row + 2)` for C, D, wire `JA0`/`JA3` |
| Bank rail | `BANKREF<bank>`, anywhere | same |

The `CIB` row of that table is **not hardcoded**. Nothing in the loader
knows that `JA0` is the top edge's answer: it reads the buffer's own
`.fixed_conn` — `JPADDOB <- S1E1_JA0` on the top edge, `JPADDOD <-
S2W1_JA3` on the right — resolves the direction prefix, and builds the
field name `CIB.<wire>MUX`. That is what nextpnr does too, by walking the
pips uphill of the wire, and it is what lets one piece of code serve both
edges. Nor are the tile *types* hardcoded: the loader asks which tile of a
position declares `PIO<L>.BASE_TYPE`, which gets the right answer through
all twelve spellings of the right edge (`PICR1`, `PICR1_DQS0`,
`PICR1_DQS3`, `PICR2`, `PICR2_DQS1`, `MIB_CIB_LR_A`, …) where nextpnr
carries a set of names per edge, and **refuses** rather than guessing if
two tiles of one position both declare it.

The left and bottom edges are still left out, and their balls are left out
of the ball map rather than placed and configured wrongly. The left edge is
the right edge mirrored and would probably work; "probably" is why it is
not there.

### The top edge, checked against this board's own gateware

All six of this board's FPGA LEDs are outputs in its own analyzer
gateware, so every bit Reticle sets to make one of those balls an output
can be compared, at absolute frame positions, with the bits `ecppack` set
for the same ball. All 48 agree:

| LED | Ball | Side | Buffer | Bits at | Pad tile bits | Second-copy bits |
|---|---|---|---|---|---|---|
| 0 | E13 | B | col 62 | col 63 | F6610B0 F6615B0 F6617B0 F6623B0 F6624B0 F6625B0 | F6662B12 F6663B12 |
| 1 | C13 | B | col 60 | col 61 | F6398B0 F6403B0 F6405B0 F6411B0 F6412B0 F6413B0 | F6450B12 F6451B12 |
| 2 | B14 | A | col 67 | col 67 | F7034B0 F7039B0 F7041B0 F7047B0 F7048B0 F7049B0 | F7086B12 F7087B12 |
| 3 | A15 | B | col 67 | col 68 | F7140B0 F7145B0 F7147B0 F7153B0 F7154B0 F7155B0 | F7192B12 F7193B12 |
| 4 | D12 | A | col 58 | col 58 | F6080B0 F6085B0 F6087B0 F6093B0 F6094B0 F6095B0 | F6132B12 F6133B12 |
| 5 | C11 | B | col 49 | col 50 | F5232B0 F5237B0 F5239B0 F5245B0 F5246B0 F5247B0 | F5284B12 F5285B12 |

That is
`tests/fpga_trellis.rs::the_led_pads_are_where_this_boards_own_gateware_has_them`,
and it is a green test rather than a paragraph.

The **ties** are the one thing that comparison cannot check, because the
gateware routes a signal into the data wire where this backend ties a
constant, so both of its constant-mux bits are clear there. The test
asserts that too, so the comparison stays honest about what it covers.

That comparison also cannot tell a tie from a route, and the reason is
worth knowing: **`CIB.JA0MUX` and the `.mux JA0` that routes into the same
wire are one mux.** `CIB.JA0MUX` offers `0`, `1` and `JA0`, where `JA0`
means "whatever the routing mux selects", so the bits that tie a wire and
the bits that route into it live in the same bit space. A finished
bitstream therefore cannot be asked "is this pad tied?" — the tie's pattern
may be a subset of the route's. What can be asked is what the pass wrote,
and `tests/fpga_trellis.rs` runs `configure_io` into an empty bitmap of its
own to ask exactly that.

### The right edge, checked against the one bitstream that reads the button

Of Great Scott Gadgets' three bitstreams for this board, exactly one has a
design that reads the USER button: `facedancer.bit`, whose top level asks
for `button_user` through its `ButtonProvider`. So it is the one file that
says where an input on the right edge is configured, and it agrees with
this backend bit for bit.

M14 is `(row 32, col 72, PIO D)` in `iodb.json`, bank 3. What Reticle
writes for it, and what `facedancer.bit` has at the same absolute frames:

| Setting | Tile | Position | Bits |
|---|---|---|---|
| `PIOD.BASE_TYPE = INPUT_LVCMOS33` | `PICR1_DQS3` | (72, 33) | F29B394 F30B394 F30B395 F31B394 F34B395 |
| `PIOD.HYSTERESIS = ON` | `PICR1_DQS3` | (72, 33) | F30B395, which the base type also wants |
| `PIOD.PULLMODE = NONE` | `PICR1_DQS3` | (72, 33) | F26B394, with F35B395 clear |
| `PIOD.BASE_TYPE = INPUT_LVCMOS33` | `PICR2` | (72, 34) | **none at all**: the pattern is empty |
| `BANK.VCCIO = 3V3` | `BANKREF3` | (71, 50) | F7474B591, with three others clear |

`tests/fpga_trellis.rs::the_user_buttons_pad_is_where_this_boards_own_gateware_has_it`
asserts all of it, and asserts that `facedancer.bit` ties none of M14's
`CIB` wires, because an input should not.

Two of those rows are things the six-LED milestone never needed.

**`PIOD.BASE_TYPE` in `PICR2` costs nothing**, which is lucky rather than
designed: Lattice's own packer writes the field there and Project Trellis
found no bits for it in that tile. So an input on side C or D of the right
edge is five bits and a bank rail.

**`PULLMODE` is not cosmetic, and this is the second `BANK.VCCIO`.** The
field's default in `bits.db` is `DOWN`: with none of its bits set the pin
has an internal pull-down, which is Lattice's own default for a PIO. On
this board the button is a 10 kΩ pull-up to 3.3 V, a normally-open switch
to ground, and a 33 kΩ series resistor with a 1 µF capacitor into the ball
(R106, SW3, R107, C78 of `indicators_buttons.kicad_sch`). An internal
pull-down of the order the ECP5 datasheet gives would divide the released
level to something like 1.4 V across that 43 kΩ, which is below `VIH` for
LVCMOS33 — so the pin would read **low whether or not anybody pressed
anything**, and the design would look dead in one direction and stuck in
the other. Great Scott Gadgets' own platform file asks for
`PULLMODE="NONE"` on that pin for the same reason, `facedancer.bit` has it,
and `configure_io` writes it for every input.

That is the same shape of omission as the bank rail: a default that is
wrong for the board, in a field the design never mentions, with no symptom
a structural check could see.

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

### Everything Lattice's own packer writes, asked cell by cell

Finding `BANK.VCCIO` by comparing against three bitstreams is how that gap
was found; it is not a reason to think there is not another. So the
question is asked the other way round as well, for every kind of thing this
backend now writes: **what does nextpnr write for one of these, in full?**
`write_io`, `write_comb`, `tie_cib_signal`, `set_pip` and `init_io_banks` in
its `ecp5/bitstream.cc` are the whole answer.

**An output pad whose data is a constant** — six settings:

| Setting | Tile | Written here |
|---|---|---|
| `PIO<s>.BASE_TYPE = OUTPUT_LVCMOS33` | the pad tile | yes, `output_pad_bits` |
| `PIO<s>.BASE_TYPE = OUTPUT_LVCMOS33` again | the second-copy tile | yes, `output_pic_bits` |
| `CIB.JB0MUX = 0` — the tristate tied low | the `CIB` | yes, `enable_bits` |
| `CIB.JA0MUX = 0` or `1` — the constant | the `CIB` | yes, `low_bits` / `high_bits` |
| `BANK.VCCIO = 3V3` | `BANKREF<bank>` | yes, since 2026-09-26 |
| `PIO<s>.PULLMODE`, `HYSTERESIS`, `SLEWRATE`, `DRIVE`, `OPENDRAIN`, `CLAMP`, `TERMINATION`, `DATAMUX_*`, `TRIMUX_TSREG` | various | **no, and neither does nextpnr**: each is behind an attribute or a direction this design does not have, or defaults to the value it would be given |

**An output pad a route drives** is the same list with the constant left
out, which is the point: `configure_io` ties `CIB.JA0MUX` only when the
netlist gives the pin a constant, because the tie and the route share one
mux and writing both would be writing two drivers.

**An input pad** — `write_io` again, with `dir == "INPUT"`:

| Setting | Written here |
|---|---|
| `PIO<s>.BASE_TYPE = INPUT_LVCMOS33`, in both tiles | yes |
| `PIO<s>.HYSTERESIS = ON` — the attribute's own default | yes |
| `PIO<s>.PULLMODE` — only when an attribute asks | **yes, always**, and the paragraph above says why this is the one place this is deliberately broader than nextpnr |
| the tristate tie | **no**, and nextpnr skips it for an input too |
| `BANK.VCCIO` | yes |

**A combinational lookup table** — `write_comb`:

| Setting | Written here |
|---|---|
| `SLICE<l>.K<n>.INIT` — the truth table | yes, `configure_logic` |
| `SLICE<l>.<X><n>MUX = 1` for every input no pip reaches | yes |
| `SLICE<l>.MODE = LOGIC` | nothing to write: it is the default and costs no bits |
| `SLICE<l>.CCU2.INJECT1_<n> = _NONE_` | nothing to write: `_NONE_` means "no bits", which is what nextpnr uses it for — the bit is shared with the cascade mux and it deliberately leaves it alone |
| `WREMUX`, `CLK1.CLKMUX` | only for `DPRAM` mode, which this does not build |

**An arc** — `set_pip` is two lines: it looks up the tile and calls
`add_arc(sink, source)`. Nothing per wire, nothing per net, no enables.
That is worth stating flatly because the 7 series is not like that: there,
touching a clock-row wire costs a buffer enable that is neither a pip's nor
a bel's, and `super::xray` carries a whole mechanism for it. Here the mux
bits are the whole of an arc.

So there is nothing left that Lattice's own packer writes for any cell this
builds and this does not. That is not the same as a guarantee — two things
it does not cover are named below — but it means the next doubt is not "a
missing bit for a cell we have".

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
- **The `.config_enum` and `.mux` bit values themselves**, which come from
  Project Trellis' fuzzing rather than from Lattice. Those are as
  trustworthy as `ecppack` is. The settings that could be checked against a
  real `ecppack` output were: the base type on both edges, hysteresis, the
  pull mode, and the bank rail on four banks. **No arc has been checked
  against an `ecppack` output**, because the reference bitstreams route
  different designs and there is nothing to compare an arc against. What
  was checked instead is that every bit of this flow's own bitstream decodes
  back, through the same database, to exactly the arcs the router chose —
  see the top of this file.

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
   pulling the pad low. The USER button is in the same file:

   ```python
   # USER button
   Resource("button_user", 0, PinsN("M14", dir="i"),
            Attrs(IO_TYPE="LVCMOS33", PULLMODE="NONE")),
   ```

   `PinsN` again, so the button is active low: the ball reads low while it
   is held.
2. **The r1.4.0 KiCad schematics** in
   `greatscottgadgets/cynthion-hardware`, whose `main` is that tag. In
   `indicators_buttons.kicad_sch` all six anodes sit on one wire up to the
   `+3V3` symbol and each cathode goes through a series resistor to nets
   `LED0`..`LED5` (refdes D7 D6 D5 D4 D3 D2) — **anode to the supply,
   cathode to the FPGA, so active low**, confirming `invert=True`
   independently. `clock_misc.kicad_sch` has the oscillator: Y1, a SiTime
   `SIT1602BC-23-33E-60.000000E`, 3.3 V, 60.000000 MHz, to A8. The same
   sheet has the button: `SW3`, valued `BTN_USER`, a normally-open SPST
   between ground and a node that `R106` (10 kΩ) pulls up to `+3V3`, with
   `R107` (33 kΩ) in series and `C78` (1 µF) to ground between that node and
   the hierarchical label `~{BTN_USER}`. So **released is high**, through
   43 kΩ, and the RC is a 33 ms debounce, which is why nothing in the fabric
   has to debounce it. That pull-up is also why `PULLMODE` has to be
   written; the pad section above has the arithmetic.
3. **Project Trellis' `iodb.json`** for the caBGA-256, which puts all six
   LED balls on row 0 — the top edge — and in bank 1, puts A8 on row 0 bank
   0 with the dedicated clock function `PCLKC0_0`, and puts **M14 on the
   right edge**: row 32, column 72, PIO D, bank 3. This is also where the
   grid columns in the tables above come from.

Bank 1's VCCIO is 3.3 V, hence LVCMOS33: `bank0_1.kicad_sch` has only
`+3V3` and `GND` power symbols. Bank 3's is too, and there the evidence is
the bitstreams rather than the schematic: all three of Great Scott Gadgets'
set `BANK.VCCIO` to `3V3` on every bank of the part, `BANKREF3` included.

**Confidence in M14, stated plainly, because a wrong pad is a design that
does nothing with no error.** High. Three sources agree on the ball
(platform file, schematic, and the reference bitstream that configures it);
the reference bitstream also fixes every bit of its configuration at
absolute frame positions, which is the same standard the LEDs were held to
and stronger than anything derived from reading a database. What is *not*
independently confirmed is the silkscreen: that the button labelled `USER`
on the board is the one the schematic calls `SW3`. The schematic's other
two switches are `BTN_RESET` and `BTN_PROGRAM`, and those have their own
labels on the board, so the risk is a mislabelled silkscreen rather than an
ambiguity.

4. **The r1.4.0 PCB layout**, `cynthion.kicad_pcb` in the same
   repository, which is where this file used to have an open question and
   now has an answer. It was opened to settle which end of the LED row is
   `led_n[0]`, and the answer came with a correction: **there are no
   per-LED digits on the board at all.** The front silkscreen near the row
   has one legend, `FPGA LEDs`, at (126.2, 109.4); the LEDs are `D2` to
   `D7` at x = 114.5, 117.5, … 129.5, all at y = 112 on a board spanning
   x 104–160; and the schematic's `LED0`..`LED5` are `D7` down to `D2`. So
   `led_n[0]` is `D7`, the end of the row at the higher x — which is the end
   **away from** the USER button (`SW3` at (106.7, 95)) and away from the
   USB-C receptacle on that same edge (`J1` at (106.65, 109)).

   Two designs in `testdata/fpga/cynthion/` said the LEDs were
   "silkscreened 0 1 2 3 4 5". They are not, and they now say so.

What is still **not** verified: that `D7` really is what Great Scott
Gadgets' own software calls LED 0 — the chain platform file → schematic net
→ refdes → layout position is four files agreeing, not an observation. It
is the thing `button_led.v` asks a person to check, because it is the first
design of the three whose appearance depends on it.

The device file says all of this in `src/fpga/devices/ecp5.dev`, under
`device ecp5-12f-CABGA256`, with `pins partial` set because ten of 256
balls are named. **A device file cannot express polarity**, so the designs
drive zero and say in their headers that zero is lit.

## What was not needed, and why that is the news

The placer and the router needed **no edits at all** to route on this
family, and `src/fpga/bitstream.rs` needed one line of visibility. That is
the same thing `docs/fpga-xray.md` and `docs/fpga-gowin.md` each report for
their vendor, and it held for a routed design as well as a constant one.

Two model changes have been needed in total, each for a reason specific to
this family.

**`NetPin::constant`**, for the six-LED milestone. Before it,
`src/fpga/place.rs` recorded that a pin was driven by a constant rather
than by a signal, and did not record *which* constant. Nothing had needed
to know: neither Gowin nor the 7 series had ever compiled a design with a
constant-driven pin, because both have interconnect and their designs route
something. An ECP5 pad tied to a fixed zero or one is the first case where
the fabric has to be told which, so `NetPin` gained
`constant: Option<Bit>`.

**`bitstream::param_bit` became visible inside `fpga`**, for this one, and
the reason is the interesting half of it: **a lookup table's truth table
cannot be a bel's `ConfigEntry::Param` here.** Every other family puts it
there and `bitstream::generate` copies the cell's parameter into it bit by
bit. On this family it depends on the *routing*.

A `LUT4` arrives with its unused inputs tied to a constant — Reticle's
mapper writes `A=btn, B=0, C=0, D=0` for an inverter — and the slice's own
input mux offers only `1`. There is no `SLICE<l>.A0MUX = 0`. So an input a
design ties to zero cannot be tied at the slice at all. What Lattice's own
packer does is tie every unused input **high** and rely on Yosys having
produced a truth table that ignores them. `configure_logic` does the same
thing but derives it: it specialises the truth table at each untied input to
the constant the netlist gives, so the result genuinely ignores that input
and tying it high is safe whichever constant was meant. For the inverter
that is a no-op — `INIT=0x5555` already ignores B, C and D — and knowing
that it is a no-op is worth more than relying on it.

The rest of the division of labour is unchanged and is
`apicula::Periphery`'s: what belongs to a pip goes through the `Arch`, and
what lives in a tile no bel owns is a pass over the placement
(`configure_io`, `configure_logic`). `src/fpga/bitstream.rs` is still at
zero behavioural edits for three vendors.

One thing that *was* worth changing is shared: `Arch::build_graph` keyed its
wire index by an owned `String`, so resolving a pip allocated two of them.
At sixteen million resolutions for this die that is most of the build time
for nothing, and the names live in the `Arch` for the whole call. The key
borrows now. Every backend gets it.

## What remains

| What | What it needs |
|---|---|
| Anything clocked | the global clock network, which is the whole of what is left of the fabric: `globals.json`'s quadrants, spines and taps, the `DCCA` buffers, the path from a pad to a spine, and the `CLK<n>.CLKMUX` / `LSR<n>.LSRMUX` settings a flip-flop needs (`trellis/sites.rs` has that table, unexercised). It is also where the one unsound simplification in this backend bites: every `.mux` source that wants a bit **clear** is in a clock tile, and the loader drops those bits |
| A counter, and so `examples`' blink on this board | the above. `globals.json` is the file to read and `super::xray`'s `enable_global_clocks` is the shape of the pass — a clock costs bits that belong to a whole column rather than to any one pip |
| The left and bottom edges' pads | the left edge is the right edge mirrored (`PICL0`/`PICL1`/`PICL2` for `PICR*`, and the `CIB` one column *east* instead of west) and could be checked against the reference bitstreams the same way the right edge was, since they use pins on every edge. The bottom edge is different again: `PICB*` puts two PIOs at a position and shares tiles with the `EFB`. Neither has been checked, and `TrellisDatabase::load` leaves those balls out of the ball map rather than placing something it would configure nowhere |
| A carry chain | `CCU2C` has no port map in the device file, on purpose: its two sum bits and internal carry do not match the `(ci, i0, i1) -> co` model Reticle maps carry onto. The `.mux` records for the cascade wires are read already |
| Block RAM | `Ecp5Stream` reads and writes the initialisation blocks — the reference files' 44 blocks round trip — and nothing generates one. The `MIB_EBR*` tiles' wires and pips are in the graph |
| Distributed RAM | `SLICEA.MODE = DPRAM`, `WREMUX`, `CLK1.CLKMUX` and the `WAD`/`WDO` wires, none of which is declared |
| An IO standard other than LVCMOS33 | the bits are in the database and the code takes the standard from the constraints; no other standard has been on a part |
| A bidirectional pad | `configure_io` reads the direction off the netlist and builds an input or an output; `BIDIR_<standard>` is in the database and the tristate would have to be routed rather than tied |
| An ECP5 over an FTDI cable | nothing, in principle: the configuration plans are transport-neutral and `jtag::Scan` encodes them for MPSSE. It is refused because that pairing has never been run |

## The three things worth knowing if you change this

**The order of a position's windows is the order Project Trellis' tile
names sort in.** `parse::tilegrid` sorts by key and
`TrellisDatabase::load` pushes in that order, and a `ConfigBit`'s `row`
addresses them end to end. Change the sort and every bit of a shared
position moves. `CIB_R1C63:CIB` sorts before `MIB_R1C63:PICT1`, which is
why a `PICT1` bit `F54B0` becomes row 106 + 54. The loader now also checks
that every position of one composition has the *same* window layout, not
just the same total size, because that is what every bit of a shared
position hangs off.

**A bel must declare its pins, and must sit where they are.** Two halves of
one lesson, learnt separately.

`Netlist::build` records no `NetPin` for a pin whose role names no wire the
`Arch` declares — it goes in `off_fabric` — so the constant behind an
output pad would never reach `configure_io`. That is how the first half was
found: two designs driving different constants produced the same 60 bits.

The second half is that a pin resolves in the tile the *bel* sits in. While
nothing routed, the pad bel was put where the pad's bits were, one column
east of the ball for side B, and its three wires were declared there by
hand. Both tiles have a `PADDOB_PIO`, so nothing complained — the bel just
had the wrong one, and a design that routed would have routed to a wire the
buffer does not read. The bel is at the ball's own position now and its
wires are the ones `bits.db` says that tile owns; a pad whose tile does not
own all three is left out of the ball map rather than placed.

**A tie and a route can be the same mux.** `CIB.JA0MUX` offers `0`, `1` and
`JA0`, where `JA0` means "whatever the routing mux selects", so the bits
that tie a wire to a constant and the bits that route a signal into it live
in one bit space. Two consequences: `configure_io` must not tie a pin a
signal drives, and no test can ask a finished bitstream whether a pad is
tied. `tests/fpga_trellis.rs` runs `configure_io` into a bitmap of its own
to ask what that pass wrote.
