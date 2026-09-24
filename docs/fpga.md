# FPGA placement, routing and bitstreams

Reticle's own back end for open FPGA families, under `src/fpga/`. It
takes the netlist `fpga::synthesize_for` produces — nothing but
primitives the device database declares — and places it, routes it and
writes its bitstream, without calling another program.

```
  .rtl / .v / .vhd
        │
   fpga::synthesize_for          synthesis, primitives, LUTs, device cells
        │
        ├── fpga::export_nextpnr ─► nextpnr-ice40 / nextpnr-ecp5
        ├── fpga::export_vendor  ─► Vivado (7 series) / Quartus / Diamond
        │
        └── fpga::place_and_route
                 │
              place.rs   analytic (conjugate gradient) + simulated annealing
                 │
              route.rs   PathFinder negotiated congestion, A* maze expansion
                 │
            bitstream.rs   .asc (IceStorm style) and .bin (Reticle's own)
```

`fpga::implement` is synthesis plus all three in one call.

## Families

| Family | Parts | Route out | Own place and route |
|--------|-------|-----------|---------------------|
| `ice40` | `ice40-lp1k-tq144`, `ice40-hx1k-tq144`, `ice40-hx8k-ct256` | `nextpnr-ice40` | yes, on a **synthetic** fabric — read the section below |
| `ecp5` | `ecp5-25f-CABGA381`, `ecp5-45f-CABGA381` | `nextpnr-ecp5` | no architecture |
| `xc7` | `xc7a35t-cpg236` (Artix-7, Digilent Basys 3) | **Vivado** | yes, on the **real** fabric, with a chip database the user supplies — see [`fpga-xray.md`](fpga-xray.md) |
| `gowin` | `gw2a-18-pg256` (GW2A-18, Sipeed Tang Primer 20K) | none yet | **partly**, on the **real** fabric, with a chip database the user supplies — see [`fpga-gowin.md`](fpga-gowin.md) |
| `generic` | `generic`, `generic-k6` | none; it is not a real part | no architecture |

`fpga::pnr_route` answers which of the two exports a family takes, and
it is the one thing about a family that is not data. `reticle fpga`
asks it and writes the matching files. A family with no nextpnr back end
does not fail quietly: `fpga::export_nextpnr` returns
`FlowError::VendorOnly`, which names the family, the device and the tool
that does read its export.

## Generated clocks: PLLs

A design asks for a clock frequency the board does not have by
declaring the clock net, using it, **leaving it undriven**, and
constraining it — next to the constraint that says what the input clock
is:

```text
# pins.rcf
set_io clk 21
create_clock -name osc -period 83.333333 clk     # the iCEstick's 12 MHz
create_clock -name sys -period 20.833333 sys     # 48 MHz, please
```

or, in the source, `(* clock_mhz = 48 *) wire sys;`. A clock constraint
on a net nothing drives is a request; one on a net something drives (a
divider in logic, say) describes a clock that already exists and is left
alone. `fpga::primitives` answers a request by instantiating the
device's PLL from the clock constrained on an input port, with
`fpga::pll::solve` choosing the dividers:

```text
pll:
  sys -> SB_PLL40_CORE from clk at 12.000 MHz: 48.000 MHz for 48.000 MHz asked (+0.0 ppm),
         vco 768.000 MHz, DIVR=0 DIVF=63 DIVQ=4 FILTER_RANGE=1
```

The solver searches every legal combination of the three dividers and
keeps the one closest to the request, with the phase detector and the
VCO inside their windows; nothing in it names a family. What it needs is
on the device's `pll` line: the ranges, each divider's parameter and how
its value relates to the division (`offset 1` for a field that stores
`divisor - 1`, `pow2` for one that stores an exponent), whether the
feedback comes from the VCO or the output, parameters that follow from
the answer (`derive CLKOP_CPHASE out -1`), parameters banded by the
phase-detector frequency (`band FILTER_RANGE 17=1 26=2 …`), the ports,
the pins to tie, and how many PLLs the die has.

The achieved frequency and the error are reported, always, and an error
over 1 % is also a warning (`F0303`): 74.25 MHz from the ULX3S's 25 MHz
comes out at 75 MHz on the ECP5, because the phase detector's lower
limit leaves only six reference dividers, and the tool says so rather
than quietly running the display 1 % fast. Measured over each family's
whole range in 0.1 MHz steps:

| PLL | reference | median error | 90th percentile | worst |
|-----|-----------|--------------|-----------------|-------|
| iCE40 `SB_PLL40_CORE` | 12 MHz | 0.39 % | 0.75 % | 1.1 % |
| ECP5 `EHXPLLL` | 25 MHz | 0.30 % | 1.4 % | 10.7 % (at the bottom of the range) |

