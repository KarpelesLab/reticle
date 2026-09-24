# A real Gowin GW2A fabric, and how far short of a bitstream it stops

## Nothing here has been loaded into a part

A Sipeed Tang Primer 20K is plugged into this machine. On 2026-09-24
`reticle program --device "FactoryAIOT Pro" --expect 0000081b --probe`
read its JTAG IDCODE, `0x0000081b`, through the board's Sipeed FT2232D
adapter at 12 MHz. **That is the entirety of what has touched the
hardware.** No design has been placed on it, no bitstream has been sent
to it, and the JTAG configuration sequence that would send one is not
written.

Compare `docs/fpga-xray.md`, which opens by saying that one design — one
lookup table and three pins — has run on a Basys 3 and been watched
driving an LED. **There is no equivalent sentence here and this document
must not be read as if there were.** What follows is a reading of an open
database, a device description, a loader and a container, and an honest
list of the four things that stand between them and a lit LED.

What *is* established, and against what:

- **the container.** `gowin_pack`, Project Apicula's own packer, was run
  for a `GW2A-18` and the file it wrote was taken apart. Against that
  file, `src/fpga/gowin.rs` agrees on the ten header lines and the six
  footer lines byte for byte, on the row count patched into the `0x3b`
  command (1342), on the geometry (1342 rows of 3376 bits, 422 bytes with
  no padding, 430 bytes and so 3440 characters to a line), on **all 1342
  row check words**, and on the footer's closing word `0x7334`. That is
  `tests/fpga_gowin.rs::the_blank_stream_is_the_reference_files_envelope_byte_for_byte`;
  it needs the reference file and skips without it.
- **the fabric's size.** Every number in the table below is produced by
  the loader from the database and re-checked by a test, not written down
  by hand.
- **the pin map.** All 207 PBGA256 IO balls the database's pinout lists
  resolve to a site the loaded fabric really has, and ten of them —
  including the dock's six user LEDs — are the balls Project Apicula's own
  `examples/primer20k.cst` names for this board.
- **what a blank bitstream carries.** The `const` tables put 462 bits on a
  die that has nothing in it. `gowin_pack`'s output for a design with
  nothing in it sets 862. Every one of Reticle's 462 is one of the
  reference's 862; the other 400 are the reference packer's configuration
  of the unused IO ring, and getting those needs the attribute names the
  database does not carry.

What is **not** established is anything at all about silicon, and the
largest single reason is in *What the database does not name*.

## Getting the database

Reticle never fetches anything. The library is sans-I/O: the database
reaches it through a `FileProvider` the caller supplies, and the caller
says where it is. The database is not in this repository and must not be.

