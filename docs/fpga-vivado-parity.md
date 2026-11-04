# Reticle against Vivado, measured

Vivado 2026.1 is installed here, so what it does is an answer to look up
rather than a thing to remember. This is the account of where Reticle's 7-series
flow stands against it. The part is the XC7A100T in FTG256, the Husky Plus's,
at speed grade 1.

**Everything here is on a static tool's word.** No design in this document has
been on a board for this comparison, and the Husky is in use elsewhere.

Fixtures are under `testdata/fpga/vivado/`, the scripts that made them under
`tools/vivado-parity/`, and only answers are committed: the designs are written
here, and nothing of the vendor's models or IP is copied.

## Cells by name

`src/fpga/devices/xc7_cells.v` holds the cells a design may name beyond what
the device file declares: lookup tables, gates, flip-flops and latches, shift
registers, distributed memories and ROMs. Elaboration adds a module from it only
when a design names it and declares nothing of that name.

**Checked:** `tools/vivado-parity/gen_cell_vectors.py` drove the vendor's model
of each with fixed pseudo-random inputs under `xsim` and kept what came back.
All 321 variants (each cell's default, a random `INIT`, and every `IS_*_INVERTED`
alone) answer the same in Reticle's simulator, in
`tests/fpga_xray_cells.rs`. Five mutations of the library, among them a
synchronous `FDCE` and a `CARRY4` with the wrong `CI`, each failed it.

The first run was worth having: my `RAM16X2S` family had vector pins where the
vendor has `O0`, `O1`, `D0`, `D1`, and my latches took an `IS_D_INVERTED` that
they do not have. An `FD_1` that seemed wrong was the testbench: a first value
counts as an edge from unknown to some simulators and not to others, so a cell
that acts on a falling edge now starts with its clock high.

**Not checked:** these become logic, so an `SRLC32E` is 32 flip-flops today, not
the one lookup table Vivado makes it. The table below says what that costs.
`MUXF7`, `MUXF8` and their `_L` and `_D` forms are the exception: a build keeps
them as the slice's own, below.

## Which primitives build

`testdata/fpga/vivado/primitives/` has one design per 7-series primitive, 146 of
them. Vivado writes a bitstream for **141**, and refuses five on this part:
`IOBUF_DCIEN`, `OBUFT_DCIEN`, `IOBUFDS_DCIEN` and `IOBUFDS_DIFF_OUT_DCIEN`, since
the part's banks are all high-range and have no DCI, and `ZHOLD_DELAY`, which it
would not place alone from a pad or from logic.

**Reticle builds 86 of the 141.** `index.txt` says which;
`the_ledger_says_what_builds` holds it exact (ignored, since it takes every probe
to a bitstream), and `the_primitives_the_ledger_calls_ok_map` runs in the gate.

**The first version of this section was wrong**, and said Vivado built all 146.
The runner passed `-quiet` to `write_bitstream`, which hid the refusals: 24 probes
had no `.bit` at all and the table called them built. Decoding one is what showed
it. The runner now fails a step with no file behind it, and of those 24, 19 were
illegal probes, a `DSP48E1`'s `PCOUT` taken to the fabric or an `ODDR` to a
flip-flop, which now obey Vivado's rules.

The 55 that do not build, by what stops them:

| what | primitives |
|---|---|
| no latch on the fabric yet | `LDCE`, `LDPE`, `LD`, `LDC`, `LDP`, `LDCE_1`, `LDPE_1` |
| block RAM and DSP by name | `RAMB18E1`, `RAMB36E1`, `FIFO18E1`, `FIFO36E1`, `DSP48E1` |
| clock buffers | `BUFGCTRL`, `BUFGMUX_CTRL`, `BUFH`, `BUFHCE`, `BUFIO`, `BUFMR`, `BUFR` with a driven `CE` |
| clock managers | `MMCME2_BASE`, `PLLE2_BASE`, `MMCME2_ADV` (`COMPENSATION=ZHOLD`) |
| pad buffers | 15 of them, chiefly the disable forms; `IOBUFDS` and `OBUFTDS` are the ones used most. `SSTL18_I` is not offered either. |
| pulls on a pad with nothing else on it | `PULLUP`, `PULLDOWN`, `KEEPER` |
| IO logic | `ISERDESE2`, `OSERDESE2`, `IDELAYE2`, `IDELAYCTRL`, `IDDR_2CLK`, `DCIRESET`, and `IDDR` with a routed reset |
| configuration | `BSCANE2`, `CAPTUREE2`, `DNA_PORT`, `EFUSE_USR`, `FRAME_ECCE2`, `ICAPE2`, `STARTUPE2`, `USR_ACCESSE2` |

Building is the least that can be asked: a probe that builds has not been
compared with what Vivado wrote for it. Vivado's rules also say something Reticle
does not: the first run's refusals are design rules (`REQP-1884`, `REQP-1939`,
`PDCN-135` and so on), and Reticle has checks of its own but not these.

`reticle fpga --device <part> --decode <file.bit>` prints what any 7-series `.bit`
sets, and each set bit the database has no name for. It is how to ask what Vivado
wrote.

## What each makes of the same design

`testdata/fpga/vivado/qor/` has thirteen small designs, each with a clock at
200 MHz and two pins, chosen to need one feature each. Vivado's numbers are its
routed report, in `vivado.txt`. Run on 2026-11-04.

