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

**Reticle builds 87 of the 141.** `index.txt` says which;
`the_ledger_says_what_builds` holds it exact (ignored, since it takes every probe
to a bitstream), and `the_primitives_the_ledger_calls_ok_map` runs in the gate.

**The first version of this section was wrong**, and said Vivado built all 146.
The runner passed `-quiet` to `write_bitstream`, which hid the refusals: 24 probes
had no `.bit` at all and the table called them built. Decoding one is what showed
it. The runner now fails a step with no file behind it, and of those 24, 19 were
illegal probes, a `DSP48E1`'s `PCOUT` taken to the fabric or an `ODDR` to a
flip-flop, which now obey Vivado's rules.

The 54 that do not build, by what stops them:

| what | primitives |
|---|---|
| no latch on the fabric yet | `LDCE`, `LDPE`, `LD`, `LDC`, `LDP`, `LDCE_1`, `LDPE_1` |
| block RAM by name | `RAMB18E1`, `RAMB36E1`, `FIFO18E1`, `FIFO36E1` |
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
sets, each set bit the database has no name for, and each frame whose ECC is wrong. It is how to ask what Vivado
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
| | Reticle, then | 1495 | 436 | 63 | 0 | 0 | 0 | 0 | 0 | 22.88 | 66 |
| | Reticle | 9 | 113 | 0 | 0 | 0 | 0 | 0 | 8 | 12.64 | 10 |
| fsm | Vivado | 33 | 30 | 0 | 0 | 1 | 0 | 0 | 0 | 3.217 | 48 |
| | Reticle | 62 | 30 | 0 | 0 | 0 | 0 | 0 | 0 | 5.23 | 8 |
| gray_cdc | Vivado | 15 | 37 | 3 | 12 | 0 | 0 | 0 | 0 | 2.255 | 46 |
| | Reticle | 26 | 61 | 3 | 0 | 0 | 0 | 0 | 0 | 2.85 | 8 |
| lutram32 | Vivado | 7 | 70 | 0 | 0 | 0 | 51 | 0 | 0 | 2.967 | 47 |
| | Reticle | 9 | 38 | 0 | 0 | 0 | 0 | 2 | 0 | 5.07 | 9 |
| mul18 | Vivado | 7 | 36 | 0 | 0 | 0 | 0 | 0 | 1 | 2.833 | 47 |
| | Reticle, then | 746 | 145 | 0 | 0 | 0 | 0 | 0 | 0 | 29.74 | 17 |
| | Reticle | 9 | 37 | 0 | 0 | 0 | 0 | 0 | 1 | 5.35 | 10 |
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

1. ~~**No hard multiplier.** `mul18` is 746 lookup tables against one `DSP48E1`.
   `docs/fpga-xray.md` has why the flow keeps them out, and it was written with
   no Vivado to ask.~~ Closed on 2026-11-04: *The hard multiply*, below. The rows
   marked "then" are what this item was written from.
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

**Two of the classes below have since been closed, and the figures are older than
that.** `testdata/fpga/vivado/placed/` was measured when Reticle left a frame's ECC
at zero and left the `PUDC_B` pad alone. Reticle now writes both, as "The container
around the frames" below says, so those two "Vivado only" classes describe that run
and not the flow today. The run has not been repeated since.

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

## The container around the frames, and `write_bitstream`'s options

**What was asked of Vivado:** one small design, `testdata/fpga/vivado/bitstream/options_probe.v`,
routed once, then `write_bitstream` from that checkpoint for every value of every
`BITSTREAM.*` property `list_property` names: 277 runs on the XC7A100T and, for the
ones that set frame bits and the three stream forms, 55 on the XC7A35T.
`bitstream_base.tcl`, `bitstream_options.tcl` and `bitstream_fixtures.py` under
`tools/vivado-parity/` made it. **What is held:** the whole stream after the sync
word, every word of it, frames, ECC and CRCs included, by length and hash, in
`tests/fpga_xray_bitstream.rs`. Reticle writes the same stream, from Vivado's
frames, for 245 runs on the XC7A100T and 50 on the XC7A35T, and 11 more with a
pad in them when the database is there to say which bits a pad's are.