A request that cannot be met — no PLL described, no input clock to
start from, every PLL of the die already in use, a reference outside the
input range — is an error naming which, and the net is left undriven for
the netlist check to find.

## Double-data-rate IO and IO delays

A port becomes a **double-data-rate** port by naming the clock of its
registers — `(* ddr = "clk" *)` on the port, or `set_io -ddr clk …` —
and it then carries **two bits per pin**: a `2N`-bit port is `N` pins,
and bit `i` and bit `i + N` share pin `i`, the low half on the rising
edge and the high half on the falling one. That is the whole convention;
the design sees an ordinary vector twice as wide as the bus, and the
netlist's port is as wide as the bus. An odd width, an `inout` port or a
clock that does not exist is an error (`F0304`) and the port is
buffered as an ordinary one.

How the two edges are registered comes from the device file, not from
the family name:

- a family whose **IO buffer registers both edges** declares the extra
  pins on it (`din1`, `dout1`, `iclk`, `oclk`, optionally `ce`) and the
  parameters that select the mode (`param_ddr_in`, `param_ddr_out`).
  The iCE40's `SB_IO` is one: `D_IN_0` / `D_IN_1` and `D_OUT_0` /
  `D_OUT_1` with `PIN_TYPE` `6'b000000` and `6'b010000`;
- a family whose buffer does not declares a **register beside it**, a
  `bel … ddr_in` with `d`, `clk`, `q0`, `q1` and a `bel … ddr_out` with
  `d0`, `d1`, `clk`, `q` (reset pins, when declared, are tied inactive).
  The ECP5's `IDDRX1F` and `ODDRX1F` are those.

An **IO delay** — `(* io_delay = 20 *)` or `set_io -delay 20 …`, in the
device's own steps — is a `bel … iodelay` with `i` and `o` between the
buffer and the fabric (or the DDR register). The parameter carrying the
number of steps is the one declared under the condition `value`, with
its largest setting as the value: `param_value DEL_VALUE=127` on the
ECP5's `DELAYG`. A family without one (iCE40) and a delay past the
largest setting are both warnings, and the port is built without it.

```text
io:
  rx (1 bits) -> TRELLIS_IO, double data rate on clk (IDDRX1F), delayed 20 step(s) (DELAYG)
  tx (2 bits) -> TRELLIS_IO, double data rate on clk (ODDRX1F)
```

`ddr_ice40` and `ddr_ecp5` in `testdata/fpga/` take the same two ports
through the whole flow on both families. The synthetic placement
architecture has no wires for the extra DDR pins, so a DDR design goes
out to nextpnr rather than through Reticle's own placer.

## Hierarchy

`fpga::synthesize_for` flattens the module it is given before anything
else (`Design::flatten`, default options), so a design with instances
maps from the library, from `reticle fpga` and from anything else that
calls it: an instance marked `keep_hierarchy` and an instance of a black
box stay instances, and everything else is inlined. The module keeps
its id; the modules it inlined stay in the design, unsynthesised and
unmapped, and `fpga::export_nextpnr` leaves them out of the JSON. The
report counts the instances inlined, and `FpgaOptions::flatten` turns
the step off for a caller that wants to do it differently. A hierarchy
that cannot be flattened (a recursive one, a connection that cannot be
inlined) is `FlowError::Hierarchy`, with the reasons as diagnostics.

## Block RAM contents

A memory with initial contents in the IR (`Memory::init`) keeps them when it becomes block RAM: every block
gets its initialisation parameters, holding the width slice and the
depth slice of the contents that block stores, and every copy of a
memory duplicated per read port gets the same. Where each bit goes is
data in the device file, on each `mode` line of a `bram` block and in
its `init_params` line, so nothing about a primitive is written in Rust:

```text
bram SB_RAM40_4K
  mode 16 256 param READ_MODE=0 WRITE_MODE=0 init high 0-15
  mode 8 512 param READ_MODE=1 WRITE_MODE=1 data 0,2,4,6,8,10,12,14 init high 0,2,4,6,8,10,12,14 1,3,5,7,9,11,13,15
  ...
  init_params INIT_ count 16 digits 1 rows 16 slot 16
```

`init_params` says the contents are rows held by `count` parameters of
`rows` rows each, `slot` bits a row, first row in the low bits. `init
low` or `init high` says which address bits pick among the words that
share a row in a narrow mode, followed by the row bits of each of those
words. `data` names the data pins a mode uses when they are not simply
the low ones, and `addr` and `pad` say where the word address starts on
the address port and what the pins below it are tied to.