| design | tool | LUT | FF | CARRY | SRL | MUXF | LUTRAM | BRAM18 | DSP | ns | s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| adders | Vivado | 231 | 481 | 56 | 0 | 0 | 0 | 0 | 0 | 3.395 | 48 |
| | Reticle | 259 | 481 | 40 | 0 | 0 | 0 | 0 | 0 | 7.36 | 33 |
| barrel | Vivado | 103 | 70 | 0 | 0 | 0 | 0 | 0 | 0 | 3.979 | 48 |
| | Reticle | 89 | 70 | 0 | 0 | 0 | 0 | 0 | 0 | 5.12 | 9 |
| bram_fifo | Vivado | 49 | 68 | 3 | 0 | 0 | 0 | 2 | 0 | 4.612 | 50 |
| | Reticle | 57 | 68 | 9 | 0 | 0 | 0 | 2 | 0 | 5.07 | 9 |
| counter64 | Vivado | 23 | 129 | 22 | 0 | 0 | 0 | 0 | 0 | 3.975 | 48 |
| | Reticle | 98 | 129 | 16 | 0 | 0 | 0 | 0 | 0 | 8.40 | 38 |
| crc32 | Vivado | 129 | 65 | 0 | 0 | 0 | 0 | 0 | 0 | 3.581 | 54 |
| | Reticle | 162 | 65 | 0 | 0 | 0 | 0 | 0 | 0 | 5.23 | 9 |
| fir8 | Vivado | 7 | 23 | 0 | 0 | 0 | 0 | 0 | 8 | 10.988 | 49 |
| | Reticle | 1495 | 436 | 63 | 0 | 0 | 0 | 0 | 0 | 22.88 | 66 |
| fsm | Vivado | 33 | 30 | 0 | 0 | 1 | 0 | 0 | 0 | 3.217 | 48 |
| | Reticle | 62 | 30 | 0 | 0 | 0 | 0 | 0 | 0 | 5.23 | 8 |
| gray_cdc | Vivado | 15 | 37 | 3 | 12 | 0 | 0 | 0 | 0 | 2.255 | 46 |
| | Reticle | 26 | 61 | 3 | 0 | 0 | 0 | 0 | 0 | 2.85 | 8 |
| lutram32 | Vivado | 7 | 70 | 0 | 0 | 0 | 51 | 0 | 0 | 2.967 | 47 |
| | Reticle | 9 | 38 | 0 | 0 | 0 | 0 | 2 | 0 | 5.07 | 9 |
| mul18 | Vivado | 7 | 36 | 0 | 0 | 0 | 0 | 0 | 1 | 2.833 | 47 |
| | Reticle | 746 | 145 | 0 | 0 | 0 | 0 | 0 | 0 | 29.74 | 17 |
| regfile_alu | Vivado | 351 | 20 | 20 | 1 | 5 | 102 | 0 | 0 | 7.859 | 64 |
| | Reticle | 373 | 22 | 8 | 0 | 0 | 64 | 0 | 0 | 10.79 | 12 |
| srl_delay | Vivado | 7 | 34 | 0 | 2 | 0 | 0 | 0 | 0 | 2.739 | 46 |
| | Reticle | 9 | 2081 | 0 | 0 | 0 | 0 | 0 | 0 | 6.75 | 38 |
| widemux | Vivado | 275 | 1049 | 0 | 0 | 192 | 0 | 0 | 0 | 4.131 | 49 |
| | Reticle | 276 | 1047 | 0 | 0 | 192 | 0 | 0 | 0 | 5.41 | 37 |

Read `ns` with care: **the two columns are not the same measurement.** Vivado's
is the data path of its worst routed path in its own timer. Reticle's is its own
routed estimate, which is a model, and it is known to overstate a carry chain
(`a_carry_chain_is_timed_as_vivado_times_it`). This said "neither has been checked
against the other's implementation yet", which [the next section](#vivados-timer-on-reticles-implementation)
has since done. `s` is the whole run for each, from one machine
under other load, including Vivado's start-up; it is a record, and no test asserts
it. Reticle's LUT column is sites, with paired tables as two, and Vivado's is
cells, which leaves out the route-through a carry's propagate may use: so
`counter64`'s 98 against 23 overstates that gap.

What the table says, largest first:

1. **No hard multiplier.** `mul18` is 746 lookup tables against one `DSP48E1`.
   `docs/fpga-xray.md` has why the flow keeps them out, and it was written with
   no Vivado to ask.
2. **No shift register.** `srl_delay` is 2081 flip-flops against 34 and two
   `SRL`s.
3. **No wide multiplexer**, as this table first read: `widemux` was 596 lookup
   tables and no `MUXF` against 275 and 192, at 5.04 ns. It is 276 and 192 now,
   for the section below, and 5.41 ns. That is the estimate going the wrong way
   by 0.37 ns on the design this is for: its latest path spends 3.29 ns on one
   connection of a select read by 257 pins, in a placement with different
   neighbours, and the two runs are one placement each, so it says little either
   way. Vivado has that design's data path at 4.131 ns.
4. **A small memory goes to block RAM.** `lutram32` uses two `RAMB18E1` where
   Vivado uses 51 distributed RAM cells.
5. **`fsm`** is 62 against 33, and why has not been looked at: the Vivado runs are
   `-quiet`, so nothing here says whether it re-encoded the machine.
6. **Reticle takes as long as Vivado on carries and long chains.** `counter64`
   spends 33 of its 38 s in the placer on 98 lookup tables, where `barrel`, with
   about as many, spends 3 s.
   Part of that was a test that scanned a move for every site it asked about,
   for every cell of a chain's move: sorting a long move took it from 32 s to
   24 s with the same bitstream, byte for byte. Most of the rest is that nearly
   every move there is the whole chain, and the anneal's number of moves does
   not know that.

