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

**Not checked:** most of these become logic, and what that costs is in the table
below. `MUXF7`, `MUXF8` and their `_L` and `_D` forms, the latches and the shift
registers do not; each has a section of its own. Nor do the distributed RAMs Vivado
puts on a `SLICEM` ([Distributed RAM](#distributed-ram)), with `--memories legacy`
the exception to that.

## Which primitives build

`testdata/fpga/vivado/primitives/` has one design per 7-series primitive, 146 of
them. Vivado writes a bitstream for **141**, and refuses five on this part:
`IOBUF_DCIEN`, `OBUFT_DCIEN`, `IOBUFDS_DCIEN` and `IOBUFDS_DIFF_OUT_DCIEN`, since
the part's banks are all high-range and have no DCI, and `ZHOLD_DELAY`, which it
would not place alone from a pad or from logic.

**Reticle builds all 141**, 132 before the `DIFF_OUT` buffers and the input delays,
130 before the section on FIFOs, 122 before the section on the configuration
primitives, 120 before the section on serialisers, 118
before the section on block RAM, 94 before the sections on pads and clock
buffers, 106 before the one on IO registers, 110 before regional clock buffers
and clock managers. `index.txt` says which;
`the_ledger_says_what_builds` holds it exact (ignored, since it takes every probe
to a bitstream), and `the_primitives_the_ledger_calls_ok_map` runs in the gate.

**The first version of this section was wrong**, and said Vivado built all 146.
The runner passed `-quiet` to `write_bitstream`, which hid the refusals: 24 probes
had no `.bit` at all and the table called them built. Decoding one is what showed
it. The runner now fails a step with no file behind it, and of those 24, 19 were
illegal probes, a `DSP48E1`'s `PCOUT` taken to the fabric or an `ODDR` to a
flip-flop, which now obey Vivado's rules.

None is left that Vivado builds and Reticle does not. What stopped each, until
its section: the eight configuration primitives, the two serialisers, the two
FIFOs, and the six `DIFF_OUT` pad buffers with `IDELAYE2`, `IDELAYCTRL` and
`DCIRESET` were each a row of a table here that listed what did not build.

Building is the least that can be asked: a probe that builds has not been
compared with what Vivado wrote for it. Vivado's rules also say something Reticle
does not: the first run's refusals are design rules (`REQP-1884`, `REQP-1939`,
`PDCN-135` and so on), and Reticle has checks of its own but not these.

`reticle fpga --device <part> --decode <file.bit>` prints what any 7-series `.bit`
sets, each set bit the database has no name for, and each frame whose ECC is wrong. It is how to ask what Vivado
wrote.

## A latch, against what Vivado wrote for it

`$dlatch` and the twelve latch primitives become the slice's own latch, on a
bel of their own beside each of the four main flip-flops. This was done by asking:
`testdata/fpga/vivado/slice_cells/` holds a design per form, 26 of them with a
latch, `tools/vivado-parity/gen_slice_cells.py` wrote them, and `vivado.features`
is what each of Vivado's bitstreams decodes to through `reticle fpga --decode`.

**Checked: all 26 agree** (`every_latch_is_what_vivado_wrote`, ignored because it
builds them all; four in the gate): the slice-wide bits (`LATCH`, `CLKINV`,
`FFSYNC`, `CEUSEDMUX`, `SRUSEDMUX`), each element's (`ZINI`, `ZRST`, its data mux),
and which of `CLK` and `SR` the interconnect drives, with the lane written `L`
because the tools place differently. Each design but one is also simulated
beside its netlist after routing, over 800 known outputs: all agree, 0 bits
unexplained in every image, and the arcs chosen are the arcs decoded. The one is
`infer_latch_en`, whose gate is a lookup table in both tools' results and closes a
step after its data moves in a simulation with no delays. **None of it has been on a
board.**

What asking found, none of which the database says and one of which the reading
got wrong:

- **`CLKINV` is the other way round from a flip-flop.** A latch open while `G` is
  high sets it; `LD_1` and `IS_G_INVERTED` clear it. The reading from
  `sites::bel_config`'s table, that the bit means "the `_1` primitives", would have
  built every latch with its gate inverted, and it would have placed, routed and
  decoded with nothing to say so. The first thing written here was that table
  extended; the comparison is what stopped it.
- **Vivado has two latches, not twelve.** `LD`, `LDC`, `LDE` and every `_1` come back
  from its synthesis as `LDCE` with pins tied or `IS_G_INVERTED` set, and `LDP` as
  `LDPE`. `fpga::retarget` does the same, so the behavioural models in
  `xc7_cells.v`, which would put a lookup table on a gate, are for simulation.
- **An inferred latch has a clear if its source does.** `if (r) l = 0; else if (g)
  l = d;` is `LDCE` with `CLR = r`, not a table on the gate; `if (!g)` is
  `IS_G_INVERTED`, not an inverter. `techcells::shape_latches` reads those while the
  cells are there. `if (a && b)` is a table in Vivado's too.
- **`IS_CLR_INVERTED = 1` does not build**: Vivado's placer says "could not place
  all instances". There is no such inverter in a slice. Reticle refuses it by name.
- **The 5FF slots.** Vivado refuses a latch on `A5FF` by name ("bel type (FF_INIT) do
  not match the cell type (LDCE)") and refuses a flip-flop in a slice with one
  ("Cannot set LOC property"), so there are four latch bels a slice and
  `place::slice_mode` keeps the two apart. `infer_latch_wide`'s ten latches take
  three slices in both.
- A latch's gate goes on the global network only when the net is a clock of
  something else (`ldce_gclk`, `CLK0.GCLK_B11`): a gate does not ask for a buffer.

Not checked and not done: a path through an open latch is not timed. A latch is
timed as a flip-flop on `G`, so a design relying on time borrowing is timed as if it
could not, which is on the safe side for setup and says nothing of hold.

## A shift register, against what Vivado wrote for it

`SRLC32E`, `SRL16E`, `SRLC16E` and what Vivado makes of the others are a `SLICEM`'s
own, and `fpga::srl` makes a chain of flip-flops into them as Vivado's synthesis
does. Same method, same fixtures: 58 designs in `slice_cells/` are about them, by
name or inferred, and `xray::srl` is the account of each bit.

**Checked:** `every_shift_register_is_what_vivado_wrote` builds 55 of them. Fifteen
agree in full: bits, contents, the address pins driven, and which of `<L>I`, `<L>X`
and the next lane each reads, the cascades inside a slice among them. The other 40 are
the five-input half and chains, and
agree in what they hold, what drives their addresses, how many shift registers and
how many flip-flops there are. All 55's routed netlists simulate as their sources do,
and each of the rest is said below. Nineteen are in the gate. **None of it has been on
a board.**

**Checked differently:** Vivado was also given Reticle's own placement and routes of
twenty of these, latches and shift registers, and of `qor/srl_delay` and
`gray_cdc` (`run_placed.sh`, stage `routes`), which is a second opinion on every one of
these bits: it took them all, and its bitstream of each is Reticle's feature for feature
but for the clock tiles this document already has, and one thing below. Vivado's timer
has `srl_delay` at 3.243 ns on Reticle's implementation, against its own 2.739 and this
tool's estimate of 4.90. `regfile_alu` it would not finish, 33 pins, in the distributed
RAM's routes and not the one shift register's. Those runs are not committed.

**What that found, and what is left:** a frame whose shift register holds a nonzero
`INIT` has ECC `0x0000` in Vivado's bitstream, and `xc7::frame_ecc` says `0x0003`
and Reticle writes that: `srlc32e` has four such frames, `srlc16e` four,
`srlc32e_cascade4` two. So the ECC of a frame leaves that table's contents out, which
reads as Vivado not counting bits the design changes as it runs. Reticle's frames in
those cases have an ECC Vivado's do not. Nothing reads it unless a design asks
(`FRAME_ECCE2`, readback), this flow does not build that primitive, and it is **not
fixed**: which bits are masked needs asking of Vivado across a distributed RAM too.

The tests' own limit is worth saying. Of 34 mutations of the two features, 28 were
caught at once and six were not, and each of the six showed something: the contents of
a sixteen-bit form moved to the lower half passed, because the comparison took whichever
half held them, and it would have built one reading zeros through an `A6` that reads
one; a chain that did not cascade, and a run of five, passed because no test had a chain
that long or asked; a `Q31` allowed off the `A` lane passed because the placer starts at
`A`; a latch beside a flip-flop passed because no build happened to try; and a constant
enable not counting as a signal passed against its test's first version, which had the
two pins on one wire where a slice has them on two. All six are now caught.

What asking found:

- **The address is `A[i]` on `<L>(i + 2)`, and every stored bit is written twice**:
  `32'hF00FA5C3` is `INIT = FF0000FFCC33F00F`. A sixteen-bit form on the six-input
  table has `SMALL`, the upper half, and its data on **`<L>X`**, not `<L>I`.
  `lutram.rs`, the precedent the work started from, has its address from `<L>1` and
  its enable on `WE`: neither holds here.
- **The enable is on `CE`, with `WEMUX.CE` set and `CEUSEDMUX` clear**, ahead of the
  mux a flip-flop's goes through, so a flip-flop with no enable may sit beside it.
  Tied high it is unrouted and reads one, which is a claim on that wire: the placer
  counts it as a signal of its own (`a_shift_register_with_its_enable_tied_keeps_the_wire_to_itself`)
  and the router is kept off it.
- **Inside a slice `Q31` runs `D` to `C` to `B` to `A`**, into the next lane's `DI1` with
  `DI1MUX` clear and no bit of its own, and a run is cut into fours from its end:
  `srlc32e_cascade5` is one in a slice of its own and four in the next.
  `place::srl_cascades` holds those to their lanes as a macro.
- **`Q31` leaves a slice from the `A` lane only, on `DMUX`**, and **the database's
  `DOUTMUX.MC31` is wrong**: it says `30_10 !30_51 30_52 !30_56 30_57`, and `30_10` is
  `A5FFMUX.IN_B`. Vivado sets the other two and not that one, in all six bitstreams
  that use it, and those were the only slice bits in 85 designs the database could
  not name. `xray::srl::correct_features` takes it out.
- **What a chain is.** One clock, one enable, no set or reset, and every stage but
  the last read by nothing else. The last stage stays a flip-flop, unless what reads
  it is a flip-flop of another enable or edge, which then registers the shift
  register. A stage reading a pad stays too, so `pure3` is three flip-flops and `pure4`
  one, a two-bit `SRL16E` and one. Vivado honours `ASYNC_REG`, `shreg_extract`,
  `keep` and `dont_touch`. A flip-flop a placement constraint names stays one here too,
  which was not asked of Vivado: a pblock over `sr_reg[*]` means those flip-flops.
- **A shift register reads `CLKINV` the way a flip-flop does, and a latch does not.**

What the tools do differently, each pinned by `where_a_shift_register_differs_from_vivado`:

- A chain's rest of up to 16 bits behind a cascade: Vivado's is an `SRL16E` on `A5LUT`,
  whose `DI1` reads `MC31`, and this one's is an `SRLC32E`, since the six-input
  sixteen-bit form reads `<L>X`.
- Vivado's sixteen-bit form it can register goes on `A5LUT`, with `A5FF` beside it,
  and it can hold two to a table. Here it is always the six-input one, which is also one
  of Vivado's own answers.
- `SRLC32E` read at a fixed address under 16 stays one; Vivado narrows it.
- `srl_style = "register"` is honoured here. Vivado 2026.1 made `chain8_srl_reg` a
  shift register all the same.
- `mem_shift` and `mem_shift16`, a chain read at an address that moves: flip-flops and
  a multiplexer here, an `SRLC32E` with its `A` wired in Vivado's.

**What it does to a design that has none.** Nothing: the sites and the wires are not
declared unless the netlist holds one, and eight of the thirteen in `qor/` are the same
bitstream byte for byte as before this. **What it does to one that has some is a
different placement.** NewAE's Husky Plus has 16 chains, 4080 flip-flops that would be
128 `SRLC32E`, 37 flip-flops nothing reads and 3 copies. Whether that is better on the
part cannot be said from here, and the board is in use, so `examples/husky/build.sh` and
`build_plus.sh` pass `--srl-min 0`, which is the netlist from before; with it all
thirteen of `qor/` are byte for byte what they were. Taking it away is a new build to
measure. **Neither has been built end to end since this.**

## What each makes of the same design

`testdata/fpga/vivado/qor/` has thirteen small designs, each with a clock at
200 MHz and two pins, chosen to need one feature each. Vivado's numbers are its
routed report, in `vivado.txt`. Run on 2026-11-04, with shift registers.

| design | tool | LUT | FF | CARRY | SRL | MUXF | LUTRAM | BRAM18 | DSP | ns | s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| adders | Vivado | 231 | 481 | 56 | 0 | 0 | 0 | 0 | 0 | 3.395 | 48 |
| | Reticle | 259 | 481 | 40 | 0 | 0 | 0 | 0 | 0 | 7.36 | 33 |
| barrel | Vivado | 103 | 70 | 0 | 0 | 0 | 0 | 0 | 0 | 3.979 | 48 |
| | Reticle | 89 | 70 | 0 | 0 | 0 | 0 | 0 | 0 | 5.12 | 9 |
| bram_fifo | Vivado | 49 | 68 | 3 | 0 | 0 | 0 | 2 | 0 | 4.612 | 50 |
| | Reticle | 57 | 68 | 9 | 0 | 0 | 0 | 2 | 0 | 5.06 | 12 |
| counter64 | Vivado | 23 | 129 | 22 | 0 | 0 | 0 | 0 | 0 | 3.975 | 48 |
| | Reticle | 98 | 129 | 16 | 0 | 0 | 0 | 0 | 0 | 8.40 | 38 |
| crc32 | Vivado | 129 | 65 | 0 | 0 | 0 | 0 | 0 | 0 | 3.581 | 54 |
| | Reticle | 162 | 65 | 0 | 0 | 0 | 0 | 0 | 0 | 5.23 | 9 |
| fir8 | Vivado | 7 | 23 | 0 | 0 | 0 | 0 | 0 | 8 | 10.988 | 49 |
| | Reticle, then | 1495 | 436 | 63 | 0 | 0 | 0 | 0 | 0 | 22.88 | 66 |
| | Reticle | 9 | 23 | 0 | 0 | 0 | 0 | 0 | 8 | 12.64 | 10 |
| fsm | Vivado | 33 | 30 | 0 | 0 | 1 | 0 | 0 | 0 | 3.217 | 48 |
| | Reticle | 62 | 30 | 0 | 0 | 0 | 0 | 0 | 0 | 5.23 | 8 |
| gray_cdc | Vivado | 15 | 37 | 3 | 12 | 0 | 0 | 0 | 0 | 2.255 | 46 |
| | Reticle | 26 | 37 | 3 | 12 | 0 | 0 | 0 | 0 | 2.68 | 9 |
| lutram32 | Vivado | 7 | 70 | 0 | 0 | 0 | 51 | 0 | 0 | 2.967 | 47 |
| | Reticle | 10 | 70 | 0 | 0 | 0 | 51 | 0 | 0 | 4.95 | 11 |
| mul18 | Vivado | 7 | 36 | 0 | 0 | 0 | 0 | 0 | 1 | 2.833 | 47 |
| | Reticle, then | 746 | 145 | 0 | 0 | 0 | 0 | 0 | 0 | 29.74 | 17 |
| | Reticle | 9 | 36 | 0 | 0 | 0 | 0 | 0 | 1 | 5.24 | 10 |
| regfile_alu | Vivado | 351 | 20 | 20 | 1 | 5 | 102 | 0 | 0 | 7.859 | 64 |
| | Reticle | 373 | 20 | 8 | 1 | 0 | 102 | 0 | 0 | 10.38 | 16 |
| srl_delay | Vivado | 7 | 34 | 0 | 2 | 0 | 0 | 0 | 0 | 2.739 | 46 |
| | Reticle | 9 | 34 | 0 | 2 | 0 | 0 | 0 | 0 | 4.90 | 9 |
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

Before shift registers, from the commit before them on the same day, where it differs:
`gray_cdc` 61 flip-flops and no `SRL`, 2.85 ns; `srl_delay` 2081 flip-flops and none,
6.75 ns, 43 s; `regfile_alu` 22 and none, 10.79 ns; `fir8` 436 flip-flops, 22.88 ns;
`mul18` 145 flip-flops, 29.74 ns. The other eight are the same bitstream, byte for byte.
`srl_delay`, `gray_cdc` and `regfile_alu` now have Vivado's flip-flops and Vivado's
shift registers, exactly. `fir8` and `mul18` are slower by a little on this tool's own
estimate with fewer flip-flops to place; a different placement, not a measured effect of
anything. The table under "Vivado's timer on Reticle's implementation" was measured
before all of this and is left as it was.

What the table says, largest first:

1. ~~**No hard multiplier.** `mul18` is 746 lookup tables against one `DSP48E1`.
   `docs/fpga-xray.md` has why the flow keeps them out, and it was written with
   no Vivado to ask.~~ Closed on 2026-11-04: *The hard multiply*, below. The rows
   marked "then" are what this item was written from.
2. **No wide multiplexer**, as this table first read: `widemux` was 596 lookup
   tables and no `MUXF` against 275 and 192, at 5.04 ns. It is 276 and 192 now,
   for the section below, and 5.41 ns. That is the estimate going the wrong way
   by 0.37 ns on the design this is for: its latest path spends 3.29 ns on one
   connection of a select read by 257 pins, in a placement with different
   neighbours, and the two runs are one placement each, so it says little either
   way. Vivado has that design's data path at 4.131 ns.
3. **A small memory went to block RAM.** `lutram32` used two `RAMB18E1` where
   Vivado uses 51 distributed RAM cells, and `regfile_alu` 64 `RAM64X1D` where
   Vivado's are 102. Both are Vivado's cells now
   ([Distributed RAM](#distributed-ram)); the rows above say so, and the LUTRAM
   column counts as Vivado's report does, a macro and its tables both.
4. **`fsm`** is 62 against 33, and why has not been looked at: the Vivado runs are
   `-quiet`, so nothing here says whether it re-encoded the machine.
5. **Reticle takes as long as Vivado on carries and long chains.** `counter64`
   spends 33 of its 38 s in the placer on 98 lookup tables, where `barrel`, with
   about as many, spends 3 s.
   Part of that was a test that scanned a move for every site it asked about,
   for every cell of a chain's move: sorting a long move took it from 32 s to
   24 s with the same bitstream, byte for byte. Most of the rest is that nearly
   every move there is the whole chain, and the anneal's number of moves does
   not know that.

Where Reticle is ahead, it is on time alone: 8 or 9 s against 48 on most of these.

## Pads: IO standards and what they take

`testdata/fpga/vivado/io/` has 282 designs from `tools/vivado-parity/gen_io.py`. Asked
of Vivado 2026.1 on the XC7A100T in FTG256, whose banks 14, 15, 34 and 35 are all
high-range. **Nothing here has been on a board**: a standard's bits are held to
Vivado's, and whether a pad then meets that standard's levels has not been measured
by anything.

**What Vivado offers.** `list_property_value IOSTANDARD` gives 37 for a port: `LVTTL`,
`LVCMOS33`/`25`/`18`/`15`/`12`, `PCI33_3`, `MOBILE_DDR`, `HSUL_12`, `HSTL_I`, `HSTL_II`,
`HSTL_I_18`, `HSTL_II_18`, `SSTL18_I`, `SSTL18_II`, `SSTL15`, `SSTL15_R`, `SSTL135`,
`SSTL135_R`, a `DIFF_` form of each of the last eleven and of `MOBILE_DDR`, and
`BLVDS_25`, `LVDS_25`, `RSDS_25`, `MINI_LVDS_25`, `PPDS_25`, `TMDS_33`. `xc7::pads` has
them; it is all of them and no other.

**Checked on 243 designs, 2,306 pads of the sweep and what the cases add:** every
standard in every direction Vivado allows, with `DRIVE`, `SLEW`, `PULLTYPE`, `IN_TERM`,
`DIFF_TERM` and `IBUF_LOW_PWR`, on both balls of a pair. Each was built by Vivado, its
`.bit` decoded (`vivado.txt`, 715 different pads), and `every_recipe_is_what_vivado_wrote` holds
`xray::io`'s recipe to every line with no database. `every_design_builds_to_vivados_bits`
(ignored, half a minute at 32 at a time) builds each from its `.xdc` and compares the
site features of every pad, its IO logic, and its bank, whole-design. **All agree.**
Interconnect is not compared: the placements differ.

### The bits

Project X-Ray fuzzed six kinds of standard, and `--decode` lists what Vivado sets for
the others as `?`. A pad is its features' bits, so what was missing was named
(`io::measured_features`), one group of bits that came and went together. **The names
are mine.**

| | what Vivado wrote |
|---|---|
| drive | The database's fields: `LVCMOS15.DRIVE.I8` is what `HSTL_I` and `SSTL15_R` write, `LVCMOS18.DRIVE.I16` `HSTL_I_18`'s, `LVCMOS25.DRIVE.I16` `SSTL18_II`'s and `BLVDS_25`'s. A standard with no choice writes one thing whatever `DRIVE` says. New bits: `HSTL_II`, `HSTL_II_18`, `PCI33_3`, `RSDS_25` and `PPDS_25` (the same), `MINI_LVDS_25` |
| `LVTTL` at 12 mA | `DRIVE.I12_I8`, which is `LVCMOS33` at 8. `LVTTL` at 16 is `LVCMOS33` at 12 |
| `SLEW FAST` | The clear field on `LVCMOS*`, `LVTTL`, `MOBILE_DDR`; the database's `SSTL135_SSTL15.SLEW.FAST` on those and `HSUL_12`; on `HSTL_I`, `HSTL_II`, `HSTL_I_18` three new bits, and on `HSTL_II_18`, `SSTL18_*` four |
| receiver | `LVCMOS25_LVCMOS33_LVTTL.IN` for those and `PCI33_3` (a new bit too); `LVCMOS12_LVCMOS15_LVCMOS18.IN` for those and `MOBILE_DDR`; `SSTL135_SSTL15.IN` for every referenced one, `HSUL_12` and `HSTL_*` too |
| `IBUF_LOW_PWR FALSE` | `ZIBUF_LOW_PWR` on a referenced input, `DIFF.ZIBUF_LOW_PWR` on any pair, and **nothing at all** on `LVCMOS*`, `LVTTL`, `PCI33_3`, `MOBILE_DDR` |
| `IN_TERM` | The field's own bits, on both halves of a pair |
| `DIFF_TERM TRUE` | Six new bits on the `_P` half, in place of `IN_ONLY`, on `LVDS_25`, `RSDS_25`, `MINI_LVDS_25`, `PPDS_25` as inputs. **Nothing** on anything else, `TMDS_33`, `BLVDS_25`, `DIFF_SSTL15` or an `IOBUFDS`, and no word about it |
| a pair | True differential: the driver on the `_P` half alone, `IN_ONLY` on the other. Otherwise: the same driver on both and `OUT_DIFF` on the tile. Neither has a second `OLOGIC` |
| `TMDS_33` input | `TMDS_33.IN_DIFF` on the `_P` half and a bit on the `_N` half the database does not name |
| an input nothing reads | `IN_ONLY` and its pull, **whatever else it asked for**: the receiver, termination and low-power all gone (`unread_*`). And it still counts for the bank's supply. A referenced one also configures the bank's `VREF` balls, which one with a reader does not |

A pad on its way out on the other half has the same bits mirrored: frames 38 and 39 swap and
bit `c` becomes `127 - c`, on every group. The sides of the die are the same (`left_*`), and a
`_SING` tile keeps its half (`sing_*`).

### The bank

**`STEPDOWN` is set on all fifty pads of a bank, bonded or not, and its clock-row tile,
when anything in the bank asks for a supply of 1.8 V or lower**, on banks 14, 15, 34 and
35. An `SSTL15` input alone does not ask, and has no `STEPDOWN`, and with an
`IN_TERM` it does, and has. So it is read off what the design rule check below reads.

**`VREF.V_*_MV` is set from the standard, with or without `INTERNAL_VREF`.** That property
**changes no bit**: `0.6`, `0.675`, `0.75` and `0.9` on an `SSTL15` input all give
`VREF.V_750_MV`, whose reference is 0.75 V. What it changes is what is refused. The
database's name suggested otherwise, and nothing here says where the part takes its
reference from under either.

**`LVDS_25_IN_USE` and `ONLY_DIFF_IN_USE` are about outputs.** `xray::io` said they went
with an LVDS input, read off the Husky Plus's bank 15, where without them "every one of those
six inputs read a constant" on the part. Vivado sets neither on a bank of LVDS inputs alone,
and both on one LVDS output, and that bank has one, `ADC_CLKP`. **So the Husky's bits do not
change, and any other design with LVDS inputs and no LVDS output in the bank loses both.** Why
the inputs read a constant is not explained by this: the board said they did, and what was
measured here is only what Vivado writes.

**Two true differential output standards in a bank take two biases.** `DIFFSTDLIMIT-1`
refuses a third. Of the two: the first is the database's `LVDS_25_IN_USE` and
`ONLY_DIFF_IN_USE` for `LVDS_25`, `RSDS_25` or `MINI_LVDS_25`, and fifteen other bits for
`PPDS_25`; the second is thirteen bits, or ten for `PPDS_25`; and the pairs on the second
get two more. `TMDS_33` has `TMDS_33_IN_USE` and its 3.3 V keeps it apart. **Which is
which is a fit**: over the fourteen designs that ask, the last port's standard takes the
first, in name order, and it is not the site order or the instance. `BLVDS_25` counts for nothing.

### What Vivado refuses, and so does Reticle

`xc7::pads::check_pads` runs after mapping and before placement, as Vivado's checks do,
by Vivado's rule name:

| rule | what |
|---|---|
| `BIVC-1` | Two pads of a bank asking for different supplies. `LVCMOS*`, `LVTTL`, `PCI33_3`, `MOBILE_DDR` and their pairs ask always; everything else asks as an output, or with a termination. So `LVDS_25` in beside `LVCMOS33` out is allowed, and with `DIFF_TERM` it is not |
| `BIVR-1` | Two referenced inputs at different voltages. A differential one has no reference |
| `BIVRU-1` | A pad on a `VREF` ball while the bank has a referenced input and no `INTERNAL_VREF`. An output needs no reference, so that is allowed |
| `IVREF-2` | `INTERNAL_VREF` other than 0.6, 0.675, 0.75, 0.9 |
| `IOSTDTYPE-1`, `Place 30-64` | A differential standard on one ball, or the reverse |
| `Place 30-49` | Bidirectional on `HSTL_I`, `HSTL_I_18`, `SSTL18_I`, their pairs, `RSDS_25`, `PPDS_25`, `TMDS_33` |
| `InOutTerm-9`, `-7`, `-3` | `IN_TERM` on a standard without it, on an output, or at 25 or 75 ohms |
| `DIFFSTDLIMIT-1` | Above |

**`BIVC-1` and `BIVR-1` are a fit, and how good it is is the point:** `tools/vivado-parity/io_pairs.tcl`
asked Vivado's own check of every ordered pair of (standard, direction) in one bank, 11,236 of
them, and `the_bank_rules_are_vivados_on_every_pair` agrees on all of them. **The first reading
was wrong once:** the pairs were asked with no termination, and said a referenced input asks
nothing of the supply. `sstl15_term_lvcmos33_out` is refused.

**`reticle_refuses_what_vivado_refuses`** builds each of the 41 Vivado refused, up to the
refusal, and looks for the rule's name. It would not catch a rule that says the right name about
the wrong pad.

**Stricter than Vivado, on purpose**, each in `STRICTER` there:

| | Vivado | Reticle |
|---|---|---|
| `DRIVE 24` on `LVCMOS33` | critical warning, and 12 mA | error |
| `IN_TERM TUNED_SPLIT`, which it lists | critical warning, and none | error |
| `IBUF_LOW_PWR` on an output | critical warning | error |
| `SLEW MEDIUM`, which it lists | warning, and slow | error |
| a pair's second ball on another pair's `_N` | moves the first ball there and says nothing | error |
| two standards on the balls of a pair | takes the first's and says nothing | error |
| a pair written backwards | critical warning for each, then `UCIO-1` | error |

**Less strict, as before:** a port with no `IOSTANDARD` is `LVCMOS33`, where Vivado has
`NSTD-1`, and a port with no ball is placed, where it has `UCIO-1`.

Warned of and ignored, as Vivado does: `DRIVE` or `SLEW` on a standard with no choice, on
an input. **A parameter on a buffer** counts the same (`inst_*`), and a constraint outranks it.

**This changes existing bitstreams, in four ways, all stated above.** A bank of LVDS inputs and no LVDS output loses `LVDS_25_IN_USE`
and `ONLY_DIFF_IN_USE`. A design with no pin in `PUDC_B`'s bank loses that pad. An input port nothing
reads loses its receiver bits. And a bank with `LVCMOS18`, `LVCMOS15` or `LVCMOS12` in it gains `STEPDOWN` on fifty pads and its
clock row, which this flow had never set: those three were offered, and their bits were the database's names alone. One thing else is new: `IOBUF u(.IO(pad), .O(x)); assign r = x;` is no longer
taken for a tristate on `r`, which drove `r` and left the pad undriven, in a design Vivado
built as written.

**Mutations.** 18 were made, and each failed a test of `tests/fpga_xray_pads.rs`: a measured
driver's bit, the mirror between halves, a fast slew of the wrong family, low power on an
`LVCMOS` input, `DIFF_TERM` on the `_N` half, `STEPDOWN` at 2.5 V or on one half of a tile, a
reference of the wrong level, the bias by the first port, the bank features for an LVDS input,
an unread input keeping its receiver, a referenced input needing its supply, a termination not
needing it, a third differential output standard, `BLVDS_25` as true differential, an
`INTERNAL_VREF` that frees nothing, and `PUDC_B` always, or at its high receiver in a low bank.
Five were caught only by building (`what_reticle_writes_is_what_vivado_wrote`), which is why it
is in the gate.

`set_io` has each of these as `-keeper`, `-in_term`,
`-diff_term`, `-ibuf_low_pwr`, and a bank's `set_internal_vref 35 0.75`;
`set_io_says_what_set_property_says` holds the two files of `rcf_sstl15` and `rcf_lvds` to
one answer.

### Buffers by name

`buf_*` in `testdata/fpga/vivado/io/` is a buffer instantiated by name on `G2`/`G1`, 111 of
them, in standards it takes, its controls routed, tied low and tied high; and the
probes of `primitives/`. **All that build agree with Vivado's site features**: the 57 `buf_*` that
built here before the `DIFF_OUT` forms and the 44 of those since, and 25 probes built from their own `.xdc`, in `every_design_builds_to_vivados_bits`
and by hand.

| buffer | what Vivado wrote, and so Reticle |
|---|---|
| `IBUFGDS` | `IBUFDS`'s |
| `OBUFTDS` | `OBUFDS`'s, and the `_P` half's `OLOGIC` with `ZINV_T1`: `T` is routed to it |
| `IOBUFDS` | the pair's driver and both receivers, `ILOGIC` `ZINV_D` and `OLOGIC` `ZINV_T1` on the `_P` half |
| `*_IBUFDISABLE`, `*_INTERMDISABLE` | `IBUFDISABLE.I` and `INTERMDISABLE.I`, on both halves of a pair, when the pin is routed or tied high; nothing tied low. `USE_IBUFDISABLE FALSE` writes the same bits. The pins reach the pad from `IOI_IMUX9` and `IOI_IMUX6` |
| `PULLUP`, `PULLDOWN`, `KEEPER` alone | On an `input` with no reader: `IN_ONLY`, and the `KEEPER`, but **not** a `PULLUP` or `PULLDOWN` cell, which Vivado drops (a `PULLTYPE` constraint it keeps). On an `output`, or a pull on an `inout`: the driver and the pull, and no data at all, which Reticle builds by leaving the buffer's data unconnected. `KEEPER` on an `inout`: an input |

**An instantiated `OBUFT`'s `T` was dropped until this**, and the pad drove whatever `T`
said: `primitive_instances` took it for a wire, as it takes an `OBUF`. The ledger called it
`ok` because it built. It is the tristate it describes now, so a `T` tied high reaches the buffer as a
released pad, `ZINV_T1` with `T1` unrouted (`buf_obuft_t1`); and so does the cell library's
`IOBUF`, which a `T` tied high had made a driven pad the same way. (A `keep` on both was taken out
again when a mutation showed nothing needs it: what removes a tied-high `T` was the missing
`ZINV_T1`, not folding.) **Tied low**,
Vivado routes a ground to `T1` and sets `ZINV_T1`; Reticle leaves `T1` unrouted with its inverter
on, as a plain output has, and the two drive alike: the one known difference, `KNOWN` in the test.

`REQP-1964` and `REQP-1965`, `IBUFDISABLE` routed on an input-only buffer in `LVCMOS*`, `LVTTL`,
`PCI33_3` or `MOBILE_DDR`, are refused as Vivado refuses them; tied either way, or on an
`IOBUF_INTERMDISABLE`, they are not. A buffer's own `IOSTANDARD`, `DRIVE`, `SLEW` and
`IBUF_LOW_PWR` count, below a constraint; the cell library's `IOBUF` takes no parameters and is
refused them (`V0006`).

**Looser than Vivado, on purpose:** a lone `PULLUP` or `PULLDOWN` on an `inout` with nothing
else in the design is `Place 30-389` there, and the same beside a clocked design (`pullup_probe`)
is an output: Reticle builds both as the latter.

**Mutations.** Ten were made to this, and each failed `every_design_builds_to_vivados_bits` or
`reticle_refuses_what_vivado_refuses`: an `OBUFT` a wire again, a released pad without `ZINV_T1`,
`IBUFDISABLE.I` never set, a disable on one half of a pair, a disable tied low in use, `REQP-1964`
not asked, a pull cell on an unread input kept, a `KEEPER` on an `inout` an output, and a lone
pull driving its data. The eleventh, the tristate's `keep`, survived, and is gone.

**The `DIFF_OUT` forms, since 2026-11-05.** This said they were not done, and that they "need a
second buffer placed on that ball, which this flow does not place; a ball named only by its `_P` in
an XDC does not even say where". The first half was the reason and the second was wrong: the second
ball is the pair's `_N`, which the first names, and no second buffer is placed. `OB` is a second
output of the buffer on the `_P` ball (`din_n`), on the `_N` half's `IOB_IBUF1`, and the router
takes it on through that half's `ILOGIC` as it takes any pad's input, which is the `ZINV_D` on
`ILOGIC_Y1` Vivado sets. **Checked: all 44 `buf_*diff_out*` designs** of `io/`, whose answers were
in `vivado.txt` before this, **and the six probes**, every pad, IO logic and clock row feature.

| buffer | what Vivado wrote, and so Reticle |
|---|---|
| `IBUFDS_DIFF_OUT`, `IBUFGDS_DIFF_OUT` and their disable forms | the same buffer without `DIFF_OUT`, and `ZINV_D` on the `_N` half's `ILOGIC` when `OB` is read. A pair read through `OB` alone is read: its receiver stays (`buf_ibufds_diff_out_ob_*`) |
| `IOBUFDS_DIFF_OUT`, `IOBUFDS_DIFF_OUT_INTERMDISABLE` | `IOBUFDS`'s, driven from `TM`; **`TS` is not routed at all**, on a true differential standard or a pseudo one, though the pad it comes from keeps its receiver (`buf_iobufds_diff_out_*`, where `TS` is its own pad) |

**Mutations.** Eight, and all eight failed `what_reticle_writes_is_what_vivado_wrote`, which has three
`DIFF_OUT` designs in the gate: no `din_n` pin, `din_n` on the wrong half, a pair read through `OB`
alone called unread, `TS`'s pad called unread, that not carried to the buffer, the plain buffer
mapped instead, `OB` not carried, and `OB` taken from the wrong bit.

**Not done.** Vivado's `CFGBVS`-style checks of bank 14 against the configuration voltage, which are warnings. The
XC7A35T, or any other part: the tables were measured on one, and the same table is given to the
others. A wrong drive or standard here would not be seen by anything but a scope.

## IO registers

`testdata/fpga/vivado/iologic/` has three designs from `tools/vivado-parity/gen_iologic.py`, an
`IDDR`, `IDDR_2CLK` or `ODDR` beside each pad of bank 35, one setting a pad: every value of
`DDR_CLK_EDGE`, `SRTYPE`, `INIT*`, each `IS_*_INVERTED`, `R` and `S` routed and tied, `CE`
routed and tied, `D1`/`D2` tied, `CB` from `~C`, from `C` and from a second clock; and three
Vivado refuses. **All 58 registers' `ILOGIC` and `OLOGIC` features are Vivado's**,
`tests/fpga_xray_ioreg.rs`, and so are those of the `IDDR`, `IDDR_2CLK` and `ODDR` probes. The
table of what each setting sets is `xray::iologic`'s documentation.

**Checked and found:** an inverted `ODDR` clock turns `DDR_CLK_EDGE`'s bit round, which a
reading from its name would not have; the same-edge modes of an `IDDR` keep a second pair of
registers' `INIT` and `SRVAL`; a `CE` tied low is not a refusal, as it was, but a ground Vivado
routes and so does this, by the block RAMs' ties.

**Mutations.** Ten, and nine failed `tests/fpga_xray_ioreg.rs`: the second registers left out
of the same-edge modes, `ZINIT_Q3` off the wrong `INIT`, a set given the reset's value, an inverted
clock keeping `ZINV_C`, the `ODDR` edge not turned round, a tied set unused, a `CE` tied low left
untied, `CB` of `~C` left as logic, and `REQP-76` let through. The tenth, the `REQP-77`/`REQP-129`
check before placing taken out, survived: the one after routing refuses the same design by the same
name, and is why the early one is only there for time. The two between a `CE` and its ground
and between `CB` and its clock passed the first version of the test, which compared site features
alone: it compares the ground ties and where each clock pin's clock comes from now.

**This changes existing bitstreams:** an `ODDR`'s `CLKB` is routed now, as Vivado routes it on
every `ODDR` here. NewAE's Husky has five, and its build differs by those routes; nothing about
that has been on a board. Vivado takes `IOI_OCLKM` to `CLKB` and this flow takes `IOI_OCLK`, both
of which the database offers for that pin; only the ends are compared.

**Not done:** `ISERDESE2`, `OSERDESE2`, `IDELAYE2` and `IDELAYCTRL`. The database has their
features (`ISERDES.*`, `OSERDES.DATA_WIDTH.*`, `IDELAY_Y*`), and Vivado's probes of each decode with
nothing unnamed, but none has a bel here, a sweep, or a test. **The serialisers are done since**
(the next section), and the claim that their probes decode with nothing unnamed was right of the
probes and wrong of the blocks: five settings set bits the database does not name. A `CE` tied low on an `IDDR` routes a
ground the way a block RAM's tie does, which has not been on a part either.

## Serialisers

`testdata/fpga/vivado/iologic/` has, beside the IO registers, what `gen_iologic.py`
writes for an `ISERDESE2` or `OSERDESE2` on each pad of bank 35, one setting a pad: every
interface type, rate and width the database names, `NUM_CE`, `INIT_*`, `SRVAL_*`, each
`IS_*_INVERTED`, `DYN_CLK_INV_EN` and `DYN_CLKDIV_INV_EN`, `OFB_USED`, `IOBDELAY`, every
control tied low, tied high and unconnected, `CLKB` from `~CLK`, `CLK`, nothing and a second
clock with each inversion, `OCLK` and `OCLKB` likewise, `CLKDIV` on `CLK`'s clock,
`CLKDIVP` on `CLKDIV`'s; on the other side `DATA_RATE_OQ` and `DATA_RATE_TQ`, every width,
`TRISTATE_WIDTH`, the four `INIT`s and `SRVAL`s, `TBYTE_CTL`, a tristate into an `OBUFT`,
and `TFB` read through a flip-flop; masters and slaves of widths ten and fourteen; and an
`OSERDESE2` read back through `OFB`. **192 serialisers in 16 designs, and every one's
`ILOGIC` and `OLOGIC` features, grounds, ties high and clock sources are Vivado's**
(`tests/fpga_xray_ioreg.rs`, 4,159 lines of `vivado.txt`), and so are the ledger's two
probes, built from their own `.xdc` and compared by hand: 24 and 22 features, the same.
So the ledger has both `ok`.

Before those, 84 designs of one `ISERDESE2` each asked what every combination of `CLKB`,
`IS_CLK_INVERTED` and `IS_CLKB_INVERTED` does in five interface types, and of `OCLKB` with
its two inversions; they are not kept, and what they said is `iserdes_clocks` and the
refused designs beside it. The tables of what each setting sets are
`xray::iologic`'s documentation, and the rules `xc7::serdes`'s.

**What was found, that no reading of the names would give:**

| | what Vivado wrote |
|---|---|
| `IS_CLK_INVERTED`, `IS_CLKB_INVERTED` at a double data rate | **no bit at all**, in every interface; at a single rate `IFF.ZINV_C` is cleared only with `IS_CLK_INVERTED` and `CLKB` unconnected or carrying an inversion against `CLK`'s net |
| `IS_OCLKB_INVERTED`, `IS_CLKDIVP_INVERTED` with nothing on it, what `OCLKB` is wired to | nothing |
| `CLKB` of `~CLK`, of `CLK`, or unconnected at a single rate | `CLK`'s clock routed to both pins: the site has no `CLKB` inverter |
| `INTERFACE_TYPE` other than `MEMORY` | `IDDR.IN_USE` beside `ISERDES.IN_USE`; `MEMORY`, the default, has none |
| `OCLK` in use, in any interface | `IFF.ZINV_OCLK`, or `IFF.INV_OCLK` inverted, and the `OLOGIC`'s `ODDR_TDDR.IN_USE` on the same half |
| `INIT_Q*`, `SRVAL_Q*` | the `IDDR`'s `IFF.ZINIT_Q*`, `IFF.ZSRVAL_Q*`, all four, whatever the reset |
| an `OSERDESE2`, always | `ODDR.SRUSED`, and `TDDR.SRUSED` unless `DATA_RATE_TQ` is `BUF`, whatever `RST` is; a slave `OQUSED` with nothing on `OQ` |
| `T1` to `T4` unconnected | each one's inverter on (`ZINV_T*` clear), a zero, as an `ODDR`'s unconnected `D2` |
| unconnected `RST`, `BITSLIP` | grounded, as tied low; unconnected `CE1`, `CE2`, `OCE`, `TCE`, `DYNCLKSEL`, `DYNCLKDIVSEL` are left, reading one, where tied low they are grounded |
| `IOBDELAY` with nothing on `DDLY` | built: `IBUF` sets `IDELMUXE3.P0`, `IFD` `IFFDELMUXE3.P0`, `BOTH` both |

**The bits the database does not name**, five settings' (`iologic::measured_features`, the
names mine): `DATA_WIDTH` 10 and 14 of an `OSERDESE2` (four bits a half, three of them the
database's own `DATA_WIDTH` field), `TBYTE_CTL`, an `ISERDESE2`'s `IS_CLKDIV_INVERTED`, and
`CLKDIVP` on `CLKDIV`'s clock with its inversion. Each is on both halves, mirrored as the
database's features are, and was seen on both but `IS_CLKDIVP_INVERTED`, whose half 1 is the
mirror alone. **Every bit of all 16 designs decodes now**; before, six had `?` lines. These
are given to the `LIOI3` side as well on the database's word: its 178 `ILOGIC` and `OLOGIC`
features have the `RIOI3` side's bits, every one.

### What Vivado refuses, and so does Reticle

Twelve designs, each refused by Vivado and here by the same name
(`what_vivado_refuses_is_refused`):

| rule | what |
|---|---|
| `AVAL-69` | an `OSERDESE2` wider than four with `TRISTATE_WIDTH` 4, the default |
| `AVAL-74` | `DATA_RATE_TQ` `SDR` or `BUF` with `TRISTATE_WIDTH` 4 |
| `PDRC-26` | `CLK` and `CLKB` one net with inversions that leave them one edge: at a double data rate with neither inverted against the other, at a single rate with the two inversions equal. **A fit over the 70 combinations asked**, in `xc7::serdes`, and it agrees with every one |
| `PDRC-37` | `T1` reading one with nothing on `TQ`, the pad held released: tied high, or unconnected and inverted |
| `INBB-3` | no `CLKB` at a double data rate, which Vivado's synthesis leaves a black box |
| `THRU-2` | `OFB` read by the fabric, or `TFB` straight into a flip-flop on a pad. Through a second flip-flop `TFB` is built |
| `Route 35-7` | `CLKDIVP` on another clock than `CLKDIV`, whose wire it shares |

Refused here and not asked of Vivado, as not measured: a width or interface the database
does not name (an `OVERSAMPLE` of eight, a `MEMORY` at a single rate), `TBYTE_SRC`,
`TBYTEIN` or `TBYTEOUT` in use, `OCLKB` on another clock, and `DDLY` connected, which needs
an `IDELAYE2` this flow does not build beside a serialiser.

**Mutations.** 17, one at a time, and 16 failed a test: `ZINV_C` never cleared at a single
rate, `IDDR.IN_USE` in `MEMORY`, `OCLK` without the `OLOGIC`'s bit, half 1's measured bits
mirrored a column off, an unconnected `T` without its inverter, an unconnected `D` read as a
one, `TDDR.SRUSED` at `BUF`, an unconnected `BITSLIP` left ungrounded, an unconnected
`DYNCLKSEL` grounded, a slave placed on any site of its kind, `PDRC-26` let through at a double
data rate, `AVAL-69` let through, `THRU-2` never asked, `CLKDIVP`'s bit never set, `CLKB` of
`~CLK` left to a lookup table, and `PDRC-37` taken out of the bitstream pass (caught by the unit
test alone: the flow refuses the same designs before placing, by the same name). The
seventeenth, `CLKDIV` taken out of the pins a clock must reach from the global network,
survived: the router brought every one of these from a global line anyway, the shorter road from
a `BUFG`. It is kept for the reason the `IDDR`'s `CLK` is, the Husky's, and no design here would
show it.

**Existing bitstreams do not change.** Eighteen designs built by the commit before this and by
this one decode to the same lines, every bit and every route: the Husky's `io_registers`,
`clock_buffers` and `io_standards`, the `IDDR`, `IDDR_2CLK` and `ODDR` sweeps, and the probes of
`IDDR`, `IDDR_2CLK`, `ODDR`, `IOBUF`, `OBUFT`, `BUFIO`, `BUFR`, `IBUFDS`, `OBUFDS`, `IOBUFDS` and
`BUFG`. The placer's one change, a site with no wire for a pin no longer stopping it from following
a fixed end, could move an instance only where a site of the kind lacks the pin, which no
`IDDR` or `ODDR` site does. The Husky Plus was not rebuilt.

**Not done.** `DDLY` from an `IDELAYE2`: what Vivado writes for the pair is not asked, and
the delay is not built. **No simulation model**: `src/fpga/devices/xc7_cells.v` is what
elaboration takes for a cell nothing declares, so a model there would replace the hard block
in every build, as it would an `IDDR`'s, which has none either; a model for simulation alone
would need a library elaboration does not read, which this flow has not got. A `PHASER`'s
clocks into `CLKDIVP` and `OCLK`, the `MEMORY_DDR3` and `MEMORY_QDR` interfaces' purpose,
which this part's flow does not build. Nothing here has been on a part.

## Input delays and their controllers

`testdata/fpga/vivado/idelay/` has 33 designs from `tools/vivado-parity/gen_idelay.py`: an `IDELAYE2`
beside each pad of bank 35, one setting or wiring a pad (every `IDELAY_TYPE`, each bit of
`IDELAY_VALUE`, `HIGH_PERFORMANCE_MODE`, `SIGNAL_PATTERN`, `CINVCTRL_SEL`, `PIPE_SEL`, the three
inverters, every control routed, tied either way and left open, `CNTVALUEOUT` read), delays into an
`IDDR`, on an `LVDS_25` pair and on both `_SING` tiles, delays of the fabric, `REFCLK_FREQUENCY` at
190, 210 and 300 MHz and mixed, controllers in one region, two and none, with and without
`IODELAY_GROUP`, the four forms of `DCIRESET`, and the five Vivado refuses. `vivado.txt` is each
bitstream decoded: every `IDELAY` and `ILOGIC` feature of each delay's half, what each of its control
pins reads (a ground, a signal, a leaf clock or nothing), the controllers' clock rows, and the
`DCIRESET`'s feature and reset. `tests/fpga_xray_idelay.rs` holds this flow to it. **Nothing here has
been on a board**: that a delay delays by what its value says has not been measured by anything.

**Checked: all 27 designs Vivado builds and this flow takes agree, 1,456 lines, and the five it refuses are refused by
the same rule.** So do the probes of `primitives/`. The table of what each setting sets is
`xray::idelay`'s documentation; what was found on the way:

- **`VAR_LOAD` and `VAR_LOAD_PIPE` write the same bits**, and a loaded delay writes every
  `ZIDELAY_VALUE` whatever its `IDELAY_VALUE` says; `FIXED` and `VARIABLE` write the value.
- **`IS_C_INVERTED` is a bit the database does not name**: frame 34 bit 88 of `IDELAY_Y0` and its
  mirror, frame 35 bit 39, of `IDELAY_Y1`, measured on both halves of a tile and of both `_SING`
  tiles (`c_inverted`). It is `IDELAY_Y<n>.IS_C_INVERTED` here; **the name is mine**. Read from the
  probe alone this would have been a primitive with nothing unnamed, which the probe's delay is,
  with its clock uninverted.
- **`IS_DATAIN_INVERTED` is set whatever the source, `IS_IDATAIN_INVERTED` only from the pad.** The
  first reading was that each is set on its own source only; `idelay_more` sets `DATAIN`'s on a
  delay of a pad.
- **`SIGNAL_PATTERN` and `REFCLK_FREQUENCY` write nothing**, and 300 MHz on speed grade 1, which
  DS181 does not offer, is taken with no word; so are two delays of one controller at 190 and 210.
- To the fabric a delay goes through the `ILOGIC`'s `O` (`IDELMUXE3.P0`), and to an `IDDR` through its
  register input (`IFFDELMUXE3.P0`). This flow declares the second as a wire of its own in front of
  the register (`ILOGIC<n>_IFFD`): the first version took it through the pad's `D`, and the router
  then carried a delay that went to both on from `D` to `O` as if it were the pad, with `ZINV_D` where
  Vivado has `IDELMUXE3.P0` (`idelay_iddr`).
- Open, `CE`, `INC`, `LD`, `LDPIPEEN`, `REGRST` and `CINVCTRL` are grounded, and so is any of them
  tied low and a `CNTVALUEIN` bit tied low; tied high, or `C` and `CNTVALUEIN` open, nothing is routed.
- **The controller has no bits.** What Vivado writes for one is its reference clock's pip in the clock
  row, and the routes of its reset and ready, which reach it through the IO logic tiles above and
  below. Alone it is taken away and its `RDY` is a one (`ctrl_alone`); a delay nothing reads is taken
  away too (`idelay_more`).
- **One controller is replicated into every clock region a delay of its group is in**, as
  `ctl_REPLICATED_0` (`Place 30-1907`), with `RDY` the AND of all of theirs in a lookup table
  (`ctl_TOP_AND`, `INIT` 8); `xray::io_delay_controllers` does the same, before placing, from the
  pads' regions, and places each in its region's clock row after. A delay of a group no controller is
  in is the ungrouped controller's (`group_mismatch`).

**Refused, as Vivado refuses them:** a delay with no controller (`PLIDC-10`), two controllers with no
group between them (`PLIDC-7`), `IDATAIN` or `DATAIN` unconnected for its `DELAY_SRC` (`REQP-83`,
`REQP-84`), and a pad read by the fabric both through its delay and around it, which is two signals
for the `ILOGIC`'s one way out and which Vivado's router gives up on (`Route 35-7`). Around it into an
`IDDR` is not that, and builds (`pad_and_delay_iddr`). **Stricter than Vivado:** an `IDELAY_VALUE` of
32, which Vivado warns of critically and takes as 0, is an error.

**Where this is not Vivado.** A delay of the fabric: Vivado put each in the top left corner of the
die (`IDELAY_X0Y199`), gave it a controller of its own there, and so had two controllers where a pad's
delay was in another region (`datain_with_pad`). This flow puts it in a region one of its group's
controllers already serves, on a delay whose `ILOGIC` and pad are free, so it has one; the delay's
features are compared without its place, and the count of controllers is the one known difference,
`KNOWN` in the test. And a controller's reference clock takes another of the clock row's twelve leaves
than Vivado's, as clocks here do; which is not compared.

### `DCIRESET`

The XC7A100T has one in its configuration tile though its banks are all high-range, with no DCI.
**Vivado builds it, and writes `CFG_CENTER_MID.DCIRESET.ENABLED`** and its pins' routes: `RST` from
`IMUX35` of the interconnect beside the tile's `VFRAME`, grounded when tied low and left when open,
and `LOCKED` out to its fabric; one whose `LOCKED` nothing reads is taken away. All four forms
(`dcireset_*`) agree, the reset's ground with them; the joins between the tile's pins and its
interconnect wires are declared from `tile_type_CFG_CENTER_MID.json`, as the XADC's are, since the
database has no `ppips` file for it. What it does on a part with no DCI to reset is not known.

**Mutations.** Twenty-one were made, and twenty failed `tests/fpga_xray_idelay.rs`: `VAR_LOAD_PIPE` not
known, a loaded delay keeping its value, the value's bits the wrong way round, `IS_IDATAIN_INVERTED` set
on a delay of the fabric, `IS_DATAIN_INVERTED` on one of a pad not, `IS_C_INVERTED` dropped, `CE` not
grounded, an open `CNTVALUEIN` grounded, the register's input through `D`, no `IDELMUXE3` on the way to the
fabric, no replicas, controllers left where the placer put them, a delay nothing reads kept, a controller
with no delay kept, `PLIDC-7` and `Route 35-7` not asked, no fall back to the ungrouped controller, a
`DCIRESET`'s reset tied low not grounded, one nothing reads kept, and one not given the configuration tile its bel is in. **One survived:**
`IS_C_INVERTED` moved to another bit, which decodes to the name this flow gave it whatever the bit is.
The answer has the raw bits of each pad's delay since, frames 32 to 35 of its tile, and that mutation
fails it now.

**This changes no existing bitstream, as far as was measured.** The delays' sites, the `ILOGIC`'s new
ways and the controller's site are declared only for a design with a delay (`XrayOptions::io_delays`),
and `DCIRESET`'s only for a design with one, as the regional buffers' are; a `DIFF_OUT` buffer's `din_n`
is a pin of every pair's `_P` site, which adds no wire. **Checked:** 134 designs, the 13 of `qor/`, the
three of `iologic/` and every probe of `primitives/` both build, built by the commit before these two
and by the second: the same bitstream, byte for byte. **Not checked: the Husky Plus.** The one checkout
of its sources on this machine is another project's working copy, which does not build, and NewAE's
`main` is not the file list `examples/husky/build_plus.sh` names; so `build_plus.sh` was not run.

**Not done.** A delay's timing (`xray::timing` has none for these sites, so a delay estimates as a
wire); an `ODELAYE2`, which a high-range bank does not have; a delay into an `ISERDESE2`'s `DDLY`,
which the section on serialisers refuses as not measured and which these two sections between them
have not asked of Vivado; `--vivado-placement` of a design with a delay; and the cell library has no model of any of these, since each is a primitive the device file
declares and a model of the same name would take its place.

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
| `PACKAGE_PIN`, `LOC` with a ball, `IOSTANDARD`, `DRIVE`, `SLEW`, `PULLUP`, `PULLDOWN`, `KEEPER`, `PULLTYPE`, `IN_TERM`, `DIFF_TERM`, `IBUF_LOW_PWR`, each also in `-dict`; a bank's `INTERNAL_VREF` | applied |
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
any command that is not XDC (above), an `IN_TERM` value Vivado says is not one, `IBUF_LOW_PWR` on an output, any other property of a bank, `LOC` on any
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
warning. (This had one refusal until the section on pads: the two probes that set a
bank's `INTERNAL_VREF`, which this flow could not.) Two others (`fsm`, `ibufds_probe`) were built to a bitstream from each, and the
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

- **`bram_fifo` and `lutram32`: Reticle's signal is on `WEBWE[7:4]`.** **Since
  resolved as Vivado has it** ([Block RAM](#block-ram)): Reticle grounds those pins
  in `TDP` and keeps the router off their wires, and Vivado finishes both on
  Reticle's routes. What follows is the finding as it was. Vivado grounds
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
- **`regfile_alu`: a `RAM64X1D`'s write enable.** Reticle brought it to the slice on
  `CLBLM_M_WE`, with `WEMUX.CE` clear. Vivado connects the same pin through `CE`, with
  `WEMUX.CE` set, on all 64 of them, and will not use the other: asked to with
  `SITE_PIPS {WEMUX:WE}`, it says `Element WEMUX is not routable`. So the net is left
  partly routed, and with `FAN7` taken by Reticle's other signals, Vivado has no way
  through either. The database offers both, and neither reading has been measured on a
  part. **Since resolved as Vivado has it** ([Distributed RAM](#distributed-ram)): every
  RAM of Vivado's has its write enable on `CE` with `WEMUX.CE`, Reticle's now do, and
  Vivado finishes `regfile_alu` with Reticle's routes fixed. That is Vivado's answer and
  not a part's; `--memories legacy` keeps the `WE` pin.

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
| 2 | Vivado only: `CLK_BUFG_REBUF_X78Y142 GCLK0_ENABLE_ABOVE` | 4 | `fir8`, `fsm`, `mul18`, `srl_delay` | **The likeliest wrong bit here.** The far end of the segment the clock row at `Y130` hangs from. Reticle would set it too, but the rule stops at the loaded region and this tile is outside it | **Open**, and the rule now known: Vivado sets both ends of every live segment, on 230 of 231 designs of `clocks/` (below). Nothing measured says the far end is needed, and it was not changed, since loading more of the column changes placement |
| 3 | Vivado only: `HCLK_CMT*_CK_BUFHCLK<n>_USED` | 23 | 10 | Set in a clock-management tile a horizontal clock row passes through, with no clock manager in use. Reticle has no such rule | **Open**, and the rule now known: one for every `BUFHCE` in use on that side of the row, on all 231 designs of `clocks/` (below). Not applied, since it changes every design's bits |
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
| Reticle only: `ZINV_RST*`, `ZINV_REGCLK*` on a block RAM | 24 | Reticle holds each reset at zero with a ground and the inverter off; Vivado connects `VCC`, which **`lutram32.pips` shows**, and leaves the inverter on. The same value if `ZINV` means what its name says, which is quoted | Harmless on that reading. **Reticle does as Vivado does now** ([Block RAM](#block-ram)), and `ZINV_REGCLK*` only beside a register |
| Vivado only: `ZALMOST_EMPTY_OFFSET`, `ZALMOST_FULL_OFFSET` | 52 | Its default for a FIFO's flags, on a block RAM that is no FIFO | Harmless on UG473's word. Reticle writes them as Vivado does now, on the lower half |
| Vivado only: `WEMUX.CE` | 64 | `regfile_alu`, as above | Reticle sets it now, as Vivado does; a RAM on a part is still to come |
| Vivado only: `<L>OUTMUX.O6` | 213 | A way for its route to leave a slice. It is in none of `bits.txt`, so it belongs to Vivado's routes and not to the cells | Not a difference in the cells |
| Vivado only: `CASCOUT_ARD_ACTIVE` | 1 | Likewise, on another block RAM's tile in `bram_fifo` | Vivado's route of an address bit through the cascade from the block below, which it takes often ([Block RAM](#block-ram)) and which Reticle does not route (`bram::is_cascade_input`) |
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

## Distributed RAM

A `SLICEM`'s lookup tables are RAM too. Until this section Reticle had one shape
of it, a `RAM64X1D` on the `D` and `C` tables, and a design naming any other was
logic; `lutram32` went to two `RAMB18E1` where Vivado uses 5 `RAM32M` and 2
`RAM32X1D`, and `regfile_alu` was 64 `RAM64X1D` where Vivado uses 10 `RAM32M` and
4 `RAM32X1D`.

**What was asked of Vivado.** `testdata/fpga/vivado/lutram/` has 50 small designs
written here, from `tools/vivado-parity/gen_lutram.py`: 22 that name a cell, with
random contents and every input from a flip-flop of its own, four of them with the
write clock inverted, and 28 memories to infer, of one read port to four, five of
them with random initial contents.
`tools/vivado-parity/lutram.sh` runs them (`lutram_probe.tcl`), and `vivado.txt`
keeps Vivado's own report of each memory cell's leaves, the bel and site pin of each
of their pins, and each slice holding a RAM as `reticle fpga --decode` reads its
bitstream. `gen_lutram_vectors.py` asks `xsim` what each does as Vivado built it,
on Vivado's own simulation models of Vivado's own routed netlist, into `xsim.txt`.
Only answers are kept.

**What Vivado wrote**, every line read off those, and each held by
`tests/fpga_xray_lutram_vivado.rs`:

- **The write enable is on the slice's `CE` pin, with `WEMUX.CE` set**, in every
  RAM of every design, inferred or named. This flow had it on `WE`, with the bit
  clear, which Vivado's router had refused (below).
- **The write address is the `D` table's inputs**, `A<i>` is input `<i + 1>` of each
  table, and a 64-word table's `INIT[n]` is the cell's `INIT[n]`. That had been
  quoted from nextpnr-xilinx and Project X-Ray's fuzzer; Vivado agrees.
- **A 32-word table is half of a `SMALL` one.** Its upper half, `O6`, is written from
  the slice's `<L>X` and holds the cell's `INIT` in bits 63:32; its lower, `O5`,
  from `<L>I` and bits 31:0. The sixth input is left unrouted, which reads one. A
  `RAM32M` port's bit 0 is the lower half and bit 1 the upper, and `INIT_A[2k]` and
  `INIT_A[2k + 1]` are word `k` of the two.
- **`<L>LUT.DI1MUX.<L>I`** gives a table its own data input; clear, it takes the next
  table up's, so a shape whose tables all hold one bit sets none (`RAM128X1D`,
  `RAM256X1S`), and `RAM32M` and `RAM64M` set `A`, `B` and `C`'s.
- **The wide shapes** take their seventh address bit on `CX` with `WA7USED` and their
  eighth on `BX` with `WA8USED`, the same wires as the multiplexers' selects, and
  leave by them: `RAM128X1S` is `D` below `C` through `F7BMUX`, `RAM128X1D`'s read
  port `B` below `A` through `F7AMUX`, `RAM256X1S` `D`, `C`, `B`, `A` through
  `F8MUX`.
- **An inverted write clock is `CLKINV`**, the slice's flip-flops' too.
- **Vivado rewrites some cells**: `RAM16X1S` and `RAM16X1D` are the 32-word ones with
  the top address bit low, `RAM32X2S` and `RAM64X2S` are two single-port cells on
  `D` and `C` of one slice, and a `RAM64X1D` with `DPO` unread is a `RAM64X1S`.

**What Reticle does now.** `src/fpga/xray/lutram.rs` declares every shape above on
each `SLICEM`, each on a kind of site of its own (`bel RAM32M lutram site lutram_m32`
in `xc7.dev`), each blocking the tables it takes and every other shape, with its
pins, modes and contents as Vivado's. A named cell is that cell, its `_1` or
`IS_WCLK_INVERTED` the falling-edge one, which `place::slice_mode` keeps from a slice
whose flip-flops are on the other edge. And mapping uses them as Vivado does, read
off its own netlists of the inferred designs and of the sweep below
(`primitives::vivado_lutram_plan`): up to 32 words, `RAM32M`s of six bits and one
more for three left over, `RAM32X1D`s for one or two; up to 64, `RAM64M`s of three
and a `RAM64X1D` for one left over; deeper, banks of 64 words with a multiplexer,
except one or two bits, which are `RAM128X1D`s; a copy per read port.

| | slices the same as Vivado's | cells the same as Vivado's |
|---|---|---|
| 22 designs naming a cell | 22 | 22 |
| 28 designs inferring one | 21 | 26 |

The seven slices that differ are Vivado packing more than one cell into a slice: two
read ports' `RAM32X1D` into one slice's four tables, two `RAM64X1D` likewise, and a
`RAM64M` whose unread `C` port shares `D`'s data input. Reticle places one cell a
slice, but for the two halves of a pair of `RAM32X1D`. The two whose cells differ are
`mem32x32` and `mem32x32r2`, which Vivado puts in one `RAMB18E1` each port where
Reticle used two, its 36-bit mode, which the next section is for. Reticle uses one
`RAMB36E1` now, the same tile, in true dual port: inferring the simple dual port is
not done.

**Checked, on all 50:** every set bit decodes, none unexplained; the arcs selected
are the arcs the router chose; the mapping is proved; every frame's ECC is what
Vivado's rule makes it; and **the routed netlist answers as Vivado's build does**,
2,000 cycles from power-up under one stimulus, Reticle's simulator on Reticle's
netlist against Vivado's simulator on Vivado's, bit for bit. The pins each shape
declares are, role by role, the site pins Vivado's report has for the same cell, 
in `vivados_pins_are_the_fabrics`.

**Checked by Vivado, on Reticle's implementation** (`run_placed.sh`, Reticle's cells
where Reticle put them and every route fixed): `qor/lutram32` and `qor/regfile_alu`,
and 14 designs here of every shape the script writes. **Vivado took every route of
every one**, 1,090 nets and none refused, and finished 15 of the 16 with no net left
unrouted; `regfile_alu` is one, which it could not finish before for its write
enables (above). Their bitstreams decoded beside Reticle's (`decode_placed.sh`) differ
in no bit of any slice: what differs is the clock tiles' classes 1 to 3 above, `mem32x32`'s block RAM classes, and a
ground Vivado gives a lookup table's unused input, which Reticle leaves at one. The
16th, `mem64x8`, Vivado routed but for four nets: a `RAM64M` with two unread ports
tied to one ground, which Vivado's own placement wires by chaining `C`'s data input
from `D`'s (`DI1MUX.CI` clear), so Reticle's route to `CI` goes nowhere. Reticle's bel
does not chain; it gives the two ports different nets instead, below.

### Block or distributed

`tools/vivado-parity/lutram_sweep.tcl` asked Vivado's synthesis about 300 memories of
one write port and one read whose address is a register, depth 2 to 16,384 by width 1
to 72, the read registered once more or not, and 26 more written to lie near the line
or to have no register to absorb (`sweep.txt`). **Every one went to block RAM exactly
when its depth rounded up to a power of two, by the width written, is 1,024 bits or
more.** 31 words of 32 bits is block RAM and 32 of 31 is not; and `lutram32` declares
32 bits and writes 31, which is why Vivado gave it distributed RAM at all: the same
memory written 32 bits wide is a `RAMB18E1`. Reticle used 256 bits of the declared
size. `primitives::VIVADO_MIN_BRAM_BITS` is Vivado's rule now, a constant data bit
left out as Vivado leaves it.

**Not the same:** Vivado's block RAM takes the address register of a read nothing
else registers (`s_async_` in `sweep.txt`), and Reticle's does not, so such a memory
is distributed RAM here however large, up to the flow's limit for building one from
logic. A read with no register at all is distributed RAM in both (`c_`), but for
Vivado taking the output register a one-bit read leaves (`c_4096x1`).

### The ECC of a frame holding one

**Vivado leaves a distributed RAM's and a shift register's contents out of its frame's
ECC**, and nothing else of the frame. The 22 named cells' bitstreams had 71 frames
whose ECC was not the frame's code, all of them RAM tables'; probes placed with `LOC`
said what it is. A `RAM64X1S` or an `SRLC32E` with contents, a logic table in other
rows of the same frames, has the code of the frame with the RAM's table zeroed; with
the logic table on the next table of the same slice, in the same word of the frame,
of the frame with the RAM's sixteen bits alone zeroed; and a RAM with no contents
changes nothing. `xc7::wanted_ecc_without` is that rule, `--decode` applies it, and
every frame of every Vivado bitstream decoded here agrees with it; Reticle writes it.
This changes every Reticle bitstream with a RAM's contents in it, in those frames'
ECC bits and nowhere else. That the contents change at run time is the likely reason
and a guess, as it was for block RAM.

### What it changes

| design | | LUT | FF | LUTRAM | BRAM18 | Reticle's estimate |
|---|---|---|---|---|---|---|
| lutram32 | Vivado | 7 | 70 | 51 | 0 | |
| | Reticle before | 9 | 38 | 0 | 2 | 5.07 |
| | Reticle | 10 | 70 | 51 | 0 | 4.95 |
| regfile_alu | Vivado | 351 | 20 | 102 | 0 | |
| | Reticle before | 373 | 20 | 64 | 0 | 10.23 |
| | Reticle | 373 | 20 | 102 | 0 | 10.38 |

LUTRAM counts as Vivado's report does, the macros and their tables both: 5 `RAM32M`
and 2 `RAM32X1D` are 51. The other eleven designs of `qor/` have no distributed
memory and are the same bitstream, byte for byte, as the commit before; so are all
thirteen with `--memories legacy`. Measured on the commit with shift registers, where
`regfile_alu`'s one `SRL16E` is Vivado's too; on the one before them its estimate
went from 10.79 ns to 10.09, and here from 10.23 to 10.38. Both are one placement
each, so neither says the RAMs made it faster or slower.

**The Husky Plus.** `examples/husky/build.sh` and `build_plus.sh` pass
`--memories legacy`, as they pass `--wide-muxes off`, and with it the Plus's
bitstream is byte for byte master's (`ea7c5f49`, the legal routing of pass 46 kept;
it was `3a8a0b4d` on the master this was first measured on, and the same there too).
Built with the default instead, its 328 `RAM64X1D` become 38 `RAM32M`, 32 `RAM64M`
and 4 `RAM32X1D`, and its 244 `RAMB18E1` stay; 2,475,262 bits are set where there
were 2,542,154, and every one decodes, the arcs the router's. The router kept pass
62 where master kept 46, and the estimate moved both ways: `clk_usb` 16.51 ns
against 16.55, `ADC_clk_fb` 7.36 against 6.74, the glitch MMCM's output 13.93
against 17.42. The whole build took 1,670 s against master's 2,838, most of it the
route, the two run at once on a shared machine. That is the reason
the scripts keep the legacy mapping: the ADC's clock is the one the board is
measured on, and it got slower. Nothing of either has been on a board.

**Mutations.** Each change below was made to the code by hand, and
`cargo test --no-fail-fast` of the library and `fpga_xray_lutram_vivado` run against
it; every one failed something, but the one noted.

| mutation | what failed |
|---|---|
| a `RAM32M` port's two bits swapped in the contents | the unit test of the layout; the slices |
| `WEMUX.CE` not set | the slices, and the packing test |
| a `RAM32X1D` upper half's data on `DI` | the unit test of the wiring only: no design here puts a lone upper half anywhere the slices or the simulation see it |
| three bits left over made a `RAM32X1D`, not a `RAM32M` | the cells; the slices |
| the threshold at 512 bits | the threshold test; the cells; the slices |
| a writer's frame ECC over the RAM's bits | every bit decodes (the frames' ECC) |
| a `RAM128X1D`'s seventh write address on `AX` | the unit test of the wiring |
| a RAM given no clock edge in the placer's rule | the unit test of a slice's rule |
| `CEUSEDMUX` on the pin, the RAM's enable enabling the flip-flops | that test |
| a dual-port cell with `DPO` unread kept dual | the cells; the slices |
| `--decode` checking the ECC over the RAM's bits | every bit decodes |
| a `RAM256X1S`'s quarters out of order | the slices |
| `IS_WCLK_INVERTED` not made the `_1` | the slices |
| an inferred RAM's initial contents dropped | **nothing**, at first: no inferred design had contents. The five `mem*init` designs were added for it, and with them the slices and the simulation fail |
| a `RAM32X2S`'s `C` bit also on `CX` | the unit test of the exclusions |

**Not done.** Vivado's packing of several cells into one slice, above. `RAM128X1S`,
`RAM128X1D` and `RAM256X1S` are not written for `run_placed.sh`, since their
multiplexers are leaves whose names were not asked. A named cell with an input left
unconnected reads what the interconnect reads, a one; what Vivado does with one was
not asked, and nothing here ties it. Shift registers (`SRL16E`, `SRLC32E`) are still flip-flops,
though their frames' ECC is now right should they be written.

## Block RAM

Before this section a block RAM was a `RAMB18E1` the mapper inferred, in the
modes nextpnr-xilinx's `fasm.cc` writes, **quoted**; a design naming a `RAMB18E1`
or a `RAMB36E1` did not build; `bram_fifo` was two `RAMB18E1` where Vivado uses one
`RAMB36E1`; and Vivado could not finish `bram_fifo` or `lutram32` on Reticle's
routes, for a signal Reticle's router had walked through `WEBWE[7:4]` (above).

**What was asked of Vivado.** `testdata/fpga/vivado/bram/` has 40 designs from
`tools/vivado-parity/gen_bram.py`: 27 that name a `RAMB18E1` or a `RAMB36E1`, every
input from a flip-flop of its own, in each width, two widths at once, simple dual
port 36 and 72 bits wide, the output registers, every inverter, each write mode, a
reset's priority, the collision mode, initial and reset values and a cascade; and
13 memories to infer, 512 by 36 to 32,768 by 1, with byte writes, with both ports
reading and writing, and with a second register on the read. `FIXTURE=bram
tools/vivado-parity/lutram.sh` runs them, and `vivado.txt` keeps the cells Vivado
left, the site pins each net of a named block reaches, and each block RAM tile of
its bitstream as `reticle fpga --decode` reads it; `xsim.txt` what each does as
Vivado built it.

**What Vivado wrote**, each held by `tests/fpga_xray_bram_vivado.rs`:

- **`ZINIT_A[j]` is bit `17 - j` of `INIT_A`**, and `ZINIT_B`, `ZSRVAL_A` and
  `ZSRVAL_B` likewise: the register's top bit first. Reticle wrote nextpnr's order,
  bit `j` on `ZINIT_A[j]`, which no build could show while every value was zero.
- **`ZINV_REGCLK<x>` is set only beside its port's output register**, and the
  register has a clock pin of its own on the site, which the cell has no port for.
  With `DO<x>_REG` Vivado routes the port's clock to it (`b18_doreg`: `CLKARDCLK`
  reaches the site's `RDCLK` and `RDRCLK`); without, a one. Reticle routed nothing
  to it, so **a registered output from this flow had no clock**: every bit
  decoded, every arc was the router's, and the netlist's simulation was right,
  since it has no pins. What found it was Vivado's router, asked to finish
  Reticle's routes, finding no way to its `VCCNet` at `REGCLKB`.
- **Simple dual port** is `SDP_READ_WIDTH_36` with `READ_WIDTH_B_18`, and
  `SDP_WRITE_WIDTH_36` with both write widths 18; port A also reads 18 on the upper
  half and on a `RAMB36E1`, not on the lower half alone. `WRITE_FIRST`, the default,
  is `NO_CHANGE` on both ports there.
- `RSTREG_PRIORITY_<x>_REGCE` and `RDADDR_COLLISION_HWCONFIG_PERFORMANCE` are their
  parameters, by name.
- **A `RAMB36E1` is its two halves**, each half as wide: 36 is 18, 18 is 9, 9 is 4
  and sets `RAMB36.BRAM36_*_1`, 4 is 2, and 2 and 1 are 1, the one with
  `BRAM36_*_1`. A word's even bits are the lower half's and its odd bits the
  upper's, data, parity, initial values and contents alike, checked bit for bit on
  random contents. Its pins are the site's `BRAM_FIFO36_*`, each address bit on both
  halves, bit 15 only on the lower and tied high. Written one bit wide, `DI[0]`
  reaches both halves and `DI[1]` nothing; nine wide, `DIP[0]` both and `DIP[1]`
  nothing; narrower than nine, `DIP[1:0]` nothing. Vivado's router found the second:
  `DIPADIP[1]`'s signal was on the pin it wanted for `DIPADIP[0]`.
- **The write enables are lanes.** A `RAMB18E1`'s site has four `WEA` and eight
  `WEBWE` pins, and which cell bit each carries is the width's: 18 wide, `WE[0]` on
  the first two and `WE[1]` on the next two; nine or narrower, `WE[0]` on all four;
  36 wide in `SDP`, `WEBWE[k]` on `2k` and `2k + 1`. **In `TDP`, `WEBWE4` to
  `WEBWE7` are grounded**, which is the open finding above resolved: Reticle grounds
  them too, and keeps the router off their wires. A `RAMB36E1`'s are the same on
  each half, interleaved: 18 wide, `WE[0]` on lanes 0 and 2 of each.
- **What a design leaves alone.** A reset, an enable or a clock tied low or
  unconnected is a one with its inverter on: `CTRL0.BYP_BOUNCE4` for a reset and
  `CLK0.FAN_BOUNCE5` for a clock, from wires that read one and that nothing drives,
  and `ZINV` clear. So is an output register's clock without a register. An address
  bit a width does not read, and a read-only port's data, are on Vivado's `<const1>`,
  which is what an input nothing drives reads; `REGCE` is grounded. None of it
  spends a ground fan but `REGCE` and the write enables.
- The tile's FIFO offsets, `ZALMOST_EMPTY_OFFSET` and `ZALMOST_FULL_OFFSET`, are
  written as zero beside a block on the lower half and not beside one on the upper
  alone, though neither is a FIFO.
- **Inference.** Vivado writes on port A and reads on port B, with `READ_FIRST` on
  the port it writes, which makes a read on the other port of the word being
  written return the old word, as the description says; and it takes a `RAMB36E1`
  where that is one block where two `RAMB18E1` would be two, and blocks side by side
  before blocks one above the other: 2048 by 36 is two `RAMB36E1` of 2048 by 18.

**What Reticle does now**, with the default `--memories vivado` (`--memories
legacy` keeps the block RAMs of before, bit for bit): `RAMB18E1` and `RAMB36E1` by
name, every width, `SDP` and `TDP`, the registers, inverters, write modes,
priorities, collision mode and initial and reset values, and a `RAMB36E1` inferred
where Vivado infers one (`src/fpga/xray/bram.rs`; `xc7.dev`'s `bram RAMB36E1`,
on a site of its own that blocks both halves, `site bram36`). Each of the points
above is Vivado's, read off these designs.

| 40 designs | the same as Vivado's |
|---|---|
| block RAM tiles, every site feature and contents bit | 34 |
| cells, by kind and count | 35 of the 37 it has an answer for |
| nets of a named block, by the site pins they reach | all 3,949 of the 27 designs' |
| answer under `xsim`, 2,000 cycles | all 38 Reticle builds |

The six that are not Vivado's are not built as Vivado builds them: a cascade
(`RAM_EXTENSION_*`), which is refused; a memory both of whose ports read and
write, which the mapper does not pair and refuses; a second read register Vivado
takes into the block (`DOB_REG`) and Reticle leaves in a slice; 512 words of 36 and
of 72, which Vivado puts in one block in simple dual port and Reticle in a
`RAMB36E1` and two, in true dual port; and byte writes, which Vivado's synthesis
stopped on. Ten of the named designs read an `x` from the first cycle under `xsim`,
the unused outputs of a narrow port or a collision of the two, and for those the
simulation says only that Reticle's netlist reads `x` where Vivado's does.

**Checked, on all 38 it builds:** every set bit decodes, none unexplained; the arcs
are the router's and the ties'; the mapping is proved; and the routed netlist,
under `xsim` on Vivado's models, answers as Vivado's build does.

**Checked by Vivado, on Reticle's implementation** (`run_placed.sh`): the 38, and
`qor/bram_fifo`, `lutram32` and `regfile_alu`. **Vivado took every route of every
one, 6,284 nets and none refused, and finished all 41**, `bram_fifo` and `lutram32`
among them, which it could not before. Their bitstreams decoded beside Reticle's
name 448,126 features of Reticle's, 448,048 of them Vivado's. What differs is the
clock tiles' classes 1 to 3 above; a ground Vivado gives a lookup table's unused
input in `lutram32` and `regfile_alu`, as before; eight bits of grounds Vivado adds
in `bram_fifo`, not followed further; and two grounds on every `RAMB36E1` Reticle
inferred, its error injection, which Vivado leaves unconnected on a block it infers
(`ib_1024x36`'s `INJECTDBITERR` is on no net). Reticle leaves them so now, which was
measured on its own bits and not again by Vivado. A run before that found
`ib_1024x32_bytes`'s unused port B, whose clock and enable Vivado gave a one through
their inverters where Reticle had left the clock alone and grounded the enable, and
Reticle does as Vivado does there too. Of Reticle's 406,866 cell features under
Vivado's own routes, every one is Vivado's.

**What it changes.** `bram_fifo` is one `RAMB36E1` where it was two `RAMB18E1`,
with the same 57 lookup tables and 68 flip-flops, at 5.06 ns where it was 5.07; the
mapper's other twelve designs of `qor/` are the bitstream they were. With `--memories
legacy` all thirteen are the bitstream before, byte for byte.

**The Husky Plus.** With `--memories legacy`, which both scripts pass, the Plus
is master's bitstream byte for byte (`ea7c5f49`, 2,542,154 bits, every one decoding).
**Built with the default instead, it does not route.** Its 244 `RAMB18E1` become 122
`RAMB36E1`, two halves of each FIFO bank's pair in one block, and the placement that
follows is far more crowded than the one before: at the router's 30th pass 2,998
nodes were oversubscribed where the build of the commit before had 19, and after
the 100th, 67 still were, so the router gave up and the bitstream it wrote
configures none of the design's signals (an earlier build of this commit, stopped
at pass 43, was going the same way). The nodes it named are ordinary interconnect,
each shared by two of the design's own signals, and none a tie's; the placement
estimate before the route had `ADC_clk_fb` at 15.84 ns where the commit before had
12.04. Why the larger blocks crowd the fabric so much more has not been found: a
`RAMB36E1` holds its two halves' pins in one tile pair, so the logic around two
FIFO banks that each had a block of their own now meets at one, but that is a
reading and not a measurement. Until it is found the default mapping is not one for
this design, and the scripts' `--memories legacy` stays for that reason as much as
for the board's.

**Why the default does not route: found, and not by the larger blocks.** *The
reading above, that the `RAMB36E1`s crowd the logic around them, was wrong*; it is
kept because the measurements that showed it are the useful part. Each was a build
of the Plus with this flow's own congestion counts (overused nodes after a pass,
`--timing`), all on seed 4, with a copy of the flow instrumented for it (nothing of
the instrument is committed):

| build | pass 10 | pass 20 | pass 40 | end |
|---|---|---|---|---|
| the default: 122 `RAMB36E1` | 21,260 | 5,488 | 2,763 | 14 left at pass 100, then 67 on seed 5: not routed |
| the same netlist's blocks as 244 `RAMB18E1` | 19,634 | 4,348 | 1,585 | routed, after about 80 passes |
| 122 `RAMB36E1`, the tied pins' wires not closed to the router | 11,063 | 1,507 | 28 | 0 overused at pass 71; the ties then fail |
| 244 `RAMB18E1`, likewise | 10,856 | 1,139 | 6 | the same |
| 122 `RAMB36E1`, the placement round that grew (below) undone | 20,682 | 4,936 | 1,902 | 107 left at pass 100 |

- **The router's own counts say the cause is the wires this commit's block RAM
  model closes to the router before routing** (`XrayFabric::block_ram_ground_fans`):
  every wire behind a pin it will tie, and each held reset's bounce wire, 32,630
  nodes on the Plus where the commit before closed 266 ground fans. Closing only the
  ground fans halves the overuse at pass 10 and routes both netlists by pass 40-70;
  then the ties fail (`tying p1_regclk to One: nothing behind ...`), because a signal
  took the wire a tie needed, which is why the wires were closed. So the closure is
  necessary as it stands and is what costs the route. Which of its wires matter was
  narrowed and not ended: reopening only the bounce wires (2,928 nodes) changed
  nothing; reopening the `BYP` and `FAN` wires (2,440) nothing; closing only the
  block RAM tile's and interface's own wires and the ground fans (24,266) took pass 10
  from 21,260 to 16,708; closing only the interconnect tiles' (8,630) did not help.
- **The `RAMB36E1`s are the last straw and not the load.** With the closure, 244
  `RAMB18E1` route, slowly, and 122 `RAMB36E1` do not quite (14 nodes); without it
  both route as fast. Vivado agrees with the mapping: its synthesis of the same sources
  (Reticle's stand-ins for NewAE's IP, `cwhusky_100.xdc`) makes **123** `RAMB36E1` and no
  `RAMB18E1`, places them at 91 % of the part's 135, and routes its general
  interconnect with no congested region; the only overlaps it reports are three
  clock-row nodes between two of the glitch generator's clocks, which the stand-ins and
  constraints explain and the block RAMs do not.
- **The placement did something on seed 4 too, and it is not the cause.** The third
  timing-driven round grew the wirelength from 753,702 to 833,472, nearly all of it on
  nets that touch no block RAM, and the first pass wanted 2.63 million pips where the
  `RAMB18E1` netlist wanted 2.43 million. Undoing that round (the last row) routes
  worse, and seed 5, whose third round moved nothing, fails as well.
- Congestion is not at the block RAMs' pins: at every pass measured, fewer than ten of
  the overused nodes were in the rows of an interconnect tile beside a used block RAM;
  about a third were two or three columns from a block RAM column, and the rest far
  from any.

**Not fixed.** A remedy has to keep the ties and give back the wires: closing a tie's
wires only once the router has been shown to want them, or choosing each tie's wires
after routing among those still free, are the two shapes it could take, and neither
was built here. Until one is, the default mapping is not one for this design, and the
scripts' `--memories legacy` stays for that reason as much as for the board's. With
`--memories legacy` the Plus is the bitstream before byte for byte, checked again on
this commit (`sha256` `e57fca47...`).

**Mutations.** Each change below was made to the code by hand, and the library's
block RAM tests and `fpga_xray_bram_vivado` run against it with `--no-fail-fast`.

| mutation | what failed |
|---|---|
| `ZINIT` in nextpnr's order | two unit tests; the tiles |
| `ZINV_REGCLK` always, nextpnr's rule | a unit test; the tiles; what is not built |
| the write enables' lanes interleaved | the unit test of the lanes; the pins |
| a `RAMB36E1`'s data on the other half | the pins |
| a `RAMB36E1`'s contents on the other half | the unit test of the halves; the tiles |
| a register's clock pin given a one | **nothing**, at first: every bit decoded, the arcs matched and the simulation answered. The count of a block's clock pips was added for it, and fails |
| a one-bit port's input on one half only | the pins |
| no cost by capacity | the tiles; the cells |
| stacked before side by side | the tiles |
| the resets grounded, not held | the tiles; what is not built |
| no FIFO offsets | the tiles; the halves; what is not built |
| `ADDR[15]` grounded | **nothing**. Bit 15 only matters to a cascade, on UG473's word; no check here sees an interconnect tie but a block's clocks and controls |
| `TDP`'s `WEBWE[7:4]` given the enable | the unit test of the lanes; the pins |
| the mapper's control ties left out of the netlist | the simulation |
| no `READ_WIDTH_A_18` on an upper half in `SDP` | the halves |
| the address below a word zeros, not ones | **nothing**: Reticle's own bits are the same, since it ties neither; only Vivado, given Reticle's netlist, grounds them (`run_placed.sh`) |
| port A's write enables driven in `SDP` | the unit test of the lanes |
| `WRITE_FIRST` kept on the port inference writes | the tiles; what is not built; the simulation |

**Not done.** A cascade, error correction, inference of simple dual port, of a register into the
block, and of a port that both reads and writes. A block's output register named
with `DO<x>_REG` is clocked as Vivado clocks it, but nothing here has run one on a
part, nor any block RAM of this section.

### `FIFO18E1` and `FIFO36E1`, as Vivado writes them

**What was asked of Vivado.** `testdata/fpga/vivado/fifo/` has 46 designs from
`tools/vivado-parity/gen_fifo.py`, each a `FIFO18E1` or `FIFO36E1` by name with its
data from a shift register, its writes and reads enabled at random after a reset at
power-up that the vendor's model accepts, and every output its width drives folded
into one pin: every width in each mode, on one clock and on two, falling through,
flag offsets of its own, initial and reset values at every width, every inverter it
has (all at once, and two alone), its output register's reset and enable from logic, the three settings of error
correction, three on a `BRAM_L` tile, and seven (`fx_*`) that ask what Vivado
refuses. `FIXTURE=fifo tools/vivado-parity/lutram.sh` ran them; `vivado.txt` keeps the
cells, the site pins each of the cell's nets reached, each block RAM tile as `--decode`
reads it, what brings a clock or a constant to the tile's clock and control pins, and
each design rule Vivado refused a bitstream by. `xsim.txt` is what each does as
Vivado built it. Two sweeps that are not fixtures asked the rest: `gen_fifo.py
--ranges`, both flag offsets past any depth at every width of both cells in each of
three modes, whose refusals `fifo_ranges.py` keeps in `ranges.txt`; and `gen_fifo.py
--values`, ten builds of random initial and reset values per width, from which
`fifo_values.py` solved which bit of `INIT` each register bit of the tile is, in
`values.txt` (twenty samples a bit, `INIT` and `SRVAL` together, none ambiguous).

**What Vivado wrote**, each held by `tests/fpga_xray_fifo_vivado.rs` or
`src/fpga/xray/fifo.rs`'s own tests:

- A FIFO is the block RAM features of its half or halves with `FIFO_MODE` set, read on
  port A and written on port B at its width (`READ_WIDTH_A_18`, `WRITE_WIDTH_B_18`;
  36 wide, `SDP_*_WIDTH_36` with all four widths 18 **and** `READ_WIDTH_A_18`, which a
  `RAMB18E1` in `SDP` on the lower half has not), both write modes `NO_CHANGE`,
  both output registers with `DO_REG`, and a `FIFO36E1` is both halves each half as
  wide; 9 wide it sets all four `RAMB36.BRAM36_*_1`, where a `RAMB36E1` sets those of
  its ports 9 wide.
- `EN_SYN` and `FIRST_WORD_FALL_THROUGH` are the tile's own, `EN_ECC_READ` and
  `EN_ECC_WRITE` the `RAMB36` site's, each set when `TRUE`.
- **The flag offsets are not the parameters**: the tile's `ZALMOST_EMPTY_OFFSET` is
  `ALMOST_EMPTY_OFFSET`, less one when the first word falls through, and
  `ZALMOST_FULL_OFFSET` is `ALMOST_FULL_OFFSET`, more one when the clocks are apart,
  each inverted bit for bit as the `Z` says: 0x0b6 and 0x3c5 asked were written 0x0b6
  and 0x3c6.
- **Three bits the database does not name**, frame 27 bits 172 to 174 of the tile, are
  the width of a half: 2, 4, 9, 18 and 36 bits are 0 to 4, so a `FIFO18E1` 4 to 36
  wide is 1 to 4 and a `FIFO36E1` 4 to 72 wide 0 to 4. Reticle names them
  `FIFO_WIDTH[0]` to `[2]` (`fifo::derived_features`), and every one of Vivado's 41
  FIFO bitstreams here now decodes with no bit unexplained. *The first reading said
  they were measured on `BRAM_R` tiles only and assumed `BRAM_L` alike*; `f18_left`,
  `f18_left_w36` and `f36_left`, held on a `BRAM_L` tile, **checked** it: the same
  bits.
- **The initial and reset values** (`values.txt`): 36 wide, a half's 36 bits are port
  A's 18 and then port B's, `INIT[17:0]` and `INIT[35:18]`; 72 wide the same of each
  half's 36, the even bits of `INIT` the lower half's and the odd the upper's.
  Narrower, port B holds `INIT[0]` and nothing else, on both halves; a `FIFO18E1`'s
  port A is `INIT[17:0]` but 9 wide with its bits 8 and 16 swapped, the word's parity
  bit on the register's parity bit; a narrower `FIFO36E1`'s halves take the word's
  data bits by evenness, its parity bits on their parity places, and fill what is
  left of each half with the bits of `INIT` past the word in order, the even ones the
  lower half's. Not one of these is the obvious reading, and only the last differs
  from what a `RAMB36E1`'s values are.
- **The pins** are the block RAM's: `RDCLK`, `RDEN`, `RST`, `RSTREG` and `REGCE` on port
  A's pins, `WRCLK` and `WREN` on port B's, the register's clock beside `RDCLK` with
  `DO_REG` and a one without, `DO` on `DOADO` then `DOBDO`, the flags and counts on
  pins of their own (`EMPTY`, `RDCOUNT0`). The data inputs move with the width: 18 or
  narrower, `DI[15:0]` on port B and `DI[31:16]` on nothing; 36 wide, `DI[15:0]` on
  port A and `DI[31:16]` on port B; a `FIFO36E1` 9 wide puts `DIP[0]` on both halves'
  first parity pin and `DIP[1]` on nothing. A `FIFO36E1`'s clocks, `RSTREG` and
  `REGCE` reach both halves, and its `RDEN`, `WREN` and `RST` the lower half alone.
  *The first reading said the upper half's enables and reset were held through their
  inverters.* Its enables are (`ZINV` clear, and the pins, which read one undriven,
  left alone); **its reset is not held at all**: `ZINV_RSTRAMARSTRAM` is clear on the
  upper half but no pip of Vivado's reaches the pin, and the two
  ones beside every `FIFO36E1` with `RSTREG` tied low are `RSTREG`'s, on both halves,
  as `f36_ctrl`, whose `RSTREG` is a signal and which has none, shows.
- **What Vivado refuses**, at `write_bitstream`, by design rules it names (the `AVAL-n`
  number counts the errors of one run, so the name is the rule): two clocks with no
  output register (`enum_EN_SYN_enum_DO_REG_1`), a first word falling through on one
  clock (`en_syn_and_fwft`), error correction narrower than 72
  (`enum_EN_ECC_READ_TRUE_enum_DATA_WIDTH_72` and its `WRITE` twin), and a flag offset
  out of range (`FIFO_AlmostRange_empty`, `FIFO_AlmostRange_full`). The ranges
  (`ranges.txt`): with the depth of a width taken as 8192 words of 4 down to 512 of
  72, on one clock both offsets 1 to depth less 2; on two, empty 5 to depth less 6 and
  full 4 to depth less 7, and falling through empty 6 to depth less 5. **A
  `FIFO18E1` is held to the depth of a `FIFO36E1` of its width**, twice its own:
  Vivado accepts an almost-full offset of 2041 on a `FIFO18E1` 18 wide, which has
  1024 words on UG473's word.
- **What Vivado builds without a word and should not**: a `DATA_WIDTH` its
  `FIFO_MODE` has not got. `fx_f18_w36_mode` (36 in `FIFO18`) is read and written 18
  wide with the width bits of 36; `fx_f18_w72` (72 for a `FIFO18E1`) 4 wide; and
  `fx_f36_w72_mode` (72 in `FIFO36`) has `SDP_*_WIDTH_36` without the widths that go
  with them everywhere else. Each decodes to a tile that is neither mode.

**What Reticle does now** (`src/fpga/xray/fifo.rs`, in the default `--memories
vivado`; `--memories legacy` has no bel for either and refuses a FIFO as before): a
`FIFO18E1` on a tile's lower half, a `FIFO36E1` on both, each a bel of its own over the
block RAM site's pins, with every point above; Vivado's refusals by its rules, and the
three shapes Vivado builds as neither mode refused with that reason. A FIFO's two
clocks go on the global network: the first build of one did not, since the global
buffers knew a block RAM's clock pins and not a FIFO's, and its three clock pins came
in over general interconnect.

| 46 designs | the same as Vivado's |
|---|---|
| refused, by the rule Vivado gives | all 4 Vivado refuses; and 3 more, which Vivado builds as neither mode |
| block RAM tiles, every site feature, the tile's own and contents | all 39 both build |
| clocks, and the ones and zeros held on the tile's pins | 38 of 39; `f36_w72_sync`'s register clocks, which have no register, Vivado holds at one from its `VCC` net through a slice and Reticle from a bounce wire |
| nets of the cell, by the site pins they reach | all 4,645 of the 39 designs' |
| answer under `xsim`, 2,000 cycles | all 38 Vivado's model will simulate; `f36_ecc_read` stops it ("Corrupted DIP": parity read by the correction that no encoder wrote) |
| initial and reset values, bit by bit (`values.txt`) | all 504 bits of every width |
| flag offset ranges (`ranges.txt`) | all 27 widths and modes |

**Checked, on all 39 it builds:** every set bit decodes, none unexplained; the arcs
are the router's and the ties'; the mapping is proved; and the routed netlist, under
`xsim` on Vivado's models, answers as Vivado's build does. `FIFO18E1` and `FIFO36E1`
are `ok` in `testdata/fpga/vivado/primitives/index.txt`: both probes build to a
bitstream. **Not checked:** Vivado has not been given Reticle's implementation of
these (`run_placed.sh`), and nothing here has run on a part. No simulation model was
added to `src/fpga/devices/xc7_cells.v`: a `RAMB18E1` has none either, and a FIFO's
flags on two clocks are the vendor model's synchronisers, which a model written here
would have to match cycle for cycle to be worth a vector; Vivado's own models are what
the simulation above runs on.

**Bitstreams of other designs.** The FIFO bels are new sites in the Vivado model's
fabric; the block RAM fixtures and `qor/` were built with this and with the commit
before, in both memory models, and compared byte for byte. With `--memories legacy`
every one is the bitstream it was (26 of `bram/`, the others refused as before, and
all 13 of `qor/`). With the default, all 13 of `qor/` and 23 of `bram/` are too; the
15 `b18_*` designs, each a lone `RAMB18E1`, now put it on the upper half of the same
tile where it was on the lower. Why was not followed: the only change to the fabric
they see is the two FIFO sites over the halves, which shifts the sites' order. Their tiles, pins, clocks and simulation are still Vivado's
(`fpga_xray_bram_vivado` holds a block on either half), so this is a placement and
not a fault; it is a changed bitstream all the same.

**Mutations.** Each made by hand to the code, and `fifo`'s unit tests and
`fpga_xray_fifo_vivado` (without the simulation) run against it:

| mutation | what failed |
|---|---|
| the width code of a half 2 wide made 1 | a unit test; the tiles |
| no one added to the full offset on two clocks | a unit test; the tiles |
| no one taken from the empty offset falling through | a unit test; the tiles |
| a `FIFO18E1` 9 wide without its parity bits swapped | the values |
| a narrow `FIFO36E1`'s values interleaved straight | the values; the tiles |
| port B's one bit of value dropped | the values; the tiles |
| 36 and 72 wide, the ports' data the other way round | a unit test; the pins |
| a `FIFO36E1` 9 wide's parity on one half | a unit test; the pins |
| the register's clock given a one with `DO_REG` | the clocks and held pins |
| the upper half's enables not held | the clocks and held pins |
| port B's `ZINV` left in | the tiles |
| no `READ_WIDTH_A_18` 36 wide | the tiles |
| no `BRAM36_*_1` 9 wide | the tiles |
| a `FIFO18E1`'s offsets at its own depth | the ranges |
| two clocks without the register let through | a unit test; the refusals; every built test, since the refused design then builds |
| a FIFO's clocks not known to the global buffers | the clocks and held pins |
| `RST`'s inverter read from `IS_RSTREG_INVERTED` | **nothing**, at first: every design inverted both or neither. `f18_inv_some`, two inverted alone, was added for it, and the tiles fail |
| the upper half's reset not held through its inverter | the tiles |
| the width bits not in the database | every build test, which refuses the feature |
| the register clock's inverter not `IS_RDCLK_INVERTED` | the tiles |

**Not done.** A FIFO inferred from a design's own logic; a FIFO on
Reticle's placement given to Vivado; a simulation model of Reticle's own; error
correction's behaviour, which only the vendor's model has run.

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

**The third thing wrong was a frame holding a distributed RAM's contents**, which
Vivado leaves out of the code: see [The ECC of a frame holding one](#the-ecc-of-a-frame-holding-one).
The eight `.bit`s above had none.

`FRAME_ECCE2` and `POST_CRC` could now mean something; neither is done, and neither
has been tried. `FRAME_ECCE2` is still in the list of primitives above that do not
build.

**Later:** `FRAME_ECCE2` builds now (see "The configuration primitives"), except
beside a shift register or a lookup table used as memory: Vivado writes those
frames' ECC as zero (`ecc_srl`, four frames), this flow writes the code, and the
primitive is what would read the difference. `POST_CRC` is still not looked at.

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

**"On every design" was wrong**, and the section on pads is what showed it: it is on
every design with a pad in that ball's bank, and every design measured here had one,
since `N11` is the clock of each. A design with its pins elsewhere has no such pad
(`lvds_in_alone`), one with a pin in bank 14 has it (`bank34_lvds`), and in a bank at
1.8 V or lower its receiver is `LVCMOS12_LVCMOS15_LVCMOS18.IN` (`bank14_stepdown`).
`finish_frames` does that now, and a design with no pin in bank 14 has nine bits fewer
than it did.

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
| ~~`READBACK.ICAP_SELECT=TOP`, `BOTTOM`~~ | ~~Vivado refuses it without an `ICAPE2` there, and Reticle places none~~ |
| ~~`STARTUP.STARTUPCLK=USERCLK`~~ | ~~Vivado refuses it without a `STARTUPE2` clock, and Reticle places none~~ |

The last two rows were true until Reticle placed an `ICAPE2` and a `STARTUPE2`. Both
options are done now and refused only where Vivado refuses them, with its rule:
`ICAP_SELECT` naming a site no `ICAPE2` is on (`Designutils 12-1536`), and `USERCLK`
with no `STARTUPE2` whose `CLK` is a signal (`Designutils 12-1517`). See "The
configuration primitives".

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
  through `ACOUT`/`BCOUT` and is left with 23 flip-flops against 113. With shift
  registers inferred (*Latches and shift registers*) Reticle also has 23, from the
  same table, but the taps are still fabric wires to each block's `A` and not its
  cascade.
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

## Clock buffers

`testdata/fpga/vivado/clocks/` has 256 small designs, each asking one parameter of a
clock buffer or a clock manager, or one wiring of it. `tools/vivado-parity/clocks.sh`
runs them, and `vivado.txt` there has each clock site Vivado's bitstream set, as
`--decode` reads it, or the rule it refused by. `tests/fpga_xray_clocks.rs` holds
this flow to it. **Nothing here has been on a board.**

### `BUFGCTRL`, and what is one

| what | what Vivado wrote |
|---|---|
| `CE0`, `CE1`, `S0`, `S1`, `IGNORE0`, `IGNORE1` a signal | `ZINV_<pin>` or `IS_IGNORE<n>_INVERTED` set, or clear under `IS_<pin>_INVERTED` |
| one of them tied | set when what the buffer sees is one, by the tie and its inverter together: tied high and inverted is clear, tied low and inverted is set |
| `PRESELECT_I0` | **`ZPRESELECT_I0` set on `FALSE`**, this primitive's default, and clear on `TRUE` |
| `PRESELECT_I1 = "TRUE"`, `INIT_OUT = 1` | a bit each |
| `BUFGMUX_CTRL` | `CE0` and `CE1` high, `S0 = !S`, `S1 = S`, `IGNORE`s low, `PRESELECT_I0` true: `IN_USE ZINV_CE0 ZINV_CE1 ZINV_S1` |
| `IBUFGDS` | the same as `IBUFDS`, to the feature |

`xray::bufg` said `ZPRESELECT_I0` was clear in all of Vivado's thirteen on NewAE's image,
and left it so. It is, for the primitives that image has, whose `PRESELECT_I0` is true.
A `BUFGCTRL` by name is the first primitive here with it the other way, so a reading
of those thirteen alone would have written every one of them with the wrong preselect.

**Refused, as Vivado does:** both enables or both selects held low (`PDCN-1567`,
`PDCN-1568`, and a pin left out counts), and `IS_I0_INVERTED` or `IS_I1_INVERTED`
(`PDRC-159`, and then its router gives up, `Route 35-19`).

**Checked:** 34 designs, each `BUFGCTRL` parameter and each pin tied either way: the
site lines are Vivado's on all 31 it builds, and the 3 it refuses are refused. Those
that the gate does not build are in the `#[ignore]`d
`every_design_of_a_built_family`. **Unchanged:** the thirteen designs of `qor/` and
eleven probes of the older buffers, pads and PLL are the same bitstream, byte for byte,
as the commit before this.

### Regional clock buffers

`BUFHCE` and `BUFH` in a clock row, `BUFR` and `BUFIO` beside a bank and `BUFMRCE`
and `BUFMR` in a clock management tile now have sites of their own, configured as
Vivado configured them. `xray::regional` has every bit and how it was found; what
this database did not name, measured here by a `LOC` on each site:

- a `BUFIO`'s two bits, and a `BUFMRCE`'s four, and its input and output muxes, in
  frames 26 to 29 of `HCLK_CMT`;
- the enable of a regional line in a horizontal clock tile, `ENABLE_REGIONAL`, in the
  tile it leaves by;
- a `ZHOLD` feedback's way into an MMCM or a PLL, a code of the input mux no fuzzer saw.

`BUFR`'s sites are not numbered as their wires are: `BUFR_Y0` is wire 2.

**Not by default.** A `BUFR` that does not divide and a `BUFMRCE` in front of it were
built on global buffers, and the Husky Plus's logic analyser is one; what a placement
of that design does can only be told on a board. So `--regional-clocks auto`, the
default, gives a real site only to a `BUFR` that divides, is enabled or cleared, is
on a `BUFMR` or is read by a `BUFIO`, and `--regional-clocks on` to every one, as
Vivado does. A `BUFG` whose one load is a real `BUFR` is taken out, as Vivado does.

And the sites, the wires only they use and the bits on them are declared only for a
design that has one (`XrayOptions::regional_clocks`), as a shift register's are. The
first version declared them always, and the Husky Plus, with no regional buffer of its
own, placed the same and routed differently from the first pass on: the router had
more of a pad's road to look down, 35 nodes more in its first pass, and every pass
after it differed. The second version declared them and took them out again, and the
Husky Plus routed the same first pass and a different second: a tile type's features
made again were not in the order its `segbits` file has, which is the order of its
pips, and the router's ties went the other way. Found by turning each addition off in
turn, three route passes of the Husky Plus each, against the commit before (which
routes the same twice). Now they are not read at all for such a design.

**What this commit does to the Husky Plus**, built by `build_plus.sh` at seed 4 from
the commit before and from this one: the same placement and the same routes, pass for
pass, and two configuration bits fewer, the user IO PLL's `CLKOUT4` and `CLKOUT5`
output enables. Those two outputs have nets and no reader; Vivado's own bitstream of
NewAE's design has the two enables clear on that PLL, and an output with no reader is
not in use here now. No board has run either.

**Refused, as Vivado does,** with a diagnostic: a `BUFIO` clocking anything but IO
logic, or on anything but a clock-capable pad or a `BUFMR` (`Route 35-4445`), or
reaching IO logic of another bank (`Place 30-126`); a `BUFMR` driving anything but a
`BUFR` or a `BUFIO` (`REQP-1936`).

### Clock managers

`MMCME2_BASE` and `PLLE2_BASE` are library cells over the `_ADV` sites. Each
parameter of both was swept one at a time on a routed cell
(`tools/vivado-parity/cmt_sweep.py`, 1,482 runs, in `registers.txt`), and Reticle's
arithmetic is held to every register field of the 1,388 Vivado builds and this flow takes:
`every_register_is_what_vivado_wrote`. That held the quoted tables (`LKTABLE` and
both filter tables, at every multiplier under each `BANDWIDTH`) and found what was
new:

- phases, to an eighth of a VCO cycle, and duty cycles, to a half, on every output;
- fractional `CLKOUT0_DIVIDE_F` and `CLKFBOUT_MULT_F`, **whose other half is in
  `CLKOUT5`'s and `CLKOUT6`'s registers**, as is a phase of `CLKOUT0` or `CLKFBOUT`;
- `POWER_REG`'s `0x9800` for a fractional counter or a phase of an odd eighth or
  quarter: f4pga's `0x9900` for a fine phase shift was this bit and another;
- `CLKOUT4_CASCADE`, and the inverters `IS_*_INVERTED`;
- `COMPENSATION` as Vivado reads it, which is the feedback's wiring more than the
  parameter: direct feedback is `INTERNAL` whatever is asked, through a `BUFG` with
  `ZHOLD` is `ZHOLD`'s bits, and `BUF_IN`, `EXTERNAL` and their misuses are what
  Vivado refuses (`REQP-1575`, `REQP-1684`, `Timing 38-290`).

**Refused, as Vivado does:** an input, or a VCO, outside its range (`PDRC-29`, `-34`,
`-38`, `-43`; each VCO window's edges measured at 1 % either side, DS181's 600 to 1200
and 800 to 1600 MHz at grade 1), **when a `create_clock` on the pad reaching the input says what it is**:
Vivado checks the clock it times the input by over `CLKIN1_PERIOD`
(`mmcm_period_clocked` built, `pll_period_clocked` refused with the clock's period in
its message). With no clock it checks the attribute, and this flow does not, since
NewAE's glitch MMCM has an attribute that fails it and builds in Vivado, behind a
multiplexer whose clocks this flow does not follow; a phase not a multiple of 45 over its division (`AVAL-139`, `-167`);
`CLKINSEL` a signal with no reset (`REQP-164`), or between two kinds of source on an
MMCM (`REQP-119`). **Not done, and refused:** spread spectrum, and a fine phase shift
that starts off a whole VCO cycle, both of which set bits this database has no name for.

### Vivado's bits beside a clock, answered

The two classes "Vivado's timer on Reticle's implementation" left open, read over the
231 bitstreams of `clocks/`:

- **`HCLK_CMT*_CK_BUFHCLK<n>_USED`**: Vivado sets it in the clock management tile of a
  row's side for **every** `BUFHCE` in use there, `BUFHCE_X<side>Y<n>`, on all 231.
  Reticle sets it only where a route crosses the tile.
- **`GCLK<n>_ENABLE_ABOVE` at `Y142`**: Vivado sets both ends of each live segment of a
  global track, from its buffer to the farthest clock row it is used in, on 230 of
  231; the far end of the segment row 130 hangs from is `Y142`'s, on 24 of them.

Both are now rules, and **neither is applied**. Whether either is needed on a part has
not been measured. Setting the first adds bits to every design with a clock; the second
needs a tile the region does not load, which moves placements. Both change the Husky
Plus's bitstream, and a build of that has only a board to judge it.

**Checked by Vivado's router**, `run_placed.sh` on 38 of these designs with Reticle's
placement and routes as `FIXED_ROUTE`s: every route of the regional buffers, `BUFMR`,
`BUFIO` and `ZHOLD` feedbacks was taken, and the bits of Vivado's bitstream of the same
implementation are Reticle's but for those two classes. Three things were found and
not done: a pad's clock into an MMCM's `CLKIN1` routed through the PLL tile's
`FREQ_BB` lines, which Vivado's node graph refuses (`Constraints 18-11693`) and its own
router never takes; an `ODDR`'s `OCLKM`, which Vivado drives with `OCLK` and Reticle
does not, on a global clock as on a `BUFIO`; and a clock manager's unused DRP and
`DCLK` inputs, which Vivado ties to ground over `GFAN` and Reticle leaves reading one.
**Nothing here has been on a board.**

## The configuration primitives

`BSCANE2`, `CAPTUREE2`, `DNA_PORT`, `EFUSE_USR`, `FRAME_ECCE2`, `ICAPE2`, `STARTUPE2`
and `USR_ACCESSE2`, on the sites of the `CFG_CENTER_MID` tile and the two beside it.
`xray::config` has every bit and how it was found. **Nothing here has been on a
board, and nothing here is to be loaded into one from this work**: what is said is
that the bits and the stream are Vivado's.

### What was asked

`testdata/fpga/vivado/config/` has 120 small designs, written here by
`tools/vivado-parity/gen_config.py`: one parameter of a primitive away from a plain
design, one wiring of its pins (each `STARTUPE2` input alone as a signal and tied
either way, each output alone), one `write_bitstream` option beside one
(`<name>.opts`, which Reticle takes as `--bitstream-option`), two of a kind, and one
of each together. A site asked for is a `(* LOC *)` attribute, which both tools read.
`config.sh` ran them through Vivado 2026.1, and `vivado.txt` has, for each: the
configuration tiles' features as `--decode` reads them, the ties to zero in the
interconnect column the tile's pins enter from, and the stream's packets other than
the frames and the CRCs, word for word; or the rule it refused by.

### What Vivado wrote

| primitive | in the tiles | in the stream |
|---|---|---|
| `BSCANE2` | `BSCAN.JTAG_CHAIN_<n>`, on `BSCAN_X0Y<n-1>` and no other site. `DISABLE_JTAG` is the option's own three bits | nothing |
| `CAPTUREE2` | nothing | `ONESHOT`, which is its default: `COR0` bit 23 |
| `DNA_PORT` | three bits no `segbits` line names: `DNA_PORT.IN_USE` here | nothing |
| `EFUSE_USR` | nothing | nothing |
| `FRAME_ECCE2` | nothing | `CTL0` bit 8 (`GLUTMASK_B`) clear, and bit 7 set for `FARSRC = "FAR"`. The first `MASK` leaves bit 8 out and the last has it, set or not |
| `ICAPE2` | three unnamed bits a site, `ICAP_X0Y0.IN_USE` and `ICAP_X0Y1.IN_USE` here; `ICAP.ICAP_WIDTH_X8` or `_X16`, which the two sites share: two of them set both widths' bits | on `ICAP_X0Y0` alone, `CTL0` bit 30, and so with `ICAP_SELECT=BOTTOM` |
| `STARTUPE2` | `STARTUP.PROG_USR`; three unnamed bits when `GSR` or `GTS` is a signal, `STARTUP.GSR_OR_GTS` here, and three when `USRCCLKO`, `USRDONEO` or `USRDONETS` is one, `STARTUP.USRCCLKO_OR_USRDONE`. A tied pin sets neither, and `USRCCLKTS`, `KEYCLEARB` and `PACK` as signals set nothing | a `CLK` that is a signal makes `STARTUPCLK` `USERCLK` (`COR0` bit 15) unless the option is given |
| `USR_ACCESSE2` | nothing | `BITSTREAM.CONFIG.USR_ACCESS`'s value, in `AXSS`, as before |

And what is not bits:

- **Where they go.** One `ICAPE2` goes on `ICAP_X0Y1`, a second on `ICAP_X0Y0`. A
  `JTAG_CHAIN` outside 1 to 4 is built as 1, with no error (`bscan_chain0`,
  `bscan_chain5`; synthesis ran `-quiet`, so a warning would not have been seen).
- **What is removed.** A `DNA_PORT`, `EFUSE_USR`, `USR_ACCESSE2` or `FRAME_ECCE2` none of
  whose outputs is read is gone, with its bits and a `FRAME_ECCE2`'s `CTL0`. A
  `BSCANE2`, `STARTUPE2`, `CAPTUREE2` or `ICAPE2` stays.
- **What an open input reads.** Vivado ties every input of a `STARTUPE2` the design
  leaves open, `KEYCLEARB`, `USRCCLKTS` and `USRDONETS` to one and the rest to zero, and
  a `CAPTUREE2`'s `CAP` to zero, over the interconnect's ground fans; a clock pin's zero
  comes over a bounce (`GFAN1`, `FAN_ALT5`, `FAN_BOUNCE5`, `CLK_L1`). It leaves a
  `DNA_PORT`'s and an `ICAPE2`'s open inputs, and a `CAPTUREE2`'s `CLK`, unrouted.
- **Where the pins are.** Every pin is on a wire of the tile joined by one fixed
  connection to the interconnect side; the database has no `ppips` file for these
  tiles, so the 319 pins and 15 further hops were transcribed from
  `tile_type_CFG_CENTER_*.json`. Thirteen of `USR_ACCESSE2`'s `DATA` leave through
  `CFG_CENTER_BOT`, 21 rows down.

### What was compared, and how it came out

**All 120 agree.** Of the 109 Vivado builds, Reticle builds 108 with the same
configuration tiles feature for feature, the same ties to zero, the same stream word
for word, every set bit named and every route the router's; the 109th is refused
(below). The 11 Vivado refuses, Reticle refuses, with the rule:
`tests/fpga_xray_config.rs`, 28 in the gate and every one in the ignored
`every_design_here`. The eight probes of `primitives/` build, and `index.txt` says so.

| refused | rule |
|---|---|
| a `BSCANE2` with a `LOC` not its chain's site | `PDRC-2` |
| a `BSCANE2` whose `TDO` is not connected | `PDCN-2` |
| two `BSCANE2`s on one chain, two `STARTUPE2`s, two `USR_ACCESSE2`s | Vivado's `Place 30-433` and `UTLZ-1`; Reticle's placer says the part has one site |
| `ICAP_SELECT` naming a site with no `ICAPE2` | `Designutils 12-1536` |
| `STARTUPCLK=USERCLK` with no `STARTUPE2` clock | `Designutils 12-1517` |

**Refused here and built by Vivado:** a `FRAME_ECCE2` beside a shift register or a
lookup table used as memory (`ecc_srl`). Vivado writes the ECC of a frame with a shift
register's contents in it as zero, four frames of `ecc_srl`, where `xc7::frame_ecc` says
`0x0f80` and `0x0f8f`; "A shift register" above found the same and left it. That frame
is exactly what a `FRAME_ECCE2` reads, so the pair is refused until the rule is known.
For a lookup table used as memory it was not asked.

The first pass at this had two things wrong that the comparison of tiles and streams
alone did not see, and both are kept here:

- **A `USR_ACCESSE2` routed nothing from `DATA[14:2]`.** Those pins leave `CFG_CENTER_MID`
  for `CFG_CENTER_BOT`, which the transcription had not reached, and the build said
  `the router gave up` and wrote a stream and a tile that were Vivado's. The test now
  counts a design that is not fully routed as wrong.
- **A `CAPTUREE2`'s open `CLK` was tied to zero** as a `STARTUPE2`'s is. Vivado leaves it,
  and it took the ties being compared to see it.

The features of two components (`STARTUP.GSR_OR_GTS`, and the database's own
`STARTUP.PROG_USR`, `BSCAN.JTAG_CHAIN_1`, `ICAP.ICAP_WIDTH_X8`) are read as pips by the
rule `parse::is_pip_feature` applies, so the flow's check that the arcs selected are
the router's called the first `STARTUPE2` a selected arc nothing chose. They are named
as not arcs (`NOT_ARCS`).

### Mutations

Eighteen mutations of the new code, each built and run against the gate's tests:
**all eighteen fail them**, and three did only after the tests were made to. A
derived feature moved by one bit passed, because the fixture holds the names this
flow gives Vivado's bits, so the bits are now held to the unnamed lines `--decode`
listed before there were names (`the_names_are_on_the_bits_vivado_set`). A
`FRAME_ECCE2` beside a shift register built passed, because the comparison takes no
frame's ECC; a design on the refused list that builds is now wrong. And a
`CAPTUREE2`'s `CAP` left open passed, because no design of the gate left it open;
`capture_open` is in the gate. The other fifteen: a chain on the next site, `GTS`
setting no bits, `SINGLE`, `FARSRC` and `ICAP_SELECT` on the wrong bit, two `ICAPE2`s
selecting the bottom one, `KEYCLEARB` tied low, the last `MASK` as `CTL0`, an unread
`DNA_PORT` kept, one `ICAPE2` on the bottom site, an open `TDO` built, no user clock by
default, clock pins left open, no `CFG_CENTER_BOT` hops, `ICAP_SELECT=TOP` unchecked
and the two widths swapped.

### Not the same as before

A design with none of these primitives loads, places, routes and writes as it did:
the sites, their wires and the bits only they use are declared only for a design with
one (`XrayOptions::config_sites`), as the regional buffers' are. **Checked**: the
thirteen designs of `qor/` and seven probes of other primitives are the same
bitstream, byte for byte, built by the commit before this and by this one, and so is
the Husky Plus by `build_plus.sh` (whose `USR_ACCESSE2` is NewAE's stand-in module, so
not this primitive).

`CTL0`'s `GLUTMASK_B` was a constant one in `BitstreamOptions::ctl0`, and the last
`MASK` was `CTL0` itself; both are what they were for any design without a
`FRAME_ECCE2`, and `tests/fpga_xray_bitstream.rs` still holds 245 streams to Vivado's.

### Cells by name

`DNA_PORT`, `EFUSE_USR`, `USR_ACCESSE2` and `CAPTUREE2` are in `xc7_cells.v` for a
simulator, and `reticle fpga` keeps all eight as the device's own cells. **Checked**
against xsim's answers in `testdata/fpga/vivado/cells/`: a `DNA_PORT` read and shifted
with a random `SIM_DNA_VALUE`, an `EFUSE_USR` with a random `SIM_EFUSE_VALUE`, and the
fixed word the vendor's `USR_ACCESSE2` gives a simulation, which is not what the part
reads (`AXSS`). A `CAPTUREE2` has no output, so nothing is checked of it. `BSCANE2`,
`STARTUPE2`, `ICAPE2` and `FRAME_ECCE2` have no model: with nothing driving the JTAG
port xsim answers `x` for every output of the first, and the rest are an oscillator,
the configuration engine and a readback, which there is nothing here to hold a model
to.

### Not done

- **The bits of a site on another part.** All of this is the XC7A100T's. The XC7A35T's
  configuration tiles are the same types, and the device file declares the primitives
  on both; what the bits are there has not been asked.
- **`ICAPE2`'s `DEVICE_ID` and `SIM_CFG_FILE_NAME`, `FRAME_ECCE2`'s
  `FRAME_RBT_IN_FILENAME`, `STARTUPE2`'s `SIM_CCLK_FREQ`** set nothing, as measured,
  and are read nowhere here.
- **`POST_CRC`**, and what a readback through `ICAPE2` reads.
- **Timing.** These pins are timed as any other cell's: no delay of Vivado's for them.