| Family | Layout | Confidence |
|--------|--------|------------|
| iCE40 `SB_RAM40_4K` | 256 rows of 16 bits; `INIT_<k>` holds rows 16k..16k+15, row 16k+i in bits 16i+15..16i | high: the iCE Technology Library's definition |
| iCE40, narrow modes | the row is address bits 7..0; the bits above pick word `s` of the row, whose data bit `j` is row bit (16/width)·j + s; the data are on pins 0,2,..,14 (512x8), 1,5,9,13 (1024x4) and 3,11 (2048x2) | medium-high: what the Yosys simulation model and block RAM mapping of the primitive do |
| ECP5 `DP16KD` | 1024 rows of 18 bits; `INITVAL_<nn>` holds rows 16nn..16nn+15 in 20-bit slots, the top two bits zero; consecutive addresses share a row (9-bit words in bits 8..0 and 17..9); the word address starts at address pin 4, 3, 2, 1, 0 for the 18-, 9-, 4-, 2- and 1-bit modes, and the 18-bit mode's two pins below it, the write byte enables, are tied high | medium-high |
| ECP5, 4-, 2- and 1-bit modes | the words fill the sixteen row bits that are not the parity bits 8 and 17 | medium: Reticle's reading, not checked against a part |

The parameters reach nextpnr and the vendor exports. Reticle's own
place and route does not put them into its `.asc`: the synthetic RAM
bel declares configuration bits for `READ_MODE` and `WRITE_MODE` only,
since where a real part keeps its block RAM contents in the bitstream
is exactly what that fabric does not know.

The iCE40 data pins and the ECP5 address alignment are not only about
contents: they are how the blocks are wired, and a netlist wired to the
low pins in those modes would read and write the wrong bits on a device.

When the database cannot say where the contents go — a primitive
without the `init` flag, a block without `init_params`, a mode without
an `init` layout, as on the invented `generic` family — the memory is
still mapped, and the loss is a warning, `F0305`: the blocks start
blank, and a ROM in them reads zeros. A read-only memory read from more
ports than a block has is duplicated, one copy per read port, each
holding the contents, exactly as a written memory with one write port
is; the copies need no keeping in step.

## Package pins

A pin constraint names a package pin the device file lists. Most lists
are partial, taken from the boards that use the part; a device whose
list is known to be incomplete says `pins partial`, and a constraint
naming a pin it does not list is then a warning (`F0202`) that the pin
is unchecked rather than an error, and `check_nextpnr_json` leaves it to
nextpnr, which knows the package. `ice40-hx8k-ct256` is one: it lists
the iCE40-HX8K breakout board's clock, LEDs, serial port (B12, B10) and
SPI flash, and the CT256 has over two hundred IO balls more. The tq144
lists stay strict.

The `xc7a35t-cpg236` list is partial in the same way and for the same
reason: it is every line of Digilent's published `Basys-3-Master.xdc`
and nothing else, about a hundred balls of the CPG236's roughly two
hundred.

## Xilinx 7 series (Artix-7, Digilent Basys 3)

**Read this before believing anything below: one design has run on
silicon and no more.** On 2026-09-24 a lookup table and three pins,
built by this flow and loaded by `reticle program`, drove an LED from
two switches on a Basys 3. Nothing larger has been tried on a part,
nothing with a clock can be built for one yet, and Vivado has not been
run over this flow's output here. What the tests prove is narrower and
exact:

- every cell of the exported netlist is a primitive `xc7.dev` declares,
  wired to pins that primitive has (`fpga::check_nextpnr_json`);
- a memory's initial contents reach the block RAM's `INIT_00`, in the
  order the device file states;
- every pin the constraints name is a pin the device file lists, and it
  comes out in the XDC;
- the three files are the ones the script reads, under the names it
  reads them by.

Whether Vivado accepts the netlist, and whether the result blinks an
LED, are separate claims and this repository makes neither.

Reticle can also write the 7-series bitstream itself, without Vivado,
from Project X-Ray's chip database. That is a separate document —
[`fpga-xray.md`](fpga-xray.md) — because it is a separate claim: what
comes out is a structurally valid Xilinx bitstream, checked against one
Vivado made for the same part, and it is **not routed and configures
nothing**. The reason, the measured size of the fabric, and the list of
what remains are all there.

### What is in the device file

`src/fpga/devices/xc7.dev` heads itself with where each group of claims
came from and how far it is to be trusted, per group, the way
`ice40.dev` does. The summary:

| Group | Confidence |
|---|---|
| primitive names, ports and widths | **high** — read line by line from Yosys' `techlibs/xilinx/cells_sim.v` and `cells_xtra.v` (the latter generated from Vivado's own `unisims`), not recalled |
| `LUT6` and its `INIT` bit order (`O = INIT[{I5..I0}]`) | **high** |
| `FDRE` / `FDSE` / `FDCE` / `FDPE` and their `_1` falling-edge twins | **high** |
| `CARRY4` semantics, and the wiring adders are mapped onto it with | **high** — the model is `cells_sim.v`'s; the wiring is checked against it exhaustively to eight bits and proved to 32 (`tests/fpga_carry.rs`) |
| `RAMB18E1` ports, width parameters and address alignment | **high** |
| `RAMB18E1` contents layout (`INIT_00`..`INIT_3F`) | **medium-high** — the 256-bit-chunks-in-address-order description is UG473's; the row-sharing order in the narrow modes was reasoned from it here |
| byte write enables (`WEA[1:0]`, `WEBWE[3:0]`) | **high** |
| `IBUF` / `OBUF` / `IOBUF` / `OBUFT`, `BUFG`, `RAM64X1D` | **high** |
| `PLLE2_BASE` / `MMCME2_BASE` ports and divider parameters | **high** |
| their VCO bands (PLL 800–1600, MMCM 600–1200 MHz, −1 grade) | **medium-high** |
| their phase-detector and input limits | **medium** |
| 32 global clock buffers | **medium-high** |
| LVCMOS drive-strength lists | **medium** for the numbers, high for the names |
| resource counts (20800 LUT6, 41600 FF, 100 RAMB18E1, 5 CMTs) | **high** — datasheet figures |
| the Basys 3 pin list | **high** that these are the balls Digilent's file names; the transcription was fetched, not recalled |
| `W5` being a clock-capable ball | **medium-high** |

### The carry chain

Adders four bits and wider go onto `CARRY4`, one instance per four bits.
`CARRY4` is not the one-bit `(ci, i0, i1) -> co` element the other
families expose: it covers four bits, takes the *propagate* `a ^ b`
rather than two operands, and computes its own sums, which is why the
`.dev` format grew a second carry shape for it (see below). The wiring
is Yosys' — a LUT per bit for `S`, `DI` from `a`, `CO[3]` into the next
instance's `CI`, and the first instance's carry in on `CYINIT` with `CI`
tied low:

```text
bel CARRY4 carry width 4 port ci=CI cyinit=CYINIT p=S di=DI s=O co=CO
```

The model it is wired against is `cells_sim.v`'s, quoted in full in
`xc7.dev`. `tests/fpga_carry.rs` checks the wiring against that model
two independent ways — exhaustive simulation of the whole mapped
netlist at one to eight bits (plain, widened for the carry out, a
constant operand, both carry-in values) and `formal::check_equivalent`
proofs at 9, 10, 16, 17 and 32 bits — plus a structural check that `CI`
and `CYINIT` are used the way the part requires rather than merely the
way the `CI | CYINIT` model permits.

What it buys, on the two examples that fit the Basys 3's part
(`fpga::synthesize_for` for `xc7a35t-cpg236`, the same flow with
`MapOptions::map_carry` off and on):

| Design | `LUT6` before | `LUT6` after | `CARRY4` | LUT depth |
|---|---|---|---|---|
| `examples/apple2` (`apple2_basys3`) | 1980 | 1877 (−5.2%) | 85, over 33 adders | 22 → 22 |
| `examples/nes` (`nes_top`) | 4326 | 4184 (−3.3%) | 137, over 75 adders | 23 → 24 |

Two things that table does not say. A `CARRY4` sits in the slice beside
its four `LUT6`s and is not a resource they compete for, so the LUTs
saved are saved outright. And the depth column is the depth of the *LUT*
graph, which no longer counts an adder at all — the dedicated carry a
7-series slice has is exactly the path the placeholder timing model in
`src/timing/delay.rs` cannot price, so the number moving by one either
way means nothing here.

An adder narrower than four bits (`MapOptions::min_carry_width`, which
is one whole `CARRY4`) stays in LUTs, and so does a `sub`: the IR's
`sub` cell is not lowered through the carry path on any family.

### What is deliberately left out, and why

- **`RAMB36E1` is not declared.** In non-cascaded use its
  `ADDRARDADDR[15]` must be tied high, and Reticle's address model can
  pad the bits *below* the word address and not the bits above it, so a
  `RAMB36E1` declared here would address the wrong half of its array.
  Fifty `RAMB36E1` and a hundred `RAMB18E1` are the same array seen two
  ways, so no capacity is lost — only the 36- and 72-bit-wide modes.
- **`DSP48E1` is not declared.** It does not multiply unless `OPMODE`,
  `ALUMODE` and `INMODE` are driven with the right constants, and the
  `.dev` `dsp` line has no way to tie an input. Declaring it would
  produce a netlist that instantiates a DSP doing something other than
  the multiply it replaced. Multiplies go to `LUT6`s.