Where Reticle is ahead, it is on time alone: 8 or 9 s against 48 on most of these.

## Reading its constraints

`reticle fpga --constraints board.xdc` reads an XDC, chosen by the file's name.
`src/fpga/xdc.rs` does it in two steps with two different judges: a *reading*,
which says what the file says in Vivado's terms, is held to Vivado; a
*translation* into `Constraints`, which says what the flow does about it, has no
judge but the diagnostics it has to give.

**Checked against Vivado 2026.1:** each `.xdc` in `testdata/fpga/vivado/xdc/` was
read by `read_xdc` after `synth_design`, and `tools/vivado-parity/gen_xdc.py`
kept what Vivado then held (`get_property` of each port, clock, pblock and cell,
and its own `write_xdc` with each object resolved) as a `.answer` beside it.
`tests/fpga_xdc.rs` holds Reticle's reading to every line of those. It is the
reading alone, so a line agreeing there says the flow was told the same thing
Vivado was, and nothing about what either did next.

**Checked, what the answers changed.** These were my readings first, and Vivado
said otherwise:

- `get_ports a` is every bit of `a`, and so is a glob on the bus's own name
  (`h?`). `get_ports {a[1:0]}` is nothing at all, not two bits.
- `a[0]` is fine as a bare word, where Tcl would call a command named `0`.
  `{a\[0\]}` in braces matches nothing, and `a\[0\]` bare does.
- A comment's trailing backslash takes the next line with it, pin and all.
- `foreach`, `if`, `proc`, `source`, `puts`, `lindex`, `string`, `concat`,
  `format`, `incr`, `append` and `unset` are refused in a constraints file
  (`Designutils 20-1307`), and Vivado carries on. `set`, `list`, `expr`, `split`,
  `join`, `llength`, `filter` and `get_property` work. I had expected `lindex`.
- `PULLDOWN true` then `PULLUP false` still leaves a pull-down; `PULLUP true` after
  it clears it. There is one pull and two flags on it.
- `DRIVE` and `SLEW` on an input are ignored with a warning, not refused.
- A bare word after `-from` is searched for as a clock, then a cell, then a port,
  then a pin, so `-from clk` is the clock and not the port of that name.
- A `-from` on a register's `Q` or a hierarchical cell is refused as no valid
  startpoint, and `set_max_delay -datapath_only` needs a `-from`.
- `create_clock` on a source already holding a clock takes it away from the older
  clock without `-add`, whose other sources stay. A clock left with none is gone,
  and both tools warn of it.

**What is read, and what becomes of it:**

| XDC | Reticle |
|---|---|
| `PACKAGE_PIN`, `LOC` with a ball, `IOSTANDARD`, `DRIVE`, `SLEW`, `PULLUP`, `PULLDOWN`, `PULLTYPE PULLUP/PULLDOWN`, each also in `-dict` | applied |
| `get_ports` with lists, globs, `-filter`, `all_inputs`, `all_outputs`; a bit by the index it was declared with | applied: `hi[4]` of `[7:4]` is bit 0, `be[0]` of `[0:3]` is bit 3 |
| `create_clock` on a port or a net | applied; a net nothing drives is **refused**, since in a `.rcf` that asks for a PLL |
| `create_generated_clock` on a scalar register's `Q` | a clock at the divided period on that net: `-divide_by`, `-multiply_by`, `-edges`, `-invert` are worked out as Vivado worked them out |
| `create_generated_clock` on a clock manager's output | a note: the flow derives it from the instance's parameters, as Vivado derives it unasked, and the period written is not compared with that |
| `set_false_path`, `set_max_delay [-datapath_only]`, `set_multicycle_path -to` with one end | applied by the timing-driven placement, on a whole cell (a pin stands for its cell only when it is `C` or `D`) |
| the same with both ends, or `set_multicycle_path -from` alone | kept in `Constraints`, with a warning: the placement cannot tell the paths between two ends apart |
| `set_input_delay -clock` on an input, against a rising edge | applied |
| `set_clock_groups`, `set_false_path` between clocks, anything only `-hold` | a note: the placement times no path between two clocks, and has no hold analysis, so it holds already |
| pblocks of `SLICE_` ranges | a region of the tiles at its corners, from the fabric database, and its fabric is loaded; a pblock's cells by register or by instance. Several ranges become their bounding box, with a warning |
| `KEEP_HIERARCHY` | applied |

**Warned, because only timing or placement can differ:** `-through`, `set_min_delay`,
`set_output_delay`, an exception naming a clock other than across two, a virtual clock
and any delay against it, `-clock_fall`, `-add_delay` for a second clock, a
non-even `-waveform`, `-add`, a clock on a bit of a bus or of a register,
`set_case_analysis` and the other commands that only inform analysis, `LOC` on a
`SLICE`, `BEL`, `ASYNC_REG`, `DONT_TOUCH`, `CLOCK_DEDICATED_ROUTE`, `MAX_FANOUT`,
debug cores, `RAMB` and `DSP` and `CLOCKREGION` ranges of a pblock.

**Refused, as an error, because going on could be a wrong bitstream:** a pin or a
pad property that cannot be read or applies to nothing, a pin given to two ports,
any command that is not XDC (above), `DIFF_TERM`, `IN_TERM`, `IBUF_LOW_PWR`,
`PULLTYPE KEEPER` and a bank's `INTERNAL_VREF` (what this flow has no way to set), `LOC` on any
site but a slice, any other property of a pin or a cell not known to be harmless,
and the configuration options below. This is stricter than Vivado, which reports
each of these as a critical warning and carries on: a pin that failed to set in
Vivado stops at `write_bitstream`, and here there may be no such later stop.

