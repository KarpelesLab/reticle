# NewAE's ChipWhisperer-Husky gateware, built with Reticle

The ChipWhisperer-Husky is NewAE Technology's side-channel capture and
fault-injection instrument. Its FPGA is a Xilinx Artix-7 **XC7A35T in
the FTG256 package** (`xc7a35tftg256-1`), and its gateware is open:
[`newaetech/chipwhisperer-husky-fpga`](https://github.com/newaetech/chipwhisperer-husky-fpga).
This directory is what Reticle needs beyond that source to build it:
stand-ins for the Vivado IP cores the gateware instantiates, its pin
constraints in Reticle's form, and the command line.

**The Husky Plus build has been loaded into a Husky Plus, and it does
not yet work as one.** Its USB register interface works, and the carry
chains and clock inputs a smaller design on the same bus uses were
checked on the part; NewAE's own test suite passes 4 of the 47 tests
that their bitstream passes 42 of, and the faults are believed to be
timing. The section *On a Husky Plus* says what was measured. The
plain Husky's build has not been on a part. What is checked, and what is
only quoted, is at the end of this page; read it before loading
anything into an instrument somebody depends on.

## What is here

```text
examples/husky/
  build.sh            the command line: NewAE's file list, defines and top
  husky.rcf           every pin of `cwhusky_top`, from NewAE's cwhusky.xdc
  fifo_generator.v    the six FIFO Generator cores, as one behavioural FIFO
  xadc_wiz_0.v        the XADC Wizard core: the XADC with the wizard's registers
  usr_accesse2.v      USR_ACCESSE2, which reads the build time: here, zero
  build_plus.sh       the Husky Plus: cwhusky_plus.xpr's files and defines, an XC7A100T
  probe/              a small design on NewAE's USB bus, and the script that checks it on a part
  xpm_memory_sdpram.v Vivado's simple dual-port RAM macro, as NewAE's own FIFOs use it
  sim/                NewAE's own testbench in Reticle's simulator: models of the
                      Xilinx primitives and cells, and the scripts that run
                      `cwhusky_tb.v` on NewAE's RTL and on Reticle's netlist
```

## Simulating NewAE's own testbench

NewAE's whole-design testbench, `fpga/sim/cwhusky_tb.v`, runs in
Reticle's simulator as NewAE's `make all VARIANT=PLUS` runs it under
iverilog, with every testbench parameter at the makefile's default or as
given:

```sh
examples/husky/sim/run_cwhusky_tb.sh husky /tmp/tb.log FIFO_SAMPLES=300
```

It takes about eight seconds. It uses NewAE's own FIFOs (`NOXILINXFIFO`,
which is NewAE's default for simulation) over `xpm_memory_sdpram.v`, and
the primitives from `sim/xilinx_primitives.v`, behavioural models written
from UG472, UG953 and XAPP888 rather than Vivado's unisims; their header
says what each leaves out. As of 2026-10-23 a capture of 90 samples reads
back all 90 correctly, and one of the testbench's checks fails: the slow
FIFO is not empty at the end of the read cycle.

### On Reticle's own netlist

The same testbench runs on the netlist Reticle maps the design to, the
`cwhusky_top.v` a `build_plus.sh` output directory gets about a minute
into the build, before placement:

```sh
examples/husky/sim/run_cwhusky_netlist.sh husky out/cwhusky_top.v /tmp/net.log
```

It takes about three and a half minutes. The cells are modelled by
`sim/xilinx_cells.v` (flip-flops, LUT6, CARRY4, RAM64X1D, RAMB18E1 in the
one configuration the mapper writes, and the I/O buffers; checked by
`sim/xilinx_cells_tb.v`), the clocking primitives by
`sim/xilinx_primitives.v`, and the FIFO Generator cores by this directory's
stand-ins, as in the build. The script rewrites two things on the way in,
and its header says which and why. `SIM_ARGS="--vcd f"` passes options to
the simulator for either script.

This is what found why no capture finished on the part (2026-10-24). Run
on the netlist, the testbench failed as the part did: the fast FIFO full,
the slow one empty, nothing read back. The first signal whose waveform
left the RTL's was `presamp_running`, at time zero: `reg presamp_running
= 1'b0;` in `fifo_top_husky.v`, set synchronously and so mapped to an
`FDSE`, whose own power-up value is 1. Reticle's flip-flop mapper gave
every flip-flop its primitive's default and ignored the register's: 540
of them in this design. Nothing clears `presamp_running` when there are
no presamples, and it holds shut the gate that moves samples from the
fast FIFO to the slow one. With the mapper fixed, the netlist reads all
90 samples, as the RTL does, and fails only the RTL's one check.

Three things in the simulator and the netlist writer had to be fixed
before the netlist would run at all, each with its own test: a port list
entry `.a(a_pad)` (the netlist's top level), a bus joined to `inout`
ports bit by bit (every bidirectional buffer), and the writer joining a
bidirectional pad's pins to its port with a one-way `assign`.

## Building

```sh
git clone https://github.com/newaetech/chipwhisperer-husky-fpga husky
git -C husky checkout 2dfe1bc
git -C husky submodule update --init fpga/fpga-common
git -C husky/fpga clone https://github.com/newaetech/tracewhisperer
git -C husky/fpga/tracewhisperer checkout 542b1cc
git -C husky/fpga/tracewhisperer submodule update --init
examples/husky/build.sh husky /tmp/husky
```

The design fits and places as of 2026-10-19, in four to five minutes;
whether it routes is the open question, and the section at the end says
what is known.

The superproject pins `fpga/tracewhisperer` at a commit that is not
public; `542b1cc` on its `pro` branch is the newest public one, and is
the one this was built against.

`build.sh` names the `sources_1` fileset of `fpga/vivado/cwhusky.xpr`,
file for file and in its order, with its three defines
(`INTERVAL_MATCHING=1`, `ESAD`, `BB_LIMIT`). Two corrections: the xpr
reaches `fpga-common/hdl/async_resetFFstyle2.v` through a path outside
the checkout (`../../plus_optimization/...`), and the copy inside is
used; and `hdl/trigger_cdc.v`, which `reg_chipwhisperer.v` instantiates,
is not in the list and is added.

An earlier reading of the xpr took two more defines, `TINYFIFO` and
`SAD_X2`, from it; both are on IP filesets (`xadc_wiz_0`, `clk_wiz_0`),
not on the design, and with them the build asked for FIFO cores the
project does not have.

## The stand-ins

### FIFO Generator

`fifo_generator.v` has one FIFO, `husky_fifo`, and a wrapper per core,
each with the core's ports and the parameters of its `.xci`
(`fpga/vivado/cwhusky.srcs/sources_1/ip/<core>/<core>.xci`): width,
depth, standard or first-word fall-through, common or independent
clocks, and the programmable thresholds.

| Core | Width x depth | Clocks | Read | Flags |
|---|---|---|---|---|
| `adc_fast_fifo` | 12 x 2048 | common | standard | overflow, underflow |
| `fast_slower_fifo` | 72 x 4096 | independent | fall-through | prog_empty 6, overflow, underflow |
| `fifo_generator_0` | 18 x 8192 | independent | fall-through | prog_full 8188, prog_empty 128, underflow |
| `trigger_fifo` | 32 x 1024 | common | standard | underflow |
| `usb_slow_fifo1` | 72 x 2048 | common | fall-through | prog_full 2047, prog_empty 4, overflow, underflow |
| `usb_slow_fifo2` | 72 x 16384 | common | fall-through | prog_full 16384, overflow, underflow |

Vivado builds these from the block RAM's own FIFO logic; here each is a
memory with its pointers and flags in the fabric. What differs, said in
the file's header too: the hardware FIFO's capacity in fall-through mode
is a word or two more than its depth, and its flags move on different
cycles after a write on the other clock. NewAE's logic around these
cores counts samples itself and reads the flags as levels, which is the
reading of their source that makes a stand-in reasonable — a reading,
not a measurement.

Together the six hold 1.83 Mbit, on a part with 1.84 Mbit of block RAM.
They fit only because the block RAM's parity bits carry data (18- and
9-bit modes): 100 `RAMB18E1`, every one the part has, which is also how
many NewAE's own image uses.

### XADC Wizard

`xadc_wiz_0.v` is the `XADC` primitive with the wizard's initial
registers, from `xadc_wiz_0.xci`, and its DRP passed straight through.
Every register was compared with the XADC tile of NewAE's image as
decoded here, and one disagrees with the `.xci`: Vivado wrote `INIT_5C`
as `0x5111` where the `.xci` says `0x9999`. This uses Vivado's value.
`tests/fpga_xray_xadc.rs` is that comparison.

### USR_ACCESSE2

On the part, `USR_ACCESSE2` reads a word the bitstream sets, which
Vivado fills with the build time. Reticle neither places that site nor
writes that register, so the stand-in returns zero — what NewAE's own
source gives it under Icarus. Software that prints the build time will
print zero.

## What the build needed from Reticle

Building this design is what found each of these, and each has a test
of its own:

- `--define`, `--include-dir`, and macros carried from file to file.
- Every primitive the gateware names: `BUFG`, `BUFGCE`, `BUFGMUX`,
  `IBUFG`, `BUFMRCE`, `BUFR`, `IBUFDS`/`OBUFDS` (on bits of vector ports,
  as the ADC's six lanes are), `IDDR`, `ODDR`, `PLLE2_ADV`,
  `MMCME2_ADV`, `PULLUP` and `XADC` — each compared with what Vivado
  wrote for the same cell on the same site of NewAE's image.
- The `xc7a35t-ftg256` package itself.
- Block RAM parity bits as data.
- Bits of one register driven by several clocked processes.
- An array addressed only by constants made registers, not a memory: the
  `esad` trigger's 256 counters did not finish mapping in 40 minutes and
  now take 12 seconds.
- LUT mapping that spends LUTs only on bits something reads, products
  with a power of two as wiring, and an aligned part-select write as one
  comparison per group: a byte written into the clock-glitch module's
  1024-bit register cost over twenty thousand LUTs, and costs about a
  thousand.
- Two walks over shared expressions that had no memory of where they had
  been, one of which asked for 64 GiB.

## Checked, and quoted

| Claim | Status |
|---|---|
| each primitive's configuration bits equal Vivado's for the same cell on the same site | **checked**, per primitive, against NewAE's `cwhusky_top.bit` decoded through prjxray-db |
| the pins, standards, drive and pulls are NewAE's | **transcribed** from `cwhusky.xdc` by script |
| the FIFO stand-ins behave as the FIFO Generator does where NewAE's logic looks | **a reading** of PG057 and of NewAE's source |
| the whole design fits the part | **checked**: 21,217 lookup-table sites of 32,600, 27,618 flip-flops of 32,600 |
| it places | **checked**: legalisation and annealing complete, in four to five minutes |
| it routes | **checked**: the Husky's converged at the 78th pass on 2026-10-20, the Plus's at the 50th on 2026-10-21; earlier, this row said it did not |
| it works on a Husky | **not checked**: nothing of the Husky's has been loaded into one |
| the Plus's bitstream configures a Husky Plus and answers on its USB bus | **checked** on 2026-10-21: `chipwhisperer` connects, reads its version and reads back what it writes |
| it works as a Husky Plus | **no**: its registers read and write (one read in a few hundred has lost a bit, varying by build), its glitch clock managers lock, and 5 of NewAE's tests pass where their bitstream passes 42; see *On a Husky Plus* |
| carry chains compute what they should | **checked** on the part: `probe/`, 309 of 309 sums, differences and comparisons, and a counter at 96 MHz |

## On a Husky Plus

A ChipWhisperer-Husky Plus (`2b3e:ace6`, firmware 1.2.0) is an XC7A100T
in the same package, and `build_plus.sh` builds its gateware from the
same checkout: `cwhusky_plus.xpr`'s files, the `PLUS` define, and the
pins of `husky.rcf`, which `cwhusky_100.xdc` puts on the same balls.
It takes 17.5 minutes and prints the same four notes the Husky's does,
with 64,596 signals and 28,185 clock pins. NewAE's reference here is
their Vivado build of the same commit, `fpga/vivado/cwhusky_plus_top.bit`,
which `chipwhisperer`'s develop branch (61aec0925) ships byte for byte
and whose register map is the checkout's; the PyPI release's is not.

**What was measured, 2026-10-21**, with `chipwhisperer` at 61aec0925
and its `tests/test_husky_dev.py` (no target attached, so 166 tests
skip either way):

| | NewAE's bitstream | this flow's |
|---|---|---|
| `test_husky_dev.py` | 42 passed, 5 failed | 4 passed, 43 failed |
| connects, reads the version (2, 88: Husky Plus) | yes | yes |
| ECHO register, written and read back | yes | yes |
| the 16-byte SAMPLES register, written and read back | yes | no |
| the board PLL's clock, as the FPGA measures it, 7.37 MHz asked for | 7,363,735 Hz | 0 |
| the ADC's clock, as the FPGA measures it, 10 MHz asked for | 10,000,133 Hz | 0 |
| TraceWhisperer's front-end clock, from the USB clock | 96 MHz | 0 |
| XADC | good | good |
| an ADC capture | the samples | 0 samples |

The five NewAE fails are four glitch-width ratios and one PLL test, on
their own bitstream; whatever they are, they are this bench's.

**What the first load showed** was the bitstream placed for wirelength
alone: it configured, answered on the bus, and read back nearly every
bit as zero. NewAE give the read path 15 ns (`set_max_delay 15` in
`cwhusky_100.xdc`) and this flow's estimate of it was 29 ns; placed with
`--timing-driven` (`xray::place_timing`), the version and ECHO read
back right. So the flow's first timing-driven placement is what made
the bus work, and it is in `build_plus.sh`.

**What is not the fault**, each checked on the part with `probe/`, a
small design on NewAE's own bus logic: carry chains (309 of 309 sums,
differences and comparisons; a 32-bit counter at 96 MHz), and the LVDS
clock from the board's PLL through `IBUFDS` and `BUFG` to a counter,
which counts 10.02 MHz once NewAE's gateware has set the PLL to 10. The
LVDS inputs' configuration bits are Vivado's, decoded, and the pulse
synchroniser every frequency measurement goes through simulates
correctly.

**What is believed to be**, and not yet shown: timing in the 96 MHz
`clk_usb` domain. A register write is decoded and lands within one
cycle, 10.4 ns, at registers across the die, and this flow's routed
estimate (Project X-Ray's pip delays, slow corner) puts 8,353 of the
domain's endpoints past that and its longest path at 41 ns: a register
read mux thirteen lookup tables deep, where Vivado uses the slice's
`F7`/`F8` multiplexers and this flow has none. The estimate is
pessimistic — it put the bus's read path at 26 ns, and the bus reads —
but not by that much. A write that does not land would leave the clock
sources and capture settings as they came up, which is what the table
shows.

**The bus clock's own road** (2026-10-21). Until then `clk_usb` reached
its buffer over general routing, 4.4 to 12.4 ns that moved with every
placement, where Vivado uses the clock-capable pin's dedicated road; the
section of `docs/fpga-xray.md` dated 2026-10-21 has the three reasons and
the fixes. It now takes the road, 2.18 ns, and the probe passes on the
part with it. The first full build with it read nothing at all.

**What the bus needs of its inputs** (2026-10-21) is why. The SAM3U
launches the bus's strobes, address and data on the very edge of
`clk_usb` that `usb_reg_main` must capture them on, so each must reach
its register before the clock does; a long clock insertion gave that for
free, and the road's does not. Measured on the part, then constrained:
`husky.rcf`'s `set_input_delay`, which the placer turns into a distance
from each pad (`docs/fpga-xray.md` has the measurements). With it, placed
afresh and nothing moved by hand:

| | NewAE's bitstream | this flow's, 2026-10-21 |
|---|---|---|
| VERSION | (2, 88) | (2, 88) |
| ECHO, 300 random 8-byte writes and reads | right | 7 wrong, all bit 5 of an odd byte |
| SAMPLES, 300 random writes and reads | right | right |
| MAX_SAMPLES, read-only | NewAE's values | the same values |
| `test_husky_dev.py` | 42 passed, 5 failed | 3 passed, 43 failed |

What NewAE's tests still fail on, by cause: `test_reg_rw`, bit 5 of a
byte at some burst positions, one read in a few hundred — a path inside
the design from the byte counter to the read register, which the
estimate puts at twice the period; the glitch modules (19), whose clock
managers do not report lock; every capture (16), which returns no
samples; the PLL (4); and `test_fpga_version`, which reads the build
time from `USR_ACCESSE2`, and the stand-in here reads zero.

**The glitch clock** (2026-10-22). Its MMCMs did not lock on the board
PLL because the clock never reached them: the PLL's pad, on the die's
left half, missed its road (the left half names the clock tile's outputs
differently), and every buffer-to-buffer cascade this flow built was
dead, for want of a bit the database does not name. Both are fixed, and
the second is in `docs/fpga-xray.md`, "The cascade's own bit". Built
again, placed afresh:

| | NewAE's bitstream | this flow's, 2026-10-22 |
|---|---|---|
| ECHO, 200 random 8-byte writes and reads | right | right |
| the ADC's clock, as the design measures it (`ADCFREQ`) | 10.0 MHz | 10.0 MHz |
| glitch MMCMs, on the board PLL at NewAE's multiplier | locked | locked |
| `test_husky_dev.py` | 42 passed, 5 failed | 5 passed, 41 failed |

The glitch tests now fail later: what they capture comes back empty,
from the logic analyser as from the ADC, so capture is the next thing.
The trace front end's clock is still not alive, and a clock looped out
on the AUX connector and back reads nothing here where NewAE's reads it.
ECHO right in 200 is one build's luck, not a fix: the paths inside the
design that made it lose a bit are as long as they were.

**Capture** (2026-10-24), the first build with flip-flops powering up as
their registers say. Through NewAE's `chipwhisperer` API: the ADC's own
test ramp (`scope.adc.test_mode`), a 10 MHz ADC clock, triggered on
`tio4` held high, three captures each, a capture exact when it is all
there and every step between samples is the same:

| samples | 8 bits | 12 bits |
|---|---|---|
| 1,000 | 3 of 3 exact | 3 of 3 exact |
| 30,000 | 3 of 3 exact | 1 of 3: samples repeated and skipped |
| 131,070 | 1 of 3: samples repeated and skipped | none: slow FIFO underflow and overflow |

Short captures are right; long ones, which keep the slow FIFO filling and
draining while the bus reads it, lose and repeat words. The routed
estimate still puts `clk_usb`'s latest endpoints in the capture logic at
13 to 15 ns of 10.4, which is the next suspect.

## Status

**It places, routes and writes a bitstream** (2026-10-20). `build.sh`
takes about 53 minutes on the machine this was built on: 25 seconds to
map, 200 to place, and the rest to route, which converged at the 78th
pass. What the flow then checked and printed:

    note: 62989 of 62989 signal(s) with a reader routed
    note: 27882 of 27882 clock pin(s) a global buffer drives arrived on the global clock network
    note: decoded: 2103642 bit(s) over 10304 tile(s) into 1290860 feature(s); 0 bit(s) unexplained, 0 tile(s) with no segbits file
    note: the interconnect arcs those bits select are exactly the ones the router chose and the block RAM ties added

What it took is the section of `docs/fpga-xray.md` dated 2026-10-20: a
global placer, the five-input flip-flops and a lookup table's flip-flops
packed beside it, a router that chooses a lookup table's inputs, a
congested signal's box that grows, and a clock that may enter its pin
only off the clock network. Not done, and the next things to know
before a part is loaded: the route is not timing-driven, and the ADC
side runs at up to 200 MHz; and nothing has compared this bitstream's
behaviour with NewAE's.

The whole design elaborates, synthesises and maps — in about 25 seconds,
where the first attempt did not finish in 83 minutes — and the result
is, measured on 2026-10-19:

| | Reticle | NewAE's image (Vivado 2022.1) |
|---|---|---|
| lookup tables | 27,158 functions in 21,217 sites (5,941 `LUT6_2` pairs) | 23,274 lookup tables with contents |
| flip-flops | 27,618 | about 26,000 with an initial value of zero |
| `RAMB18E1` | 100 | 100 |

On 2026-10-16 it was 35,176 single-function `LUT6` cells and did not fit.
What closed it, in `docs/fpga-xray.md`'s two sections on the lookup
table's second output: pairing small functions into `LUT6_2`, then
sharing `esad`'s two adders per counter as one carry chain whose generate
comes off the propagate table's `O5`. The functions went *up* (39,026)
and the sites down, to 30,855 on 2026-10-17.

On 2026-10-19 the mapper stopped spending lookup tables on three things,
which `docs/fpga-xray.md`'s section on them has: a hold multiplexer for
every bit of a register written a byte at a time from the bus, which is
every register file here; a multiplexer for every bit of `esad`'s
counters, which restart at their increment; and a barrel shifter where
the clock-glitch module picks a 13-bit field at a stride of 13. It also
started spending some where it had spent none, and **that was a fault**:
the trigger sequencer's wait windows and TraceWhisperer's match patterns
are unpacked wire arrays, and synthesis had read every element of them
as `x`. Fixed, they cost 77 lookup tables, and every earlier build of
this design had those triggers built from constants.

`build.sh` asks for the whole die (`--region 0,0,114,156`) because the
default region, a box around the constrained pins, holds 26,800
flip-flop sites.

The database describes this part with the XC7A50T's tile grid, 8,150
slices or 32,600 lookup tables, and decoding NewAE's image finds
configured slices in about 93 % of those tiles — more than the 20,800
lookup tables the XC7A35T is specified with. Why Vivado uses them has not
been looked into; what is measured is that it does, so 32,600 is the
ceiling that matters. Reticle's netlist was over it until 2026-10-17,
and is 21,217 sites of it now.

What is known about the gap that remains:

- **Four flip-flops per slice, of eight** — until 2026-10-20, when the
  `A5FF`..`D5FF` were offered, fed from the bypass input or the lookup
  table's `O5`, as Vivado feeds them. The whole die is still asked for.
- **No wide multiplexers.** Vivado builds 7- and 8-input selections from
  the slice's `MUXF7`/`MUXF8`; Reticle from lookup tables.
- **`esad`**, NewAE's extended SAD trigger, is half the design: 256
  counters, about 53 lookup tables each after the sharing (59 before).

Fitting is not placing. On 2026-10-17, with the whole die asked for,
the legaliser stopped with

    error: `oadc.U_reg_openadc_adcfifo.ddr_la_start_address$ff$ff7` found
    no `ff` site it may take: the design needs 27618 of the 32600 the part
    has, and every one still free shares a pin or a tile's own wires with
    something placed that wants another signal there

— the sites are there and were refused. The error now counts the free
sites by the rule that refused each, and of the 8,926 still free 6,214
were in slices whose clock already carried another clock, 876 in slices
whose clock enable did, and 1,836 had no wire left for `D`. That is the
legaliser filling slices with no regard to control sets — each
flip-flop takes the nearest legal site, and a slice one flip-flop of a
clock domain opened is closed to every other — more than it is a lack of
sites. *That was half the story*: counted by control set, the design
needed more slices than the part has at four flip-flops a slice, because
the flip-flop mapper gave each bit of `esad`'s counters a reset net of
its own (2,764 control sets, 1,014 once fixed). With that fixed and the
legaliser joining a slice before opening one, placement completes;
`docs/fpga-xray.md` has both, and the section after them has why a
build then took hours and what it takes now. Nothing routed a design
this full on this die until 2026-10-20 (the status above), and the ADC
side runs at up to 200 MHz while the 7-series placer is not
timing-driven. Nothing has been loaded into a Husky; none is attached
to the machine this was built on.