- **`IDDR` / `ODDR` / `IDELAYE2` are not declared**, so a `ddr` port or
  an `io_delay` is a warning and an ordinary buffer.
- **No tile grid**, so a placement region on this part is reported as
  uncheckable. The XDC still writes pblocks as `SLICE_X..Y..`, which is
  the right syntax; it is only unchecked here.
- **The parity bits of the block RAM are unused.** The 18 kbit block is
  16 kbit of data plus 2 kbit of parity in a second parameter series,
  and Reticle's contents model has one series, so the width modes are
  16/8/4/2/1 and the block holds 16 kbit here.
- **`CLKIN1_PERIOD` is not set.** It is a `real` parameter and the
  `.dev` model carries integers, sized constants and strings. The output
  frequency does not depend on it — the three integer dividers set that,
  and Reticle does write those — but Vivado uses it for jitter and will
  say so.

### The route out: Vivado, and why not nextpnr-xilinx

There are two ways a 7-series netlist becomes a bitstream, and Reticle
recommends the first:

1. **Vivado.** Emit structural Verilog of 7-series primitives plus the
   XDC; Vivado WebPACK, which is free for this part, implements it.
   This needs no place-and-route database, no fuzzing and no second
   tool, and the primitives and the constraint syntax are the vendor's
   own, which is the part of the chain least likely to be wrong.
2. **nextpnr-xilinx with Project X-Ray.** Fully open, and the honest
   objection is not ideology: it needs a chip database this repository
   does not ship and cannot verify, and it would put a second
   unverified layer under an already unverified one. Until someone can
   run it on a board and say it works, the recommendation stands.

So `xc7` takes the `export_vendor` route. `reticle fpga --device
xc7a35t-cpg236 --constraints board.rcf top.v` writes three files:

```
top.v      structural Verilog: LUT6, FDRE, RAMB18E1, IBUF, OBUF, BUFG, …
top.xdc    PACKAGE_PIN, IOSTANDARD, DRIVE, SLEW, create_clock, pblocks
top.tcl    create_project / read_verilog / read_xdc / synth_design /
           opt_design / place_design / route_design / write_bitstream
```

and prints the command that runs them:

```
vivado -mode batch -source top.tcl
```

The part string comes from the device name, the package and the speed
grade run together — `xc7a35t` + `cpg236` + `-1` gives
`xc7a35tcpg236-1` — so nothing about this part in particular is written
in Rust. `synth_design` has nothing to infer: the netlist is already
primitives, and elaborating it is also the point at which Vivado checks
that every cell and pin Reticle named really exists.

`fpga::export_nextpnr` on this family returns `FlowError::VendorOnly`
naming Vivado and `export_vendor`, rather than a half-written file or a
generic "unknown family".

### Three things the device model learned here

All three were gaps the 7 series found, and all three are general:

- **A carry element several bits wide.** The `.dev` `carry` line
  described one bit: two operand bits and a carry in, a carry out, and
  the sum XORed outside. A line that says `width N` describes the other
  shape instead — `p` the propagate, `di` the generate source, `s` the
  sums the element computes itself, `co` the carry out of each bit, and
  `ci` / `cyinit` the two ways in — and `BelKind::wide_carry` tells the
  two apart. The one-bit form is unchanged, so iCE40's `SB_CARRY` and
  the generic families map exactly as before and the ECP5's `CCU2C`,
  which is a third shape again (two bits, its own LUTs inside), still
  declines with its note.
- **One IO buffer per direction.** Xilinx has `IBUF`, `OBUF` and
  `IOBUF`, which do not even agree on what the pad pin is called (`I`,
  `O`, `IO`). A `bel … io for in|out|inout` line says which directions a
  buffer serves and `Device::io_bel` picks between them; a line with no
  `for` serves every direction, which is what iCE40 and ECP5 keep doing.
- **A block RAM pin several bits wide**, written `we=WEA*2`. The
  7-series byte write enables are two and four bits, and a memory
  written a whole word at a time drives all of them alike. Before this,
  the one-bit write enable would have reached `WEA[0]` and left the rest
  undriven, writing one byte of each word and dropping the others —
  silently.

A fourth is smaller: a PLL whose feedback loop is closed *outside* the
block (`PLLE2_BASE` wants `CLKFBOUT` wired to `CLKFBIN`) names both ends
as `fbout` and `fb`, and mapping runs a net between them.

### The board

`examples/soc/board/basys3.rcf` constrains the Basys 3's 100 MHz
oscillator (W5) and its USB-UART pair (B18 receive, A18 transmit), and
lists in comments — because a constraint naming a port the design lacks
is an error — the sixteen switches, sixteen LEDs, seven-segment display,
five buttons, four Pmod headers and the VGA pins, all transcribed from
Digilent's master XDC.