**The configuration options were measured**, one at a time on a routed design
against its bitstream without, from the first sync word on:

| option | bytes that differ |
|---|---|
| `CFGBVS`, `CONFIG_VOLTAGE` | 0 (note: nothing to do) |
| `BITSTREAM.CONFIG.CONFIGRATE 33` | 5 |
| `BITSTREAM.GENERAL.CRC DISABLE` | 10 |
| `BITSTREAM.CONFIG.EXTMASTERCCLK_EN DIV-1` | 16 |
| `BITSTREAM.CONFIG.SPI_FALL_EDGE YES` | 390 |
| `BITSTREAM.CONFIG.SPI_BUSWIDTH 4`, `CONFIG_MODE SPIx4` | 474 |
| `BITSTREAM.GENERAL.COMPRESS TRUE` | the file is 346 588 bytes, not 3 825 788 |

Those are warned of: **that they only change how the part loads is my reading of
what they are, not something the measurement shows**, which says how many bytes
and not which. Everything else named `BITSTREAM.*` is refused, whether or not
the byte count looks small (`STARTUPCLK`, 5; `USERID`, 12; `READBACK.SECURITY`,
12; `USR_ACCESS`, 414; `UNUSEDPIN`, 324 or 620): a design reads some of those, and a
board depends on others. Refused, too, is a `BITSTREAM.*` Vivado does not have.

**Checked on real files.** Every design under `testdata/fpga/vivado/` with both
binds the same pins from its `.xdc` as from its `.rcf`, 161 designs, with no
warning, and one refusal: the two probes that set a bank's `INTERNAL_VREF`, which
this flow cannot and which is an error, so those two files do not build from their
`.xdc` as they stand. Two others (`fsm`, `ibufds_probe`) were built to a bitstream from each, and the
two are identical from the first sync word on, in an `#[ignore]`d test. `ibufds_probe` matters because of a
difference between the languages: an XDC names a differential pair's `P` ball
alone, its `.rcf` names both, and the flow gets the pair from the `P`.
A pblock was placed on the real fabric and held every flip-flop it named, inside
the tiles Vivado says its corners are in. NewAE's `cwhusky_100.xdc`, which is not
committed, was read once by hand against a stub of the Husky's ports: its 94 pins
are `examples/husky/husky.rcf`'s 94, with its `USR_ACCESS` line taken out, since
that line is refused. It gives 38 warnings for `set_output_delay` alone.

**Not checked:**

- Whether a design built from an XDC meets what the XDC asks. Nothing here has
  compared Vivado's slack with Reticle's under a translated exception.
- VHDL against Vivado. A range is recorded the same way, and
  `a_vhdl_range_is_kept` holds it against Reticle's own reading of Verilog's
  convention, not against an answer of Vivado's.
- A multi-dimensional packed port (`[1:0][3:0]`). It is not read as bits, so its
  constraints are refused as naming nothing.
- `get_cells` on a name Vivado's synthesis invents, such as `a_IBUF[0]_inst` or
  `mem_reg_0_15_0_0`: only registers, instances and nets of the source are known,
  and memories are not among them. The exception is `<port>_IBUF` and
  `<port>_IBUF_BUFG` of a scalar input, for `get_nets`. What names another is
  reported as naming nothing.
- `get_ports -regexp`, and so on for each query. There are no regular expressions
  here, so a query using one is unreadable: an error for a pin, a warning for
  timing.

**Not the same as Vivado, on purpose or not yet:** an unstated `IOSTANDARD` is
`LVCMOS18` in Vivado, which then refuses to write a bitstream without a stated one
(`NSTD-1`, quoted, not measured here), and `LVCMOS33` in the 7-series flow. A
`-rise_from`-style exception is dropped with a warning, as are the rest of the
edge-specific forms. `reticle timing --constraints` refuses an XDC with a message,
since its queries read the source and that command has synthesised it by then.

## Vivado's timer on Reticle's implementation

`reticle fpga --bitstream <f> --vivado-placement <x>.tcl` writes what Vivado needs
to load an implementation with no synthesis: a structural netlist, every cell's
site and bel and every lookup table's `LOCK_PINS`, and every route.
`tools/vivado-parity/run_placed.tcl` runs them, `decode_placed.sh` compares the
bits, and the answers are in `testdata/fpga/vivado/placed/`. Run on 2026-11-04 on
the thirteen designs of `qor/`. **Nothing here has been on a board.**

The netlist is not `--routed-netlist`'s, which `link_design` refuses;
`src/fpga/xray/vivado.rs` says how it differs, and why. Lookup tables are the
same cells: that their bits come out the same is checked below.

### What it says about placement and routes

Data path of the worst routed path of `clk` in ns, in Vivado's timer, at grade 1
as the table above. All but the last two columns of figures are Vivado's.

