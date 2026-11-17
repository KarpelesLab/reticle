# A real Lattice iCE40, and a real `.bin`

Reticle places, routes and writes a bitstream for the **iCE40UP5K** on its
real fabric, read from Project IceStorm's chip database, and the bitstream
loads and runs. This is the account of how that backend is built, what was
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
`.deb` against a pinned SHA-256, and takes three files out of it:
`chipdb-5k.txt` (27 MB), `timings_up5k.txt`, and the package's `copyright`,
which carries IceStorm's ISC licence, saved as `LICENSE`.

A `.deb` is an `ar` archive holding an xz-compressed tar. The `ar` and tar
readers are `src/bin/reticle/deb.rs`; the xz is `compcol`'s, as for
Project Apicula's databases, which is why the fetch is behind the
`icestorm` Cargo feature. The rest of the backend is in `fpga` and needs no
dependency: a database unpacked by hand and named by `RETICLE_ICESTORMDB`
works in any build.

Other prebuilt sources were looked at and not taken: oss-cad-suite and apio
tarballs (hundreds of megabytes for one 27 MB file), and nextpnr's own
compiled `.bba` databases (not icebox's text, so not what icebox's decoder
reads back). The timing file is fetched and not yet read.

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
  "Block RAM and single-port RAM" below.

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
| a used IO block also sets `IoCtrl.cf_bit_39` (paired block 0) or `cf_bit_35` (paired block 1) | the same | all four references, input and output alike; meaning not known |
| every column buffer of every global network enabled, 1 248 bits | every `.colbuf` source tile | all four references, identically, whatever the design |
| every IP-connect and DSP tile's eight logic cells set to pass their third input through, `11000011001100001100`, with every `Cascade.*` bit beside them | all 60 such tiles | all four references, identically |

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
| `RamConfig.PowerUp` | `ramb` | set: the RAM is in use (on the 5k; a 1k has it the other way round, which is one reason only the 5k is supported) |
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

For the board designs: `rgb_blink` sets 6 509 bits that decode into 260
arcs, `uart_echo` 7 630 bits into 519, and `board_demo` (both) 8 580 into
821, all with nothing unexplained and the arcs exactly the router's
(`the_board_designs_build_and_every_bit_decodes`). Breaking the pattern
order of a multiplexer makes 236 bits of `rgb_blink` unexplained (CHECKED by
mutation). `ram_selftest` sets 23 069 bits into 5 911 arcs, with eight
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
complete). The first read after a configuration has one `0x00` byte before
the design's first line — the transmit pin held low while the part
configures, by the look of it; not investigated.

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

This backend's own build of `board_demo` after the change (now 8 580 bits,
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

- no DSP (`SB_MAC16`) or PLL is placed: a design using one is refused by
  name, before placement. What each would take, as read off the database
  and nextpnr, not attempted:
  - **`SB_MAC16`**: eight `.extra_cell MAC16` records give each block's
    ports bit by bit and its parameters as `CBIT`s, the same shape as the
    single-port RAMs, so the bel is the same few lines; the work is the
    two dozen parameters nextpnr's `bitstream.cc` lists (`A_REG` ..
    `MODE_8x8`, `TOPOUTPUT_SELECT` and the rest) each onto its bits, and a
    model of the block to prove anything against. The mapper does not
    infer it (the device declares no `dsp` block for the part), so a design
    instantiates it, and a board test is a multiply read back over the
    serial line;
  - **`SB_PLL40_CORE`/`_PAD`**: one `.extra_cell PLL` at (12, 31) with its
    divider, filter and feedback parameters as bits, and its output driving
    a global network through a fixed buffer; the `_PAD` form takes over an
    IO pin. The device file's PLL solver already chooses the dividers. The
    risk is in the global network the output claims, and the check is a
    frequency measured on the board (a baud rate derived from it);
- a logic tile's flip-flops share one clock-polarity bit, and a design that
  needs both edges in one tile is refused after placement rather than
  placed around;
- no timing: the router minimises congestion and wirelength, and nothing
  reads `timings_up5k.txt`. Timing-driven routing would need that file read
  into delays per multiplexer and per cell, a timing analysis over the
  placed and routed graph, and criticality fed to the placer and router —
  the largest of the three, and the one a 12 MHz design does not yet need;
- only the 5k is fetched and only `sg48` has been placed; the LP/HX device
  files still use the synthetic fabric.