**The Basys 3 has no HDMI and no DVI connector.** Its video output is
VGA at twelve bits, four per channel, through a resistor ladder. The
`ip/dvi_tx` package in this repository drives TMDS differential pairs
and cannot drive this board; its `video_timing` module, which is
counters and sync generation and nothing else, is reusable unchanged for
a VGA path, and the serialiser and TMDS encoder are not.

## Read this first: the architecture is synthetic

Placement and routing need to know what wire is where, which
programmable interconnect point (pip) joins which two of them, and which
configuration bit switches each one on. For iCE40 that knowledge is
Project IceStorm's chip database, produced by years of fuzzing real
parts. **It is not in this repository and it has not been guessed.**

What ships instead is an iCE40-*shaped* fabric, built by
`src/fpga/arch/synthetic.rs`, whose header says at length what it borrows
and what it invents. In short:

| Borrowed | Invented |
|---|---|
| the 14 × 18 tile grid of the 1k dice | everything inside a tile |
| an IO ring, RAM in columns 3 and 10, logic elsewhere | which tile holds which bel |
| the tile bitmap sizes (`logic_tile` 16×54, `io_tile` 16×18, `ramb`/`ramt` 16×42) | every bit position in them |
| eight global clock networks | which tiles their buffers sit in |
| the primitive names, pins and parameters (from the `.dev` database) | the package pin to site map |

So a placement, a routing and a bitstream produced here are *structurally*
correct and *electrically* meaningless: the `.asc` has the right shape and
reads back to the same bits, and it will not program a part. **This is
still true.** Nothing in the 7-series work described in
[`fpga-xray.md`](fpga-xray.md) touches this fabric; the iCE40 database is
still not here, the positions below are still invented, and an `.asc`
from them still programs nothing. What the
tests prove is that the algorithms are right — the placer respects the
constraints, the router reaches every sink from its driver, the bitstream
round-trips — and that the flow is reproducible byte for byte.

The synthetic fabric also differs from the real part in four ways that a
design must not lean on, all listed in `arch::synthetic`: two logic cells
per tile rather than eight; the LUT, the flip-flop and the carry as three
separate bels on three separate sites rather than one packed logic cell;
per-flip-flop clock, enable and reset wires rather than tile-wide shared
ones; and a carry chain that can escape onto the local tracks.

## The `.arch` format

A routing architecture is data, in the same line-oriented text format the
`.dev` and `.rcf` files use: one directive per line, whitespace
separated, `#` or `//` comments. The grammar is deliberately flat — an
`arch` block has exactly one `end`, and a tile type's members name their
tile type instead of nesting — so one file may hold `device` blocks and
`arch` blocks side by side and either parser skips the other's.

`testdata/fpga/tiny.arch` is a complete hand-written example. The
directives:

| Directive | Meaning |
|---|---|
| `arch <name>` … `end` | one architecture |
| `family <name>` | the device family it belongs to |
| `part <device>` | a device name this architecture serves; repeatable |
| `grid <width> <height>` | the tile grid |
| `asc_device <word>` | what the `.asc` `.device` line carries (`1k`) |
| `global <wire>` | a wire that reaches the whole die; one node for the part |
| `tiletype <name> asc <keyword> bits <rows> <cols>` | a tile type and its configuration bitmap |
| `wire <tiletype> <name> span <dx> <dy>` | a wire that starts in a tile and reaches `(dx, dy)` tiles away |
| `bel <tiletype> <name> <kind> pin <role>=<wire> …` | a placeable element and the wire each pin reaches |
| `config <tiletype> <bel> cell <PRIMITIVE> bits <r>.<c>,…` | bits set when a cell of that primitive sits on the bel |
| `config <tiletype> <bel> param <NAME> <index> <r>.<c>` | the bit carrying bit `index` of the cell's parameter |
| `pip <tiletype> <from> <to> bits <r>.<c>,…` | a directed programmable connection and its bits |
| `tile <x> <y> <tiletype>` | one tile |
| `tiles <tiletype> rect <x0> <y0> <x1> <y1>` | a rectangle of them |
| `pinmap <package pin> <site>` | which site a package pin reaches |

A **wire reference** in a pip or a bel pin says which node is meant:
`local0` is the wire of that name starting in this tile, `sp4e@-4,0` the
one starting four tiles west (so it arrives here), and `*glb0` the global
wire `glb0`.

`Arch::to_text` writes the directives in that order and
`parse(to_text(a)) == a` holds, so an architecture can be generated,
diffed and reviewed like source. The one normalisation is that
`tiles … rect` comes back as one `tile` line per position.