| design | Vivado's own flow | Reticle's netlist, Vivado places and routes | Reticle's placement, Vivado routes | Reticle's placement and routes | Reticle's estimate of that last endpoint | Reticle's own worst | that implementation at grade 3 |
|---|---|---|---|---|---|---|---|
| adders | 3.395 | 7.048 | 11.585 | 11.494 | 7.281 | 7.36 | 8.255 |
| barrel | 3.979 | 3.760 | 4.122 | 4.302 | 2.954 | 5.12 | 3.139 |
| bram_fifo | 4.612 | 4.699 | 5.829 | 5.947* | 4.053 | 5.07 | 4.331* |
| counter64 | 3.975 | 4.158 | 5.515 | 6.367 | 5.067 | 8.40 | 4.688 |
| crc32 | 3.581 | 5.544 | 7.783 | 9.114 | 4.736 | 5.23 | 6.590 |
| fir8 | 10.988 | 26.088 | 39.749 | 41.081 | 22.805 | 22.88 | 29.537 |
| fsm | 3.217 | 3.816 | 4.765 | 5.234 | 3.460 | 5.23 | 3.731 |
| gray_cdc | 2.255 | 2.262 | 2.150 | 2.107 | 2.314 | 2.85 | 1.582 |
| lutram32 | 2.967 | 4.588 | 5.408 | 5.408* | 4.530 | 5.07 | 3.985* |
| mul18 | 2.833 | 36.218 | 50.851 | 50.350 | 29.739 | 29.74 | 36.625 |
| regfile_alu | 7.859 | 10.052 | 16.512 | 16.803* | 10.790 | 10.79 | 12.260* |
| srl_delay | 2.739 | 3.409 | 3.898 | 4.491 | 2.989 | 6.75 | 3.211 |
| widemux | 4.131 | 4.780 | 6.254 | 7.346 | 4.655 | 5.04 | 5.437 |

\* Vivado could not finish these with Reticle's routes fixed; see below. Its timer
reports what was routed, so a path through what was not is not a measurement.

The columns differ by one thing at a time, so what each step costs can be read off
(medians over the thirteen):

- **The netlist:** Vivado's flow against Vivado on Reticle's netlist, 1.25 times.
  Mostly what the first table already said, up to 12.8 for `mul18`, which is
  746 lookup tables and no `DSP48E1`. Note that this column has no `opt_design`,
  and Vivado's own flow does.
- **The placement:** Vivado's placement against Reticle's on the same netlist, with
  Vivado routing both, **1.31 times** slower, from 0.95 on `gray_cdc` to 1.64 on
  `adders` and `regfile_alu`. This is the first answer this project has had to "is
  our placement any good?". Vivado's placer also chose its lookup tables' pins
  here, where Reticle's are locked to what its router chose.
- **The routes:** Vivado's routes against Reticle's on the same placement, **1.03
  times**, from 0.98 to 1.17. On these designs the router is the smallest of the
  three, and it is not where to look first.

### How far Reticle's estimate is

The estimate's `0.34 ns` out of a flip-flop is grade 3's; grade 1's is `0.518`,
which Vivado's report above says. **So the fair comparison is grade 3,** which is
what `xc7.dev` declares for this part, and which this document's opening sentence
and the first table are not at. On the same placement and the same routes, at grade
3, as estimate / Vivado:

| | median over the designs | lowest design | highest design |
|---|---|---|---|
| the endpoint Vivado says is worst | 0.93 | 0.72 (`crc32`) | 1.43 (`gray_cdc`) |
| every endpoint both time, each design's median | 1.47 | 1.13 (`crc32`) | 1.81 (`lutram32`) |
| every connection both time, each design's median | 0.93 | 0.77 (`crc32`) | 2.14 (`srl_delay`) |

**The estimate is right about the critical path largely because two errors cancel.**
A typical endpoint is overstated by half. A connection on its own ranges from 0.04 to
7.8 times Vivado's across these designs, and short fan-out-of-one wires are the ones
overstated, `srl_delay`'s by 2.1. Then a long path, which is mostly wire, comes out
near or under what Vivado says, and `crc32`'s worst is 4.74 against 6.59.

This matters more than the figure: **the placer steers by this model,** and its worst
path is not Vivado's worst path. On `fsm` its own worst, 5.23 ns, is a path to a pad
with no constraint on it, which Vivado does not time at all, and Vivado's worst is one
it puts at 3.46. `a_typical_connection_is_estimated_within_a_stated_band_of_vivados_timer`
pins the state of it, and `the_estimate_of_the_worst_endpoint_is_near_vivados` is the
target, ignored, that fails on `crc32` and `gray_cdc` at 1.35 either way.

Why the figures are what they are has not been looked at beyond this. That `0.3 ns` a
cell against grade 3's lookup table is part of it is a reading of one report, not a
measurement over them.

### Vivado's routing graph took every route

**Every route Reticle made, in all thirteen designs, is accepted as a `FIXED_ROUTE`:
8,966 nets and none refused.** Another 1,240 stay inside a site and have no route to
give. The database's wire names are Vivado's verbatim, and a wire's node is Vivado's
to say, so the script asks it (`get_nodes`). `_MUX_` and `_CARRY_` wires are Reticle's
own, inside a slice, and are left out. `route_design -preserve` then routes what
Reticle does not think of as a net, the constants, and finishes ten designs of the
thirteen with no net unrouted.

The first attempt had 4 of 99 refused on `fsm`, and it was the writer: Vivado reads a
branch that opens with another branch as having left the wrong node.

Vivado could not finish three, for two reasons:

- **`bram_fifo` and `lutram32`: Reticle's signal is on `WEBWE[7:4]`.** Vivado grounds
  those pins of a `RAMB18E1`, in `TDP` where the library says they are not read, and
  says of Reticle's fixed route `conflicts with pin GNDTerm of net GNDNet`. On
  `lutram32` that is `INT_R_X51Y97 FAN_ALT1`, carrying `in[36]` as a hop, which
  reaches `BRAM_RAMB18_WEBWE4` by a wire with no switch on it. So on the part, that
  pin reads that signal. `bram::tie_policy` already says this happens and accepts it,
  on the library's word and nextpnr's, **quoted**: this confirms that it occurs and
  that Vivado would not do it, and does not measure what it does. Of `bram_fifo`'s two,
  one is `BRAM_FIFO18_WEBWE7` likewise, and the other, `INT_L_X52Y94 BYP_ALT7`, was
  not followed further. A first try left a block RAM's unread inputs undriven in the
  netlist, and changed nothing, since these pins are not in the netlist at all.