**Nothing here has been on a board.** A stream Vivado would write has not thereby
been loaded by a part: not one option below has configured anything, and the Husky
is in use elsewhere. Before any of this Reticle's stream already differed from Vivado's default only in its
frames, and on 2026-09-24 one had configured a Basys 3.

### Frame ECC

**Word 50 of each frame holds a 13-bit ECC, and Reticle now writes it.** This
changes every 7-series bitstream Reticle writes, in every frame that holds anything:
102 to 273 frames on the small designs below. `xc7::frame_ecc` is the code:

- It is a Hamming code over the frame's other 3219 bits. A set bit contributes
  `word * 32 + bit + k`, where `k` is `0x320` for words 0 to 6, `0x340` for 7 to 37
  and `0x360` from 38, which steps over `0x400` and `0x800`: the check bits' own
  positions. The XOR of those, low 12 bits, is bits 11:0.
- Bit 12 is the parity of the data bits and of those twelve.
- The high 19 bits of word 50 are ordinary data, and count. They are the horizontal
  clock tiles' (`HCLK_*`, `CLK_HROW_*`, `HCLK_CMT`, `HCLK_IOI3`), which is why the
  earlier survey in `docs/fpga-xray.md` found ECC bits in those tiles. No `segbits`
  line of any tile names one of the 13, checked across every tile with a window on
  word 50.

**Checked:** equal on all 59,536 frames of eight `.bit`s, of which 7,775 hold anything
and 3.4 million bits are set, on two parts: this run's small designs, two generated
ones with random lookup tables and block RAMs full of random data
(`gen_ecc_design.py`), and two of Project X-Ray's reference bitstreams, which an
older Vivado wrote. Whole frames of the two generated ones are committed in
`ecc_frames.txt`, and `the_frame_ecc_is_exact_on_vivados_frames` holds them to it. The
count of eight was by a script and is not in a test; the committed sample is.

**The first formula was wrong, and came from memory:** I started from my recollection
of Project X-Ray's `ecc.cc`, which matched bits 11:0 and missed bit 12 on half the
frames. The fold of the parity was misremembered. So the code here is what Vivado's
frames say, and the recollection is why I knew to look for this shape.

**The second thing wrong was assuming it covers every frame.** A block RAM frame's
13 bits are **zero**, whatever it holds: 1,858 frames with data, and not one with an
ECC. `bram_fifo`, with empty memories, passed either way; it took a design with
contents to show it. That contents change at run time is the likely reason, and it
is a guess. So `xc7::wanted_ecc` is by frame address.

`reticle fpga --decode` no longer lists these bits as `?`. It counts them, and each
frame whose ECC is wrong is a `!` line and a number in the summary: a `.bit` from this flow before today has one of those for nearly every used frame. On Vivado's own
frames for `options_probe`: **2,418 bits, 606 of them ECC, none unexplained, no ECC
wrong**, and 2,244, 437, none and none on the XC7A35T.

`FRAME_ECCE2` and `POST_CRC` could now mean something; neither is done, and neither
has been tried. `FRAME_ECCE2` is still in the list of primitives above that do not
build.

**What else this changed, checked.** Four designs built by the code before this and
after it, same placement and route (`options_probe`, `crc32`, `bram_fifo`, `fsm`): the
packet lines differ only in the `FDRI` hash and the first CRC, and the frames only in
those 13 bits, on 102 to 273 frames, and the nine bits of the `PUDC_B` pad below. By a
script, and not in a test: the old build is not to hand in one.

The random designs say something too: **1.76 million set bits on the XC7A100T and 1.61
million on the XC7A35T decode with none unexplained**, in each of the plain, compressed
and per-frame forms, to the same list of features. Their CRCs all read back. That is Project X-Ray's database against what a random design uses, not against every
  feature of the part.

### The stream

`bit_packets.py` dumps one. `<tag>.options.txt` starts with Vivado's for the base
design, and every run after it says what differs.

**On default options there is no difference in the stream at all**: every register
Project X-Ray's reference bitstream had shown, and they are still what Vivado 2026.1
writes. `the_default_stream_is_vivados_word_for_word` holds it, CRCs included, so the
order of `TIMER`, `WBSTAR`, `CMD`, reserved register 19, `COR0`, `COR1`, `IDCODE`,
`CTL0`, `CTL1`, `FAR` and the end sequence has been measured against this Vivado and
not only an older one.