`Arch::build_graph` expands an architecture into a `RoutingGraph`: a
`Vec<Wire>` of nodes, a `Vec<Pip>` of edges (each with the tile it lives
in and its configuration bits), a `Vec<ArchSite>` of placeable locations
named `X<x>Y<y>/<bel>`, and compressed-sparse-row neighbour lists in both
directions. Ids are handed out in a fixed order — tiles row-major, each
tile's wires in declaration order — so the same architecture always gives
the same graph.

A pip's bits are normally a **multiplexer code**, not a bit of its own:
every wire is driven by one multiplexer, and the pips into it share a
small field, with code 0 meaning off. That is how a real database encodes
them, and the bitstream writer does not care either way — it sets
whatever bits the pip lists.

## Dropping in a real iCE40 database

Everything the flow needs is in the format above, so a converter from
Project IceStorm's chip files replaces the built-in architecture with no
code change: parse it with `Arch::parse` and pass it as
`PnrOptions::arch`. What such a converter has to emit:

1. **The grid and the tiles.** Icebox's chip database gives the die size
   and the type of every tile: one `grid` line and one `tile` line each.
2. **The tile types** with IceStorm's real bitmap sizes: `logic_tile`
   16 × 54, `io_tile` 16 × 18, `ramb_tile` and `ramt_tile` 16 × 42 on the
   1k dice.
3. **Every wire** icebox names for that tile type — `local_g0_0`,
   `sp4_h_r_0`, `sp12_v_b_0`, `lutff_0/in_0`, `glb_netwk_0`, … — with the
   span its segments imply, as a `wire` line.
4. **Every pip**: each entry of icebox's `.buffer` and `.routing` blocks
   becomes a `pip` line, with the `B<row>[<col>]` positions it lists as
   that pip's configuration bits.
5. **Every bel**: one per `lutff_*`, `io_*`, `ram` and `gb`, with the wire
   each pin reaches, plus `config` lines for `LC_<n>` (the sixteen
   `LUT_INIT` bits, the flip-flop enable and set/reset bits, the carry
   enable) and for `IoCtrl` / `IOB_<n>` (`PIN_TYPE`, `PULLUP`).
6. **The package**: one `pinmap` line per package pin, from the package
   file.

With that in place, `Bitstream::write_asc` produces an `.asc` that
`icepack` turns into a real bitstream and `icebox_vlog` can explain. The
bel kinds a bel must use are the device database's role keywords —
`lut`, `ff`, `carry`, `io`, `gb`, `bram` — and the pin roles are derived
from the `.dev` port maps: a role with several port names (a LUT's
`i=I0,I1,I2,I3`) takes that role plus its position (`i0`…`i3`), a block
RAM's signals take their physical port's index as a prefix (`p0_clk`,
`p1_addr3`), and a port connected to more than one bit takes the bit
number (`p0_dout7`).

## Placement

`fpga::place` runs two passes.

**Analytic.** The netlist becomes a weighted graph — every signal a clique
over the instances it touches, weighted so that a wide net does not
outvote a two-pin one — and the placement minimising total squared wire
length is the solution of two sparse symmetric systems, one for `x` and
one for `y`. They are solved by a hand-written conjugate-gradient
iteration (`place::solve`). Instances tied to a package pin are the fixed
boundary that makes the systems non-singular; a small pull towards the
middle of the die keeps a design with no fixed pins bounded. The result
is a cloud of real numbers, so every instance is then assigned the
nearest free site of the kind it needs.

**Annealing.** Simulated annealing over a half-perimeter wirelength cost,
with a move set of swaps, moves to a free site and macro relocations. The
starting temperature is twenty times the spread of the cost changes a
random walk sees; the schedule is geometric, and it stops when the
temperature falls below `0.005 * cost / nets` or after
`PlaceOptions::max_temperatures` steps. The best placement seen is kept,
so the pass can only improve on what legalisation produced.

Constraints (`fpga::Constraints`) are honoured as follows:

- a **pin constraint** (`set_io`) is hard: the cell lands on the site the
  architecture's `pinmap` gives for that package pin and never moves; two
  cells on one pin is an error;
- a **region** confines every cell whose name matches the pattern, in
  legalisation and in every annealing move;
- **`keep_hierarchy`** gives each matched group its own region: the box
  its members legalised into, grown by one tile, so the group can improve
  inside it but cannot be scattered;
- an **`rloc` macro** is rigid — its members keep the exact tile offsets
  the constraint states, and the annealer moves the macro, not its
  members.

A design that does not fit is reported, not approximated: `PlaceError`
says which kind of site ran out and by how much, which two cells want one
package pin, or which region cannot hold what was assigned to it.

## Routing

