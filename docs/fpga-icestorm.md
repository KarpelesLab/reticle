# A real Lattice iCE40, and a real `.bin`

Reticle places, routes and writes a bitstream for the **iCE40UP5K** on its
real fabric, read from Project IceStorm's chip database, and the bitstream
loads and runs. Since 2026-11-19 it does the same for every other iCE40
die IceStorm maps and nextpnr places — the LP384, the LP and HX 1k and 8k
(the HX4K included) and the iCE40 Ultra — which no board here carries;
see "Every iCE40 part". This is the account of how that backend is built, what was
checked and how, and the faults found on the way. Like
[`fpga-trellis.md`](fpga-trellis.md), it says of every claim whether it was
**CHECKED** — measured, against a database, a vendor tool's output or a
board — or only **QUOTED** from somewhere.

```sh
reticle fetch icestorm            # once; needs a build with the `icestorm` feature
reticle fpga testdata/fpga/ice40up/uart_echo.v --device ice40-up5k-sg48 \
    --constraints testdata/fpga/ice40up/uart_echo.rcf --bitstream uart_echo.bin
reticle program --ice40 uart_echo.bin       # configuration memory only
reticle program --ice40 --write-flash uart_echo.bin   # and to stay
```

`docs/programming.md` ("iCE40: SPI, not JTAG") is the programmer's half.

## The database, and where a prebuilt one comes from

IceStorm's `chipdb-*.txt` files are not in its git repository: icebox's
Python writes them when IceStorm is built. So `reticle fetch icestorm`
takes a **prebuilt** copy — Debian's `fpga-icestorm-chipdb` package,
version `0~20250207git7fbf8c0+dfsg1-1` (Debian 13), architecture
independent, built from IceStorm commit `7fbf8c0` — checks the whole
`.deb` against a pinned SHA-256, and takes every file out of it: the six
dice's chip databases (`chipdb-384.txt`, `-1k`, `-lm4k`, `-u4k`, `-5k`,
`-8k`), the seven timing files (`timings_lp384.txt`, `_lp1k`, `_hx1k`,
`_lp8k`, `_hx8k`, `_u4k`, `_up5k`), and the package's `copyright`, which
carries IceStorm's ISC licence, saved as `LICENSE`: an 8 MB download, 112
MB unpacked. Until 2026-11-19 it took the 5k's two files only; a copy
cached then is completed from the same package, checked whole again, the
next time it is looked for (`Database::top_up`, which until then knew only
the per-file manifests of the other databases).

A `.deb` is an `ar` archive holding an xz-compressed tar. The `ar` and tar
readers are `src/bin/reticle/deb.rs`; the xz is `compcol`'s, as for
Project Apicula's databases, which is why the fetch is behind the
`icestorm` Cargo feature. The rest of the backend is in `fpga` and needs no
dependency: a database unpacked by hand and named by `RETICLE_ICESTORMDB`
works in any build.

Other prebuilt sources were looked at and not taken: oss-cad-suite and apio
tarballs (hundreds of megabytes for one 27 MB file), and nextpnr's own
compiled `.bba` databases (not icebox's text, so not what icebox's decoder
reads back).

## Every iCE40 part