Three differences remain in the file, and none is in the stream a part reads:

| what | Vivado | Reticle | why |
|---|---|---|---|
| `.bit` field `a` | `<top>;UserID=0XFFFFFFFF;Version=2026.1;SW_CRC=...;SW_HDR_SHA256=...` | `<top>;UserID=0XFFFFFFFF;Version=reticle` | Not the same tool. The last two differ per file written, are not a CRC-32 or SHA-256 of the stream, the header, or any part of them I tried, and are not in the stream. Not derived |
| `.bit` fields `c`, `d` | the clock | `0000/00/00`, `00:00:00` | Two runs of a design give the same bytes here |
| `.rbt` header | `Created by Bitstream 2026.1 SW Build ...` and the clock | `Created by Reticle`, the same fixed date | The same two reasons. The body is the same bits |

And one difference that **is** in the frames, on every design: **Vivado configures the
`PUDC_B` pad whether the design uses it or not**, as an input with a pull-up
(`IN_ONLY`, `LVCMOS25_LVCMOS33_LVTTL.IN`, `PULLTYPE.PULLUP`, nine bits). It came out of
`--decode` naming a pad nothing had asked for. `XrayDatabase::finish_frames` now does
the same, unless the design has used that pad. So this too changes every Reticle
bitstream. Its effect on a part has not been measured.

**The rule of the tail.** Vivado keeps a file's length: anything an option adds before
the frames comes out of the 400 no-operations at the end, 5 for a `BSPI`, 2 for an
`AXSS`, 2 for `SPI_FALL_EDGE` and 64 for `SPI_32BIT_ADDR`'s longer lead-in.

**Forms of the frames**, read from the streams and exact on each:

- `COMPRESS`: `CTL1`'s bit 12, and a run of frames by `FDRI` up to the first with a
  duplicate in its row, which then goes to itself and each duplicate by `MFWR`. Only
  frames of one row are duplicates of each other, though a run can cross rows, over
  the two pad frames. A run that stops at a written frame has a zero frame after it,
  which is never committed. Eight no-operations follow each `MFWR` on the block RAM
  bus, none on the other. **Checked** exact on five streams, of which two are committed
  and three, `bram_fifo` and the two random designs, were checked with a script: a row
  full of random block RAM is what showed the row crossing, and it is hundreds of
  kilobytes of frames. `a_compressed_run_crosses_a_row_over_its_pads` pins that by hand
  from the rule, so it would not catch the rule being wrong in a way those three did
  not reach. My first reading did not cross rows, and matched exactly on every stream
  until the first random one.
- `PERFRAMECRC`: `CTL1`'s bit 21, and each frame followed by `FAR` of itself and a CRC;
  a row ends with a zero frame and its own. No CRC before the end sequence, and `CTL1`
  cleared before `GRESTORE`.
- `DEBUGBITSTREAM`: a type 1 `FDRI` per frame, each followed by `LOUT` of its address,
  and pad frames as **bare zero words with no packet**. Vivado's CRC counts those as
  `FDRI` writes and leaves `LOUT` out. `read_bit` reads it so: both are odd enough that
  no engine's behaviour should be inferred from them.

`Bit::image` replays any of these into the memory it leaves, so `--decode` reads them,
and a compressed stream from here is checked by replay as well as by hash. It first
chose a form by comparing the `FDRI` word count with the layout's, which is wrong: a
small compressed stream can match by chance, and the row-crossing test's did.

### Options

Given as `reticle fpga --bitstream-option NAME=VALUE`, with Vivado's names.
`xc7::options::option_table()` is the list and `BitstreamOptions::set(name, value)` is
the one call: that is what reading `set_property BITSTREAM.*` from an XDC comes to.

**Done, and exact against Vivado's stream for every value tried:**