It is [Project Apicula](https://github.com/YosysHQ/apicula), `apycula` on
PyPI, **MIT** (copyright 2019 Pepijn de Vos). Unlike Project X-Ray's, it
**ships prebuilt** — no vendor IDE has to be run — and one file per die is
all that is needed:

```sh
pip download --no-deps --no-binary :all: apycula==0.33 -d /tmp/apycula
tar -C /tmp/apycula -xzf /tmp/apycula/apycula-0.33.tar.gz
export RETICLE_GOWINDB=/tmp/apycula/apycula-0.33/apycula
```

That directory holds thirteen `<device>.msgpack.xz` files. The one this
work uses is `GW2A-18.msgpack.xz`: **375 216 bytes compressed, 5 993 401
open.** The source distribution also carries `doc/` and `examples/`, which
are where several facts below come from.

`RETICLE_GOWINDB` is what `tests/fpga_gowin.rs` reads. **CI has neither
it nor a board**, and every test that needs the database skips with a line
saying what is missing. A missing database never fails the build.

### And the reference bitstream

One test compares the container against a file the reference packer wrote.
Producing that file needs `apycula` installed and a minimal nextpnr-shaped
JSON, because `gowin_pack` reads nextpnr's output rather than a netlist:

```sh
pip install apycula==0.33
cat > /tmp/empty.json <<'EOF'
{
  "creator": "handwritten",
  "modules": { "top": {
    "settings": { "arch.name": "himbaechel", "arch.type": "gowin",
                  "packer.chipdb": "GW2A-18",
                  "packer.partno": "GW2A-LV18PG256C8/I7" },
    "attributes": {}, "ports": {}, "cells": {},
    "netnames": {
      "$PACKER_GND": { "hide_name": 1, "bits": ["0"], "attributes": { "ROUTING": "" } },
      "$PACKER_VCC": { "hide_name": 1, "bits": ["1"], "attributes": { "ROUTING": "" } }
    }
  } }
}
EOF
gowin_pack -d GW2A-18 -o /tmp/empty.fs /tmp/empty.json
export RETICLE_GOWIN_FS=/tmp/empty.fs
```

That is 4 618 782 bytes of ASCII `0`s and `1`s, and it is the oracle the
whole of `src/fpga/gowin.rs` was built against. With `-c` it is 699 990
bytes; the reader handles both and the writer writes the uncompressed
form.

It is worth being clear about what kind of oracle this is. It is **not** a
vendor-produced bitstream for this board, the way
`artix7/harness/basys3/swbut/design.bit` is for the Basys 3. It is an
independent implementation of the same open format, so agreeing with it
settles the *container* and settles nothing about the *configuration*. A
bitstream from the Gowin IDE for this part would be a better oracle and
none was available here.

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
| `pin_bank` | 384 `IOLOC`s to an IO bank number, 0..7 | yes |
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

This is the largest gap, and it is not a gap in the loader.

A Gowin bel's configuration is a set of *attribute values*:
`IO_TYPE=LVCMOS33`, `PULLMODE=NONE`, `SLEWRATE=FAST`, `REGMODE=FF`,
`SRMODE=ASYNC`. The database encodes those in two steps —
`logicinfo[<table>]` maps `(attribute id, value id)` to a small integer
code, and `shortval`/`longval` map tuples of codes to the bits to set —
and `fuses_for` implements the second step, tested, including the
negative-code rule above.

**The attribute and value names are not in the file.** They are Python
dictionaries in `apycula/attrids.py`, 62 KB of them: `iob_attrids`,
`iob_attrvals`, `cls_attrids`, `cls_attrvals`, `iologic_*`, `pll_*`,
`bsram_*`, `dsp_*` and the rest. Without them, a code is a number and
there is no way to say "this buffer is LVCMOS33".

So, precisely:

- the lookup table is here and tested;
- the names are not, so **no bel is configured except the LUT4**, whose
  bits are named by position instead;
- which means **an IO buffer gets its pins and no bits**. A bitstream from
  this flow would route a signal to a pad and leave the pad
  unconfigured — not driving, not pulled, with no IO standard.

That is also what the 400-bit difference against the reference bitstream
is: `gowin_pack` configures every *unused* IO of the ring (as an
`LVCMOS18` input, per its own `get_default_unused_io_type`), and this flow
configures none of them, used or unused.

Transcribing the tables this needs — with provenance, entry by entry, the
way `src/fpga/xray/sites.rs` does for Xilinx pin names — is the first
thing phase two has to do. `attrids.py` is MIT and its ids are
stable across the database versions that ship with it, which is the reason
it is a transcription job and not a research one.

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
0x0a 0x00 0x00 0x00 <user:4>    USERCODE
8 x 0xff
0x08 0x00 0x00 0x00             done
8 x 0xff
0xff 0xff
```

**Every byte of that but two fields is data from the chip database.**
`cmd_hdr` and `cmd_ftr` are arrays of byte strings in the file itself, and
this module writes them verbatim; the two fields it fills in are the row
count in the `0x3b` command and, on request, the USERCODE in the `0x0a`
command. That is the same division of labour `src/fpga/xc7.rs` has with
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
  `nodes` table's 6306 `GLOBAL_CLK` memberships are, and a GW2A-18 has no
  `BUFG` bel anywhere in its grid.
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

## How far towards a bitstream this got

`ApiculaFabric::blank_stream` produces a **structurally complete `.fs`
file** for a GW2A-18: the right geometry, the database's own commands, the
row count, all 1342 check words, and the 462 bits the `const` tables
always set. It is 4 618 782 bytes and it **configures nothing**.

That is the honest end of phase one. There is no design in it, because
there is no way to put one in yet: the four things below are what stand
between it and a lit LED, in rough order of how much stands behind each.

1. **The attribute names.** Without `attrids.py`'s tables transcribed, an
   IO buffer has pins and no bits, and so does every flip-flop, IO logic
   block, PLL and block RAM. This is a transcription job with a clear
   source and it is the first thing to do.
2. **Per-tile pips in `Arch`.** Without them the `nodes` table cannot be
   loaded, so no clock reaches the global network and nothing sequential
   routes. This is a model change of maybe a hundred lines plus the text
   round-trip, and it helps the 7-series loader too.
3. **A configuration sequence.** Reading this part over JTAG is proven;
   writing it is not written at all. `reticle program` knows the TAP state
   machine and the FTDI encoding, and the part's instruction register is
   **eight bits** where a 7-series' is six, which is why the IDCODE is
   read with `jtag::idcode_after_reset` — an instruction nothing has to
   name. What a Gowin part wants after that (the `0x15` / `0x17` / `0x3a`
   family of instructions, an erase, a status poll) is phase two.
4. **A flip-flop's D pin, and packing.** A Gowin flip-flop's data input
   comes from the LUT beside it inside the slice and has no tile wire —
   the database's flip-flop portmap has no `D` at all, exactly as a
   7-series `AFF`'s does not. Until packing can place a LUT and a
   flip-flop on one slice, a sequential design will not route even with
   (1) and (2) done.

Until at least 1 and 2 are done, what this flow writes is a bitstream
whose envelope matches a working one and which contains no design.

## Where the code is

| | |
|---|---|
| `src/msgpack.rs` | the MessagePack reader, read-only, with the two guards that make a malformed file cheap to refuse |
| `src/fpga/gowin.rs` | the `.fs` container: the commands, the die bitmap, CRC-16/ARC, a reader that handles compression |
| `src/fpga/apicula/mod.rs` | the loader: the database as an `Arch` plus a `DieLayout`, with the measurement and the region |
| `src/fpga/apicula/parse.rs` | the four derived rules, each with the test that re-derives it |
| `src/fpga/devices/gowin.dev` | the device: primitives, pins, banks, the PLL, and a reason for each absence |
| `tests/fpga_gowin.rs` | all of it against the real database, skipping without one |
| `ApiculaDatabase::nodes`, `code_table`, `logicinfo` | what phase two needs, reachable without re-reading the file |

`src/fpga/arch/synthetic.rs` is untouched and still says what it always
said. `src/fpga/xray/` is untouched. Nothing here changes either.