- **`regfile_alu`: a `RAM64X1D`'s write enable.** Reticle brings it to the slice on
  `CLBLM_M_WE`, with `WEMUX.CE` clear. Vivado connects the same pin through `CE`, with
  `WEMUX.CE` set, on all 64 of them, and will not use the other: asked to with
  `SITE_PIPS {WEMUX:WE}`, it says `Element WEMUX is not routable`. So the net is left
  partly routed, and with `FAN7` taken by Reticle's other signals, Vivado has no way
  through either. The database offers both, and neither reading has been measured on a
  part: a RAM of Reticle's still rests on `xray::lutram`'s quoted account.

Vivado will not write a bitstream for those three ("`RTSTAT-2` may not change
severity"), and that is left as its answer.

### The same placement and routes, the same bits

For the ten that finished, Vivado's `write_bitstream` and Reticle's, both read by
`reticle fpga --decode`. Together they name **184,663 features of Reticle's, and
184,645 of them are in Vivado's**: every lookup table's `INIT`, every slice mode and
every routing pip, on every design. The 18 that are not are all class 1 below.
**No bit of Reticle's was found wrong.** In `bits.txt`, by class:

| | class | count | designs | what it is | answered? |
|---|---|---|---|---|---|
| 1 | Reticle only: `CLK_BUFG_REBUF GCLK<n>_ENABLE_*` on rebuffers with no live segment | 18 | 10 | Deliberate and documented at `XrayFabric::enable_global_clocks`: Reticle sets both enables in each rebuffer of a used column, nextpnr's rule, and Vivado only the ends of each live segment | Quoted, harmless on nextpnr's word |
| 2 | Vivado only: `CLK_BUFG_REBUF_X78Y142 GCLK0_ENABLE_ABOVE` | 4 | `fir8`, `fsm`, `mul18`, `srl_delay` | **The likeliest wrong bit here.** The far end of the segment the clock row at `Y130` hangs from. Reticle would set it too, but the rule stops at the loaded region and this tile is outside it | **Open.** Nothing measured says the far end is needed, and it was not changed, since loading more of the column changes placement |
| 3 | Vivado only: `HCLK_CMT*_CK_BUFHCLK<n>_USED` | 23 | 10 | Set in a clock-management tile a horizontal clock row passes through, with no clock manager in use. Reticle has no such rule | **Open** |
| 4 | Vivado only: `LIOB33_X0Y93 IOB_Y0` as an input with a pull-up | 30 | 10, and all 13 with `cells.txt` | That site is `IOB_X0Y94`, `IO_L3P_T0_DQS_PUDC_B_14` (`package_pins.csv`): Vivado's treatment of the `PUDC_B` pad, whichever design it is. Nothing here connects it | Named, and whether it matters is open |
| 5 | Vivado only: `<n>.GND_WIRE`, `BYP_ALT<n>.GFAN1` | 13 | `fir8` | A ground to a `CARRY4`'s `DX`, on seven carries. **Checked:** on all seven, bit 3's `S` and `DI` are constants and its `O` and `CO` have no reader, so nothing sees it. Vivado obeys the netlist and Reticle saves the wire | Harmless |
| | Vivado only, not features: the low 13 bits of word 50 of each frame | 861 to 8,552 a design | all | **Checked:** on all 23 of Vivado's bitstreams, in all 9464 frames, that is the Hamming code of the frame (`frame_ecc.py`). Reticle's are zero. `--decode` counts them apart now, as `frame_ecc`, which also leaves nothing unnamed in the database's own Basys 3 harness bitstream, a thing `tests/fpga_xray.rs` had taken for what a real configuration needs. Whether a part ever reads them under a plain load is not measured, and Reticle's own bitstreams have loaded with them clear (`docs/fpga-xray.md`) | Named |

A pin tied high, like a fractured table's `A6`, shows no
difference at all, since an unset `IMUX` already says `VCC_WIRE`. That is a limit of
this comparison and not a third agreement: Vivado and Reticle cannot differ there
in a way this can see.

`cells.txt` asks a weaker question of all thirteen, including the three above:
Reticle's bitstream against Vivado's on Reticle's placement with Vivado's routes,
leaving out routing. Of 140,546 cell features on Reticle's side, 140,522 are in Vivado's. What differs:

| class | count | what it is | answered? |
|---|---|---|---|
| Reticle only: `ZINV_RST*`, `ZINV_REGCLK*` on a block RAM | 24 | Reticle holds each reset at zero with a ground and the inverter off; Vivado connects `VCC`, which **`lutram32.pips` shows**, and leaves the inverter on. The same value if `ZINV` means what its name says, which is quoted | Harmless on that reading |
| Vivado only: `ZALMOST_EMPTY_OFFSET`, `ZALMOST_FULL_OFFSET` | 52 | Its default for a FIFO's flags, on a block RAM that is no FIFO | Harmless on UG473's word |
| Vivado only: `WEMUX.CE` | 64 | `regfile_alu`, as above | **Open** until a RAM is on a part |
| Vivado only: `<L>OUTMUX.O6` | 213 | A way for its route to leave a slice. It is in none of `bits.txt`, so it belongs to Vivado's routes and not to the cells | Not a difference in the cells |
| Vivado only: `CASCOUT_ARD_ACTIVE` | 1 | Likewise, on another block RAM's tile in `bram_fifo` | Not looked at further |
| Vivado only: the `PUDC_B` pad | 39 | Class 4 | As above |

**What this does not check.** That Vivado's bitstream of the same implementation holds
the same bits says nothing about whether it does what the design says; that is
`every_block_maps_to_the_logic_it_was_mapped_from`. It says the physical facts Reticle
had only quoted agree with Vivado's for the cells here: `LUT6_2`s, a `CARRY4`'s modes,
`FDRE`s, `IBUF`, `OBUF`, `BUFG`, the routes and a `RAMB18E1`'s `INIT` and modes. A
`RAM64X1D` and that `RAMB18E1` have had that only from `cells.txt`. Any primitive these
designs do not use has had nothing from it.

**Checked by breaking it.** A lookup table's written table wrong in one entry shows
as 169 differing `INIT` features on `fsm`, so the comparison can see a wrong bit. The
pins locked wrongly, Vivado refuses (`Cannot use A6 pin`). Six mutations of the writer
and of `--decode`'s range each failed a test in `tests/fpga_xray_vivado.rs` or
`xray::vivado`.

**Not done.** The Vivado numbers are of one placement each, Reticle's first seed. The
fixtures quote a run: nothing fails when the placement moves on, and only
`run_placed.sh` asks again. No design here has a clock manager, a pad register, a
latch or a `DSP48E1` of Reticle's, so those have not been through this. Whether this
gives the same bits on another part has not been asked. `SITE_PIPS` was tried for one
slice's `WEMUX` and not pursued. And the Vivado `.bit`s that were decoded here have
never been loaded, and must not be: all ran with `NSTD-1` and `UCIO-1` lowered to
warnings, for that purpose alone.

## Wide multiplexers

A slice has a `MUXF7` over each two of its lookup tables and a `MUXF8` over the
two `MUXF7`s. Reticle mapped nothing to them until this section, so a 64-to-1
was 37 tables where Vivado has 17.

`testdata/fpga/vivado/muxf/` has nineteen small designs, each asking one thing.
`tools/vivado-parity/muxf.sh` runs them through Vivado, and `vivado.txt` there
keeps its own report of where each cell went and its bitstream as decoded by
`XrayDatabase::decode`.

**What Vivado wrote, and a reading that would have been wrong.** All of this is
from those decodes and reports, and each is checked against the build by
`tests/fpga_xray_muxf.rs`:

- `F7AMUX` has `I0` from `B6LUT` and `I1` from `A6LUT`; `F7BMUX` has `I0` from
  `D6LUT` and `I1` from `C6LUT`; `F8MUX` has `I0` from `F7BMUX` and `I1` from
  `F7AMUX`. **The higher letter is `I0`.** Guessing from the lanes' order gives
  the other, and that bitstream would decode perfectly and choose the wrong input
  every time.
- The selects are `AX`, `CX` and `BX`, and nothing else is set for them.
- Neither multiplexer has a bit of its own. Only its way out does:
  `AFFMUX.F7` or `AOUTMUX.F7`, `CFFMUX.F7` or `COUTMUX.F7`, and `BFFMUX.F8` or
  `BOUTMUX.F8`.
- A `MUXF7` or `MUXF8` read by one flip-flop has it in the same slice. Read by
  anything else as well (`f7_d`), the flip-flop was elsewhere and the output
  went out by the output mux.
- A table read by two `MUXF7`s is copied (`f7_shared`), so that wire is one
  multiplexer's.

| design | Vivado | Reticle | same slices? |
|---|---|---|---|
| `f7_ff`, `f7_out`, `f7_two`, `f7_d`, `f8_ff`, `f8_out` | 2 to 6 tables, the cells named | the same count, the cells named | **yes, exactly** |
| `mux8` | 2 tables, 1 `MUXF7` | the same | **yes** |
| `mux16` | 4, 2, 1 `MUXF8` | the same | **yes** |
| `mux64` | 17, 8, 4 `MUXF8` | the same | **yes** |
| `f7_fanout` | 4 tables, 1 `MUXF7` | 5, 1 | **yes**; the fifth is an inverter the mapper already wrote for a table read in both polarities, and was not looked into |
| `mux32` | 9 tables, 4 `MUXF7` | 9, 4, 2 `MUXF8` | no: both cover it, this one a level differently |
| `f7_from_ff` | a `LUT3` | one table | no cell to compare |
| `f7_pair` | 6 tables, 1 `MUXF7` | the same | not comparable: Vivado leaves other contents in a `LUT3`'s unread half, and these are compared by a truth table's ones |
| `tie` | 4 tables, no `MUXF7`, written as logic | the same | no multiplexer to compare: Vivado did not take one where it saves nothing either |
| `f7_half` | 1 table, `MUXF7`, the flip-flop through a route-through table | 2 tables | no; two table sites either way |
| `f7_shared` | the table copied, 5 tables, 2 `MUXF7` | 5 tables, 1 `MUXF7` | no; the same count |
| `f7_carry` | 12 tables, a `MUXF7` on a carry's propagate table | 12, no `MUXF7` | no; the same count |
| `f8_from_ff` | `MUXF8`, four route-through tables, selects grounded | one table | no; fewer sites |
| `f8_half` | 2 tables, `MUXF7`, `MUXF8`, route-throughs | 3 tables, `MUXF7` | no |

"Same slices" is a line for each multiplexer: the ones of the truth table on each
data input, which says which table has which input however its pins are
permuted, whether the output is registered or leaves, and whether the select's
bypass input is reached. Which of a slice's two `MUXF7`s holds a lone one is left
out, since Vivado uses either. Interconnect is compared by those ends and no
further; the routes are the router's.

**How it gets there.** Mapping offers a two-to-one choice between two tables as
a `MUXF7` that costs no table, and between two `MUXF7`s as a `MUXF8`, beside each
node's cuts (`synth::techmap`'s docs). Offered over the optimised graph that
found nearly nothing: rewriting takes a multiplexer tree apart, and `mux8` came
to 4 tables three levels deep. So a tree of at least three full levels is lifted
out before optimising and put back after (`synth::aig::muxtree`), and with that
the covers above come out as Vivado's without anything aiming at them.