| where it lands | options |
|---|---|
| `COR0` | `STARTUP.GWE_CYCLE`, `GTS_CYCLE`, `LCK_CYCLE`, `MATCH_CYCLE`, `DONE_CYCLE`, `DONEPIPE`, `STARTUPCLK` (`CCLK`, `JTAGCLK`), `CONFIG.CONFIGRATE` |
| `COR1` | `BPI_PAGE_SIZE`, `BPI_1ST_READ_CYCLE`, `READBACK.XADCPARTIALRECONFIG` |
| `CTL0` | `INITSIGNALSERROR`, `OVERTEMPPOWERDOWN`, `SELECTMAPABORT`, `CONFIGFALLBACK`, `READBACK.SECURITY`, `ENCRYPTION.ENCRYPTKEYSELECT` |
| `WBSTAR`, `CMD` | `NEXT_CONFIG_ADDR`, `NEXT_CONFIG_REBOOT`, `REVISIONSELECT`, `REVISIONSELECT_TRISTATE`. An address alone writes `IPROG` and enables the fallback |
| `TIMER` | `TIMER_CFG`, `TIMER_USR`. One count: with two, Vivado writes `TIMER_CFG`'s and says nothing |
| `AXSS` | `USR_ACCESS`. `TIMESTAMP` is Vivado's packed form, and UTC here where Vivado's is local: the standard library has no zone. It is the one thing here that reads a clock |
| `BSPI`, `CMD` | `SPI_BUSWIDTH`, `SPI_32BIT_ADDR`, `SPI_FALL_EDGE`, `BPI_SYNC_MODE`, `CONFIG_MODE` |
| stream | `GENERAL.COMPRESS`, `PERFRAMECRC`, `DEBUGBITSTREAM`, `CRC` |
| `CFG_CENTER_MID` frames 26, 27 | `USERID` (bits taking turns between the frames, inverted, so the default all-ones is no bits), `M0PIN`, `M1PIN`, `M2PIN`, `TCKPIN`, `TDIPIN`, `TDOPIN`, `TMSPIN`, `CCLKPIN`, `DONEPIN`, `INITPIN`, `PROGPIN`, `DISABLE_JTAG` |
| `MONITOR_BOT` frames 26 to 29 | `JTAG_XADC`, `XADCENHANCEDLINEARITY`, `XADCPOWERDOWN` |
| pads | `UNUSEDPIN`, on every pad of the die, bonded or not, 300 on the XC7A100T and 250 on the XC7A35T. `EXTMASTERCCLK_EN` also takes `CONFIGRATE`'s field, and configures the `EMCCLK` pad as `PUDC_B`'s |

The frame bits are stated by tile, and the same bits come out on both parts, which is
what says they are the tile's. `--decode` names them, as `USERID[7]=0` and
`M0PIN.PULLNONE`, since no `segbits` file does.

**Accepted, and no effect on what was measured**: `CCLK_TRISTATE`, `DCIUPDATEMODE`,
`READBACK.ACTIVERECONFIG`, `CFGBVS`, `CONFIG_VOLTAGE`. A part with DCI might see
`DCIUPDATEMODE`; this one has none.

**Refused here, with why:**

| option | why |
|---|---|
| `ENCRYPTION.ENCRYPT`, `KEY0`, `HKEY`, `KEYFILE`, `STARTCBC` | No AES-256 or HMAC, and a stream that claimed to be encrypted and was not would be worse than a refusal |
| `CONFIG.PERSIST=YES` | Vivado refuses it without `CONFIG_MODE` (`PRST-1`), and with one configures every pin of that interface. 21 to 239 words on what was measured. No table of those pins here |
| `READBACK.ICAP_SELECT=TOP`, `BOTTOM` | Vivado refuses it without an `ICAPE2` there, and Reticle places none |
| `STARTUP.STARTUPCLK=USERCLK` | Vivado refuses it without a `STARTUPE2` clock, and Reticle places none |

**Vivado's own refusals, which `BitstreamOptions::check` makes too**: `COMPRESS` with
`PERFRAMECRC` (`Designutils 20-2046`) or `DEBUGBITSTREAM` (`20-1859`), `CRC=DISABLE` with
`PERFRAMECRC` (`20-2047`), `SPI_BUSWIDTH=NONE` with `SPI_32BIT_ADDR=YES` (`20-1850`), an
SPI `CONFIG_MODE` against a different `SPI_BUSWIDTH` (`20-1856`), and `CONFIG_VOLTAGE`
without `CFGBVS` (`CFGBVS-2`) or 1.5 and 1.8 without `GND` (`CFGBVS-4`).

