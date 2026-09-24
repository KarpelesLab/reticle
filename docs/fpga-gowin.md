# A real Gowin GW2A fabric, and a design running on one

## One design produced by this has run on a real part

On 2026-09-24 `examples/primer20k/key_led.v` — one button through one
lookup table to one LED — was synthesised, placed, routed and written as
a `.fs` by `reticle fpga`, loaded into the SRAM of a Sipeed Tang Primer
20K (a GW2A-LV18PG256C8/I7) by `reticle program`, which read `DONE` set
and no error, and **confirmed working by a person**: pressing the dock's
button S4 changed the LED silkscreened LED1, and nothing else responded.
No Gowin or Apicula tool took part in producing or loading it.

That is one design, of one lookup table and two pins, on one board. It
establishes that the chain from Verilog to configured Gowin silicon
closes. **It does not establish that anything larger works**: nothing
clocked has been tried, and the two gaps *How far this got* lists
still stand between this flow and a flip-flop.

What stands behind it, and against what each part was checked:

- **every bit, against `gowin_pack`.** For three designs Project
  Apicula's own packer turns into a `.fs` — nothing at all, two IO
  buffers, and a buffer-LUT-buffer design routed by hand through 17 pips
  (`testdata/fpga/gowin/*.json`) — Reticle, given the same placement and
  routing, writes **the same file byte for byte**: 862, 771 and 842 bits,
  the header, every row's check word, and the footer's checksum. That
  covers the `const` bits, IO buffers, IO banks, every unused IO of the
  ring, a LUT's truth table, the defaults of a slice whose registers are
  empty, and pip bits. See `tests/fpga_gowin.rs`, and *Reproducing the
  reference files* below for how to make them.
- **the placement and routing are Reticle's own**, and were checked
  independently: Apicula's `gowin_unpack`, which knows nothing of Reticle,
  decodes the `key_led` bitstream to an input buffer on T3 driving a LUT4
  with `INIT=16'h5555` six hops away, whose output reaches an output buffer
  on N16 five hops further, with no other path between the two pads.
- **the loader, against the part.** `reticle program` erases the SRAM,
  shifts the file and reads the status register, which reads `0x2020`
  (`DONE` and `MEMORY_ERASE`) after the load: UG290's success value. The
  sequence is under *Loading it* below, including the one step Gowin's
  documents leave out.
- **the fabric's size.** Every number in the table below is produced by
  the loader from the database and re-checked by a test, not written down
  by hand.
- **the pin map.** All 207 PBGA256 IO balls the database's pinout lists
  resolve to a site the loaded fabric really has. On the dock, ball T3 is
  button S4 and ball N16 is LED1; Apicula's `examples/primer20k.cst`
  calls N16 `led[2]`, so its index is not the silkscreen's.

## Loading it

```sh
cargo build --features cli,apicula,program
reticle fpga --device gw2a-18-pg256 \
    --constraints examples/primer20k/key_led.rcf \
    --bitstream key_led.fs examples/primer20k/key_led.v
reticle program key_led.fs
```

`apicula` is the feature that reads the database (it needs an xz
decoder), and `program` the one that talks USB; both are off by default
because each has a dependency. The database itself is fetched on first
use into `~/.cache/reticle`.

`reticle program` takes a `.fs` as it takes a `.bit`: it reads the file
and the IDCODE its `0x06` command names before opening the cable, reads
the part's identifier, and writes nothing unless the two agree. Only the
SRAM is written, and a power cycle reloads whatever the board's flash
holds. The sequence is Gowin's, from UG290 and TN653, in
`src/program/gowin.rs`:

```text
CONFIG_ENABLE, wait for edit mode; ERASE_SRAM, NOOP, wait for memory
erase; XFER_DONE, NOOP, CONFIG_DISABLE, NOOP, wait for edit mode to clear;
CONFIG_ENABLE, ADDRESS_INITIALIZE, TRANSFER, <the whole .fs through DR>,
0x0a <checksum>, 0x08, CONFIG_DISABLE, NOOP; read the status register
```

**The `0x0a`/`0x08` step is not in Gowin's documents** and is not
optional. Without it the part erased, took the file, and left `DONE`
clear with no error bit; with it, `DONE` came up. It is what
openFPGALoader sends on every load, and the value is the checksum the
file's own footer carries in its `0x0a` command — which `gowin_pack`
computes, and Reticle now does too (`FsStream::fill_checksum`), as the
configuration rows joined and summed as sixteen-bit words.

The instruction set is an enum holding only these opcodes, none of which
reaches flash; the ones that do (`0x16`, the pass-through to the board's
SPI flash, among them) are listed in `gowin::FORBIDDEN`, and a test keeps
the two apart. `reticle program --probe` now reads a Gowin part's status
register with Gowin's own instruction (`0x41`) instead of stopping at the
identifier.

The dock's adapter is a Sipeed "JTAG Debugger" presenting as an FT2232D
(`0403:6010`, bcdDevice `0x0500`); `reticle program` drives it with
openFPGALoader's pin setup for this board, `SIPEED_PINS`. It clocked the
4.6 Mbit file out in about a second, faster than the 1 MHz TCK asked for,
which fits the adapter being a microcontroller emulating an FTDI part.