Since 2026-11-19 the backend places on every die IceStorm maps that
nextpnr also supports, and the device files of the LP and HX parts are on
the real fabric rather than the synthetic one (`docs/fpga.md`, "Read this
first"):

| Part (device file) | Die | Package | Timing file | Has |
|---|---|---|---|---|
| `ice40-lp384-qn32` | 384 | `qn32` | `timings_lp384.txt` | logic only: no block RAM, no PLL |
| `ice40-lp1k-tq144` | 1k | `tq144` | `timings_lp1k.txt` | 16 block RAMs, one PLL |
| `ice40-hx1k-tq144` (iCEstick) | 1k | `tq144` | `timings_hx1k.txt` | the same |
| `ice40-hx1k-vq100` | 1k | `vq100` | `timings_hx1k.txt` | 16 block RAMs; the package does not bond the PLL |
| `ice40-hx4k-tq144` | 8k | `tq144:4k` | `timings_hx8k.txt` | 32 block RAMs, two PLLs |
| `ice40-hx8k-ct256` (HX8K breakout) | 8k | `ct256` | `timings_hx8k.txt` | 32 block RAMs, two PLLs |
| `ice40-lp8k-cm81` | 8k | `cm81` | `timings_lp8k.txt` | 32 block RAMs; one PLL (the `cm81` does not bond the top one) |
| `ice40-u4k-sg48` (iCE5LP4K) | u4k | `sg48` | `timings_u4k.txt` | 20 block RAMs, four DSP blocks, one PLL, both oscillators |
| `ice40-up5k-sg48` | 5k | `sg48` | `timings_up5k.txt` | everything above, and the board |

**No LP, HX or iCE40 Ultra part was on a board.** Everything below about
them was checked against IceStorm's data and tools and against nextpnr,
never against silicon. The lm4k die IceStorm also ships has no timing
file, no nextpnr support, and no device file here.

Every pin, count and grid of those device files is the chip database's
(`every_device_file_is_its_parts_database`): each package's pins are its
`.pins` section, the ones that can drive a global network marked `clock`.
The LP/HX lists used to be the pins of the boards that use the parts, the
CT256 one `pins partial`; every pin they had is in the new lists, as the
same kind.

### What differs between the dice

Read from nextpnr's `bitstream.cc` and checked by building on each die and
comparing with nextpnr's and icepack's output (below). QUOTED where the
only source is nextpnr's code.

| What | 384 | 1k | 8k | u4k | 5k |
|---|---|---|---|---|---|
| an IO block's input enable `IE` | set = enabled | **set = disabled**, and every IO block nothing uses has it set | set = enabled | set = enabled | set = enabled |
| `IoCtrl.cf_bit_39`/`35` beside a used block | — | — | — | — | set when the block has no pull-up |
| a block RAM's `RamConfig.PowerUp` | no RAM | **set on every RAM *not* used** | set on a used RAM | set on a used RAM | set on a used RAM |
| column buffers enabled | **none** | the `.colbuf` sources (rows 4, 5, 12, 13; a RAM's 3, 5, 11, 13) | rows 8, 9, 24, 25 | rows 4, 5, 16, 17 | rows 4, 5, 14, 15, 26, 27 |
| IP-connect and DSP pass-through cells | — | — | — | every such tile | every such tile but (25, 14) |
| a logic cell's data setup, added to its table's delay | +30 ps (LP) | +30 (LP), +20 (HX) | +30 (LP), +20 (HX) | −50 ps | −50 ps |

The 1k's two inversions are the ones `icebox_vlog` reads the same way, and
its own input-enable check (`-R`) passes on every 1k build here. That
check assumes the 1k's polarity on every die — it fails on nextpnr's own
8k, u4k and 5k builds as on this backend's — so it was run on the 1k only.
The column-buffer rows are nextpnr's, and on every die but the 384 they
are exactly the database's `.colbuf` sources, which is what the backend
reads (`colbuf_rows_are_nextpnrs`); on the 384 nextpnr enables none and
neither does this backend (QUOTED: no 384 build was ever loaded anywhere).
The setup offsets are nextpnr's `getPortClockingInfo`.

A **PLL** is per package as well as per die: IceStorm's `.extra_cell PLL`
record has a `LOCKED` line naming the packages that do not bond it (the
1k's in six packages, the 8k's top one in the `cm81`), and the database
reader refused that line until now — the 1k database did not parse. The
8k has two PLLs, at the middle of its bottom and top edges, so each PLL is
given its site before placement: the first the package has whose output IO
blocks are none of the design's pins (`assign_pll_sites`). A PLL on a part
without one is refused by name, and so is a hard block the part lacks —
the device file does not declare it, so synthesis names the primitive and
the device (`what_a_part_lacks_is_refused_by_name`).

### How they were checked

Five designs — `testdata/fpga/ice40/ff_zoo.v` (every kind of register,
both clock edges, enables, synchronous and asynchronous sets and resets,
a carry chain), `uart_echo.v`, `testdata/fpga/ice40/bram_check.v` (a
256 x 16 and a 512 x 8 block RAM, contents from `initial`, written at run
time, every word read on the pins), and both PLL designs — were built on
every part that has what they need: 39 builds. For each:

1. **every set bit decodes** and the arcs are exactly the router's, the
   block RAMs and PLLs read back as placed — `implement` returns nothing
   else, and `every_part_builds_its_designs_and_every_bit_decodes` builds
   each design on each die (one package per die);
2. **icepack's `.bin` of Reticle's `.asc` is Reticle's `.bin`**, byte for
   byte from the synchronisation word: 39 of 39;
3. **IceStorm's own decode, simulated against the source**: `icebox_vlog`
   turned each `.asc` into Verilog, which `reticle sim` ran beside the
   source on the same stimulus — random inputs every cycle for `ff_zoo`
   (outputs compared before each edge of the clock), forty serial frames
   for `uart_echo` (`tx` compared every cycle), 3 000 cycles of random
   writes for `bram_check` (every word read compared, the decode's block
   RAMs simulated by a model written from the library's description of
   the cell): **26 of 26 builds match the source on every cycle compared**
   — every part's `ff_zoo` and `uart_echo`, every part with block RAM's
   `bram_check` — and so does every nextpnr build of the same netlists.
   The PLL designs were not simulated (a PLL has no model here, and
   `icebox_vlog` stops on the 5k's PLL record, nextpnr's builds and this
   backend's alike);
4. **nextpnr's builds of the same netlists decode cleanly**: Reticle's JSON
   export placed and routed by nextpnr-ice40 (built from its repository at
   `1759078` with every die), packed by icepack, then read back by this
   backend's decoder with nothing unexplained
   (`every_bit_of_a_reference_decodes`, given the files) and icepack's
   bytes reproduced (`icepacks_bytes_are_reproduced`): 38 of 38 builds,
   0 bits unexplained in any. nextpnr did not route the 39th, `pll2f_uart`,
   on the HX4K or the LP8K within ten minutes (it stalls with one or two
   wires overused; the same netlist routed on the HX8K's CT256): this
   backend's builds of both decode, pack and simulate where the others do.

All 39 checks CHECKED on 2026-11-19, by hand: none of IceStorm's Python,
nextpnr or the simulation's testbenches is in CI. The testbenches and
the scripts are not committed; the designs are.

What the simulation is worth was checked by mutation: one lookup-table
bit of `uart_echo`'s HX1K build flipped in the `.asc` made the decode
differ from the source on 21 944 cycles, another on 14 664, and a third,
a bit of a table input the cell does not read, on none.

The **timing analysis** was held to nextpnr on the same 38 nextpnr builds
(`the_timing_analysis_agrees_with_nextpnr_and_icetime`, given the files
named `<device>--<design>.asc`, each part timed with its own figures and
its family's setup): every clock of every design within 4.0 % of
nextpnr's report, most within 1 % — CHECKED. `ff_zoo`, the one design on
both edges of its clock, is shown and not held: nextpnr times a path from
one edge to the other as half a period, which this analysis does not time,
and on the 8k it reports 239.6 and 151.8 MHz (HX8K, LP8K) where this
analysis, which leaves that path out, says 285.8 and 192.4.

### Faults found on the way to the other parts

- **The 1k database did not parse.** Its PLL record has a `LOCKED` line,
  a list of packages, which the reader took for a port entry and refused.
  Found by the first load of `chipdb-1k.txt`; `LOCKED` is now kept as the
  packages the cell is absent from.
- **Both clock edges in one logic tile were refused after placement.**
  `ff_zoo` on the HX1K was refused with "holds flip-flops on both clock
  edges": the placer had no idea that a tile's eight cells share one
  `NegClk` bit. It does now — an `ICESTORM_LC` on a falling edge has its
  own tile mode (`place::slice_mode`, the mechanism the 7 series' shared
  `CLKINV` already used) — and the backend's check stays as the check that
  it worked. This was in "What this does not do yet".
- **`cf_bit_39`/`35` were written beside every used UltraPlus IO block.**
  nextpnr writes them only for a block without a pull-up (with the default
  100 kΩ pull-up, neither). No board design had a pull-up, so every
  reference agreed; read in `bitstream.cc` while porting the other dice's
  rules, and the backend now writes them as nextpnr does. Not measured.

## What the database becomes

`src/fpga/icestorm/db.rs` parses the text; `src/fpga/icestorm/mod.rs`
builds the flow's `Arch` from it. CHECKED, by
`the_5k_fabric_has_the_parts_shape`: 103 383 nets become 103 383 nodes,
1 219 112 multiplexer options become as many pips, 5 280 logic cells, 96
IO blocks, and **no reference resolves to nothing**. It builds in well
under a second.

- **One tile type per grid position**, corners included. icebox's tiles of
  one kind are not alike at the edges, and the database states each tile's
  multiplexers separately anyway.
- **One wire per net**, declared in the tile at the corner of the box its
  segments cover, so the placer's distance estimates see a long wire as
  long. The eight `glb_netwk_*` nets are the architecture's globals.
- **One pip per multiplexer option**, its bits the ones of the option's
  pattern. A pattern's zeros are not stated: every bit belongs to exactly
  one multiplexer, and only one option of a multiplexer is ever used.
- A **logic cell is one bel** (`lc`), with the lookup table's four inputs,
  the carry in and out, the output, and the tile's shared clock, enable and
  set/reset. A second bel on the same cell (`lcc`) has no clock, enable or
  set/reset pin; the two exclude each other. That second bel is not a
  nicety, see "Faults found".
- An `io` bel per IO block, a `gb` bel per global buffer (`.gbufin`), and
  bels for the RGB LED driver and the two oscillators, with pins on the
  fabric wires the database's `.extra_cell` records name.
- A `bram` bel per block RAM and a `spram` bel per single-port RAM: see
  "Block RAM and single-port RAM" below; and a `mac16` bel per DSP block,
  see "DSP blocks"; and a `pll` bel, see "The PLL and the oscillators".

## Packing: three cells are one

An iCE40 logic cell's flip-flop takes its data from its own lookup table
and nowhere else, and its carry's operands are the table's second and third
inputs. `src/fpga/icestorm/pack.rs` therefore turns the mapped netlist's
`SB_LUT4`, `SB_CARRY` and `SB_DFF*` into `ICESTORM_LC` cells before the
placer sees them, by eight rules its module documentation lists. The ones
worth knowing here:

- a carry takes the table that reads only its operands and its carry in,
  **with the table's inputs moved** onto the carry's wires and its truth
  table rewritten to match (Reticle's mapping writes a sum bit as a table of
  the carry in and one operand on inputs 0 and 1);
- a chain whose carry in is a constant starts with a *feed-in* cell, and a
  chain's last carry out read by logic gets a *feed-out* cell above it,
  because a carry out reaches nothing but the cell above;
- a constant on a table's input is folded into the truth table, because an
  unrouted input reads zero (CHECKED: `icebox_vlog`'s default for every
  logic input, and every board design here depends on it);
- a register declared to start at one is refused (a fault, below).

An enable over a synchronous reset is no longer packing's business: the
`SB_DFFESR` that reaches packing already has `E | R` on its enable,
because flip-flop mapping put it there for every consumer of the netlist
(see "A reset that waited for an enable").

The rigid shapes the carry chain needs are not stated anywhere: the placer
finds them on its own, because a cell's carry in is a wire only the cell
below drives (`place::build_clusters`). Nothing in `place.rs` names an
iCE40.

**Equivalence is held across packing**, the same guarantee
`every_block_maps_to_the_logic_it_was_mapped_from` gives mapping. The
`packing` tests in `tests/fpga_icestorm.rs` replace every cell of the mapped
netlist with a model of the iCE Technology Library cell — as Yosys's
`techlibs/ice40/cells_sim.v` writes it, so `SB_DFFESR` is `if (E) Q <= R ?
0 : D` — and every packed cell with a model of the *silicon* as
`icebox_vlog` decodes it, and prove the two equivalent for a counter with
enable and reset, a down counter with a zero flag, and an adder with its
carry out. Moving a table's inputs without its truth table makes the
counter differ at frame 1 (CHECKED by mutation, before the change below).

**And the mapped netlist is held to the source.** The
`*_maps_to_the_logic_it_was_written_as` tests prove the mapped netlist,
read through the same library models, equivalent to the source with only
its processes lowered, for every flip-flop shape with an enable and a set
or reset: synchronous over an enable (reset to `0101`, so both
`SB_DFFESR` and `SB_DFFESS`), active-low, falling-edge, asynchronous with
an enable, and synchronous with none. Removing the enable widening from
mapping makes the first three differ at frame 2; widening by the reset's
wire rather than its active level makes the active-low one differ at
frame 1 (both CHECKED by mutation). What they do not see: a register's
power-up value, because the formal engine starts a register that has a
reset at its reset value whatever the silicon does.

## The bits a database does not state

Read off four bitstreams that nextpnr and `icepack` wrote for this board's
part, decoded through the database (`every_bit_of_a_reference_decodes`
decodes them and finds **every set bit explained**: 10 243 to 24 263 bits
each). Three things are in every
one of them and no database record says to write them; `IceFabric::configure`
writes all three:

| What | Where | CHECKED by |
|---|---|---|
| an IO block's input enable (`IE`) and pull-up disable (`REN`) are in *another* block's bits, the one `.ieren` names; on the 5k `IE` set means enabled, and `REN` set means no pull-up | the paired block of the same tile | all four references; and a design that echoes the FT2232H's serial line through the part works |
| a used IO block also sets `IoCtrl.cf_bit_39` (paired block 0) or `cf_bit_35` (paired block 1) — when it has no pull-up, as every block of those references had (corrected 2026-11-19, below) | the same | all four references, input and output alike; meaning not known |
| every column buffer of every global network enabled, 1 248 bits | every `.colbuf` source tile | all four references, identically, whatever the design |
| every IP-connect and DSP tile's eight logic cells set to pass their third input through, `11000011001100001100`, with every `Cascade.*` bit beside them | 59 of the 60 such tiles: not the IP-connect tile at (25, 14), see below | all four references, identically |

**Corrected, 2026-11-18**: that row said "all 60 such tiles", and this
backend wrote all 60. The references say 59. nextpnr leaves (25, 14) alone
— "mystery bits not set in this one tile, other than to route through the
DSP carry out if used", `bitstream.cc` — and sets only that tile's first
cell when the DSP block below it, at (25, 10), is used, because the block's
carry out leaves through that cell (`CO 25 14 slf_op_0` in its record).
A comparison of every IP-connect and DSP tile of `uart_echo` against
nextpnr's build found the one tile that differed; the earlier check had
looked at whether the pattern was there, not at where it was not. The
designs ran either way. The backend now does what nextpnr does, which is 72
bits fewer in every design, and **the carry out needs that cell**: with it
left out, `dsp_selftest`'s carry outs read wrong on 19 of 64 lines on the
board (CHECKED by mutation).

The RGB LED driver's parameters set the bits its `.extra_cell` record names
(`RGBA_DRV_EN`, `CURRENT_MODE`, `RGB<n>_CURRENT_<i>`), and its three outputs
*are* pads 39 to 41 with nothing written in their IO tiles. CHECKED: for the
same parameters, `rgb_blink.v` sets exactly the configuration bits a
reference bitstream sets for its RGB driver.

## Block RAM and single-port RAM

**Block RAM** (`SB_RAM40_4K`, thirty of them). A block RAM is two tiles, a
`ramb` and the `ramt` above it, and is one `bram` bel anchored on the
`ramb`. Its pins are the `ram/*` wires of both tiles; which side is in
which tile differs between family members — on the 5k the read side is in
the bottom tile, `RADDR`, `RCLK`, `RE`, `RCLKE` and data bits 8 to 15, and
the write side in the top one (CHECKED against the database; nextpnr's
`chipdb.py` calls these "8K-style memories") — so each wire is looked for in
both tiles, as `icebox_vlog` does. The pin roles are the ones the device
file's `port` lines give the primitive: `p0_*` the read port, `p1_*` the
write port, and `MASK0`..`MASK15` for the mask no `port` line names.

Its bits, from nextpnr's `bitstream.cc` and read back the same way by
`icebox_vlog`:

| Bit | Where | Meaning |
|---|---|---|
| `RamConfig.PowerUp` | `ramb` | set: the RAM is in use (on every die but the 1k, which sets it on the RAMs *not* in use: "Every iCE40 part") |
| `RamConfig.CBIT_0`, `CBIT_1` | `ramt` | `WRITE_MODE` bits 0 and 1 |
| `RamConfig.CBIT_2`, `CBIT_3` | `ramt` | `READ_MODE` bits 0 and 1 |
| `.ram_data X Y` | the `ramb`'s position | the contents: line `k` is `INIT_<k>`, bit `i` of the line bit `i` of the parameter |

`NegClk` in either tile is a clock inverted; no Reticle mapping asks for
one, and the two sources disagree about which tile holds which clock on
the 5k (`icebox_vlog` reads the bottom tile as the *write* clock there,
nextpnr writes the *read* clock into it), so neither is relied on.

An unrouted `RCLKE` or `WCLKE` reads one and every other unrouted RAM input
zero (`icebox_vlog`'s defaults, QUOTED). So packing (rule 9) wires every
input bit tied to a one — a read enable, an `MASKWREN` bit — to a cell
driving one, leaves a zero unrouted, and refuses a clock enable tied to
zero.

**Single-port RAM** (`SB_SPRAM256KA`, four of 16K x 16). They sit at the four
places IceStorm's `.extra_cell SPRAM` records give, two in each bottom
corner, with their ports on wires of the IP-connect tiles above (`ADDRESS_0`
is `lutff_0/in_1` of the tile at (0, 2), `DATAOUT_0` is `slf_op_0` of (0,
1)). Each is a `spram` bel there; its one bit is `SPRAM_EN`, which the
record names as `CBIT_0` or `CBIT_1` of the IP-connect tile at (0, 1) or
(25, 1). `POWEROFF` is active low, so a design ties it high, and packing
wires that one to the cell driving one like any other.

**The third check.** `implement` reads the RAMs back out of the bitstream —
which block RAMs are powered, their modes from the `ramt` bits, their
contents from `.ram_data`, and which single-port RAMs are enabled — and
returns nothing unless that is exactly what was placed, each block RAM with
the modes and contents its cell's parameters ask for. Contents for a block
RAM that is not powered are an unexplained bit. CHECKED: dropping the
contents on their way into the image makes it fail; but a fault made the
same way on the way in and on the way back would not, so the layout is the
board's to confirm (below), and it did.

**Against nextpnr.** `ram_selftest.v`, exported by Reticle and placed and
routed by nextpnr, gives an `.asc` that this backend decodes with nothing
unexplained — RAM bits and contents included — into the same eight block
RAMs, modes and contents, as Reticle's own build of the same design
(`every_bit_of_a_reference_decodes`, given the file in
`RETICLE_ICE40_REF`); and `icepack`'s `.bin` of it, block RAM banks and
all, is reproduced byte for byte (`icepacks_bytes_are_reproduced`). Both
CHECKED, by hand on this machine; no reference ships.

## DSP blocks (`SB_MAC16`)

Eight, at the places IceStorm's `.extra_cell MAC16` records give: (0, 5),
(0, 10), (0, 15), (0, 23) and the same rows at x = 25. Each record names
every port bit's wire — `A_0` is `lutff_0/in_3` of the block's third tile,
`O_17` is `mult/O_17` of its third — and every parameter bit as a `CBIT_<i>`
of one of the block's tiles, so a block is a `mac16` bel on its anchor tile
like a single-port RAM, with its pins the ports bit by bit (`A3`, `CLK0`,
`O31`). The cascade ports (`ACCUMCI`, `ACCUMCO`, `SIGNEXTIN`, `SIGNEXTOUT`)
have no fabric wire; a design connecting an input of them is refused.

Its nineteen parameters are nextpnr's list (`bitstream.cc`), each bit of
each where the record says (`IceFabric::configure_dsps`), with two
exceptions found on the board, below. `NEG_TRIGGER` has no bit, and a cell
asking for it is refused. A constant on a port: a one goes to a cell
driving one, like every other hard cell's (packing rule 9); a zero is left
unrouted, because an unrouted DSP input reads zero — QUOTED from nextpnr's
packer, which does the same, and CHECKED by the board, where the blocks'
unused holds, loads and adder controls are exactly that; and a `CE` tied to
zero is refused.

**The third check, extended.** `implement` reads every block's nineteen
parameters back out of the bits and returns nothing unless each placed
block's are what its cell asks for and every other block's are zero. A
block whose parameters are all zero (a plain unregistered 16 x 16 multiply)
reads back the same as an unused one; nothing in the bits says which.
CHECKED against nextpnr: the same eight configurations read back from its
build of `dsp_selftest.v` as from this backend's, but for the two names
below (`every_bit_of_a_reference_decodes`, given the file).

**The model.** `tests/ice40_mac16` is a model of the cell, cycle by cycle,
written from Yosys's `techlibs/ice40/cells_sim.v` (`SB_MAC16`): the input
registers `C`, `A`, `B`, `D`; four 8 x 8 partial products `F` (`A[15:8] x
B[15:8]`), `J` (`A[7:0] x B[15:8]`), `K` (`A[15:8] x B[7:0]`), `G` (`A[7:0]
x B[7:0]`) and their registers; their 32-bit sum `L` and its register `H`;
and the two 16-bit adder/accumulators. Checked by
`the_dsp_model_multiplies_and_accumulates` against plain arithmetic —
unsigned and signed 16 x 16 products of the corners, the split's two 8 x 8
products, an accumulator — and by the board, below.

**On the board**, `dsp_selftest.v` puts all eight blocks to work, each
configured another way, on one stream of 64 vectors (eight corner cases,
then a linear-feedback sequence), and prints each step's vector and all
eight 32-bit outputs and carry outs; the host replays the stream through
the model with each block's parameters and wiring and wants every digit
equal (`every_dsp_block_computes_what_the_model_says_on_the_board`). The
blocks: a 16 x 16 unsigned multiply; a signed one; one signed by unsigned
with every pipeline register on; the 8 x 8 split, signed by unsigned, its
top product registered; the split unsigned with each product added to `C`
or a registered `D`; a 32-bit multiply-accumulate loaded from a registered
`C`; the same signed, with add/subtract, a hold on `A` and on the bottom
accumulator, and the carry select's XOR; and a 32-bit adder carrying in
from the fabric's `CI`. 2026-11-18, every output and carry out of every
step as the model says, twice — CHECKED. Every bit of every parameter
changes that report under the model, in at least one block
(`every_dsp_parameter_bit_shows_in_the_report`), so a bit written into
the wrong place cannot pass it; planted on the board, `A_SIGNED` and
`B_SIGNED` swapped both ways — which `implement`'s own check cannot see —
made two blocks disagree on 40 lines (CHECKED by mutation).

## The PLL and the oscillators

**`SB_PLL40_CORE` and `SB_PLL40_2F_CORE`.** One PLL, the `.extra_cell PLL`
record at (12, 31): a `pll` bel whose inputs are the record's wires
(`REFERENCECLK` is `fabout` of (10, 31), `LOCK` is `neigh_op_tnl_1` of (1,
30)) and whose outputs are not wires but IO blocks: `PLLOUT_A 12 31 1`
means the clock comes out of that block's `D_IN_0` into the fabric
(`PLLOUTCORE`) and onto the global network the block's pad drives, 7
(`PLLOUTGLOBAL`); port B the same through (13, 31, 0) and network 2.
nextpnr's `chipdb.py` (`add_pll_clock_output`) reads them the same way.
Those two IO blocks are therefore not pins while the PLL is placed (pins 35
and 37), and the placer refuses a design that wants both.

Its bits, as nextpnr's `bitstream.cc` and `pack.cc` write them: sixteen
parameters as `PLLCONFIG_<i>` bits of the IO tiles (10..14, 31), the
library's names translated — `PLLTYPE` 3 for the CORE form and 7 for 2F,
`FEEDBACK_PATH` `SIMPLE` 1, `PLLOUT_SELECT` `GENCLK` 0 and `GENCLK_HALF` 1,
a `DYNAMIC` delay its mode bit with the field all ones; each output's IO
block `PINTYPE_0` (and `PINTYPE_1` with its ICEGATE); and for each output
driving its network, that network's `padin_glb_netwk` extra bit, the one a
pad sets. `implement` reads the parameters and the pad-input bits back and
refuses a bitstream that differs from what was placed (`PllConfig`).

Three things the placer and router had to be told:

- **`LOCK` reaches one tile.** Its wire feeds only the lookup-table inputs
  of the logic tile at (1, 30), so packing puts a pass-through cell between
  the PLL and its readers (rule 10) and `implement` confines that cell to
  that tile with a region constraint. nextpnr does the same.
- **A network has two drivers.** Network 7 is driven by the PLL through its
  pad and by the global buffer fed from the fabric at its `.gbufin` tile,
  (19, 0). The first build put the design's clock on that buffer and the
  PLL's output on the same network: 2 signals on `glb_netwk_7`. The PLL's
  bel now blocks those buffers (a `blocks` name may be another tile's bel,
  `X19Y0/gb7`), and since the placer seats clock buffers before a cell with
  one site, `implement` confines the design's clock buffers to the largest
  rectangle of the die holding none of the blocked ones. A placed PLL costs
  a design one or two of its eight global buffers.
- **Mapping must not buffer the global output.** It put an `SB_GB` behind
  `PLLOUTGLOBAL`, which nothing can route to; a `pll` line's `outglobal`
  port is on the network already, as a buffer's output is.

An instance of a PLL by name is now a cell (it used to stay an instance,
which nextpnr's JSON check refused too); `BYPASS` tied to zero is left
unrouted, and reads zero: CHECKED by the board, where a bypassed PLL would
have run the transmitter at 12 MHz.

**The pad forms** (`SB_PLL40_PAD`, `_2_PAD`, `_2F_PAD`) take their
reference from the PLL's own pad, the IO block its first output leaves
through: pin 35 of the UltraPlus's and the iCE40 Ultra's SG48, 49 of the
1k's TQ144, R9 of the 8k's CT256. Until 2026-11-19 they were refused; now
packing takes the link off as nextpnr does (`place_plls`, `pack_plls`):
the `PACKAGEPIN` must be read from an input pin's IO buffer and nothing
else, that buffer is removed, the PLL is put on the site whose pad that
pin is (refused by name on any other pin, naming the pads there are), and
the pad's IO block is configured as nextpnr's `sb_io_used_by_pll_pad` does,
input enabled. `PLLTYPE` is 2, 4 and 6. On the UltraPlus, the HX1K's
TQ144 and the HX8K's CT256, nextpnr's builds of the same netlists decode
with nothing unexplained, icepack's bytes reproduced, and **every IO
configuration and `PLLCONFIG` bit of every tile the PLL's record names is
the same** in this backend's build and nextpnr's — 612, 468 and 936 bits
for each of `SB_PLL40_PAD` and `SB_PLL40_2F_PAD` — with the same
pad-input extra bit (CHECKED, 2026-11-19;
`the_pll_pad_forms_build_on_every_die` builds all three forms on four
dice). **Not on a board**: the attached one's oscillator is on pin 20,
not on the PLL's pad.

**The solver.** `fpga::pll::solve` on the UltraPlus's `SB_PLL40_CORE` line
gives the same DIVR, DIVF, DIVQ and FILTER_RANGE as IceStorm's `icepll`
(built from its source) for all 4 667 requests tried: every output from 16
to 275 MHz in steps of 0.1 MHz from 12 MHz, and of 0.5 MHz from 10, 25, 30
and 100 MHz — CHECKED (`the_pll_solver_gives_what_icepll_gives`, given the
binary in `RETICLE_ICEPLL`). Both search the same ranges (reference 10..133
MHz, phase detector 10..133, VCO 533..1066, DIVF up to 127 in `SIMPLE`
mode, DIVQ 1..6) in the same order with the same strict tie-break, and
compute `out = in * (DIVF + 1) / ((DIVR + 1) * 2^DIVQ)`, the `SIMPLE`
feedback equation of Lattice's iCE40 sysCLOCK PLL design guide (TN1251) —
QUOTED: the guide was not to hand, and icepll's source is what was read
against; icepll's own comment says the guide's DIVF limit of 63 holds only
outside `SIMPLE` mode.

**Against nextpnr.** nextpnr places and routes Reticle's export of both PLL
designs, and its bitstreams decode to the same PLL parameters and the same
pad-input networks as this backend's (`every_bit_of_a_reference_decodes`,
given the files). nextpnr also reports their 48 MHz domain at 42.9 MHz
under its timing model — a design this backend placed with no timing at
all, which ran on the board at 48; see "Timing" below.

**On the board**, 2026-11-18:

| Design | What it shows | Result |
|---|---|---|
| `pll_uart.v` | `SB_PLL40_CORE`, 12 MHz to 48 (DIVR 0, DIVF 63, DIVQ 4, FILTER_RANGE 1, what icepll prints), its global output clocking a transmitter at 417 clocks a bit, which is 115200 baud at 48 MHz and garbage at any other frequency; each line counts the PLL's cycles in 2^20 of the reference's, and reports `LOCK` | every line `L1 N400000`: exactly four times the board's oscillator, locked — CHECKED |
| `pll2f_uart.v` | `SB_PLL40_2F_CORE`, port A `GENCLK` and port B `GENCLK_HALF`, both global, B counted in a domain of its own | every line `L1 N400000 H200000` — CHECKED |
| `osc_count.v` | `SB_HFOSC` divided to 12 MHz and `SB_LFOSC`, each clocking a counter, counted against the crystal | 12.022 MHz and 9 989 Hz, within the data sheet's tolerances — CHECKED |

Planted on the board, DIVF's bits 5 and 6 swapped both ways — which
`implement`'s readback cannot see — turned the PLL's lines into garbage
(DIVF 95, a 72 MHz clock): CHECKED by mutation.

### The oscillators had never driven anything

`SB_HFOSC` and `SB_LFOSC` had bels from the start, and nothing had run on
one. The first `osc_count` build counted **zero** cycles of either. Their
clocks are global networks 4 and 5 themselves (`CLKHF 0 29 glb_netwk_4`),
reached through each network's pad-input path, and nextpnr's packer puts a
pad-input global buffer behind each (`create_padin_gbuf`), which sets that
network's `padin_glb_netwk` bit; this backend set none. It does now, the
readback counts the bit, and mapping no longer puts a global buffer behind
an oscillator either (`global_outputs` on its `bel` line). Like the PLL,
an oscillator's bel blocks the fabric's buffer onto its network.

## Timing

`reticle fetch icestorm` has always taken `timings_up5k.txt`; it is read
now. With it beside the chip database, every build reports each clock's
longest register-to-register path, and `--timing-driven` places and routes
by it (`src/fpga/icestorm/timing.rs`).

**The figures.** icetime's format: per cell, `IOPATH` arcs and `SETUP`
lines, each figure `min:typ:max` for rise and fall, in picoseconds. One
number is kept per arc, the larger `max`, as nextpnr's `chipdb.py` reads
the file (icetime takes the largest of all six, which is the same number
here).

**A connection's cell** is not in the database: a multiplexer is a
`LocalMux`, an `InMux`, a `Span4Mux` and so on by the wires it joins. The
rule is nextpnr's `pipdelay`, ported line for line — the destination's kind
decides, and for a span-4 the source's too. icetime chooses differently (a
span's multiplexer by how far along the span the connection leaves it,
`Odrv4`/`Odrv12` at a cell's output, a `CascadeMux` on a third input).

**The analysis** (`timing::analyse`) reads the image back — the arcs, each
logic cell's table, carry and flip-flop, the block RAMs — and finds per
clock the longest path from a register to a register: clock-to-output,
routing and logic, then setup (a data input's table delay less 50 ps, as
nextpnr has it for the UltraPlus; 100 ps for any other register input).
Paths between clocks and to or from pins are not timed; a single-port
RAM's ports are taken for registers, as nextpnr takes them, and a DSP
block's were until 2026-11-19 — an unregistered multiply is now logic
("How fast the DSP multiplier is"), and the `dsp_selftest` rows below were
measured before that.

**Checked against nextpnr and icetime** on the same bitstreams
(`the_timing_analysis_agrees_with_nextpnr_and_icetime`, given the files in
`RETICLE_ICE40_TIMING_REF`): six designs nextpnr placed and routed from
Reticle's export, and Reticle's own builds of two, plain and timing-driven.

| Bitstream | nextpnr's report, per clock | this analysis | icetime `-i` | this analysis |
|---|---|---|---|---|
| `uart_echo`, nextpnr's | 64.66 MHz | 64.41 MHz (−0.4 %) | 15.19 ns | 15.53 ns (+2.2 %) |
| `ram_selftest`, nextpnr's | 29.72 MHz | 29.71 MHz (−0.0 %) | 33.69 ns | 33.66 ns (−0.1 %) |
| `dsp_selftest`, nextpnr's | 48.69 MHz | 48.83 MHz (+0.3 %) | 31.78 ns | 20.48 ns (−35.6 %) |
| `pll_uart`, nextpnr's | 48.69 MHz; `sys` 42.91 | 48.70 (+0.0 %); 42.94 (+0.1 %) | | |
| `pll2f_uart`, nextpnr's | 48.69; `half` 45.04; `sys` 42.72 | 48.89 (+0.4 %); 45.02 (−0.1 %); 41.62 (−2.6 %) | | |
| `osc_count`, nextpnr's | 48.69; `hf` 43.94; `lf` 56.32 | 46.92 (−3.6 %); 43.97 (+0.1 %); 56.29 (−0.0 %) | | |
| `uart_echo`, Reticle's | | | 15.21 ns | 15.61 ns (+2.6 %) |
| `uart_echo`, Reticle's, timing-driven | | | 13.98 ns | 14.38 ns (+2.8 %) |
| `ram_selftest`, Reticle's | | | 41.16 ns | 42.25 ns (+2.7 %) |
| `ram_selftest`, Reticle's, timing-driven | | | 37.13 ns | 37.72 ns (+1.6 %) |

So: within 3.6 % of nextpnr on every clock of six designs, most within
0.5 % — its model, ported; and within 2.8 % of icetime, an independent
model, on every single-clock design without a DSP block — CHECKED, the
bound asserted. Two disagreements were understood and not fixed: icetime
times a DSP block's multiplier as logic (its `SB_MAC16_*` cells), where
nextpnr and this analysis took the block's ports for registers, so for
`dsp_selftest`'s unregistered multipliers icetime's 31.78 ns was the more
honest figure — measured on the board since, and the analysis now times
that path too ("How fast the DSP multiplier is"); and icetime times every
path as one whatever its clocks,
so it is not compared on a design of several. The −3.6 % on `osc_count`'s
crystal clock and −2.6 % on `pll2f_uart`'s `sys` were not run down. None
of the three is the silicon: `pll_uart` ran at 48 MHz on the board from a
build all three time at 41 to 43 MHz, as a slow-corner model should.

**Timing-driven placement and routing** (`--timing-driven`) use the
machinery the 7-series flow does (`docs/fpga-placement.md`): a
`PlaceTiming` per netlist pin — a registered cell's output starts paths
and the table inputs it reads end them, an unregistered one's pass them on
with each input's own delay to the output and to the carry
(`timing::place_timing`) — weighs each signal by its criticality over three
rounds of annealing, and the router weighs delay against congestion per
connection, from each node's delay by its wire's kind
(`timing::node_delays`). A connection is estimated before routing at
`1.76 + 0.12 * tiles` ns (`timing::WIRE`: a `LocalMux` and an `InMux` to the
next tile, a span's multiplexer every four or twelve).

What it buys, five seeds each (`the_timing_driven_flow_is_faster`, ignored;
the slowest clock's frequency by the analysis, least / median / most):

| Design | wirelength | timing-driven |
|---|---|---|
| `uart_echo`, 12 MHz constraint | 55.0 / 58.4 / 62.6 MHz | 62.1 / 66.4 / 68.4 MHz |
| `ram_selftest`, 12 MHz | 21.8 / 22.6 / 23.4 | 25.8 / 26.1 / 28.6 |
| `dsp_selftest`, 12 MHz | 38.5 / 41.0 / 43.6 | 35.9 / 48.0 / 50.3 |
| `pll_uart`, 48 MHz from the PLL | 39.7 / 42.8 / 45.1 | 46.9 / 48.5 / 48.8 |
| `pll2f_uart`, 48 and 24 | 34.8 / 36.2 / 38.7 | 28.0 / 40.4 / 43.2 |
| `osc_count`, 12 MHz and the oscillators | 34.7 / 39.7 / 41.8 | 42.1 / 43.3 / 44.7 |
| `uart_echo`, every clock asked 10 ns | 55.0 / 58.4 / 62.6 | 64.3 / 66.5 / 69.9 |
| `ram_selftest`, 10 ns | 21.8 / 22.6 / 23.4 | 26.5 / 28.1 / 28.5 |
| `dsp_selftest`, 10 ns | 38.5 / 41.0 / 43.6 | 49.5 / 53.2 / 56.1 |
| `osc_count`, 10 ns | 34.7 / 39.7 / 41.8 | 40.7 / 42.7 / 45.6 |

The median rises on every design, by 7 to 30 %. With the designs' own
constraints the least does not always: a design whose 12 MHz leaves every
path slack gives the timing rounds nothing to weigh, and one seed of
`dsp_selftest` came out slower than any wirelength placement; `pll2f_uart`'s
28.0 MHz is its 24 MHz clock, which meets its period. Asked for 10 ns, every
seed of every design is faster than the wirelength placement's median, and
for three of the four faster than its best. `pll_uart`, which nextpnr
places at 42.9 MHz and the plain flow at 42.8 in the median, meets its 48
MHz in timing-driven builds — and one so built ran on the board, every line
`L1 N400000` (CHECKED, 2026-11-18).

## The UltraPlus's other hard IP, and warm boot

Since 2026-11-19 the backend places every hard block the 5k's database
records: the two I2C and two SPI blocks, the LED dimmer (`SB_LEDDA_IP`),
the warm-boot block (`SB_WARMBOOT`, on every die), the open-drain pads
(`SB_IO_OD`) and the I3C pads (`SB_IO_I3C`). Each is a bel at the place
its `.extra_cell` record gives, with a pin per port on the wire the record
names; their ports are one bit each in the library (`SBADRI3` is a port),
so a pin's role is the port's name and bit 0.

| Block | Site | Bits written | Read back by the decode check |
|---|---|---|---|
| `SB_SPI` | by `BUS_ADDR74`: `0b0000` the block at (0, 0), `0b0010` the one at (25, 0) | `SPI_ENABLE_0`..`3`, set when used | yes |
| `SB_I2C` | `0b0001` the block at (0, 31), `0b0011` the one at (25, 31) | `I2C_ENABLE_0`, `_1`; `SDA_INPUT_DELAYED`, `SDA_OUTPUT_DELAYED` on unless an attribute clears them | yes |
| `SB_LEDDA_IP` | (0, 31) | none | nothing to read |
| `SB_WARMBOOT` | (25, 0) on the 5k, (0, 0) on the others | none | nothing to read |
| `SB_IO_OD` | the IO blocks of pins 39 to 41 only | an IO block's, as `SB_IO` | as an IO block |
| `SB_IO_I3C` | the IO blocks of pins 23 and 25 only, with its `PU_ENB`, `WEAK_PU_ENB` routed to the record's wires | an IO block's | as an IO block |

The bus addresses and the bits are nextpnr's (`pack.cc`, `map_ba74`;
`bitstream.cc`), QUOTED. `SB_IO_OD` is the plain IO cell on the pads the
RGB driver can drive: IceStorm's `ultraplus.rst` says so ("if the LED
driver is ignored or disabled on a pin then the pin can be used as an
open-drain IO using the standard IO cell") and so does Lattice's iCE40 LED
Driver Usage Guide (TN1288, Appendix B, which instantiates it on those
pins), both QUOTED; nextpnr does not support it. Neither does it support
`SB_IO_I3C`. Both are refused by name on any other pad
(`what_the_hard_ip_refuses`), and the device file says that an instance
of either is the port's IO buffer, so none is inserted beside it. The
decode check now reads back the configuration bits of every hard cell that
has any — the RGB driver, the high-frequency oscillator, the I2C and SPI
blocks — and refuses a bitstream whose bits are not the placed cells'
(`Decoded::hard_ip`).

**Against nextpnr**: nextpnr's builds of `sbus_probe.v` and
`ledda_breathe.v` from Reticle's export decode with nothing unexplained,
icepack's bytes reproduced, to the same hard cells at the same sites with
the same bits as this backend's builds (`every_bit_of_a_reference_decodes`,
given the files) — CHECKED, 2026-11-19.

**On the board** (2026-11-19, the iCE40UP5K-SG48 board with an FT2232H):

| Design | What it shows | Result |
|---|---|---|
| `sbus_probe.v` | both blocks on their system buses: each of the SPI block's sixteen registers (bus address `0x0_`) and the I2C block's (`0x1_`) written a value of its own, then read back, every line | the SPI block acknowledges registers 6 to 15 and the I2C block 3 and 6 to 15; SPI 7 to 11, 13 and 15 and I2C 3, 7 to 11 and 13 give back exactly what was written, SPI 12 and I2C 12 read `50` and `20` and SPI 14 `BD` (a status and a receive register, as Lattice's register map names those addresses: QUOTED), and no other address answers — CHECKED, every line, `the_hard_spi_and_i2c_blocks_answer_their_buses_on_the_board` |
| `ledda_breathe.v` | the dimmer, its registers written over its control bus as TN1288's Appendix D lays them out, driving the RGB driver; the fabric measures the red output's duty every 22 ms and prints it | the duty ramps from 0 to `0x1A4C`/16 of 2^18 clocks (about 41 %, for a programmed 0x60/255 = 38 %) over a second, holds, falls, `LEDDON` drops for a second, and again, about every three seconds — CHECKED, `the_led_dimmer_breathes_on_the_board`. **That the LED visibly breathes has not been confirmed by a person.** |
| `io_od_blink.v` | the three open-drain pads driven as plain IO, each pulsed low for one clock in 64 so that nothing but the pad limits the current, stepping red, green, blue | configured; **not confirmed by a person, and nothing measured it** |
| `warmboot_demo.v`, twice | a multi-image flash (`reticle program --ice40 --write-flash wb0.bin --warm-boot wb1.bin`, the layout IceStorm's `icemulti` writes, ported as `image::multiboot`): image 0 prints `WB0 0`..`7` and boots image 1, which prints `WB1 0`..`7` and boots image 0 | `WB0 0`..`7`, `WB1 0`..`7`, `WB0 0`.. for as long as it was watched — CHECKED; then the flash was restored from a backup read first, read back identical (SHA-256 `603573a7...`), and booted `ram_selftest` with `SP:PPPP BR:PPPP` |

The I2C and SPI blocks' own sides (SCL, SDA, MOSI ...) reach no pin in
`sbus_probe`: the board has nothing on an I2C or SPI bus, and its spare
pins are not wired to each other, so no loopback was possible. What was
checked of them is the bus, the sites, the bits and nextpnr's agreement.
The I3C pads are pins 23 and 25, which on this board go to a multiplexer
whose outputs are not to be driven, so `SB_IO_I3C` was not loaded at all:
it is checked by the decode and by its refusal elsewhere only.

## How fast the DSP multiplier is

nextpnr takes every port of an `SB_MAC16` for a register, and so did this
analysis; icetime times an unregistered multiply as logic, through its
`SB_MAC16_MUL_U_16X16_BYPASS` cell, whose slowest arc (`B[1]` to `O[31]`)
is 3.36 / 5.87 / 9.05 ns (min / typ / max). The two disagree about any
design with an unregistered block: nextpnr would pass one at a clock
icetime says it cannot make. Measured on the board (2026-11-19) with
`dsp_speed.v`: a fabric register, a DSP block with no register at all
(16 x 16 unsigned, `*OUTPUT_SELECT` 3), a fabric register; compared every
cycle, three cycles later, with a block with every register on; the PLL
stepped, 2^22 cycles each:

| Clock | Unregistered: cycles differing | Every register on (the control) |
|---|---|---|
| 24 to 96 MHz, eleven steps | 0 | 0 at 96 |
| 99 MHz | 1 | |
| 102 MHz | 1 217 | |
| 105 MHz | 35 940 | |
| 108 MHz | 332 819 | 0 |
| 114 MHz | 6 522 468 | |
| 120, 132 MHz | | 0 |

So **icetime is right that the path exists** and nextpnr, and this
analysis as it was, wrong to leave it out: fabric register to fabric
register through the multiplier is about 10.1 ns at room temperature on
this unit, while the registered block runs at 132 MHz. And **icetime's
figure is a slow corner**: its maximum alone is 9.05 ns, and with the
routing this analysis now gives the whole path 19.6 ns (icetime 17.9) on a
smaller design of the same shape — about twice what the board does, as
`pll_uart` ran at 48 MHz from builds all three tools time at 41 to 43.

The analysis now times an output half that selects a multiplier product
with no register between it and an unregistered operand as logic, with
icetime's arcs for that multiply (`timing::dsp_arcs`), and any other
output from the block's clock; a registered operand is a register input
(`an_unregistered_dsp_multiply_is_timed_as_logic`: 19.6 ns unregistered,
8.0 with every register on). What it does not do: time an adder's output
through the multiplier (`*OUTPUT_SELECT` 0 and 1 are taken for registered),
or change what timing-driven placement weighs, which still takes the ports
for registers. And the comparison with nextpnr no longer holds a design
with a DSP block to its bound, since the two now differ on purpose.

## The configuration image, and `icepack` ported

`src/fpga/icestorm/image.rs` holds every tile's bitmap, the extra bits and
the block RAM contents, writes and reads the `.asc`, and packs and unpacks
the `.bin` — a port of `icepack.cc`'s bank arithmetic
(`CramIndexConverter`, `BramIndexConverter`, `write_bits`, `read_bits`),
read from IceStorm's source. **CHECKED**: each of the four reference `.asc`
files packs to *exactly* the bytes `icepack` wrote for it, from the
synchronisation word to the end, and each `.bin` unpacks to the same
tiles (`icepacks_bytes_are_reproduced`, given the files in
`RETICLE_ICE40_REF`; none ship). The CRC was wrong the first time — the
`0x22` command byte goes into the CRC before the CRC is taken — and the
round-trip test found it before any board did.

## The two checks

`icestorm::implement` returns nothing that fails either (or the third, for
RAMs, above):

1. **every set bit decodes**: each multiplexer whose field is not zero must
   hold a pattern that selects a source, and every other set bit must be a
   named function bit of its tile or a known extra bit;
2. **the arcs are the router's**: the set of `(tile, destination,
   source)` arcs the bits select must equal the set of arcs, with bits, that
   the router chose.

For the board designs: `rgb_blink` sets 6 437 bits that decode into 260
arcs, `uart_echo` 7 558 bits into 519, and `board_demo` (both) 8 508 into
821, all with nothing unexplained and the arcs exactly the router's
(`the_board_designs_build_and_every_bit_decodes`). Breaking the pattern
order of a multiplexer makes 236 bits of `rgb_blink` unexplained (CHECKED by
mutation). `ram_selftest` sets 22 997 bits into 5 911 arcs, with eight
block RAMs holding 8 192 set content bits and four single-port RAMs, all
read back as placed (`both_kinds_of_ram_build_and_read_back`).

**And the vendor's own decoder agrees.** IceStorm's `icebox_vlog`, run on
this flow's `.asc`, writes Verilog for what the bits do; that Verilog,
simulated against a testbench sending a byte, echoes it correctly for
`uart_echo`, and its four-bit counter reads as `c + 1` term by term. That
was run by hand from an IceStorm checkout and is not a test, because
nothing in CI has Python's icebox.

## On a board

An iCE40UP5K-SG48 board with an FT2232H, 2026-11-17, every load into
configuration memory with `reticle program --ice40` and the last one
written to the flash:

| Design | What it shows | Result |
|---|---|---|
| a wire, `assign tx = rx` | IO input and output configuration, routing | bytes sent on the FT2232H's second channel come back — CHECKED |
| one flip-flop between them, on the 12 MHz clock | a logic cell's flip-flop, a clock | the same — CHECKED |
| `uart_echo.v` | 96 logic cells: carry chains, enables, resets, a state machine | `HAL hello` comes back as `IBM!ifmmp` — CHECKED |
| `rgb_blink.v` | the RGB LED driver, a 24-bit counter | the LED steps red, green, blue, about a second each — CHECKED, by eye, in `board_demo` below |
| `board_demo.v` (both), written to the flash | boot from flash | after `--write-flash` and a release of `CRESET_B`, `CDONE` rose and the echo answered — CHECKED; and on 2026-11-17 the user saw the RGB LED step red → green → blue about once a second, booted from the flash — CHECKED **by a person's eyes, not measured**: nothing recorded the colours or the period |
| `ram_selftest.v`, 2026-11-18 | all four single-port RAMs and a block RAM in each width written and read back; four more block RAMs, contents from `initial`, dumped | `SP:PPPP BR:PPPP`, and every one of the 1 024 + 512 + 1 024 + 2 048 words dumped equal to the formula in the RTL — CHECKED, every digit compared on the host with the formulas the RTL states, on every run |

`ram_selftest` built by this backend is what the board was left booting
from its flash on 2026-11-18 (written with `--write-flash`, read back and
compared, then booted again with `--boot`: `CDONE` high and the report
complete). The unit attached later that day came with a blank flash; it
was left the same way, booting `ram_selftest` as this backend builds it
after the DSP and PLL work (22 997 bits), every digit of its report checked
after a boot from flash.

**The board checks are tests now** (`board` in `tests/fpga_icestorm.rs`,
`#[ignore]`d, run with `--ignored --test-threads 1`): each builds its
design with Reticle's own flow, loads it into configuration memory and
checks the serial line. `the_echo_answers_every_byte_on_the_board` sends
all 256 byte values one at a time and wants each back plus one, then `HAL
hello` back as `IBM!ifmmp`; `both_kinds_of_ram_pass_and_dump_their_contents_on_the_board`
wants `SP:PPPP BR:PPPP` and every dumped digit equal to the RTL's formula,
on the report printed after configuration and again on one a byte asks
for. Both passed on 2026-11-18 — CHECKED. They skip, saying why, without
the database, the FT2232H or its serial port, which is found in sysfs as
interface 1 of the `0403:6010` device rather than assumed to be
`/dev/ttyUSB1`. What checks the checker, with no board:
`the_report_check_rejects_a_wrong_report`.

### The stray `0x00`, not reproduced

The first board's first read after a configuration had one `0x00` before
the design's first line. On the unit attached on 2026-11-18 — a different
one: its flash answers with another unique ID, and its flash was blank, so
`ram_selftest` was written to it again — it could not be made to happen:

| Tried | What came over the line |
|---|---|
| line open across a load into configuration memory (each board test does this) | nothing until the design spoke; no byte, no break |
| line opened after the load, three loads, with the earlier scripts' settings | the report, starting `SP:` |
| line open across a boot from flash | the report, starting `SP:` |
| line open while the FPGA was held in reset for about three seconds (a flash read) | nothing at all, no break |

So it does not come from the design on this unit: the transmit flip-flop
starts at zero and the pin drives `~out_n[0]`, high, from the first cycle,
and in reset the line stays high. The earlier scripts' `termios` had
`IGNBRK`, `BRKINT` and `PARMRK` all clear, and a tty set up that way
delivers a **break** — the line held low for longer than a frame — as a
single `0x00`: exactly what was seen. That fits a line floating low while
that unit was held in reset, but that unit is not here to confirm it, so
it is a reading, not a finding. The tests now set `PARMRK`, which tells a
break (`FF 00 00`) and a framing error (`FF 00 x`) from a received byte,
and the echo test fails on anything at all before its first byte.

Later the same day it was seen once more, on this unit: a reader opened
with the earlier scripts' settings after a `--write-flash` (which holds the
part in reset, then boots it) read `00 0a` at once, bytes that had arrived
while the port was closed. Repeated with `PARMRK` and the port open across
the same flash write and boot, the line showed no break and no stray byte.
So it lives in what reaches the host while nobody reads the port, not in
anything the line does while someone does; the board tests open the port
before they touch the board, and have never seen it.

Writing the flash and then loading configuration memory twice left the
flash's image unchanged, read back byte for byte — CHECKED, because a load
holds the flash's `/CS` low for the whole bitstream.

**The board check is not a tautology**, and was checked not to be: a build
with two faults the flow's own checks cannot see — the contents' sixteen
lines written in reverse order *and* read back reversed, and the
single-port RAMs' `MASKWREN` bits 0 and 1 swapped in the fabric — passed
every check of `implement` (every bit explained, the RAMs read back as
placed) and on the board printed `SP:FFFF`, and the 16-, 8- and 4-bit
dumps differed in 512, 1 024 and 1 024 digits. (The 2-bit one happened to
read the same: its formula is symmetric under that reversal.) The same
design built through nextpnr from Reticle's export passed the same way as
Reticle's own build, which is what says the *mapping's* layout of the
narrow modes — `docs/fpga.md`, "Block RAM contents" — is the silicon's.

## Faults found, and how

### A reset that waited for an enable

The first `uart_echo` on the board answered one byte, `00`. Simulating
IceStorm's own decode of the bitstream failed the same way, and so did the
packed netlist under a model of the silicon, while the mapped netlist under
the *library* model passed. The difference was one cell's meaning:

- Reticle's mapping writes `SB_DFFESR`/`SB_DFFESS` with the **reset winning
  over the enable** — the generic register it comes from is `dff pos en srst`
  and `synth::opt::ff_opt` says so in as many words, and `--verify` proves
  the mapping against that;
- the silicon applies a synchronous set/reset **only on an enabled edge**:
  `icebox_vlog` decodes a configured cell as `if (cen) q <= sr ? set : d`.

So a reset asserted while the enable was low was lost. Packing first gave
such a cell `E | R` as its enable, and the echo worked. **That fixed only
this backend**: the netlist Reticle exports to nextpnr asked for the
reset-wins meaning from a cell whose library meaning is enable-gated, and
the cell's meaning was not packing's to correct. The library is the
reference — Yosys's `cells_sim.v` models `SB_DFFESR`, `SB_DFFESS`,
`SB_DFFNESR` and `SB_DFFNESS` as `if (E) Q <= R ? v : D`, and its
`ff_map.v` maps only `$_SDFFCE_*` (enable over reset) onto them — so the
mistake was in mapping, and mapping is where it is fixed:

- the device file says so, `mode posedge,enable,sync_reset,enable_gates_reset`
  on those four lines (`FfVariant::enable_gates_reset`);
- flip-flop mapping, finding no reset-wins flip-flop with an enable on the
  device, takes the gated one and feeds its enable `en | rst` — one
  lookup table per distinct enable and reset, at the reset's *active*
  level, shared by every register on them (`techcells::FfCtx::widen`);
- packing passes `CEN` and `SR` through unchanged. Its own widening is
  gone, so nothing compensates twice (a second `| R` would have been
  harmless logically, and a wasted table per pair).

It costs tables: 27 in `seph_mcu` on an HX1K and 34 in `z80`
(`docs/ip-library.md`), which is what any flow pays on this family; Yosys
does the same thing through `dfflegalize`.

**The nextpnr path, end to end, on the board** (2026-11-17, nextpnr-ice40
built from its repository at `1759078`, icepack from IceStorm at
`00da9c3`): `uart_echo.v` exported as JSON by the build from before the
change, placed and routed by nextpnr and loaded into configuration memory,
does not echo `HAL hello` (one stray byte on the first try, then
silence); the same design exported after it
answers `IBM!ifmmp`. CHECKED. Two things were found on the way:

- nextpnr refused the export outright — *No wire found for port
  PACKAGE_PIN on source cell tx$io0* — because the JSON wrote an output
  `SB_IO`'s pad as an `output`. nextpnr's iCE40 packer finds a pad only
  through an `inout` `PACKAGE_PIN`, which is how the library declares it.
  The export now writes every iCE40 IO buffer's pad as `inout`
  (`emit_json_with`). The before-and-after above both carry that fix, the
  "before" by editing its JSON by hand, so that they differ in the
  flip-flops only. So the iCE40 JSON export had never worked with
  nextpnr: no previous claim here said it had, and none should be read
  into the `.json` goldens, which are compared byte for byte and not run.
- ECP5's `TRELLIS_IO` is left writing its pad by direction: nextpnr-ecp5
  places it that way and a Cynthion runs it.

This backend's own build of `board_demo` after the change (then 8 580 bits,
above) was loaded into configuration memory and echoes as before —
CHECKED.

**The other flip-flops, checked for the same class of fault** (each against
`cells_sim.v`, and each but the last by a proof in `tests/fpga_icestorm.rs`):

| Primitive | Library | Reticle's `dff` | Mismatch? |
|---|---|---|---|
| `SB_DFFESR`, `SB_DFFESS`, `SB_DFFNESR`, `SB_DFFNESS` | reset only on an enabled edge | reset wins | **yes — fixed above** |
| `SB_DFFER`, `SB_DFFES` (and `N`) | `always @(posedge C, posedge R) if (R) Q <= 0; else if (E) Q <= D;` | asynchronous reset wins | no — proved |
| `SB_DFFSR`, `SB_DFFSS` (and `N`) | `if (R) Q <= 0; else Q <= D;` | the same | no — proved |
| `SB_DFFR`, `SB_DFFS`, `SB_DFFE`, `SB_DFF` (and `N`) | no enable and reset to disagree about | — | no — read, not separately proved |

The other families' flip-flops with an enable and a synchronous reset let
the reset win, as Reticle's `dff` does: the 7 series `FDRE`, ECP5's
`TRELLIS_FF` (Yosys maps `$_SDFFE_*`, reset over enable, onto it with
`SRMODE="LSR_OVER_CE"`) and Gowin's `DFFRE` (`if (RESET) ... else if (CE)`
in Yosys's model). QUOTED from those models, not measured here.

### Two DSP registers by other names

The first run of `dsp_selftest` on the board agreed with the model on
seven blocks and on half of the eighth: the split's top product, asked for
registered (`TOP_8x8_MULT_REG`), came out one step early — not registered.
nextpnr's build of the same netlist did exactly the same, and its bits read
back as this backend's, so the two flows agreed with each other and with
IceStorm's names and not with the model. Four more runs, each block's
parameters changed to set one register bit alone and its output read
against every subset of the registers it could have turned on, settled
which bit does what — CHECKED:

| Bit (anchor tile) | IceStorm's name | Registers, on the silicon | The library's meaning of that name |
|---|---|---|---|
| `CBIT_4` | `TOP_8x8_MULT_REG` | `K` only, `A[15:8] x B[7:0]` | `F`, the top product |
| `CBIT_5` | `BOT_8x8_MULT_REG` | `G` | `G` |
| `CBIT_6` | `PIPELINE_16x16_MULT_REG1` | `F` and `J` | `J` and `K`, the middle products |
| `CBIT_7` | `PIPELINE_16x16_MULT_REG2` | `H` | `H` |

So the top product's register is the bit IceStorm calls
`PIPELINE_16x16_MULT_REG1`, and that bit also registers one middle
product. A design that sets the two parameters together — every 16 x 16
pipeline does, and Yosys sets them together whenever it absorbs a register
— cannot tell, which is presumably why it has lasted. This backend writes
the library's `TOP_8x8_MULT_REG` into `CBIT_6` and its
`PIPELINE_16x16_MULT_REG1` into `CBIT_4` (`DspConfig::as_bits`), and
refuses the two set differently when the 16 x 16 sum reaches an output or
an adder, since no bitstream holds that
(`a_dsp_block_the_silicon_cannot_hold_is_refused`). After the change all
eight blocks agree with the model. A netlist exported to nextpnr still
gets IceStorm's mapping; that is nextpnr's and IceStorm's to correct, and
nothing here changes what the export says.

What it took: a model to compare with, and a test that printed every
block's output rather than a pass or a fail, so the wrong half of one
word, one step late, was visible as such.

### A register that started at one

`reg [9:0] out = 10'h3ff` — a transmitter idling high — came up at zero, and
the line held a break. An iCE40 flip-flop always starts at zero and nothing
in the flow inverts one to start at one. Packing now refuses such a register
by name (rule 8), and `uart_echo.v` keeps its two idle-high registers
inverted.

### A placer rule that read "no pin" as "a different signal"

The first counter would not place: *no run of that shape is free*. The
placer's rule that pins on one wire must carry one signal treats a cell
with no signal on a tile's shared clock as wanting a *different* one, which
is right for a flip-flop (an unconnected enable reads one, and would be
wrong beside a driven one) and wrong for a cell whose flip-flop is unused.
Rather than change that rule for every family, a cell with no flip-flop is
an `ICESTORM_LC_COMB` on an `lcc` bel that has no such pins.

### A carry out that two readers wanted

`ram_selftest.v` was refused at first: *the carry out of `ch$add$carry0` is
read by the next carry and by other logic*. An iCE40 carry out reaches only
the cell above it. Packing now puts a *tap* between the two, nextpnr's way:
a cell whose carry passes its carry in through (its `I1` a one, its `I2` a
zero) and whose table hands it to the fabric. Proved equivalent in
`a_carry_read_inside_and_outside_its_chain_packs_to_the_same_logic`, which
fails with the tap's `I1` left at zero (CHECKED by mutation).

The first version of it then failed to *route*: a carry out was still read
by logic from the carry's own output. Packing had decided whether anything
outside the chain read a carry out by counting the readers inside — the
next carry's carry in and its table's inputs — and taking them from the
total. But the next cell had the carry in on its fourth input without any
table of the netlist reading it there, so the count came out even with a
reader outside. It now counts the readers outside directly.

### The rest, found before a board

The `icepack` CRC (above). And one reading confirmed rather than assumed:
an unrouted `CEN` reads one, which is what lets a flip-flop with no enable
leave it unconnected — CHECKED in `icebox_vlog`'s defaults, and on the
board, where the echo's line resynchroniser has no enable and works.

## What this does not do yet

- the DSP blocks' cascade (`ACCUMCI`/`ACCUMCO`, `SIGNEXTIN`/`SIGNEXTOUT`)
  between neighbouring blocks is not chained, and the mapper does not
  infer a DSP block from a `*`: a design instantiates `SB_MAC16`;
- the PLL's pad forms have not run on a board, its phase-shift and delay modes
  (`SHIFTREG_*`, `DELAY`, `PHASE_AND_DELAY`, `EXTERNAL` feedback, dynamic
  delay) are written as nextpnr writes them and never run, and nothing
  drives an oscillator's trim inputs;
- the timing analysis does not time paths between clocks, to or from pins,
  through a DSP block's adder, between the two edges of one clock, or with
  the clock network's skew, and the timing-driven flow is not the default;
- the I2C and SPI blocks' bus sides were checked on the board, their I2C
  and SPI sides not, and `SB_IO_I3C` not at all; the open-drain pads and
  the dimmer's light await a person's eyes;
- no LP, HX or iCE40 Ultra part has been on a board: their builds are
  checked against IceStorm and nextpnr only ("Every iCE40 part"), the
  iCE40 Ultra's LED drivers (`SB_RGB_DRV`, `SB_LED_DRV_CUR`) are not
  placed, and nothing is done for the lm4k die.