**Names that do not exist.** `STARTUP.DRIVEDONE`, `CONFIG.DONEPROG` and
`CONFIG.OVERTEMPSHUTDOWN` were on my list, from older tools: Vivado 2026.1 says
`Cannot set property ... because the property does not exist`, on this family. The
last is `OVERTEMPPOWERDOWN` now. Reticle refuses them the same way, and so a
constraint file that carries them has something to say about it.

**Side files.** `--bin-file`: the stream as it stands, the same bytes as Vivado's
`.bin` holds of Vivado's `.bit`. `--raw-bitfile`: the same as ones and zeros under a
header. `--logic-location-file`: where each slice flip-flop and each `RAMB18E1` bit
reads back, with positions held to Vivado's own for all 32 flip-flop bels of four
slices (frame 31 of the tile) and a sample of 864 block RAM bits, numbered as half of
the `RAMB36` a `RAMB18` is in. It is not all of Vivado's file: an IO register and a
lookup table used as memory have lines there and none here, none of them measured, and
net names are this flow's own.

**Not done:**

- **`-mask_file`.** The `.msk` is the same packets around a mask of each frame's words
  in place of its data, a 1 for a bit readback should not compare. It depends on the design (2,639,948 ones for `bram_fifo` and 2,621,276 for
  `options_probe`) and has a shape by tile and frame I have not broken down: reading it
  is the next step, and it has not been done.
- `-readback_file` (`.rba`, `.rbb`, `.rbd`, `.msd`), `-reference_bitfile`, `-cell`:
  not looked at beyond seeing that the first writes those four files.
## The hard multiply

Everything below is on Vivado's word and Reticle's own decode. **Nothing here has
been on a board**, on any part, and that a bit means what Vivado writes it for is
the most it can say.

### What was asked

`testdata/fpga/vivado/dsp/` holds Vivado's answers to four questions, each from a
script in `tools/vivado-parity/`:

| file | question | script |
|---|---|---|
| `sweep.txt` | what each parameter sets in the tile | `gen_dsp_sweep.py`, `dsp_sweep_report.py` |
| `inference.txt` | what it makes of each multiply | `gen_dsp_inference.py`, `dsp_facts.tcl`, `dsp_inference_report.py` |
| `answers/`, with the sources in `designs/` | what it wrote for each block, feature by feature and each input by what it is on | `dsp_answers.py` |
| `vectors/` | what the vendor's model answers, in 52 settings | `gen_dsp_vectors.py` |

The database's `USE_MULT` gap was the old reason for the refusal. **It was the
wrong reading**, and `src/fpga/xray/dsp.rs` keeps it: `MULTIPLY` and `DYNAMIC` set no
bit, and `NONE` sets one the database has no name for. Reticle names that one and
three others in the same position (`SEL_PATTERN_C`, `A1_UNUSED`, `B1_UNUSED`), each
measured on both blocks and both tile types, so a block in a Vivado `.bit` now
decodes with nothing unnamed in its tile.

A constant on an input takes one of three shapes (`src/fpga/xray/dsp.rs` has the
table): a mux of the pin's own for 39 of them, the pin's inverter for `ALUMODE[1:0]`,
`OPMODE[5:0]` and `CARRYIN`, and a route for the rest. **A zero on `OPMODE[1]` is not
a route at all**, which nothing in the database says.

### What was compared, and how it came out