A named `MUXF7` is kept as that cell, not made logic, unless a slice could not
hold it (`techcells::legalise_wide_muxes`): one whose data is not two tables of
its own becomes a table, which costs what Vivado's route-through does or less.

The placer has no rule for any of this. Each data pin is the same wire as the
output it reads, as a carry's propagate is, so the macros come out of what it
already reads off the routing graph (`place::build_clusters`), and the select
competes for its bypass input in the pools that wire was already in.

**Timing.** Vivado's reports at grade 1 have a `MUXF7` at 0.212 to 0.247 ns from
data and 0.296 from `S`, and a `MUXF8` at 0.098 to 0.104. `xray::timing` takes
0.247 and 0.104, for every input alike, so a path from a `MUXF7`'s select is
0.05 ns short. Into either from its data is a connection of no delay.
`a_wide_multiplexer_is_timed_with_vivados_figures` holds those; the rest of a
path's estimate is no closer to Vivado's for this.

**Checked, on each of those nineteen designs, by the flow and by the test:** every
set bit decodes, none unexplained; the arcs selected are the arcs the router
chose; the mapping is proved; and a simulation of the routed netlist answers as
the RTL does over 3,000 cycles. **Nothing here has been on a board**, and a
slice configured this way rests on Vivado's having written the same bits.