`fpga::route` is PathFinder (McMurchie and Ebeling, 1995). One iteration
rips up every signal and routes it again through an A\* maze expansion
over the routing graph. The cost of entering a node is

```
cost(n) = (1 + present_factor * overuse(n)) * (1 + history(n))
```

where `overuse(n)` counts the signals beyond its capacity, `history(n)`
accumulates every iteration in which the node was oversubscribed, and
`present_factor` grows each iteration. History is what makes it converge
rather than oscillate: a node that has been fought over stays expensive
once it is free, so the loser looks elsewhere instead of taking it back.

A multi-sink signal is routed sink by sink onto a growing tree, with the
nodes already in the tree free, which is how the trunk comes out shared.

`RoutingReport` keeps the overuse of every iteration, so the convergence
is visible rather than asserted; the golden `.route` files in
`testdata/fpga/` are exactly that history. A design the fabric cannot
carry is reported — `RouteError::Unroutable` names the signal and the
sink for a path that does not exist at any price,
`RouteError::Congested` names the worst contended wires when the
iteration cap is reached — rather than looped on.

`Routing::verify` walks every sink backwards through the pips its signal
was given until it reaches the source, which is the property that
matters; `tests/fpga_pnr.rs` does the same walk independently.

## Bitstreams

`fpga::bitstream::generate` sets, for each routed pip, the configuration
bits that pip lists in the tile that holds it, and for each placed cell
the bits its bel's `config` entries give: a `cell` entry for the
primitive it is (which is how twenty `SB_DFF*` variants become one mode
field) and a `param` entry per bit of a parameter it carries (`LUT_INIT`,
`PIN_TYPE`, `READ_MODE`). Nothing else. Every position comes from the
architecture file; no bit number appears in Rust.

Two output formats:

- **`.asc`** — the IceStorm-style text: a `.device` line, then one block
  per tile headed `.<keyword>_tile <x> <y>` followed by one line of
  `0`/`1` per bitmap row. `Bitstream::parse_asc` reads it back, so the
  writer and the reader check each other.
- **`.bin`** — *Reticle's own* container, documented on
  `Bitstream::write_bin`: magic `RTBS`, a version, the device word, then
  each tile's position, keyword, bitmap size and packed bits. **It is not
  an iCE40 configuration image**; producing one of those from an `.asc`
  is what `icepack` does, with the database this crate does not ship.

`Bitstream::to_summary` gives the compact, diffable view a reviewer
wants: the totals and the set bits of each tile that has any.

## Determinism

Golden files are compared byte for byte, on three platforms, so every
choice is pinned:

- the single source of randomness is `place::Rng`, an xorshift seeded
  from `PlaceOptions::seed` (default `0x5EED_1CE4_0000_0001`);
- every iteration order is over a `Vec` or a `BTreeMap`, never a
  `HashMap`;
- the router is not random at all: signals go in netlist order, sinks in
  pin order, and the priority queue breaks ties by node id;
- the annealer avoids the two library calls whose last bit is not
  guaranteed to match across platforms. The acceptance probability of an
  uphill move is the Cauchy rule `T / (T + Δ)` rather than `exp(−Δ/T)`,
  and the moves per temperature use an integer cube root rather than
  `powf`. Both are monotone in exactly the same way.

## Tests

- Unit tests next to the code: the graph expansion and the `.arch`
  reader, the placer's legalisation and each kind of constraint, the
  router on small fabrics with known-routable, known-congested and
  known-disconnected cases, and the bitstream round trips.
- `tests/fpga_flow.rs`: seventeen cases through synthesis, primitive
  mapping, LUT mapping and the export, with the mapped design, the
  report, the diagnostics and every exported file as goldens. The four
  `*_xc7` cases add the Vivado route: `blinky_xc7` for the LUTs, the
  flip-flops and the per-direction IO buffers, `ram_xc7` for the block
  RAM's contents, address alignment and byte enables, `logicram_xc7` for
  the distributed RAM below the block threshold, and `pll_xc7` for the
  feedback loop closed outside the block. Five further tests check the
  7-series claims one by one, and one checks that the nextpnr export
  declines this family by name.
- `tests/fpga_pnr.rs`: the three iCE40 designs of `testdata/fpga/`
  through the whole flow, with `<name>.place`, `<name>.route` and
  `<name>.bits` golden files and one full `<name>.asc`. Rewrite them with
  `UPDATE_EXPECT=1` and read the diff.
- `tests/fpga_pnr.rs` also walks every routed sink back to its driver
  independently of `Routing::verify`, and runs `icepack` over the result
  when one happens to be installed, reporting what it said rather than
  failing: a synthetic architecture is not one `icepack` has any reason
  to accept.