## Getting the database

It is [Project Apicula](https://github.com/YosysHQ/apicula), `apycula` on
PyPI, **MIT** (copyright 2019 Pepijn de Vos). It is not in this repository
and must not be. Unlike Project X-Ray's, it **ships prebuilt** — no vendor
IDE has to be run — and one file per die is all that is needed.

The command line fetches it:

```sh
reticle fetch apicula
```

That downloads the 0.33 wheel from PyPI (4 MB), checks it against the
SHA-256 PyPI publishes for it, which is built into the binary, and
unpacks its twelve `<device>.msgpack.xz` files and its licence into
`$XDG_CACHE_HOME/reticle/apicula/0.33` (`~/.cache/reticle/...` without
`XDG_CACHE_HOME`). The download is done by the system's `curl`, over
HTTPS only; the library itself still never fetches anything, and reads
the files through a `FileProvider` like every other database.

The one this work uses is `GW2A-18.msgpack.xz`: **375 216 bytes
compressed, 5 993 401 open.** The wheel carries only the package; the
*source* distribution also carries `doc/` and `examples/`, which are
where several facts below come from, and is worth having to read them:

```sh
pip download --no-deps --no-binary :all: apycula==0.33 -d /tmp/apycula
tar -C /tmp/apycula -xzf /tmp/apycula/apycula-0.33.tar.gz
export RETICLE_GOWINDB=/tmp/apycula/apycula-0.33/apycula
```

`tests/fpga_gowin.rs` reads `RETICLE_GOWINDB` if it is set and the cache
if it is not. **CI has neither, nor a board**, and every test that needs
the database skips with a line saying what is missing. A missing database
never fails the build, and a test never fetches one.

### Reproducing the reference files

Three tests compare Reticle's output with a file Project Apicula's own
packer wrote. `gowin_pack` reads nextpnr's JSON rather than a netlist, so
the inputs are hand-written nextpnr-shaped JSON, placed and (for the LUT)
routed by hand, in `testdata/fpga/gowin/`:

```sh
pip install apycula==0.33
for d in empty io lut; do
    gowin_pack -d GW2A-18 -o /tmp/$d.fs testdata/fpga/gowin/$d.json
done
export RETICLE_GOWIN_FS=/tmp/empty.fs RETICLE_GOWIN_IO_FS=/tmp/io.fs \
       RETICLE_GOWIN_LUT_FS=/tmp/lut.fs
```

| Input | What is in it | Bits |
|---|---|---|
| `empty.json` | nothing | 862 |
| `io.json` | an `OBUF` on C13 and an `IBUF` on T3, LVCMOS33, connected to nothing | 771 |
| `lut.json` | the same buffers, and a LUT4 inverter between them over 17 pips | 842 |

Each is 4 618 782 bytes of ASCII `0`s and `1`s. With `-c` the empty one
is 699 990 bytes; the reader handles both and the writer writes the
uncompressed form.

`io.json` and `lut.json` list the output buffer **first**, and that is
deliberate. `gowin_pack` means to set a bank's voltage from its outputs
and to fall back to 1.2 V for a bank of inputs only, but a mutable default
argument (`make_IoBelDesc(bel, flags={})`) marks every buffer processed
after the first output as an output as well, so an input-only bank gets
1.2 V or its standard's voltage depending on the order of the file.
Reticle always gives a used bank its standard's voltage — an `LVCMOS33`
input sits in a bank the board supplies at 3.3 V — which is what
`gowin_pack` writes in this order. In the other order the files differ in
exactly bank 4's 51 bits.

It is worth being clear about what kind of oracle this is. It is **not** a
vendor-produced bitstream for this board, the way
`artix7/harness/basys3/swbut/design.bit` is for the Basys 3. It is an
independent implementation of the same open format, so agreeing with it
settles that Reticle reads the database as Apicula does; the part itself
is what settled that the result configures anything.

## Which die, and why that is a real question

The board's marking reads `GW2A-LV18PG256C8/I7`. Apicula ships two
databases that could serve it, `GW2A-18` and `GW2A-18C`, and the
difference matters less than it looks:

| | `GW2A-18` | `GW2A-18C` |
|---|---|---|
| grid | identical, 55 x 56 | identical |
| die bitmap | identical, 1342 x 3376 | identical |
| `.fs` header | identical, IDCODE `0x0000081b` | identical |
| part numbers listed | 21, including `GW2A-LV18PG256C8/I7` | 45, including it, **and every `GW2AR-*`** |
| `chip_flags` | `HAS_SP32`, `NEED_SP_FIX`, `NEED_BLKSEL_FIX`, ... | `HAS_SP32`, `NEED_BSRAM_OUTREG_FIX`, `NEED_BSRAM_DP_CE_FIX`, ... |

So **the IDCODE cannot choose**: a GW2A-18, a GW2A-18C and a GW2AR-18C all
answer `0x0000081b`, which is exactly what `reticle program --probe`
reports ("Gowin GW2A(R)-18(C)") and why it hedges. Both databases list
this part number, so the marking cannot choose either. What differs is a
set of block-RAM errata flags, and this work touches no block RAM.

This flow uses **`GW2A-18`**, and not by a coin toss: it is what Project
Apicula's own `examples/Makefile` uses for a Tang Primer 20K —
`gowin_pack -c -d GW2A-18` with `nextpnr ... --device
GW2A-LV18PG256C8/I7`. That is the configuration upstream builds and lists
in its default `all` target. `tests/fpga_gowin.rs::the_c_variant_is_the_same_fabric_with_different_errata`
re-derives the table above so it cannot drift.

## The file format, key by key

The file is **one MessagePack map inside an xz stream**. Two consequences
shaped the code:

- The xz is decoded by `compcol`, the Karpeles Lab compression collection
  (MIT, `no_std`, `unsafe_code = "forbid"` in its own lints), taken as
  `default-features = false, features = ["xz", "std"]`, which pulls in **no
  transitive dependencies** and calls no system library. It is the second
  and last dependency in the crate and it is optional, behind the new
  `apicula` feature, so a default build still resolves to zero
  dependencies.
- The MessagePack is `src/msgpack.rs`, written here. It is read-only —
  nothing in Reticle produces MessagePack — and it sits at the crate root
  beside `src/json.rs` for the reason that one does: a second caller is
  expected, and writing the parser twice would be the only alternative.

**Two things about the encoding are not optional.** The map is a Python
`@dataclass` written out field by field (`apycula/chipdb.py`'s `Device`),
so its keys are that class's field names in declaration order — which is
why `grid` is first. And a great many of its inner maps are keyed on
**tuples**, because in Python a `dict` keyed on `(row, col)` is
idiomatic. A reader that modelled a map as `HashMap<String, _>` could not
hold this file at all; `Msgpack::Map` is therefore a vector of pairs of
whole values, in file order.

The root map has **38 entries**. This is what each holds for `GW2A-18`,
and whether this loader reads it.

| Key | What it is | Read? |
|---|---|---|
| `grid` | 55 arrays of 56 integers: the tile type number of every tile | yes |
| `center_row`, `center_col` | `27`, `27` — the root of the clock tree | no |
| `empty_cell_row`, `empty_cell_col` | `-1`, `-1` — the GW5AT-60B has a missing corner; this die has none | no |
| `tiles` | 74 tile types, keyed by number. Each has `width`, `height`, `ttyp`, `pips`, `clock_pips`, `alonenode`, `alonenode_6`, `bels` | yes |
| `timing` | 8 timing classes (`C8/I7`, `C7/I6`, `A6`, `C9/I8` and an `_LV` twin of each), each a tree of four-float delay quadruples | no |
| `wire_delay` | 1021 wire names to a delay class (`LUT_IN`, ...) | no |
| `packages` | 21 part numbers to `(package, device, speed)` | yes |
| `pinout` | `{device: {package: {ball: (IOLOC, [functions])}}}`, nine packages | yes |
| `sip_cst` | empty here; system-in-package constraints | no |
| `pin_bank` | 384 `IOLOC`s to an IO bank number, 0..7 | not by the loader; it is where `gowin.dev`'s `bank` clauses came from |
| `cmd_hdr` | 10 byte strings: the `.fs` file's header commands | yes |
| `cmd_ftr` | 6 byte strings: its footer | yes |
| `template` | `nil` here | no |
| `logicinfo` | 25 tables mapping `(attribute id, value id)` to a code. **The ids are numbers; their names are not in the file** | reachable |
| `rev_li` | empty; a cache the writer does not persist | no |
| `longfuses` | 14 tile types' single-feature fuse tables (`DLLDEL0`, `DCS6`, ...) | reachable |
| `shortval` | 74 tile types' two-feature tables (`CLS0..3`, `LUT`, `IOLOGICA/B`, `PLL`, `BSRAM_SP/DP/SDP/ROM`, `HCLK`, `DSP0/1`, `OSC`, `GSR`, `CFG`) | reachable |
| `longval` | 16 tile types' sixteen-feature tables (`IOBA`, `IOBB`, `BANK`) | reachable |
| `const` | 7 tile types' always-set bits; 462 bits over the die | yes |
| `nodes` | **14 748 always-connected networks with 89 583 members**, each `(row, col, wire)`, typed `GLOBAL_CLK`, `HCLK`, `TILE_CLK`, `PLL_I/O`, `BSRAM_I/O`, `DSP_I/O`, `HCLK_CTRL` | counted, **not loaded** |
| `bottom_io` | empty for this die; two special wires some dice need for a bottom-row output | no |
| `simplio_rows` | `(9, 27, 45)` — the rows with simplified IO, also where block RAM init data lands | no |
| `pad_pll` | 5 `IOLOC`s that are a PLL's clock or feedback pad | no |
| `tile_types` | 7 letters to sets of type numbers: `C` logic, `M` logic with distributed RAM, `I` IO, `B` block RAM, `D` DSP, `P` PLL, `E` empty | yes |
| `diff_io_types` | the 8 differential buffer primitives | no |
| `hclk_pips` | 1081 pips over 202 *cells* — position-dependent, so not a tile-type property | counted, **not loaded** |
| `extra_func` | 216 cells' extra functions: `dqce`, `dcs`, `osc`, `gsr`, `buf`, disabled blocks | no |
| `chip_flags` | 5 errata flags, and the one place `GW2A-18` and `GW2A-18C` really differ | no |
| `segments` | 280 segmented clock columns with their extents and gate wires | no |
| `dcs_prefix` | `"CLK"`; the GW5 renames these inputs | no |
| `io_cfg` | 384 `IOLOC`s to alternative pin configurations | no |
| `corner_tiles_io` | which edge each of the four corners counts as: `(0,0)` and `(0,55)` top, `(54,0)` and `(54,55)` bottom | yes |
| `spine_select_wires` | empty here; a GW5AST-138C needs VCC or GND on a mux to use a spine | no |
| `last_top_row` | `54` | no |
| `io2hclk`, `hclk_div2` | empty here | no |

### The four rules the file implies and does not state

Each of these had to be worked out, each is now a test that re-derives it,
and each is a place this could still be wrong. They live in
`src/fpga/apicula/parse.rs`.

#### 1. An inter-tile wire's name says which tile it started in

A Gowin wire that leaves its tile has a **different name in each tile it
crosses**. The rule is `{direction}{length}{number}{segment}`, stated in
Apicula's `doc/filestructure.md`: `W270` is a westward two-hop wire,
number 7, segment 0 — the root — and `W272` is the same metal two tiles
west. The reader is `chipdb.py::wire2global` and its regular expression is
`([NESW])([128]\d)(\d)`.

Measured over the whole database: **168 of the 3918 wire names match, in
64 families**, and the segments a family uses are `0..=length` for the
one- and two-hop wires and `{0, 4, 8}` for the eight-hop ones. So the
length digit *is* the wire's reach, which is what lets the root's span be
derived rather than tabulated.

The step per segment is `chipdb.py`'s `dirlut`, in `(row, col)`: `N` is
`(+1, 0)`, `E` is `(0, -1)`, `S` is `(-1, 0)`, `W` is `(0, +1)`. So an
`E` wire's root is at a *lower* column and extends towards higher ones.

#### 2. An `IOLOC` is derived from a tile's position, and the corners need the database

`chipdb.py::rc2tbrl_0`: the edge letter comes from which side the tile is
on, the index is the one-based column for the top and bottom edges and the
one-based row for the left and right ones. A corner is on two edges at
once, and `corner_tiles_io` says which letter wins — for this die both top
corners are `T` and both bottom ones `B`, so **the die has no `IOL1` or
`IOR1` at all**.

The loader builds this map *forwards*, over every tile of the ring, and
inverts it; it never reads a name and guesses where it points. Getting
this backwards would put a design's output on the wrong ball, which is the
sort of error only a board finds.

#### 3. A fuse table's negative codes are why an empty bitstream is not blank

`chipdb.py::get_table_fuses`. A row's key is a tuple of attribute
*codes* — two for `shortval`, sixteen for `longval`, one for `longfuses` —
and the row's bits apply when every code is satisfied:

| code | meaning |
|---|---|
| `0` | the key ends here; the rest is padding |
| positive | this attribute value must be set |
| negative | this attribute value must **not** be set |

The negative form is the interesting one: a row whose whole key is
negative applies when *nothing* is set, so its bits are **on by default**
and a design turns them off. A logic tile's `CLS0` table has such a row
(`(-7, 0)` sets bit `(20, 3)`), and this is a large part of why
`gowin_pack`'s output for an empty design is not 4 530 592 zeros.

#### 4. A lookup table's fuse means a ZERO

`gowin_pack.py::get_LUT4_fuses`: `if lutbit == '0': bits.update(
lutmap[bitnum])`. `bels["LUT<n>"].flags[i]` is where bit `i` of the
sixteen-bit `INIT` lives, and the fuse is set when that bit is **zero**.

The model already had a word for this — `ConfigEntry::ParamZero`, which
exists because Project X-Ray stores a flip-flop's `INIT` the same way
round — so no new mechanism was needed. Getting the polarity backwards
inverts every lookup table in a design and nothing but a part would
notice.

## The measured size of the fabric

Produced by the loader, printed by `ApiculaStats::to_text`, and asserted by
`tests/fpga_gowin.rs::the_fabric_measures_itself_and_the_numbers_are_the_documented_ones`.

| | |
|---|---|
| grid | 55 x 56 = 3080 tiles |
| tile types | 74 |
| die bitmap | 1342 rows x 3376 columns = 4 530 592 bits |
| bytes to a bitmap row | 422, exactly, with no padding bits |
| pips over the die | 8 095 135 |
| clock pips over the die | 5 198 |
| distinct configuration bit patterns among them | 10 915 |
| bels over the die | 54 111 |
| `nodes` | 14 748 networks, 89 583 members |
| `hclk_pips` | 1 081 over 202 cells |
| bits the `const` tables always set | 462 |
| PBGA256 IO balls | 207, across 8 banks |

And the logic, counted from the database's bels rather than transcribed
from a datasheet — which is how the device file's resource counts are kept
honest:

| | | Gowin's published figure |
|---|---|---|
| LUT4 | 20 736 | 20 736 |
| flip-flops | 15 552 | 15 552 |
| ALUs (the carry element) | 15 552 | — |
| distributed RAMs (`RAM16`, 16 x 4 bits) | 648 | — |
| IO buffers | 384 (192 `IOBA` + 192 `IOBB`) | — |
| block RAMs | 46 | 828 kbit = 46 x 18 kbit |
| DSP blocks | 12, with 48 `MULT18X18` | 48 18x18 multipliers |
| PLLs | 4 `RPLLA` | 4 rPLL |
| IO logic blocks | 408 | — |

The whole die expands into a routing graph this crate holds comfortably,
also measured:

| | |
|---|---|
| nodes | 626 001 |
| edges declared | 8 100 333 |
| of those, kept | 7 872 211 (97%) |
| distinct bit patterns, shared by all of them | 9 939 |
| `RoutingGraph::heap_bytes` | 296 MiB |

296 MiB against the `xc7a50t`'s 1386 MiB is partly a smaller die and
partly the next section.

## Where Reticle's model fitted, and where it did not

The brief expectation was that the placer, router and bitstream writer
would need no edits, as they needed none when a synthetic iCE40 was
swapped for a real Artix-7. **That held for the architecture and the graph,
and it did not hold for the bitstream writer.** `src/fpga/arch/`,
`src/fpga/place.rs`, `src/fpga/route.rs` and `src/fpga/bitstream.rs` are
untouched; a Gowin `.fs` needed a new container, `src/fpga/gowin.rs`,
which is what `src/fpga/xc7.rs` is for the 7 series. That is what a
"bitstream-oriented flow" means: the graph is generic and the envelope is
not.

Three mismatches are worth naming, and one of them is a fit rather than a
mismatch.

### 1. The tile bitmap fits exactly, which the 7 series did not

A Gowin die bitmap **is** the per-tile bitmaps laid out in a grid. Tile
`(row, col)` occupies `height` rows and `width` columns at the offset
given by the heights of the rows above it and the widths of the columns
left of it, so a tile's `ConfigBit { row, col }` is that tile's own bit and
nothing is translated. There is no frame address anywhere.

That is precisely what `TileType::bit_rows` and `bit_cols` have always
meant — the model `arch::synthetic` invented for an iCE40. The one wrinkle
is that a tile's size depends on where it is: a GW2A-18's rows are 24, 26
or 28 bits tall and its columns 60 or 68 wide, so the geometry comes out
beside the architecture as a `gowin::DieLayout`, the way an
`xc7::FrameMap` does.

### 2. An inter-tile wire is a span, and this is what spans were for

`Arch` describes a wire by a *span*: `sp4e` starts here and reaches four
tiles east, and every tile it crosses sees the same node. The 7-series
loader could not use that — a long line has a different name in each tile
and only `tileconn.json` says which equals which — so it gives every wire
span `0 0` and emits each join as a pair of zero-bit pips, which costs a
third of its 30.9 million edges.

Here the mapping is arithmetic, so the wire is declared **once, under its
root name, with the span its length digit gives it**, and every pip that
calls it by a later segment refers to it with `WireRef::at`. The cost is
**no extra graph edges at all**.

**One thing is given up for that, deliberately.** At the die's edge a
wire's root would fall outside the grid, and Gowin reflects it back and
flips its direction: `S212` in the top row is the same metal as `N21` two
rows down. That depends on the tile's *position*, and a pip belongs to a
tile *type*, so it cannot be written down once per type. Expressing it
would need one tile type per edge situation — 421 of them, measured — and
the loader does something simpler instead: it always names the
*unreflected* root, which for a wrapped wire is off the grid, so the pip
is dropped and counted.

**A wrapped wire is therefore missing, never wrong.** That is the only
acceptable way round, because a wrong join shorts two nets and no
structural check would notice. On a GW2A-18 it is **16 872 of 517 440
inter-tile wire references, 3.3%**, all within eight tiles of an edge, and
`ApiculaStats::edge_wraps` counts them. Teaching `Arch` an edge-variant
tile type is what would recover them, and nothing in phase one needed
them.

### 3. The `nodes` table needs a pip that belongs to a tile, not to a type

This is the real limit, and it is the reason nothing clocked can route.

`nodes` is 14 748 named networks with 89 583 members, each member a
`(row, col, wire)`. It is where the global clock network lives, and the
high-speed clock spines, and the wiring that reaches into block RAM and
DSP cells. `Arch` has the vocabulary for it: a node is a **global wire**
(`Arch::globals`, one node for the whole die) and a membership is a
zero-bit pip joining a tile's local wire to it.

What it cannot say is **which tiles**. A membership belongs to one tile and
a pip is declared by a tile type, and those memberships touch **all 3080
tiles** of a GW2A-18 — so writing them down would need one tile type per
tile: 3080 copies of pip lists that run to 2624 entries each, which is
hundreds of megabytes of `PipDecl` before any graph is built. The same
applies to `hclk_pips`, which the database keys by cell for exactly this
reason.

So the loader counts them and does not emit them, and leaves them reachable
through `ApiculaDatabase::nodes`. **Teaching `Arch` a per-tile pip —
`tile_pips: Vec<(u32, u32, PipDecl)>`, resolved in `build_graph` — is the
single change that unblocks clocks, block RAM and DSP on this family**, and
it would also give the 7-series loader a cheaper way to spend its ten
million join edges.

## What the database does not name

A Gowin bel's configuration is a set of *attribute values*:
`IO_TYPE=LVCMOS33`, `PULLMODE=NONE`, `SLEWRATE=FAST`, `REGMODE=FF`,
`SRMODE=ASYNC`. The database encodes those in two steps —
`logicinfo[<table>]` maps `(attribute id, value id)` to a small integer
code, and `shortval`/`longval` map tuples of codes to the bits to set —
and `fuses_for` implements the second step, including the negative-code
rule above.

**The attribute and value names are not in the file.** They are Python
dictionaries in `apycula/attrids.py`. The four the flow needs — the IO
block's and the logic slice's, `iob_attrids`, `iob_attrvals`,
`cls_attrids` and `cls_attrvals` — are transcribed into
`src/fpga/apicula/attrids.rs` by `tools/gen-apicula-attrids.py`, which
records the source file's SHA-256 and carries Apicula's MIT notice.

`src/fpga/apicula/config.rs` then does what `gowin_pack` does, function by
function, and says which:

- an **input buffer** gets `default_ibuf_attrs` and an **output buffer**
  `default_obuf_attrs`, each with its bank's `IO_TYPE` and `BANK_VCCIO`;
- every **unused IO** gets its bank's `IO_TYPE` and `BANK_VCCIO`, and a
  bank with nothing in it is `LVCMOS18` at 1.8 V;
- every **bank** gets its standard and voltage, and `PULL_STRENGTH` when
  used; which tile holds a bank is `Device.bank_tiles`, a Python property
  and not a field, so it is recomputed from the `BANK<n>` bels;
- a **slice** whose LUTs are used and whose registers are empty gets the
  empty registers' state, `REG0_REGSET=RESET` and the rest.

An attribute value `logicinfo` has no code for is dropped, as
`add_attr_val` drops it: that is how most defaults work. The bits of an
unused IO ring and eight idle banks are the 400 a blank stream lacked
before this, and with them it is `gowin_pack`'s empty design byte for
byte.

**One IO standard per design**, as for the 7 series: the buffer bits are
worked out once per load, from `ApiculaOptions::io_standard`, and the
command line refuses constraints that ask for two. The standards
configured are the single-ended `LVCMOS` ones, 1.0 V to 3.3 V.

## The `.fs` container

`src/fpga/gowin.rs`. A Gowin configuration file is **text**: every byte is
written as eight `0`/`1` characters, one line per command and one line per
row of the die bitmap.

```text
20 x 0xff                       preamble
0xff 0xff                       a two-byte field the vendor programmer displays
0xa5 0xc3                       the magic
0x06 0x00 0x00 0x00 <idcode:4>  check the part's JTAG IDCODE
0x10 ...                        loading rate, compression (bit 13), program-done bypass
0x51 ...                        the three bytes that stand in for runs of zeros
0x0b 0x00 0x00 0x00             security
0xd2 0x00 0xff 0xff <addr:4>    SPI flash address — excluded from the check word
0x12 0x00 0x00 0x00
0x3b 0x80 <rows:2>              load configuration: how many rows follow
<422 bytes> <crc:2> 6 x 0xff    ... 1342 of these, one per bitmap row
18 x 0xff <crc:2>               end of the grid
0x0a 0x00 0x00 0x00 <sum:4>     checksum of the configuration data
8 x 0xff
0x08 0x00 0x00 0x00             done
8 x 0xff
0xff 0xff
```

**Every byte of that but two fields is data from the chip database.**
`cmd_hdr` and `cmd_ftr` are arrays of byte strings in the file itself, and
this module writes them verbatim; the two fields it fills in are the row
count in the `0x3b` command and the `0x0a` command's value. Gowin's tools
call that line the USERCODE, but `gowin_pack` always fills it with a
checksum of the configuration data, and so does `FsStream::fill_checksum`:
every row as the file writes it, joined, packed into bytes and summed as
sixteen-bit big-endian words. For a design with nothing in it that is
`0xbf45`. The loader needs it (see *Loading it*). That is the same division of labour `src/fpga/xc7.rs` has with
`part.json`: the database knows the part and the container knows the
envelope.

The check word is **CRC-16/ARC** — polynomial `0x8005` reflected, initial
value zero, no final exclusive-or — **written low byte first**. Each row's
word covers the six `0xff` bytes that ended the *previous* line followed
by that row's own bytes, and the first row's covers the header from the
`0x06` command onward **with the `0xd2` line left out**. That last
exception is a measurement and not a guess: with the `0xd2` line included,
not one of the reference file's 1342 words matches; with it excluded, all
1342 do.

A row is written **from its highest column to its lowest**, with any
padding bits (none on this die, four on a GW1N-9) as **ones** in front.
Getting that backwards mirrors the whole die, and every check but a real
part would pass it.

Compression replaces runs of two, four or eight zero bytes with three byte
values the `0x51` command names, chosen because they appear nowhere in the
data. The reader handles it; the writer does not use it, because the only
thing it buys is file size and a writer that picks the wrong three bytes
produces a file that decompresses into something else.

## The device file

`src/fpga/devices/gowin.dev`, one device: `gw2a-18-pg256`. It follows
`xc7.dev`'s provenance-and-confidence style and its header states each
group's source and confidence at length; this is the summary.

| Group | What is declared | Provenance | Confidence |
|---|---|---|---|
| resource counts | 20736 LUT4, 15552 FF, 15552 ALU, 648 SSRAM, 384 IOB, 46 BSRAM, 12 DSP, 4 PLL | **counted from the database**, re-counted by a test | HIGH |
| LUT4 / LUT3 / LUT2 / LUT1 | `i=I0..I3 o=F`, `INIT` 16 bits | the database's own bel portmap | HIGH |
| flip-flops, 20 primitives | `DFF{,E,R,RE,S,SE,C,CE,P,PE}` and their `DFFN*` twins; `clk=CLK d=D q=Q`, `en=CE`, `rst=RESET/SET/CLEAR/PRESET` | names from `gowin_pack`'s own `get_DFF*_fuses` methods; port names from Gowin SUG283 | names HIGH, ports MEDIUM-HIGH |
| `shared_reset` | set | **measured**: DFF0/DFF1 share `CLK0/LSR0/CE0`, DFF2/3 share `CLK1/...`, DFF4/5 `CLK2/...` | HIGH |
| IBUF, OBUF | `pad=I din=O` and `pad=O dout=I` | Apicula's own example designs for this board | HIGH |
| IO standards | LVCMOS33/25/18/15/12, LVTTL33; slew SLOW/FAST | names are `IO_TYPE` and `SLEWRATE` values in `attrids.py` | names HIGH, drive lists MEDIUM |
| IO banks | 8, and which ball is in which | **measured** from `pin_bank` | HIGH |
| pin list | all 207 PBGA256 IO balls, with bank and special functions | **generated** from the database's `pinout` | HIGH |
| global clocks | 8 | the package pinout's own `GCLKT_0..7` / `GCLKC_0..7` | HIGH |
| rPLL | ports, parameters, `feedback out`, PFD 3–500, VCO 500–1250 MHz | ports from Apicula's `examples/clock-rPLL.v`; limits from `gowin_pll.py`'s `GW2A-18 C8/I7` entry, which cites DS117E/DS861E/DS226E | HIGH |

### And what it deliberately does not declare

Every one of these is a case where declaring the primitive would produce a
netlist that is wrong rather than merely large, so it is declared `other`
— known to a netlist check, never instantiated — and the file says why.
`tests/fpga_gowin.rs::the_device_file_declares_only_what_this_flow_can_actually_build`
asserts each absence, so none of them is an oversight anyone can
accidentally undo.

- **No carry chain.** Gowin's carry element is the ALU, and in add mode it
  computes `SUM = I0 ^ I1 ^ CIN` and `COUT = MAJ(I0, I1, CIN)` from the
  two *operands*. Reticle's two carry shapes both expect a separate LUT to
  supply something: the one-bit shape wants the sum computed outside, the
  wide shape wants a propagate `a ^ b`. The Gowin ALU **is** the slice's
  LUT4 reconfigured, so there is no separate LUT to put either in.
  Declaring it under either shape would produce a netlist with a LUT4 and
  an ALU competing for one slice. Adders stay in LUT4s: correct, and
  large.
- **No bidirectional buffer.** A Gowin `IOBUF`'s output enable is `OEN`
  and it is **active low** — every Apicula example writes `.OEN(~key)`.
  The `.dev` `io` line has no way to say so, and declaring `oe=OEN` would
  give every tristate design an inverted enable: a bus that drives when it
  should listen.
- **No global clock buffer.** Not an omission: on this family a clock
  enters the global network *by being routed onto it*, which is what the
  6306 of the `nodes` table's 14 748 networks that are typed `GLOBAL_CLK`
  are, and a GW2A-18 has no `BUFG` bel anywhere in its grid.
- **No block RAM.** Two reasons, either sufficient. A block responds only
  when its `BLKSEL[2:0]` input matches its `BLK_SEL` parameter and the
  `bram` model cannot tie an input to a constant, so a declared block
  would silently read zero. And its contents — `INIT_RAM_00` ..
  `INIT_RAM_3F` — do **not** live in the die bitmap: `gowin_pack` writes
  them into a separate region addressed through a
  `logicinfo['BSRAM_INIT']` table that `fpga::gowin` does not write, so a
  ROM would come up blank.
- **No distributed RAM.** `RAM16SDP4`'s `DI`, `WAD`, `RAD` and `DO` are
  Verilog *vectors*, and the `.dev` `lutram` line names a wide port as a
  list of individual pins (right for the ECP5's `TRELLIS_DPR16X4` and the
  7 series' `RAM64X1D`, which really do have one port per bit). An
  instance would not elaborate.
- **No DSP.** `xc7.dev`'s reason for leaving out the DSP48E1: a Gowin
  multiplier does not multiply unless `ASEL`, `BSEL`, `ASIGN`, `BSIGN` and
  the mode parameters are driven with the right constants, and the `dsp`
  line cannot state a tied input.
- **No DDR or SERDES**, and **no differential buffers**. The latter needs a
  *pin pair*, and Reticle's constraints model names one pin per port, so
  it cannot check that `tmds_d_p[0]` and `tmds_d_n[0]` are the two halves
  of one pad. The dock's HDMI connector is exactly this.
- **A PLL that is described and not configurable.** Everything the solver
  needs is in the file except one thing: `ODIV_SEL`'s legal values are 2,
  4, 8, 16, 32, 48, 64, 80, 96, 112 and 128, and `PllDivider` describes a
  divider as a minimum, a maximum and an offset. Declaring `divide out
  ODIV_SEL 2 128` would let the search choose 3 and write a PLL that does
  not run at the frequency it claims, so **no `divide out` clause is
  written** and `fpga::pll::solve` refuses with a message naming the
  missing divider. Teaching `PllDivider` an explicit value list is a small
  change and it is the whole of what stands in the way.

### And what it cannot say at all

`xc7.dev` can say "V17 is slide switch 0", because Digilent publishes a
board file. **Nothing on this machine holds a Tang Primer 20K schematic**,
so this file says which ball is in which bank and which balls are
clock-capable, and says nothing about what any of them is wired to.

What *is* available, and is used by the tests rather than written into the
device file, is Project Apicula's own `examples/primer20k.cst` — the
constraint file its `make primer20k` target builds every one of its
example designs with. It names **C13, A13, N16, N14, L14 and L16 as the
dock's six user LEDs**, T3 as a button, H11 as the clock its examples take
27 MHz from, A15/D14 as the UART pair, and G16/H15, H14/H16, J15/K16,
K14/K15 as the four HDMI differential pairs. A board file is not a device
description, so it stays in the test that uses it.

The device file also cannot say what each bank's VCCIO is wired to, so
every bank is listed as accepting all five voltages the part supports.
**Do not read those lists as board truth**; a constraint asking for
LVCMOS18 on a bank the dock wired to 3.3 V will be accepted.

## How far this got

A combinational design goes from Verilog to a running GW2A-18: through
synthesis, Reticle's placer and router over Apicula's fabric, the
configuration above, the `.fs` container and the JTAG loader. Of the four
gaps this section used to list, two are closed — the attribute names and
the configuration sequence — and two are not:

1. **Per-tile pips in `Arch`.** Without them the `nodes` table cannot be
   loaded, so no clock reaches the global network and nothing sequential
   routes. This is a model change of maybe a hundred lines plus the text
   round-trip, and it helps the 7-series loader too.
2. **A flip-flop's D pin, and packing.** A Gowin flip-flop's data input
   comes from the LUT beside it inside the slice and has no tile wire —
   the database's flip-flop portmap has no `D` at all, exactly as a
   7-series `AFF`'s does not. Until packing can place a LUT and a
   flip-flop on one slice, a sequential design will not route even with
   (1) done. The slice defaults for a register pair *are* written already,
   for the case where the registers are empty.

Until both are done, what this flow can put on a part is combinational.
Beyond that, what is untried: an IO standard other than LVCMOS33, a
tristate or bidirectional buffer, a pin pull other than the default, and
the DONE, READY and other dual-purpose pins as IO, which `gowin_pack` does
with flags that set bits in the configuration tile and this flow does not
write, so it refuses those pins.

## Where the code is

| | |
|---|---|
| `src/msgpack.rs` | the MessagePack reader, read-only, with the two guards that make a malformed file cheap to refuse |
| `src/fpga/gowin.rs` | the `.fs` container: the commands, the die bitmap, CRC-16/ARC, a reader that handles compression |
| `src/fpga/apicula/mod.rs` | the loader: the database as an `Arch` plus a `DieLayout`, with the measurement and the region |
| `src/fpga/apicula/parse.rs` | the four derived rules, each with the test that re-derives it |
| `src/fpga/apicula/attrids.rs` | Apicula's IO and slice attribute names, generated by `tools/gen-apicula-attrids.py` |
| `src/fpga/apicula/config.rs` | IO buffers, unused IO, banks and slice defaults, after `gowin_pack` |
| `src/program/gowin.rs` | the JTAG load sequence, SRAM only |
| `examples/primer20k/` | the design that ran |
| `testdata/fpga/gowin/` | the inputs of the three reference files |
| `src/fpga/devices/gowin.dev` | the device: primitives, pins, banks, the PLL, and a reason for each absence |
| `tests/fpga_gowin.rs` | all of it against the real database, skipping without one |
| `ApiculaDatabase::nodes`, `code_table`, `logicinfo` | the raw tables, reachable without re-reading the file |

`src/fpga/arch/`, `place.rs`, `route.rs` and `bitstream.rs` are untouched:
as for the 7 series, the graph is generic and the envelope is not.
