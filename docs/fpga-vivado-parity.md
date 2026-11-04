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
| | Reticle | 596 | 1047 | 0 | 0 | 0 | 0 | 0 | 0 | 5.04 | 30 |

Read `ns` with care: **the two columns are not the same measurement.** Vivado's
is the data path of its worst routed path in its own timer. Reticle's is its own
routed estimate, which is a model, and it is known to overstate a carry chain
(`a_carry_chain_is_timed_as_vivado_times_it`). Neither has been checked against
the other's implementation yet. `s` is the whole run for each, from one machine
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
3. **No wide multiplexer.** `widemux` is 596 lookup tables against 275 and 192
   `MUXF`s.
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