- **Every block against Vivado's.** 170 designs, 69 naming the block and 101 inferring
  it, each built by Reticle and compared with what Vivado wrote for the same source:
  every site feature, and each input by whether it is a zero, a one or a signal.
  (Interconnect by its ends: which wires a signal takes is the router's own.) **153
  are the same**, and the other 17 are named below with why, and checked still to
  differ. `the_blocks_of_every_design_are_vivados` runs them, ignored for its 10
  minutes; six are in the gate.
- **Every bit decodes.** All 170 decode with 0 bits unexplained, and the arcs they
  select are exactly the router's and the ties', `reticle fpga` refusing otherwise.
- **Vivado's bitstream of Reticle's implementation.** `--vivado-placement` hands
  `mul18` and `fir8` to Vivado as placed and routed, which accepts them with 0 errors
  and 0 critical warnings from `link_design` to `write_bitstream`. Decoded,
  **no feature of any `DSP_L` tile differs.** `mul18` has 1073 of its 1075 features in
  common, the rest clock tiles and a pad, as in every design of the section above;
  `fir8` 3311 of 3314, with two `CTRL` wires taking their ground from `GFAN0` here and
  `GFAN1` there.
- **Vivado's timer on those routes.** `fir8`'s worst path through Reticle's placement
  is 10.988 ns, the same five blocks' cascade and the same figure as Vivado's own
  implementation, since none of it is routing. Reticle's estimate says 12.64: it adds
  the launch and setup Vivado's data path does not. `mul18` is 3.762 ns against
  Vivado's own 2.833, with Reticle's estimate 5.35 on another endpoint, its output
  pad.

### What the block does

`dsp48e1_model.v` is written here from UG479's diagrams and holds none of the
vendor's model. `tests/fpga_dsp_vivado.rs` holds it to the 52 settings of `vectors/`;
its header lists what it does not cover. The first run agreed in 51, and the other
was the model's carry out, which it cannot state when `X` and `Y` are both non-zero;
that setting no longer asks about carries. With it, every multiply is checked
against its own source as a mapped netlist, cycle by cycle: 194 register settings
with the result on ports, 78 sums of products, 21 designs with a register a block
cannot hold, and about 40 of Vivado's designs by their parity pin.

**Those found real faults.** Three stages after a product with an enable took the
wrong two. The block's clock stayed off the global network, with every bit of the
block still matching Vivado's, because that comparison did not look at the clock's
route; it does now. And they found that `reticle sim` reads a signed memory's
element as unsigned (`reg signed [15:0] t [0:1]`, `t[0] * 5` with `t[0] = -3` gives
327665), so `fir8`'s source simulates to other answers than `xsim` gives. That is
not fixed here. `fir8` is held to `xsim`'s answer, `answers/fir8.xsim`.

`xsim_routed.py` runs what `--routed-netlist` wrote under `xsim` with the vendor's
own models of every cell, against the source. 98 designs and 599 cycles each, all the same: that is
the vendor's `DSP48E1`, `FDRE`, `LUT6` and `CARRY4` running what the bitstream was
made from.

### The delays

Read from `report_timing` at speed grade 1, a design for each register setting; in
`src/fpga/xray/timing.rs`.

| from | to | registers on | ns |
|---|---|---|---|
| `A` | `P` | none | 3.841 |
| `B` | `P` | none | 3.656 |
| `CLK` | `P` | `A2`/`B2` only | 4.009 |
| `CLK` | `P` | `M` | 2.215 to 2.305 |
| `CLK` | `P` | `P` | 0.434 |
| `CLK` | `PCOUT` | `M` | 2.411 |
| `PCIN` | `PCOUT` | none | 1.713 |
| `A` | setup at `A2` | | 0.362 |
| `B` | setup at `B2` | | 0.450 |
| `A` | setup at `M` | | 1.928 |
| `B` | setup at `M` | | 1.743 |
| `A` | setup at `P` | | 3.722 |
| `B` | setup at `P` | | 3.536 |
| `PCIN` | setup at `P` | | 1.400 |

A quoted figure and not a check: nothing has compared what Reticle's estimate says
of a routed path through a block with what Vivado's timer does.

### Where it is not what Vivado does

Each is the same function at a cost, or registers somewhere else. Each is named by
`NOT_THE_SAME` in `tests/fpga_dsp_vivado.rs`, so closing one has to say so.

- **`fir8`'s shift register stays in the fabric.** Vivado passes it down the column
  through `ACOUT`/`BCOUT` and is left with 23 flip-flops against 113.
- **An operand one bit past a port** (`s_26x18`, `s_25x19`, `u_18x18`) is one block
  there, with the odd bit's partial product on `C` under a dynamic `OPMODE`, and two
  here.
- **A split multiply's registers.** Vivado takes each block's `P`; this takes `M`,
  and only the one stage after the product.
- **The pre-adder is not inferred.** `(a + d) * b` is one block there (`f_preadd`),
  and three and a carry chain here.
- **A product read twice** is two blocks there, joined by `PCOUT`, and one and a
  carry chain here (`f_two`).
- **A subtraction** stays in the fabric here; `ALUMODE` could do it.
- **An asynchronous reset stays out of the block.** Vivado takes `P` with its reset
  at zero and drops the register between, in a way this has not understood (`c_arst`).
- **A square** is `AREG` 1, `M` and `P` there, `AREG` 2 and `P` here: one latency.

`mul18` is 37 flip-flops against 36: its shift register's top bit, read only by the
register now in the block, is left. That is the flow's clean-up working a word at a
time and not this block's. Nine lookup tables against seven is the parity tree's.

### Inference is on, and what that did to the Husky

`--bitstream` on the 7 series now infers. **The Husky Plus and the plain Husky use no
`DSP48E1`**, measured: their one multiply is a `16x2` that stays in logic, and their
netlists through the `--bitstream` path are byte for byte what they were
(`--netlist`, against the binary of the commit before), so the default needs no
option to protect them. The fabric itself did change, by 9,360 pips nothing routes through unless a block
asks: **the whole Husky Plus build, seed 4, was the same bitstream byte for byte**,
3,825,897 bytes built by this work and by the commit it began from, `98081d3`, before
any later change to master. Neither has been loaded into anything, and frame ECC, on
master since, changes every bitstream this flow writes.

**The registers that absorb** copy Vivado's choices for the designs asked about.
That these are its rules in general is a reading of those designs, not a statement
of its.

### Mutations

A test that passes against a broken implementation is worse than none, so each of
these was applied and a test had to fail:

All 33 were caught. Three survived the first run, a block's kept register, the
operand order and the second block's unnamed bits; each now has a test, and the table
is after them. `tests/fpga_dsp_vivado.rs` and `tests/fpga_dsp.rs` say what each would
not catch.

| mutation | caught by |
|---|---|
| model: PCIN shifted logically | `the_model_answers_as_the_vendors_does` |
| model: A1 read in place of A2 | `the_model_answers_as_the_vendors_does` |
| split: top piece zero-extended | `what_one_block_cannot_take_goes_over_several` |
| split: no shift on the cascade | `what_one_block_cannot_take_goes_over_several` |
| split: a column at the wrong place | `what_one_block_cannot_take_goes_over_several` |
| absorb: A1 and A2 enables swapped | `every_register_setting_computes_the_source` |
| absorb: main path enables swapped | `every_register_setting_computes_the_source` |
| absorb: reset polarity ignored | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| absorb: async reset taken in | `every_register_setting_computes_the_source` |
| absorb: reset to one taken in | `every_register_setting_computes_the_source` |
| absorb: falling edge taken in | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| absorb: resets on A not required alike | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| absorb: clock not compared | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| absorb: read-elsewhere product taken | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| absorb: third stage with controls moved back | `every_register_setting_computes_the_source` |
| fold: narrow product extended regardless | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| fold: a term read elsewhere taken | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| fold: two clocks in a sum | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| fold: C upper half | `every_sum_of_products_computes_the_source` |
| fold: wrong Z on a chain | `every_sum_of_products_computes_the_source` |
| map: constant low bits wrong sign | `every_mapped_design_computes_the_source` |
| xray: inverter sense flipped | `each_parameter_sets_what_the_sweep_measured` |
| xray: zero on an inverting pin routed | `a_constant_takes_the_shape_its_pin_has` |
| xray: USE_MULT_NONE at the wrong bit | `the_blocks_of_a_few_designs_are_vivados` |
| xray: a mux wire wrong | `the_pin_table_is_the_databases` |
| xray: ground and power swapped | `the_blocks_of_a_few_designs_are_vivados` |
| timing: launch of M | `a_blocks_paths_are_the_ones_its_registers_make` |
| map: wider operand on b | `the_wider_operand_goes_on_the_wider_port` |
| xray: DSP_1 stride wrong | `the_unnamed_bits_are_where_the_sweep_found_them` |
| timing: cascade not followed | `a_chain_of_sums_is_one_long_path` |
| clock: a block off the network | `the_blocks_of_a_few_designs_are_vivados` |
| xray: a kept register | `what_a_block_cannot_hold_stays_out_and_is_still_right` |
| device: smallest moved | `small_multiplies_stay_in_logic_as_vivado_leaves_them` |