`every_block_maps_to_the_logic_it_was_mapped_from` now proves each block at each
width both with and without wide multiplexers, and fails if none is chosen at
either width. A wide network is proved against the graph it was mapped from and
that graph against the one from before it was optimised: one miter of the two
ends left 11,904 AND nodes on the Husky Plus, and the two separately finish.

**What a design with nothing to gain gets.** The network is mapped both ways and
the wide one kept only when it has fewer tables, so such a design is the same
to the bit: `a_design_that_saves_nothing_is_not_moved`. And the sites and wires
this adds to every slice do not move a placement either. Measured against the
commit before, on the thirteen designs of the table above: twelve are the same
bitstream, byte for byte past the header, and the thirteenth is `widemux`.

**The Husky Plus does save something, and is not given it.** Mapped with them,
its 26,684 tables are 25,220, with 843 `MUXF7` and 226 `MUXF8`: 1,464 tables
fewer, 5.5 %, at the same depth of 18, proved. That is a different netlist and so
a different placement, and what a placement of this design does can only be told
on a board, which is in use elsewhere. So `examples/husky/build.sh` and
`build_plus.sh` pass `--wide-muxes off`, which is the build measured at 39 of 40.

Measured on the Plus's whole flow at seed 4, from one checkout of NewAE's sources
(not checked to be the commit that board run used):

| | commit before | `--wide-muxes off` | on |
|---|---|---|---|
| bitstream, past the header | | **the same, byte for byte** | another |
| set bits; unexplained | 2,498,313; 0 | the same | 2,561,298; 0 |
| arcs selected are the router's | yes | yes | yes |
| route passes to converge | 58 | 58 | 78 |
| pips | 2,988,562 | the same | 3,339,068 |
| `clk_usb` estimate of 10.4 ns | 16.55 | the same | 16.55 |
| `ADC_clk_fb` estimate of 4 ns | 6.74 | the same | 7.92 |
| whole run, beside other work | 50 min | 50 min | 61 min |

So on this one placement, fewer tables cost more wire and a slower ADC clock, by
the estimate. One seed is not a comparison of the two, and the estimate is not the
part, whose seeds 1 to 5 ranged from 34 to 39 tests. It is a reason to want a
board before taking the flag out, and no more than that.

**Mutations.** 24 were made to this work and every one failed a test: each pin of
each multiplexer, each way out, the tag, the polarity on the way in, a tree put
back wrong, a table claimed twice, a flip-flop joined wrongly, the timing
figures, and the choice between networks. Three got past the tests first written.
A select on another lane's bypass input hid behind a flip-flop reading the one it
should have reached; `a_slices_wide_multiplexers_are_where_vivado_has_them` is
why that fails now. `lutpack`'s refusal to pair a data table survived a design
where nothing had to leave by `O5`, which `f7_pair` now does. And a tie going to the
wide network survived until `a_wide_network_with_no_fewer_tables_is_not_kept`.

**Not done.** Only the `.dev` file's own sites hold them: a family with no such
line maps as it did. `MUXF7` over a flip-flop's output by route-through, as
Vivado does for one named, is not offered. Which multiplexers to use is decided
by the whole design's table count, so a design where keeping trees loses more
elsewhere than the multiplexers save gets none; a design was not found that does.
A wide multiplexer in a `LUT6_2`'s lane is refused, not considered, so that a
data table never shares its site. `regfile_alu` and `fsm`, where Vivado uses 5 and
1, gain nothing here: this flow's tables do not have the tree shape, and why was
not looked at.
