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
- **an enable over a synchronous reset becomes `E | R`**, and a register
  declared to start at one is refused. Both are faults, below.

The rigid shapes the carry chain needs are not stated anywhere: the placer
finds them on its own, because a cell's carry in is a wire only the cell
below drives (`place::build_clusters`). Nothing in `place.rs` names an
iCE40.

**Equivalence is held across packing**, the same guarantee
`every_block_maps_to_the_logic_it_was_mapped_from` gives mapping. The
`packing` tests in `tests/fpga_icestorm.rs` replace every cell of the mapped
netlist with a model of the library cell as the mapping means it, and every
packed cell with a model of the *silicon* as `icebox_vlog` decodes it, and
prove the two equivalent for a counter with enable and reset, a down counter
with a zero flag, and an adder with its carry out. Removing rule 7, or
moving a table's inputs without its truth table, makes the counter differ
at frame 1 (both CHECKED by mutation).

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

`icestorm::implement` returns nothing that fails either:

1. **every set bit decodes**: each multiplexer whose field is not zero must
   hold a pattern that selects a source, and every other set bit must be a
   named function bit of its tile or a known extra bit;
2. **the arcs are the router's**: the set of `(tile, destination,
   source)` arcs the bits select must equal the set of arcs, with bits, that
   the router chose.

For the board designs: `rgb_blink` sets 6 496 bits that decode into 252
arcs, `uart_echo` 7 704 bits into 541, and `board_demo` (both) 8 614, all
with nothing unexplained and the arcs exactly the router's
(`the_board_designs_build_and_every_bit_decodes`). Breaking the pattern
order of a multiplexer makes 236 bits of `rgb_blink` unexplained (CHECKED by
mutation).

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

So a reset asserted while the enable was low was lost. Packing now gives
such a cell `E | R` as its enable (rule 7), the equivalence test above
holds it, and the echo works. **This also means the netlist Reticle exports
to nextpnr for any iCE40 asks for the reset-wins meaning from a cell whose
library meaning is enable-gated.** That path was not changed here and has
not been run against nextpnr; it is the next thing to look at.

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

### The rest, found before a board

The `icepack` CRC (above). And one reading confirmed rather than assumed:
an unrouted `CEN` reads one, which is what lets a flip-flop with no enable
leave it unconnected — CHECKED in `icebox_vlog`'s defaults, and on the
board, where the echo's line resynchroniser has no enable and works.

## What this does not do yet

- no block RAM, SPRAM, DSP or PLL is placed: a design using one is refused
  by name, before placement;
- a logic tile's flip-flops share one clock-polarity bit, and a design that
  needs both edges in one tile is refused after placement rather than
  placed around;
- a carry out read both by the next carry and by logic is refused;
- no timing: the router minimises congestion and wirelength, and nothing
  reads `timings_up5k.txt`;
- only the 5k is fetched and only `sg48` has been placed; the LP/HX device
  files still use the synthetic fabric.
