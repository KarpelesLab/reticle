# Reticle against Vivado, measured

Vivado 2026.1 is installed here, so what it does is an answer to look up
rather than a thing to remember. This is the account of where Reticle's 7-series
flow stands against it. The part is the XC7A100T in FTG256, the Husky Plus's,
at speed grade 1, except in [The XC7A35T, in both its packages](#the-xc7a35t-in-both-its-packages),
which asks the primitives' questions again of the other two 7-series parts Reticle's
device list has.

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

## Simulating the hard blocks

`src/fpga/devices/xc7_sim.v` holds models of the hard blocks for a simulator alone:
`reticle sim --device <7-series part>` takes from it, after `xc7_cells.v`, a module a
design names and declares nothing of, and a build never reads it, so a `RAMB18E1` a
design names is still the block in a bitstream. `docs/simulation.md` says how to use it. It
has the lookup tables as well, in place of `xc7_cells.v`'s for a simulator, because
xsim's resolve an unknown input their function ignores and the language's `INIT[index]`
does not (`docs/simulation.md`, *Vendor cells*).

**Checked:** each testbench in `testdata/sim/xc7/` prints under Reticle's simulator what
it printed under `xsim` with Vivado's `unisims_ver`, `secureip` and `glbl`, byte for
byte (`tests/sim_xc7_primitives.rs`; `tools/vivado-parity/gen_sim_xc7.sh` asks `xsim`):

| Testbench | What it drives |
|---|---|
| `iddr`, `iddr_global_reset` | `IDDR` in each `DDR_CLK_EDGE`, both `SRTYPE`s, the inversions, CE, R and S at random; the global reset |
| `oddr`, `oddr_same_edge`, `oddr_same_edge_inverted` | `ODDR` likewise, and CE held low across whole cycles in `SAME_EDGE` |
| `oserdese2`, `oserdese2_reset` | `OSERDESE2` at SDR 2 to 8 and DDR 4, 6, 8, OQ and TQ; eight moments of coming out of reset |
| `iserdese2`, `iserdese2_reset` | `ISERDESE2` in `NETWORKING` at SDR 2 to 8 and DDR 4, 6, 8, all eight Q; eight moments of coming out of reset |
| `idelaye2`, `idelayctrl` | `IDELAYE2` `FIXED` at several values and two reference frequencies, from `DATAIN`, inverted, `VARIABLE` and `VAR_LOAD` stepped and loaded while edges are in the line; `IDELAYCTRL`'s `RDY` |
| `fifo_one_clock` | `FIFO18E1` at 18, 9 and 36 bits and `FIFO36E1` at 4, on one clock (`EN_SYN TRUE`): filled, emptied, every flag and count, `DO_REG` |
| `startupe2` | `STARTUPE2`'s `EOS`, `CFGMCLK`, `CFGCLK` and `PREQ` |
| `ramb18e1`, `ramb36e1` | `TDP` and `SDP`, every width, mixed widths on a port, each write mode, the output registers, both resets and their priority, byte enables, parity, `INIT_xx`/`INITP_xx`, inversions |
| `bufg`, `bufg_switches`, `bufgctrl` | `BUFG`, `BUFGCE` (both `CE_TYPE`s), `BUFGCE_1`, `BUFGMUX`, `BUFGMUX_1`, `BUFGMUX_CTRL`, `BUFGP`; 80 switches and 140 `BUFGCTRL` experiments, one control at a time at each phase of two clocks |
| `bufr` | `BUFR` at every divide with CE and CLR, `BUFH`, `BUFHCE`, `BUFIO`, `BUFMR`, `BUFMRCE` |
| `clock_managers` | `PLLE2_ADV`, `MMCME2_ADV` and both `_BASE`s: first edges, lock, period, high time and phase of every output, and a second reset |
| `dsp48e1_global_reset` | the `DSP48E1` model that was `testdata/fpga/vivado/dsp/dsp48e1_model.v`, through the global reset; the 52 settings of `dsp/vectors/` still hold it otherwise |
| `io_buffers` | every input, output, tristate, bidirectional and differential buffer the ledger builds, the disables, `PULLUP` and `PULLDOWN` |
| `ramb18e1_byte_write` | the `RAMB18E1` Vivado made of the language corpus's byte-enabled memory (`v_ram_b`), as Vivado tied it, through the global reset and after |
| `dsp48e1_preadd` | the `DSP48E1` Vivado made of `vh_pipeline`'s `(a + b) * a`: pre-adder on, `INMODE = 5'b00100`, `BREG = 2`, no `MREG`, as Vivado tied it, through the global reset and after |
| `lut_unknown_inputs` | `LUT1` to `LUT6`, their `_L` and `_D` forms and `LUT6_2` with unknown and floating inputs, and a `LUT6_2` whose `O5` reaches its own `I1` |

What the first runs found, each a model written from the documentation and wrong:
xsim holds every block at its initial value for 100 ns (`glbl`'s global reset) and
ignores its clocks; a 9-bit block RAM port's parity bit is bit 8 of `INIT_A`, not 16;
an `ODDR` in `SAME_EDGE` loads its falling edge only if CE was high at the rising edge
before; a `SAME_EDGE` `IDDR` resets each register at its own edge; the global reset
holds a clock buffer low whatever its `INIT_OUT`, and xsim's `BUFHCE` ignores
`INIT_OUT` altogether; `OSERDESE2` holds OQ at `SRVAL_OQ` for three `CLK` cycles after
the first `CLKDIV` edge out of reset; a PLL's outputs run half a VCO period early, and
an MMCM's and a PLL's lock take `3*M*D + 15` and `+ 17` input cycles. The latencies of
`OSERDESE2` are UG471's table, and xsim's; `ISERDESE2`'s Q1 is the newest sample, Q2 to
Q8 older ones even beyond the width, a reset empties the samples, and its lag is 2, 2, 4,
6, 8, 10, 12 half periods of `CLK` for SDR 2 to 8 and 3, 3, 5 for DDR 4, 6, 8. An
`IDELAYE2` adds 600 ps to its taps, a tap is 1/64 of the reference period in whole
picoseconds (78 at 200 MHz), and its line is ideal: a step while an edge is in it
can put a short pulse out; `VAR_LOAD` starts at zero whatever `IDELAY_VALUE` says.
`IDELAYCTRL`'s `RDY` rises 100 ps after the falling edge that follows two rising edges
of `REFCLK`, and comes back at once from a later reset. A FIFO's count is its pointer
with the bits above the pointer's width set (`c01` for one word of 18 bits), and zero
until it first moves. `STARTUPE2`'s `EOS` rises with the end of the global reset and
its `CFGMCLK` is 65 MHz from time zero.

The simulator had six faults these found, each fixed or refused with a test of its
own: a fractional delay rounded to the unit and not the precision; `m[i][j]` read
element `j` of `m` (and `mem[a][7:0] <= d` was dropped, which is how
`ib_1024x32_bytes` came to be built wrongly: see [Block RAM](#block-ram); it is
written as xsim writes it since, [Byte writes as Vivado arranges
them](#byte-writes-as-vivado-arranges-them)); an integer
division inside a real expression was divided as reals; a function's `real` argument
was one bit wide; `q <= #(d) v` with `d` a variable assigned at once; and an array of
`real` failed validation or rounded what it held. The last two are refused now.

**Not checked against xsim, or known to differ:** `BUFGCTRL` with its selects and
enables changing faster than a switch completes; `OSERDESE2` with OCE or TCE moving in
the middle of a word, DATA_WIDTH 10 and 14, and SHIFTIN/SHIFTOUT; a clock manager's
fractional multiplier (xsim locks it much later), a VCO period that is not a whole
picosecond, an MMCM with D above 1 and M odd, `CLKOUT4_CASCADE`'s phase, the DRP and
the phase shift; `ISERDESE2`'s `BITSLIP`, its `MEMORY` and `OVERSAMPLE` modes and CE
low; `IDELAYE2`'s `VAR_LOAD_PIPE` pipeline, `REGRST` and first nanosecond;
`IDELAYCTRL`'s `RDY` when `REFCLK` stops; a FIFO's `RSTREG`, `FWFT` and ECC;
`STARTUPE2`'s `GSR` and `GTS` inputs; a block RAM's address collisions (xsim prints a warning and reads x),
cascades and ECC; a differential input at x or z on one side; `KEEPER`, and a pull on
a net something else drives, which need strengths. **Not modelled at all:** a FIFO
on two clocks (`EN_SYN FALSE`, the default), whose flags cross through xsim's
synchronisers in a way not measured yet (the model refuses it at elaboration, naming
why), and the other configuration blocks (`BSCANE2`, `ICAPE2`, `FRAME_ECCE2`, `XADC`,
`DCIRESET`); a design naming one is refused at elaboration as before.

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

**On the XC7A35T, in FTG256 and in CPG236, it is 141 of 141 too**, with the same five
refused by Vivado: [The XC7A35T, in both its packages](#the-xc7a35t-in-both-its-packages).
The ledgers are `xc7a35t-ftg256/index.txt` and `xc7a35t-cpg236/index.txt` here, and
both tests above read all three.

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
**Fixed since** (2026-11-06): the rule was found for the distributed RAM ("The ECC of
a frame holding one", below) and `--decode` read it for both, but the writer left the
shift registers out of what it leaves out, which no test compared. It leaves both out
now, and `tests/fpga_xray_config.rs` compares every frame's ECC.

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
an `IDELAYE2` this flow does not build beside a serialiser. **The last is built since**: see
[A delay into a serialiser](#a-delay-into-a-serialiser).

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
would need a library elaboration does not read, which this flow has not got. (Since
2026-11-06 it has one: [Simulating the hard blocks](#simulating-the-hard-blocks). `IDDR`
and `ODDR` are in it, checked against `xsim`, and so is `IDELAYE2`.) A `PHASER`'s
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
wire; **done since**, "A delay's own arc" below); an `ODELAYE2`, which a high-range bank does not have; a delay into an `ISERDESE2`'s `DDLY`,
which the section on serialisers refuses as not measured and which these two sections between them
have not asked of Vivado (**asked and built since**, below); `--vivado-placement` of a design with a delay; and the cell library has no model of any of these, since each is a primitive the device file
declares and a model of the same name would take its place.

### A delay's own arc

"Estimates as a wire" above was not so: `xray::timing` took a delay for a flip-flop on its
`C`, so a fixed delay, with no clock, ended its pad's path and started its register's in no
domain at all, and nothing through it was timed. It is a cell on the path now, at the arc
Vivado's timer gives it (`xray::timing::idelay_arc`), and its count and controls are a register
on `C`. **What was asked:** `report_timing` and the cell's arcs on Vivado's own routed
`idelay_sweep`, `idelay_iddr`, `idelay_lvds`, `datain_default` and `datain_value`, and four
probes of `REFCLK_FREQUENCY` (190, 210, 300 MHz), `HIGH_PERFORMANCE_MODE` and `SIGNAL_PATTERN`,
34 delays: the slow corner's maximum, speed grade 1. **What it said**, fitted, and within
1.2 ps of every one (`an_input_delays_arc_is_vivados_timers`):

| | |
|---|---|
| intercept | 0.815 ns from `IDATAIN` (0.816 on two), 1.012 from `DATAIN` |
| a tap | `a / (64 × REFCLK_FREQUENCY) + jitter`: `a` 1.1132, or 1.094 in `HIGH_PERFORMANCE_MODE`; `jitter` 9 ps for the `DATA` pattern, 5 in `HIGH_PERFORMANCE_MODE`, none for `CLOCK` |
| taps | `IDELAY_VALUE`, **whatever `IDELAY_TYPE`**: a `VAR_LOAD` of 9 is nine taps to the timer though nothing loads 9 |
| `C` to `CNTVALUEOUT` | 0.751 ns |
| at speed grade 3 | the same taps; the intercepts 0.601 and 0.748 ns, checked on 22 of the delays. The legacy delay model takes grade 1's, as its other figures are, and the Vivado model grade 3's |
| at speed grade 2 | the same taps (31 of them 2.975 ns, as at 1 and 3); the intercepts 0.679 and 0.969 ns, on `idelay_sweep` and `ddly_datain`. The Vivado model takes the build's grade's since [Speed grades](#speed-grades); it took grade 3's at every grade before |

The form is mine: `1 / (64 f)` reads as a tap of a chain calibrated to half the reference
period over 32 taps, and the 9 and 5 ps as the data pattern's jitter a tap, which is how they
fit and is not read from a document. The setup of a variable delay's controls was not asked,
and is a flip-flop's here. **Not compared:** a routed path through a delay in Reticle's estimate
against Vivado's report of the same placement, which `--vivado-placement` of a design with a
delay, not done, would ask. **Mutations:** four, all caught by the two tests: the delay timed as
a flip-flop again, `DATAIN`'s intercept for `IDATAIN`'s, the reference clock ignored, and the
`CLOCK` pattern given the data's jitter.

### A delay into a serialiser

`gen_idelay.py` writes eleven more designs since 2026-11-06, `ddly_*`: an `IDELAYE2` of a pad
on an `ISERDESE2`'s `DDLY`. `ddly_sweep` has seventeen, one a pad: `IOBDELAY` `IFD` with the pad
on `D` and without, `IBUF` and `BOTH` with `O` read, `NETWORKING` at a single and a double rate,
`OVERSAMPLE`, `MEMORY_DDR3`, `MEMORY_QDR`, an `IDELAY_VALUE`, a variable and a loaded delay,
`IS_D_INVERTED` with and without the pad, `IS_IDATAIN_INVERTED` and `HIGH_PERFORMANCE_MODE`; and
alone, `IOBDELAY("NONE")` with and without the pad, `IBUF` with nothing on `D`, a master and slave
of width ten, the delay read by the fabric too under each `IOBDELAY`, a delay of the fabric on
`DDLY`, and a signal of the fabric on it with no delay. `vivado.txt` and `tests/fpga_xray_idelay.rs`
take them as they take the rest: every `IDELAY` and `ILOGIC` feature of the pad's half, the
delay's controls and its raw bits.

**Checked: the six Vivado builds that this flow takes agree, 853 lines more, 2,309 in all over 33
designs; the four Vivado refuses are refused by its rule.** What was found:

- **Nothing is written that the two do not write alone.** The delay's features, and the
  serialiser's with `IOBDELAY`'s muxes as `xray::iologic` already had them with nothing on
  `DDLY`, `ZINV_D` only with the pad routed to `D`.
- **`IOBDELAY("NONE")` with a delay on `DDLY` builds**, delay and all, though the registers then
  read `D`; with nothing on `D` too.
- **Read by the fabric as well, only `BOTH` builds.** `IFD`, `IBUF` and `NONE` are each `PDIL-1`,
  "Site pin to site pin route-thru requires ... 'IOBDELAY' ... a value of 'BOTH'". `IBUF` was the
  guess that would build, and does not.
- **A signal of the fabric on `DDLY`** is `Route 35-7`: the delay's `DATAOUT` is the one road to
  it. Refused here before placing, by that name.
- **A delay of the fabric on `DDLY`** Vivado builds (`ddly_datain`), delay and serialiser in one
  tile. This flow put them in two and could not route between them; it is refused, in the test's
  `STRICTER`. Not built is better than a delay going nowhere. *Built since 2026-11-07*, the two in
  one tile and every feature Vivado's, and compared in the gate: what held them apart is in
  "Packing against Vivado's", change 3.
- **The ties.** Beside a serialiser, the router took `BYP_ALT6`, which is behind a delay's
  `CINVCTRL`, as a hop to the serialiser's pins, and the ground Vivado gives `CINVCTRL` could not
  then be made. A delay's grounded controls are closed to the router now, as the Vivado model of
  a block RAM's are. **That can move a route in any design with a delay**; it changes no feature
  any of the 33 compares.

**Mutations.** Four, and all four failed the tests: the `PDIL-1` check taking `IBUF` as well, the
delay's ties left open to the router, `DDLY` from a signal of the fabric let through (to a route
that does not exist), and the delay of the fabric let through. Nothing here has been on a part.

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

**Since 2026-11-06 those options are the bitstream's.** The two paragraphs above were
written before `xc7::options` did each option as Vivado does ("The container around the
frames" below), and were left so after: an XDC's `CONFIGRATE` was warned of as not
followed, its `UNUSEDPIN` refused, while `--bitstream-option` wrote both exactly. Now an
option the table knows is checked as `--bitstream-option` checks it, refused only when the
table refuses it (`ENCRYPT`, `PERSIST`) or the value is not one, and goes into
`Constraints::bitstream_options`; the 7-series bitstream sets them before the command
line's, which override one of the same name, and an export's `.xdc` carries them for
Vivado. **Checked:** two designs of `testdata/fpga/vivado/config/` set eighteen options in
their `.xdc` alone (`xdc_spi`: `CONFIG_MODE`, `SPI_BUSWIDTH`, `CONFIGRATE`, `USR_ACCESS`,
`UNUSEDPIN`, `CFGBVS`, `CONFIG_VOLTAGE`; `xdc_startup`: `USERID`, `DONE_CYCLE`,
`GTS_CYCLE`, `SPI_FALL_EDGE`, `SPI_BUSWIDTH`, `SPI_32BIT_ADDR`, `CONFIGFALLBACK`,
`NEXT_CONFIG_ADDR`, `TIMER_CFG`, `JTAG_XADC`, `M0PIN`), Vivado read the same file, and the
stream's packets are its word for word, the configuration tiles feature for feature.
**That found one thing wrong in the stream, not in the reading:** with `SPI_32BIT_ADDR`,
Vivado writes `WBSTAR` eight bits down, the revision bits with it (`0x00004000` for
`0x00400000`), and refuses an address with any of the low eight bits set
(`Designutils 12-2356`). The options' sweep had set each of the two alone. Both are
Reticle's now (`BitstreamOptions::wbstar`). `COMPRESS` and `PERFRAMECRC` were left out of
the two designs: their packets follow the frames, which differ by placement.

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

  **That last reading was half wrong** ([Placement and routing against Vivado's
  own flow](#placement-and-routing-against-vivados-own-flow)): Vivado routed with
  the lookup tables' pins locked where Reticle's router had put them, so the
  comparison left out the router's choice of pin, which is its own. With the pins
  free, Vivado's router took 1.5 ns off `regfile_alu`'s 10.4 and 0.3 to 0.7 ns off
  five more designs; which input of a table a connection ends on is part of the
  routing, and under Vivado's timer it is up to 0.39 ns a level.

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

**Since looked at, and the reading above partly wrong**
([The delay model, fitted to Vivado's timer](#the-delay-model-fitted-to-vivados-timer)):
the comparison a connection at a time matched many of a lookup table's sinks with
another pin of the same table, so the 0.04 to 7.8 above is in part the comparison's;
matched by the wire a route reached its sink on, the legacy model is 0.22 to 4.7. And
the cell figures were part of it, as guessed, but so were a slice's pins, a branch's
load, and paths the analysis cut through paired tables. The table above is left as it
was measured, on the legacy model, which is still `--delay-model legacy`.

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

## The delay model, fitted to Vivado's timer

**Reticle's estimate of a 7-series design now agrees with Vivado's timer a
connection at a time, not only on the worst path,** at speed grade 3, on Reticle's
own routes. Before, a connection was 0.42 to 1.55 times Vivado's figure (5th to 95th
percentile) and an endpoint 1.3 times at the median; with the fitted model a
connection is 0.95 to 1.18 and every design's endpoints are within 3 % at the median.
`--delay-model vivado` is the default; `--delay-model legacy` is the model before,
and the Husky scripts keep it, so that their bitstreams are the ones measured on a
board, byte for byte (checked below). Run on 2026-11-06. **Nothing here has been on
a board**: it is all Vivado's timer, which is the oracle and not the part.

**Grade 3 only, as written, and every build was timed by it whatever its part's
grade**: on the XC7A35T, whose devices declare grade 1, that put every estimate about
28 % under Vivado's figure for the part. Each grade has its own figures since, and a
build times at its device's ([Speed grades](#speed-grades)).

### What was asked

The corpus is the thirteen designs of `qor/`, `analyser_fifo.v` and
`carry_counter.v` from this directory, and two pieces of the Husky Plus built
whole, NewAE's `esad.v` and `sad.v` with what they instantiate (about 1,000 and
1,450 lookup tables, two clocks each, their ports on pads). A third,
`sad_x4_slowclock.v`, did not elaborate (`V0008`, a register driven from two
places) and is not in it. The pieces are built from the Husky sources, which are not
in this repository; only Vivado's numbers about them are.

`run_placed.sh` with `TIMING=1` builds each with Reticle (`--timing-driven`), gives
Vivado the placement and every route as `FIXED_ROUTE` at `xc7a100tftg256-3`, and
`timing_dump.tcl` asks Vivado's timer, of that checkpoint: every net's pips and the
node above and below each, every sink's interconnect delay (`get_net_delays
-interconnect_only`, the slow corner's maximum), every cell's timing arcs
(`get_timing_arcs`), and every endpoint's worst path with its setup and clock skew.
Three designs, `analyser_fifo`, `esad` and `sad`, Vivado could not finish routing
around Reticle's routes (a ground net, `GND CONFLICTS` or `PARTIAL`), so
`run_placed.tcl` now keeps a checkpoint of an unfinished route too: the nets that are
routed are Reticle's and are timed. That three of seventeen leave Vivado a ground it
cannot finish is a finding of its own, **open**.

`timing_table.py fit` fits the model to the run built with the legacy model, and
`compare` sets Reticle's estimate beside Vivado's figures. Committed, in
`testdata/fpga/vivado/timing/`: `classes.txt` (the fit), `arcs.txt` (every cell arc,
by cell, slice and bel), `sample.txt` (one connection in 27 of the fit, with its pips)
and `summary.txt` (both runs compared). Numbers only.

**The comparison before matched the wrong connections.** `placed_table.py` paired a
lookup table's sink with Reticle's by the pin `LOCK_PINS` says Vivado's logical pin is
on, and Reticle's role by its index; but the router chooses a table's pins and the
roles follow it, so on many tables the two named different connections. A sink Vivado
puts at 3.1 ns was compared with Reticle's figure for another pin of the same table,
0.1 ns. Reticle's `connection` lines now name the wire the route reached the sink on,
and `compare` matches by that, which is the same connection by construction. **So the
"0.04 to 7.8 times" of "How far Reticle's estimate is" above overstates the legacy
model's spread**; matched by wire it is 0.22 to 4.7, and 0.42 to 1.55 from the 5th to
the 95th percentile. That section is left as it was measured.

### What the error was made of

- **A slice's input pins are not alike, and the model had them alike.** Vivado puts a
  lookup table's `A1` to `A6` differences in the wire to the pin, not in the table's
  arc (which is 0.097 ns for any pin): `A6` costs nothing past its `IMUX`, `A1` and
  `A2` about 0.2 to 0.39 ns. prjxray's pip delays give every `IMUX` to pin the same.
  Fitting one figure to each class of wire with a pin keeping its number took the
  fit's spread from 0.72 to 1.82 (5th to 95th) to 0.94 to 1.07, on the first three
  designs alone.
- **A branch is load.** Each branch more than one leaving a wire on a connection's
  way adds 8 ps in Vivado's figures; without it a sink of a wide signal is overstated
  and one of a narrow signal understated, 4 % each way at the extremes. With it, the
  fit's 5th to 95th went from 0.90 to 1.11 to 0.94 to 1.05.
- **A node took the largest delay of the pips into it,** prjxray's, whatever the
  route's pip was, which is part of why the legacy model's median connection is 0.92
  and its spread wide.
- **Reticle's own wires inside a slice were charged as untimed pips.** `_MUX_` and
  `_CARRY_` wires, and the slice's output multiplexers, cost 175 ps in the legacy model
  where Vivado puts the delay in the cell's arc; scaled into the fitted model they made
  a lookup table's output through `AMUX` 0.11 ns dear. Charged nothing now.
- **The cells were grade 1's or prjxray's.** A flip-flop's clock to `Q` is 0.341 ns in
  a `SLICEL` and 0.393 in a `SLICEM`, 0.313 and 0.361 on the `5FF` bels; a lookup
  table 0.097 on most arcs and 0.21 to 0.25 on a quarter, by nothing this flow can
  tell apart (not the pin, the half, the fanout nor the slice), 0.13 on average; the
  data input's setup -0.05 from its own slice and 0.079 through a bypass pin, the
  enable's 0.218, the reset's 0.33; a block RAM's clock to output 1.846; a carry
  chain from `CI` to `CO3` 0.089 and from `S0` 0.395, where the legacy model has 0.3
  for any arc; the input and output buffers 1.34 and 3.15. All **measured**, in
  `arcs.txt`; the few the corpus does not exercise are derived, and
  `xray::timing::VIVADO` says which.
- **A paired table's halves cut paths.** The analysis ordered cells by instance, and
  a `LUT6_2` is two functions: `A.o5` to `B` to `A.i0`, which only `A.o6` reads, is a
  loop by instance and none by pin. The loop was cut where it was met, and with it the
  path: `crc32`'s worst came out at 4.09 ns against Vivado's 6.31, with every
  connection on it within 5 %. The fitted model orders by output
  (`PinDelays::per_output`). The legacy model keeps the old order, since the Husky's
  measured builds were placed with it; it is wrong there too.

Not a cause, checked: the clock. Vivado's skew on these designs is -0.02 to -0.18 ns;
the comparison below adds it to Vivado's side, and Reticle models none.

### The fit

`xray::delay::CLASSES`: 274 classes of the wire a pip enters, the tile type without
its side and the wire without its track number, a slice's pins and outputs keeping
theirs; a class crossed by fewer than ten connections has no entry and costs 0.631 of
prjxray's delay for the pip. Least squares with no figure below zero, over 27,016
connections of the legacy-built run. **On those, the model is 1.002 of Vivado's at the
median, 0.952 at the 5th percentile and 1.045 at the 95th. Fitted on eight designs and
tried on the other nine, 0.94 to 1.07 on six of the nine,** which is what says the
table is not just the corpus's; to 1.18 and 1.20 at the 95th on `mul18` and
`analyser_fifo`, which were not looked at further; and on `fir8` a
`DSP48E1`'s cascade, a class the eight had never seen, charged prjxray's figure for it,
sixty times Vivado's. That class is in the table now, fitted on all seventeen. A carry chain's own hop (`COUT` to `CIN`), which Vivado
puts at 0 to 7 ps, is fitted and left out of the ratios, as is anything under 20 ps.

The placement's straight line (`timing::WIRE`) was fitted to Vivado's figures against
tile distance too: 0.414 ns, 0.0358 a tile and 0.0244 a tile more on a wide signal,
against the legacy 0.31, 0.048 and 0.025. These designs' connections are short; how
the line holds past forty tiles is not measured.

### How it came out

Reticle's estimate of each connection on its own routes over Vivado's, matched by the
wire the sink was reached on, median / 5th / 95th percentile; and of each endpoint both
time, Reticle's arrival with its capture over Vivado's data path, setup and skew:

| design | connections, legacy | connections, fitted | endpoints, legacy | endpoints, fitted |
|---|---|---|---|---|
| adders | 0.768 / 0.415 / 1.438 | 1.003 / 0.912 / 1.113 | 1.479 / 0.925 / 2.048 | 0.991 / 0.787 / 1.099 |
| analyser_fifo | 0.945 / 0.362 / 1.541 | 0.999 / 0.937 / 1.290 | 1.204 / 0.969 / 1.573 | 1.006 / 0.892 / 1.058 |
| barrel | 0.869 / 0.429 / 1.411 | 1.002 / 0.952 / 1.253 | 1.320 / 0.963 / 1.619 | 1.004 / 0.878 / 1.084 |
| bram_fifo | 0.985 / 0.371 / 1.707 | 1.000 / 0.954 / 1.306 | 1.290 / 0.986 / 1.530 | 1.003 / 0.884 / 1.058 |
| carry_counter | 0.660 / 0.272 / 1.405 | 1.009 / 0.984 / 1.063 | 1.535 / 1.126 / 2.197 | 1.005 / 0.793 / 1.065 |
| counter64 | 0.702 / 0.272 / 1.539 | 1.007 / 0.981 / 1.058 | 1.447 / 0.950 / 2.885 | 0.970 / 0.861 / 1.047 |
| crc32 | 0.801 / 0.419 / 1.419 | 1.000 / 0.963 / 1.060 | 1.141 / 0.790 / 1.637 | 0.969 / 0.873 / 1.042 |
| esad | 0.961 / 0.450 / 1.749 | 1.003 / 0.966 / 1.135 | 1.308 / 0.937 / 1.691 | 1.004 / 0.894 / 1.116 |
| fir8 | 1.143 / 0.796 / 1.464 | 0.990 / 0.968 / 1.319 | 1.389 / 1.187 / 1.480 | 0.973 / 0.877 / 1.075 |
| fsm | 0.767 / 0.414 / 1.457 | 0.992 / 0.958 / 1.053 | 1.379 / 0.921 / 1.528 | 0.997 / 0.897 / 1.037 |
| gray_cdc | 0.826 / 0.287 / 1.404 | 1.003 / 0.929 / 1.058 | 1.315 / 0.976 / 1.706 | 1.008 / 0.953 / 1.035 |
| lutram32 | 0.884 / 0.449 / 1.876 | 1.011 / 0.976 / 1.054 | 1.370 / 1.095 / 1.657 | 0.981 / 0.873 / 1.129 |
| mul18 | 1.135 / 0.678 / 1.647 | 1.008 / 0.965 / 1.462 | 1.489 / 1.275 / 1.749 | 0.983 / 0.871 / 1.141 |
| regfile_alu | 0.882 / 0.529 / 1.345 | 0.994 / 0.901 / 1.047 | 1.309 / 1.105 / 1.613 | 0.990 / 0.840 / 1.037 |
| sad | 0.930 / 0.387 / 1.603 | 1.005 / 0.969 / 1.127 | 1.238 / 0.923 / 1.580 | 1.001 / 0.895 / 1.089 |
| srl_delay | 0.816 / 0.283 / 1.657 | 1.007 / 0.981 / 1.460 | 1.498 / 1.378 / 1.849 | 0.997 / 0.815 / 1.143 |
| widemux | 0.968 / 0.553 / 1.340 | 1.008 / 0.960 / 1.305 | 1.426 / 1.291 / 1.615 | 1.007 / 0.876 / 1.114 |

Over every design: **17,887 connections under the legacy model, 0.924 at the median,
0.424 at the 5th percentile, 1.545 at the 95th, 0.222 to 4.713; 18,166 under the
fitted model, 1.003, 0.949, 1.179, 0.753 to 2.924.** The two are different builds, each
timed on its own routes. The fitted table was fitted to the legacy build's routes, so
the fitted build's are routes it had not seen.

The endpoint each clock's worst is at, among the flip-flop endpoints both time, as
Vivado's data path, setup and skew, and Reticle's estimate of that endpoint:

| design | clock | Vivado, legacy build | estimate there | Vivado, fitted build | estimate there |
|---|---|---|---|---|---|
| adders | clk | 8.257 | 7.281 (0.88) | 8.197 | 8.448 (1.03) |
| analyser_fifo | rd_clk | 9.244 | 8.355 (0.90) | 7.823 | 7.791 (1.00) |
| analyser_fifo | wr_clk | 3.564 | 3.590 (1.01) | 3.834 | 4.043 (1.05) |
| barrel | clk | 3.129 | 3.024 (0.97) | 3.605 | 3.555 (0.99) |
| bram_fifo | clk | 4.470 | 4.642 (1.04) | 4.370 | 4.317 (0.99) |
| carry_counter | clk | 2.917 | 2.988 (1.02) | 2.344 | 2.463 (1.05) |
| counter64 | clk | 5.182 | 5.067 (0.98) | 5.133 | 4.954 (0.97) |
| crc32 | clk | 6.581 | 4.736 (0.72) | 5.322 | 5.068 (0.95) |
| esad | adc_sampleclk | 4.112 | 3.684 (0.90) | 4.555 | 4.555 (1.00) |
| esad | clk_usb | 4.212 | 4.578 (1.09) | 3.505 | 3.422 (0.98) |
| fir8 | clk | 4.968 | 5.682 (1.14) | 4.816 | 4.738 (0.98) |
| fsm | clk | 3.721 | 3.460 (0.93) | 3.607 | 3.351 (0.93) |
| gray_cdc | clk | 1.874 | 1.871 (1.00) | 2.037 | 1.861 (0.91) |
| gray_cdc | clk2 | 2.014 | 2.112 (1.05) | 2.039 | 2.055 (1.01) |
| lutram32 | clk | 2.573 | 2.890 (1.12) | 2.542 | 2.342 (0.92) |
| mul18 | clk | 3.010 | 3.025 (1.00) | 2.831 | 2.842 (1.00) |
| regfile_alu | clk | 13.179 | 10.390 (0.79) | 11.039 | 11.244 (1.02) |
| sad | adc_sampleclk | 4.290 | 4.194 (0.98) | 4.644 | 4.500 (0.97) |
| sad | clk_usb | 4.975 | 5.446 (1.09) | 4.083 | 4.051 (0.99) |
| srl_delay | clk | 2.347 | 2.279 (0.97) | 2.237 | 2.230 (1.00) |
| widemux | clk | 4.710 | 5.068 (1.08) | 4.865 | 4.616 (0.95) |

From 0.72 to 1.14 under the legacy model to 0.91 to 1.05 under the fitted one, which
is the target the ignored `the_estimate_of_the_worst_endpoint_is_near_vivados` set
(1.35 either way), met with room on every clock.

**What is left.** A short register-to-register endpoint is still up to 20 % under
Vivado's: a flip-flop's setup through its bypass pin is 0.03 or 0.15 ns, half and half,
by nothing this flow could see (not the enable, the reset or the bel), and the model
charges the mean. A lookup table's arc is the mean of two figures likewise. What
makes the last few percent of connections over Vivado's, the 95th percentile of
1.18 where the fit's own is 1.05, has not been looked at.

### What it does to placement

Every timing-driven build steers by this model now, so every build of a 7-series
design changes. Vivado's worst data path, by its own report on Reticle's routes, of
each clock group, legacy build against fitted build (ns, and slack):

| design | clock | period | legacy | slack | fitted | slack |
|---|---|---|---|---|---|---|
| adders | clk | 5 | 8.255 | -3.292 | 8.130 | -3.232 |
| analyser_fifo | rd_clk | 1 | 9.228 | -8.279 | 7.812 | -6.859 |
| analyser_fifo | wr_clk | 1 | 3.556 | -2.598 | 3.875 | -2.869 |
| barrel | clk | 5 | 3.139 | 1.835 | 3.596 | 1.360 |
| bram_fifo | clk | 5 | 4.425 | 0.494 | 4.281 | 0.596 |
| carry_counter | clk | 1 | 2.920 | -1.951 | 2.348 | -1.379 |
| counter64 | clk | 5 | 4.688 | -0.217 | 4.630 | -0.169 |
| crc32 | clk | 5 | 6.590 | -1.617 | 5.346 | -0.358 |
| esad | adc_sampleclk | 5 | 9.577 | -4.902 | 8.834 | -4.114 |
| esad | clk_usb | 10 | 1.329 | 3.320 | 1.026 | 3.643 |
| fir8 | clk | 5 | 8.067 | -4.161 | 8.067 | -4.161 |
| fsm | clk | 5 | 3.731 | 1.244 | 3.592 | 1.357 |
| gray_cdc | clk | 5 | 1.847 | 3.090 | 1.946 | 2.927 |
| gray_cdc | clk2 | 7 | 1.611 | -0.992 | 1.343 | -0.716 |
| lutram32 | clk | 5 | 2.342 | 2.392 | 2.305 | 2.423 |
| mul18 | clk | 5 | 2.938 | 1.955 | 2.759 | 2.134 |
| regfile_alu | clk | 5 | 13.186 | -8.214 | 11.027 | -6.074 |
| sad | adc_sampleclk | 5 | 8.709 | -4.021 | 7.472 | -2.778 |
| sad | clk_usb | 10 | 1.264 | 3.349 | 3.467 | 1.195 |
| srl_delay | clk | 5 | 2.351 | 2.618 | 2.228 | 2.728 |
| widemux | clk | 5 | 4.567 | 0.256 | 4.871 | 0.100 |

**The fitted build's worst path is 0.96 of the legacy build's at the median, from 0.77
to 2.74,** and every failing clock but `analyser_fifo`'s write clock is better:
`crc32` by 1.24 ns, `regfile_alu` by 2.16, `sad` by 1.24, `analyser_fifo`'s read clock
by 1.42, `carry_counter` by 0.57. Worse: `barrel`, `widemux`, `gray_cdc`'s `clk` and
the write clock, by 0.1 to 0.46 ns, each with slack to spare but the last, and `sad`'s
`clk_usb`, 1.26 to 3.47 ns of a 10 ns period. Both are one seed each, which is how much
a placement moves; that is noise of the order of these smaller differences (two runs of
the fitted model a day apart, which differed only in the setup of a bypass pin, moved
`sad`'s `adc_sampleclk` from 6.57 to 7.47). The gap to Vivado's own flow, the first
table of this document, is still mostly the placement and the netlist.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--delay-model legacy`, beside
`--memories legacy`, with a comment why. Built with it from this tree: `build_plus.sh`
writes `e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`, the Husky
Plus legacy seed 4, and `build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd`, what the commit
before this one writes. **Checked**, byte for byte. Everything the fitted model adds is
behind the choice: the class table, the branch charge, the cells' figures and their
own pins' delays, the order by output, and the placement's line. The legacy order's
cut loop counted a waiting cell below zero, which a debug build stopped on and a
release build wrapped past; it wraps on purpose now, in both, which is what every
measured build did.

### Tests

`tests/fpga_xray_delay.rs`, in the gate and needing neither Vivado nor the database:
that the source holds the table `classes.txt` has; that over `sample.txt` the table
costs every connection within 3 % of Vivado's at the median and 0.9 to 1.1 from the 5th
to the 95th, and prjxray's delays alone do not; that `summary.txt`'s fitted run is
within 3 % of Vivado at every design's median, and the legacy run was not; and that
each clock's worst endpoint is within 1.35 of Vivado's. The last two read the quoted
run, and catch a regeneration that lost the agreement and not a change to the flow
since, which only `run_placed.sh` asks. `tests/fpga_xray_timing.rs` holds the database
side: under the fitted model a wire costs its class, the two sides alike, and a join
costs nothing. Unit tests hold the carry chain's arcs, a flip-flop's own setups, a
pin's own delay forwards and backwards, and a paired table's path through both halves.

**Mutations.** Twelve, each made by a script and put back, with the library's tests of
`place`, `xray::timing` and `xray::delay` and the two test files above run against
it: a class keeping its side (`INT_L` apart from `INT_R`); a slice pin losing its
number; the branch charge at zero; a join charged its class; one entry of the table
off by 30 ps; the carry's outputs counted `CO` first; the enable's setup given to the
data input; a pin's own launch, through or capture ignored, one at a time; and the
analysis back on instance order under the fitted model. Eleven failed a test. **One
survived:** a wire of no class and no prjxray figure charged 110 ps, as the legacy
model's untimed pip once was, since too few of `sample.txt`'s connections cross one.
`a_wire_of_no_class_costs_a_share_of_prjxrays_figure` fails on it now. The setup
through a bypass pin is not held by anything in the gate: nothing there places a
design and asks Vivado.

### Not done

- The Husky Plus itself is not in the corpus: built whole and given to Vivado it is
  an hour each way, and the pieces stood in. Whether the line `VIVADO_WIRE` holds over
  its long connections is the first thing that would say.
- No design here has a clock manager, a pad register or an IO delay of Reticle's, so
  their figures are the legacy model's.
- Clock skew is not modelled; Vivado's is up to 0.18 ns on these designs.
- The router's own analysis, each pass, does not charge the branches; only the
  reported estimate and the next placement's do. **Done since**, under the fitted
  model (`route::Timing::branch_ps`, below); it moved no clock by more than a
  seed's spread.
- One placement each. The fixtures quote runs, and nothing fails when the flow moves
  on but a regeneration.
- What a build steered by the fitted model does on a part. Nothing here has been on
  one, and no Husky build with it has been made for one.

## Placement and routing against Vivado's own flow

**Asked: what clock does each tool reach on the same design, by the same timer?**
Vivado's own flow (synthesis to routes) against Reticle's placement and routes,
both judged by Vivado 2026.1's timer at `xc7a100tftg256-3`, on the seventeen
designs of "The delay model, fitted to Vivado's timer". A clock's figure is its
period less the worst setup slack of the paths from it to itself: the period it
achieved, setup and skew included, paths between clocks left out since Reticle
times none. Run on 2026-11-06. **Everything here is Vivado's timer; nothing has
been on a part**, and the Husky is in use elsewhere.

`tools/vivado-parity/achieved.sh` does all of it and `achieved.py table` writes
the table; `testdata/fpga/vivado/qor/achieved.txt` keeps the figures, and
`tests/fpga_xray_achieved.rs` holds them. How it is asked:

- **Vivado is asked three times.** Given slack it stops: `adders` at its 5 ns
  constraint came out at 2.820 ns, at 2.40 at 2.247, and at 1.91 at 2.042. So it
  runs at the constraint given, at 0.85 of what that achieved, and at 0.85 of the
  better of the two, and its figure is the best of the three. `synth_design`,
  `opt_design`, `place_design`, `phys_opt_design`, `route_design`, defaults all.
- **Reticle is asked once a seed, at five seeds,** on the second set of periods.
  Its criticality is against the longer of the period and the longest path, so any
  period it does not meet asks the same of it. One seed moved `counter64` from 4.37
  to 6.49 ns and `sad`'s `clk_usb` from 2.83 to 4.39, so every figure of Reticle's
  is a median, with the five beside it.
- **And Vivado places and routes Reticle's netlist,** `place_design` and
  `route_design` alone: the column that says how much of the gap is placement and
  routing rather than the netlist. Without `phys_opt_design`, which rewires lookup
  tables and is not placement: with it the column was lower by up to 1.6 ns
  (`adders`, 3.14 against 4.92).
- A design's clocks are asynchronous to each other in Vivado's constraints. The
  pads have no input or output delay, so Vivado times none of them.

### Where the time went

Seed 1 of each build, four ways (achieved ns): Reticle's routes; Vivado routing
Reticle's placement with each lookup table's pins locked where Reticle's router
put them; the same with the pins free; and Vivado placing and routing Reticle's
netlist. Before this section's changes (the column `before` below) and after:

| design | clock | Reticle's routes | Vivado's, pins locked | Vivado's, pins free | Vivado places too |
|---|---|---|---|---|---|
| adders | clk | 7.233 | 7.341 | 7.218 | 4.924 |
| crc32 | clk | 5.828 | 5.463 | 5.169 | 3.807 |
| regfile_alu | clk | 9.650 | 9.601 | 8.812 | 7.223 |
| widemux | clk | 3.915 | 3.167 | 2.854 | 2.745 |
| analyser_fifo | rd_clk | 7.281 | 7.564 | 6.771 | 5.589 |
| fsm | clk | 2.905 | 2.998 | 2.744 | 2.392 |
| *after:* adders | clk | 6.295 | 6.248 | 6.115 | 5.019 |
| crc32 | clk | 4.586 | 4.360 | 4.197 | 3.978 |
| regfile_alu | clk | 8.930 | 8.900 | 8.762 | 7.223 |
| widemux | clk | 3.402 | 3.062 | 2.802 | 2.772 |
| analyser_fifo | rd_clk | 6.365 | 6.224 | 6.278 | 5.612 |
| fsm | clk | 2.752 | 2.718 | 2.662 | 2.598 |
| esad | adc_sampleclk | 3.575 | 4.545 | 3.674 | 4.431 |

The first rows are not quite `before`: they are a build with the pads untimed, the
branch charge and a premium on fast inputs that was not kept (the first two changes
below and the last thing tried), which is where this was asked. So
the earlier reading that the router is the least of it held only for the routes
with the pins fixed: **which input of a table a connection ends on was up to 1.5 ns**
(`regfile_alu`), and after the changes below it is under 0.3 everywhere asked. The
routes themselves are within 0.25 ns of Vivado's but on `widemux` (0.34), whose
select is read by 257 pins; and the placement is the rest, 1.1 to 1.5 ns on `adders` and
`regfile_alu` and 0.6 to 0.8 on `analyser_fifo`. On `esad` and `sad`, the Husky
Plus's pieces, Reticle's placement and routes are faster than Vivado's own placement
of the same netlist.

### What was changed

All behind `--delay-model vivado`, the default; `--delay-model legacy` builds as
before, byte for byte (below). In the order they were measured, each against the
build before it, medians of the clocks' achieved periods; the geometric mean is of
each clock's median over Vivado's own flow's figure.

1. **A pad no constraint names is not timed** (`xray::place_timing`), as Vivado's
   timer times none. Timed in every domain it was the latest path of its round: on
   `counter64` the output's register at column 52 against its pin at 147, and the
   anneal spent its pull on paths Vivado never asks about. `counter64` went from
   6.24 to 4.11 ns (three seeds), `sad`'s `clk_usb` from 3.10 to 2.68, `widemux`
   4.15 to 3.62; the geometric mean from 1.572 to 1.526. Reticle has no
   `set_output_delay`, so under the fitted model no output pin is timed.
2. **Each routing pass charges a branch's load** (`route::Timing::branch_ps`),
   which the routed report and the next placement already did. Within a seed's
   spread either way (1.526 to 1.532); kept because the pass's analysis and the
   report then agree.
3. **A critical search's estimate is not inflated** (`xray::CRITICAL_ASTAR_WEIGHT`,
   1.0 for a critical search where the router's 2.0 stays for the rest). At 2.0 a
   critical connection's estimate is half again its real way, and the search takes
   the first that heads for the sink: on `widemux` a connection of six tiles went
   east by five and back. `widemux` 4.12 to 3.58 ns, `sad`'s `adc_sampleclk` 3.94
   to 3.74; geometric mean 1.532 to 1.496.
4. **A critical connection may take a table's input another signal holds**
   (`xray::CLAIM_INPUTS`): from a criticality of 0.3 after the exponent a search may
   end on a held input of its class, and the negotiation moves the holder.
   `regfile_alu` 10.34 to 9.16 ns, `esad`'s `adc_sampleclk` 3.25 to 3.04; 1.492 to
   1.468.
5. **The first pass starts from the placement's estimate**
   (`route::Timing::seed_from_placement`): every connection's criticality from the
   placement's line in tile distance, and the signals in the order of their most
   critical, so that the first routed are the ones a fast input matters to. 1.468
   to 1.444.
6. **The tables on the longest chains stay unpaired** (`xray::SPARE_DEEP_PERCENT`,
   `PairOptions::spare_deep`). A `LUT6_2` gives up `A6`, the input that costs
   nothing, and holds both halves in one site; Vivado given the same netlist parts
   them. A table on a chain of tables at least 75 % as long as the longest is left
   alone. Against the build before, which still had the premium on fast inputs
   tried below, the build kept (sparing, no premium): `adders` 7.47 to 6.45 ns,
   `crc32` 5.50 to 4.61, `analyser_fifo`'s read clock 7.36 to 6.37; 1.444 to 1.437,
   and 1.435 with the premium kept too.

Tried and not kept, each over three or five seeds, each within a seed's spread of
the build it was tried on: weighing the anneal's connections again at every
temperature (VPR's way), 1.578 against 1.532; a fully critical connection's weight
doubled, 1.544; criticality to the power one rather than three, 1.547; six timing
rounds rather than three, 1.529; a timing round started wider and hotter (16 tiles,
40 % acceptance), 1.488 against 1.492; keeping the placement of the round the
estimate liked best, 1.541 against 1.532, which says the estimate a round ends on is
no guide to Vivado's figure at this scale; a premium on a table's fast inputs for an
uncritical connection, 1.514 against 1.532 alone and nothing measurable beside the
claim (1.435 with, 1.437 without); the claim from 0.1, 1.477; sparing from 50 %, no
better than 75; and pairing none at all (`--pair-tables needed` on designs this
small), within a seed of sparing the long chains, which keeps the pairing a dense
design needs.

### Before and after

Medians over five seeds, each seed's figure beside it (ns). `before` is 04b70219,
`after` this change; "Vivado's, Reticle's netlist" is `before`'s seed 1:

| design | clock | Vivado | Vivado's, Reticle's netlist | before | after |
|---|---|---|---|---|---|
| adders | clk | 2.042 | 4.924 | 7.431 (7.23 8.24 7.43 8.24 7.16) | 6.450 (6.29 6.13 6.53 6.45 6.72) |
| analyser_fifo | rd_clk | 3.887 | 5.589 | 7.606 (7.91 7.61 7.17 7.35 8.07) | 6.365 (6.37 7.16 6.90 6.10 6.04) |
| analyser_fifo | wr_clk | 2.417 | 2.740 | 3.745 (3.98 3.67 3.75 3.86 3.73) | 3.782 (3.78 3.71 3.87 3.67 3.92) |
| barrel | clk | 2.376 | 2.431 | 2.894 (2.96 3.25 2.88 2.89 2.88) | 3.013 (2.87 3.01 3.13 2.76 3.35) |
| bram_fifo | clk | 3.388 | 3.530 | 4.058 (3.94 4.21 3.80 4.28 4.06) | 4.038 (4.04 4.05 3.90 4.16 3.90) |
| carry_counter | clk | 1.855 | 1.944 | 2.455 (2.35 2.26 2.55 2.69 2.46) | 2.466 (2.40 2.49 2.48 2.42 2.47) |
| counter64 | clk | 3.220 | 3.401 | 5.896 (6.24 6.49 4.37 5.90 5.36) | 4.266 (4.10 4.30 4.05 4.66 4.27) |
| crc32 | clk | 2.234 | 3.807 | 5.405 (5.31 5.41 5.35 5.56 5.55) | 4.612 (4.59 4.79 4.88 4.61 4.52) |
| esad | adc_sampleclk | 2.131 | 3.788 | 3.334 (3.92 4.15 3.09 3.15 3.33) | 2.974 (3.58 2.97 2.82 3.23 2.91) |
| esad | clk_usb | 1.727 | 2.523 | 2.972 (2.97 3.25 2.79 2.97 3.00) | 2.499 (2.49 3.14 2.69 2.36 2.50) |
| fir8 | clk | 9.142 | 9.164 | 9.161 (9.16 9.16 9.16 9.16 9.16) | 9.161 (9.16 9.16 9.16 9.16 9.16) |
| fsm | clk | 1.903 | 2.392 | 3.031 (3.03 2.98 3.28 3.02 3.08) | 2.947 (2.75 2.96 2.95 2.85 3.23) |
| gray_cdc | clk | 1.466 | 1.438 | 1.547 (1.55 1.55 1.55 1.50 1.58) | 1.503 (1.41 1.50 1.50 1.43 1.50) |
| gray_cdc | clk2 | 1.267 | 1.513 | 1.670 (1.67 2.04 1.61 1.73 1.59) | 1.736 (1.54 1.74 1.74 1.60 1.74) |
| lutram32 | clk | 1.894 | 1.807 | 2.261 (2.38 2.22 2.34 2.26 2.26) | 2.267 (2.31 2.16 2.35 2.25 2.27) |
| mul18 | clk | 1.935 | 1.969 | 2.421 (2.47 2.27 2.50 2.42 2.38) | 2.484 (2.48 2.27 2.51 2.57 2.47) |
| regfile_alu | clk | 5.511 | 7.223 | 10.139 (9.49 9.70 10.91 10.48 10.14) | 9.142 (8.93 9.61 9.14 8.85 9.61) |
| sad | adc_sampleclk | 2.207 | 4.240 | 4.316 (4.32 4.25 4.91 4.31 5.10) | 3.549 (3.55 3.48 3.38 3.67 3.81) |
| sad | clk_usb | 1.726 | 2.649 | 3.809 (3.10 2.83 4.39 3.81 3.87) | 2.588 (2.52 2.75 2.59 2.62 2.57) |
| srl_delay | clk | 1.645 | 1.846 | 2.039 (2.09 2.01 2.01 2.05 2.04) | 1.871 (1.87 1.97 2.03 1.81 1.87) |
| widemux | clk | 2.438 | 2.745 | 4.123 (4.15 4.12 4.22 4.09 3.90) | 3.588 (3.40 4.41 3.59 3.45 3.84) |

**Over the twenty-one clocks, Reticle's median is 1.576 times Vivado's own figure
before and 1.437 after, and 1.244 times Vivado's placement and routes of the same
netlist before and 1.134 after.** Better by more than a seed's spread: `adders`,
`analyser_fifo`'s read clock, `counter64`, `crc32`, `esad` and `sad` both clocks,
`regfile_alu`, `srl_delay`, `widemux`. Worse, all within their seeds' spread:
`barrel` (2.89 to 3.01), `gray_cdc`'s `clk2` (1.67 to 1.74), `mul18` (2.42 to 2.48),
`analyser_fifo`'s write clock (3.75 to 3.78). The clocks the fitted model's first
placement did worse on than the legacy model's ("What it does to placement") were
`barrel`, `widemux`, `gray_cdc`'s `clk`, the write clock and `sad`'s `clk_usb`;
`widemux`, `gray_cdc`'s `clk` and `clk_usb` are better here, the other two not.

The rest of the gap to Vivado's own figure is the netlist: `fir8` is the same
9.16 ns in both tools' placements, and `adders`, `crc32` and `regfile_alu` have 15,
6 and 19 levels on their latest paths where Vivado's synthesis makes 8, 3 and 7 to
11, from subtractions this flow keeps off the carry chain
(`PrimitiveOptions::min_subtract_carry_width`, for a failure on the part) and its
own covering. *Measured since, and mostly closed*: "Synthesis against Vivado's",
below, counts both tools' netlists by Vivado's counting (`adders` was fifteen tables
and no `CARRY4` on its worst path against Vivado's one and eight) and takes the
geometric mean from 1.437 to 1.274.

### The Husky's bitstreams

`examples/husky/build_plus.sh` with this tree writes
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`, seed 4's,
byte for byte. **Checked.** Each change above is off under `--delay-model legacy`:
the pads are timed in every domain, the branch charge is zero, the claim never, the
estimate's weight the router's own, the first pass uncritical and in netlist order,
and `related` pairs by inputs alone.

### Tests

`tests/fpga_xray_achieved.rs` reads `achieved.txt`: every clock has Vivado's figure
and five seeds of each build, and after is at least 5 % under before over the corpus
with no clock more than 5 % worse. That catches a regeneration that lost the result
and not the flow moving on since, which only `achieved.sh` asks. Unit tests hold
each change: an unconstrained pad untimed under the fitted model and timed under
the legacy one (`an_input_delay_launches_its_pad_late_in_its_clocks_domain`); a
branch charged to the sinks past it (`a_pass_charges_a_branch_to_the_sinks_past_it`);
a critical connection taking a held fast input, and a slack one not
(`a_critical_connection_claims_a_held_fast_input`); the order of a pass and the
estimate's weight (`critical_signals_go_first_and_weigh_their_estimate_as_told`);
and a chain's tables spared while shallow ones pair
(`a_table_on_the_longest_chain_is_spared_its_partner`). **Mutations**, eight, each
made by a script and put back, each failing its test: the claim never made, and
made by every search; the branch charged to every pip; a chain counted from one end;
the deep tables offered as partners; the fitted model timing an unconstrained pad;
the order reversed; and the estimate's weight given to an uncritical search. Swapping
the fixture's `before` and `after` fails `tests/fpga_xray_achieved.rs`.

### Not done

- **The placement.** It is most of what is left on `adders`, `regfile_alu` and
  `analyser_fifo` (1.1 to 1.5 ns), and nothing tried on the anneal's weighting moved
  it beyond a seed. *Measured since, in "Packing against Vivado's": that ripple is the
  legacy netlist's, and what is left is flip-flops, eight a slice in Vivado's and four
  in Reticle's.* Vivado packs a ripple of tables four to a slice where Reticle
  puts two to a tile; what the placer would need to know of a slice's own wires for
  that is not looked at. *Measured since, in "Filling slices": with today's netlists
  Reticle's placements routed by Vivado are within about 1 % of Vivado's placement of
  the same netlist, and the rest is the routes.*
- **`widemux`'s routes**, 0.3 ns from Vivado's on the same placement and pins: a
  select read by 257 pins.
- Clock skew is still not modelled; Vivado's is up to 0.18 ns on these designs, as
  above.
- The seeding's criticality is the placement's straight line, and is not checked
  against the first pass's own.
- Vivado's lookup tables' arcs are 0.097 ns on its own placements of the paths read
  here and 0.2 to 0.26 on a quarter of Reticle's, by nothing found: over the timing
  run's 18,000 arcs not the physical pin (a quarter slow on each of `A1` to `A5`, a
  third on `A6`), the half of a pair, the fanout or the input's own delay, and not
  the edge, rise and fall being equal; nor what the fit checked before. The model
  charges the mean, 0.13.
- None of it has been on a part, and no Husky build has been made with the fitted
  model.

## Synthesis against Vivado's

**Asked: what does Vivado's synthesis make of each design of the timing corpus,
what does Reticle's, and what does closing the difference do to the clock?** The
section before left most of the gap to Vivado's own flow in the netlist: `adders`
fifteen tables deep where Vivado's is one and eight `CARRY4`s, `crc32` seven where
Vivado's is three, `regfile_alu` eighteen. Run on 2026-11-06, at
`xc7a100tftg256-3`, with Vivado 2026.1. **Everything here is Vivado's synthesis and
Vivado's timer; nothing has been on a part.**

`tools/vivado-parity/netlist_qor.sh` counts a netlist by Vivado's own counting
(`netlist_qor.tcl`: its cells by primitive group, and `report_design_analysis`'s
levels of logic over each clock's 5,000 worst paths): `synth` runs `synth_design`
on a design, `netlist` links a Reticle build's netlist (`--vivado-placement`)
unplaced. `testdata/fpga/vivado/qor/netlist.txt` keeps the counts, and
`achieved.txt` the periods of a third build, `synthesis`, by `achieved.sh` as
before. `achieved.sh` now skips a stage whose result is finished, so a second build
is timed beside the first's Vivado runs without asking Vivado again.

### What Vivado does with one shape at a time

Asked of a registered `a OP b` alone at each width (`synth_design -mode
out_of_context`), the cells it made. **Measured**:

| Shape | In tables | On the chain |
|---|---|---|
| `a - b` | 3 to 5 bits | 6 and above, a `LUT2` a bit |
| `a < b`, `<=`, signed or not | 3 to 6 bits | 7 and above: two bits a lane, a `LUT4` for each bit (the lane's `>` and its equality), a `CARRY4` for four lanes |
| `a == b`, `!=` | 3 to 8 bits | 9 and above: three bits a lane, a `LUT6` each |
| `a < 1000`, `a == 1000` (against a constant) | every width to 32 | never |
| `a + 1` | 5 to 11 bits | 24 bits |
| `(a + b) == c` | 3 to 5 bits | 6 and above, the sum's chain and the equality's |

Reticle had subtractions in tables at every width
(`PrimitiveOptions::min_subtract_carry_width`, off for the Husky Plus's reason
below), orderings in tables at every width, and equalities on the chain only against
a sum or a difference, and only from sixteen bits in kept lanes.

### Where the netlists differed

Each design's cells (tables / `CARRY4` / flip-flops) and its worst path's levels as
tables + `CARRY4`s, all by Vivado's counting; Reticle's are seed 1's netlist of the
`after` build of the section before (`before`, which is what `--synthesis legacy`
builds, byte for byte on all seventeen) and of the default now (`after`). Reticle's
tables include the buffers `xray::legalise_carries` puts in front of a chain.

| design | Vivado | before | after |
|---|---|---|---|
| adders | 231 / 56 / 481, 1+8 | 289 / 40 / 481, 15+0 | 231 / 56 / 481, 1+8 |
| analyser_fifo `rd_clk` | 131 / 26 / 221, 6+1 | 237 / 12 / 213, 14+1 | 253 / 26 / 213, 5+3 |
| analyser_fifo `wr_clk` | 2+2 | 7+0 | 3+2 |
| barrel | 103 / 0 / 70, 3 | 105 / 0 / 70, 3 | 87 / 0 / 70, 3 |
| bram_fifo | 49 / 3 / 68, 3 | 60 / 9 / 68, 4+1 | 55 / 12 / 68, 3 |
| carry_counter | 12 / 11 / 65, 0+8 | 49 / 8 / 65, 4 | 55 / 11 / 65, 2+3 |
| counter64 | 23 / 22 / 129, 1+6 | 98 / 16 / 129, 5 | 109 / 22 / 129, 2+6 |
| crc32 | 129 / 0 / 65, 3 | 204 / 0 / 65, 7 | 191 / 0 / 65, 2 |
| esad `adc_sampleclk` | 1046 / 68 / 1078, 2 | 1048 / 59 / 1084, 3 | 1187 / 93 / 1085, 3 |
| esad `clk_usb` | 3 | 3+1 | 3+1 |
| fir8 | 7 / 0 / 23, 0 | 9 / 0 / 23, 0 | 7 / 0 / 23, 0 |
| fsm | 33 / 0 / 30, 3 | 72 / 0 / 30, 4 | 72 / 0 / 30, 3 |
| gray_cdc `clk` | 15 / 3 / 37, 0+3 | 26 / 3 / 37, 1+3 | 26 / 3 / 37, 1+3 |
| lutram32 | 7 / 0 / 70, 2 | 10 / 0 / 70, 3 | 8 / 0 / 70, 2 |
| mul18 | 7 / 0 / 36, 2 | 9 / 0 / 36, 3 | 7 / 0 / 36, 2 |
| regfile_alu | 351 / 20 / 20, 6 (12 most) | 422 / 8 / 20, 18 | 504 / 20 / 20, 7+7 |
| sad `adc_sampleclk` | 1414 / 132 / 1018, 2 | 1553 / 108 / 1272, 3 | 1847 / 174 / 1273, 3 |
| srl_delay | 7 / 0 / 34, 2 | 9 / 0 / 34, 3 | 7 / 0 / 34, 2 |
| widemux | 275 / 0 / 1049, 2 | 276 / 0 / 1047, 2 | 276 / 0 / 1047, 2 |

`adders` is now Vivado's netlist cell for cell, and `fir8`, `mul18` and
`srl_delay` its count of tables; `crc32` one table shallower than Vivado's and 62
tables larger. Where `after` has more tables than `before` (`analyser_fifo`,
`carry_counter`, `counter64`, `esad`, `sad`, `regfile_alu`) the most of them are
chains' buffer tables: a lane whose propagate is a flip-flop's output, which Vivado
makes without one, gets a table of its own here (`counter64`'s 22 `CARRY4`s and 23
tables against Reticle's 22 and 109). That is the packing's, and is not touched.

### What was changed

All behind `--synthesis vivado`, the default on the 7 series
(`FpgaOptions::vivado_synthesis`); `--synthesis legacy` is the netlist before,
byte for byte (checked on the seventeen designs, and the Husky's bitstreams below).
In the order they were made:

1. **Subtractions on the chain from six bits**, Vivado's threshold
   (`flow::VIVADO_MIN_SUBTRACT_CARRY_WIDTH`), by the mapping that was there off
   (`subtract_on_the_chain`).
2. **Orderings between two signals on the chain from seven bits**
   (`magnitude_on_the_chain`, new), Vivado's shape: two bits a lane, the lane's `>`
   its generate and its equality its propagate, written as the adder the carry
   element maps (`x = gt`, `y = ge`, so `x ^ y` is the equality and where they agree
   the carry is the `>`); `a < b` is `b > a`, `>=` starts from a one, and a signed
   ordering is the unsigned one with the sign bits flipped.
3. **Equalities between two signals on the chain from nine bits**
   (`compare_any_on_the_chain` over `compare_on_the_chain`), against a sum or not;
   neither an equality nor an ordering against a constant goes there, as Vivado's
   do not.
4. **A chain's ends on the `CARRY4`'s own pins** (`MapOptions::carry_ends`): a
   constant carry in that both operands spelled as a low lane (`{a, 1} + {~b, 1}`)
   on `CYINIT`, and a top sum of `{0, x} + {0, y}` off `CO`. An eight-bit `a > b` was
   two `CARRY4`s where Vivado has one. *A carry in that is a signal is not taken* (until
   2026-11-07: "Packing against Vivado's", change 2, takes it):
   the shared adders' `s ? c - d : a + b` has one, and `xray::legalise_carries`
   refused it — bit 0's generate wants the same `AX` pin the signal would take.
   Vivado puts it there, its generate on `O5`; this flow would have to put it there
   first.

   1 to 4 together, measured alone (the column `arith` below): the geometric mean
   from 1.437 to 1.324, `adders` 6.45 to 2.43 ns, `regfile_alu` 9.14 to 6.88,
   `analyser_fifo`'s write clock 3.78 to 3.06.
5. **Each node's cuts kept shallowest first** (`CutOptions::depth_priority`). The
   priority cuts kept each size its quota and nothing for depth; half of each list
   now goes to the shallowest. `fsm` four tables deep to three, `widemux` three to
   two, `regfile_alu` eight to six, `esad` and `sad` five to four.
6. **XOR logic collapsed and rebuilt** (`aig::xor`, `MapOptions::xor_balance`).
   An XOR is three AND nodes the AND balancing cannot see through, so a CRC reaches
   the mapper as the chain it was written as. Each linear XOR's value is the parity
   of a set of leaves; the ones something else reads are rebuilt from their sets,
   the pairs two or more share first (Paar's algorithm) while a pair costs no set
   its shallowest tree of tables, then each set as that tree, Golumbic's rule:
   `k` signals to a table, the first table taking only as many as make the rest
   whole. `crc32` from seven tables deep to two; `srl_delay`'s 32-input parity, and
   `fir8`'s and `mul18`'s, Vivado's seven tables two deep, and `lutram32`'s two deep. Three
   versions were measured on the way, `crc32` each time:
   - balancing each XOR tree alone, as `balance` does an AND: no change at all, every
     XOR of a CRC being shared;
   - collapsed, and rebuilt as a tree of pairs, shallowest first: 145 tables three
     deep, 2.81 ns;
   - rebuilt for tables, a pair taken if it costs a set one table more: 151 tables
     three deep, 2.91 ns; costing none (kept): 191 tables two deep, 2.68 ns.
7. **An input a hard block drives arrives a table late**
   (`MapOptions::late_hard_outputs`): a carry chain's sum, a multiplier's or a
   memory's output is no longer taken into the first table of a parity as readily
   as a flip-flop's.
8. **A node read in both polarities is two tables over its leaves**
   (`CutOptions::duplicate_complements`), not a table and an inverter after it: as
   many tables, a level less. `analyser_fifo` had ten inverters, several of them a
   chain's propagate.
9. **An ordering against a constant in kept lanes of six bits**
   (`ordering_in_lanes`): each lane's `>` and equality one table, and the chain over
   the lanes in another. Left to the optimiser, `rused_next <= PROG_EMPTY` at the end
   of `analyser_fifo`'s read clock was an AND-OR chain three tables deep; it is two.

The mapping is proved as before (`techmap::verify`), now against the graph from
before the XOR rebuilding, so the proof covers it too. **A rebuilt CRC's proof did
not finish** — `fraig` under its conflict limit left 2,092 AND nodes of `crc32` —
so the miter is first reduced by linear algebra (`verify::merge_linear`): a node
that evaluates, on its own two-level cone, as the XOR of two nodes is the parity of
their sets, and two nodes that are the same parity of the same nodes are merged.
That is exact, and independent of `aig::xor`'s own reading of an XOR. Every one of
the seventeen is proved.

Tried and not kept: **area recovery against required times from the deepest
output**, written as an option, measured and taken out again. It saved a
few tables in a thousand, and it cost `analyser_fifo` a level in its gray code just
before a subtraction: the logic before a chain looks to the mapper like logic before
a flip-flop, with the slack of the deepest output. Over five seeds it was 1.266
against 1.274 kept, the clocks moving both ways by a seed's spread (`analyser_fifo`'s
read clock 5.25 to 5.60 ns, `esad`'s `adc_sampleclk` 3.03 to 2.79).

### Before and after

Medians over five seeds, each seed's figure beside it (ns). `legacy` is the `after`
build of the section before, what `--synthesis legacy` builds; `arith` changes 1 to
4 alone; `synthesis` all of them, the default. "Vivado's, Reticle's netlist" is
Vivado's placement and routes of `synthesis`'s seed-1 netlist:

| design | clock | Vivado | Vivado's, Reticle's netlist | legacy | arith | synthesis |
|---|---|---|---|---|---|---|
| adders | clk | 2.042 | 2.238 | 6.450 (6.29 6.13 6.53 6.45 6.72) | 2.429 (2.57 2.43 2.38 2.45 2.40) | 2.337 (2.34 2.33 2.45 2.31 2.53) |
| analyser_fifo | rd_clk | 3.887 | 5.091 | 6.365 (6.37 7.16 6.90 6.10 6.04) | 5.897 (6.05 5.70 6.00 5.69 5.90) | 5.248 (5.53 5.20 5.62 5.24 5.25) |
| analyser_fifo | wr_clk | 2.417 | 2.983 | 3.782 (3.78 3.71 3.87 3.67 3.92) | 3.059 (3.06 3.11 3.12 3.04 3.05) | 3.036 (3.26 2.97 3.04 2.95 3.26) |
| barrel | clk | 2.376 | 2.780 | 3.013 (2.87 3.01 3.13 2.76 3.35) | 3.013 (2.87 3.01 3.13 2.76 3.35) | 2.948 (2.96 2.98 2.75 2.95 2.85) |
| bram_fifo | clk | 3.388 | 3.598 | 4.038 (4.04 4.05 3.90 4.16 3.90) | 3.931 (4.09 3.93 3.94 3.88 3.87) | 3.945 (3.95 3.93 4.02 3.91 3.94) |
| carry_counter | clk | 1.855 | 2.352 | 2.466 (2.40 2.49 2.48 2.42 2.47) | 2.407 (2.37 2.62 2.35 2.41 2.73) | 2.385 (2.38 2.38 2.32 2.38 2.44) |
| counter64 | clk | 3.220 | 3.721 | 4.266 (4.10 4.30 4.05 4.66 4.27) | 3.953 (4.05 4.03 3.95 3.83 3.87) | 4.040 (3.98 4.14 4.04 3.83 4.06) |
| crc32 | clk | 2.234 | 2.331 | 4.612 (4.59 4.79 4.88 4.61 4.52) | 4.612 (4.59 4.79 4.88 4.61 4.52) | 2.676 (2.75 2.84 2.62 2.65 2.68) |
| esad | adc_sampleclk | 2.131 | 3.506 | 2.974 (3.58 2.97 2.82 3.23 2.91) | 2.993 (2.96 3.13 3.10 2.92 2.99) | 3.033 (3.16 3.12 2.87 2.96 3.03) |
| esad | clk_usb | 1.727 | 2.422 | 2.499 (2.49 3.14 2.69 2.36 2.50) | 2.374 (2.57 2.37 2.62 2.33 2.35) | 2.443 (2.43 2.51 2.54 2.44 2.31) |
| fir8 | clk | 9.142 | 9.164 | 9.161 (9.16 9.16 9.16 9.16 9.16) | 9.161 (9.16 9.16 9.16 9.16 9.16) | 9.247 (9.25 9.16 9.25 9.25 9.16) |
| fsm | clk | 1.903 | 2.444 | 2.947 (2.75 2.96 2.95 2.85 3.23) | 2.947 (2.75 2.96 2.95 2.85 3.23) | 2.887 (2.96 2.62 2.80 2.89 2.95) |
| gray_cdc | clk | 1.466 | 1.447 | 1.503 (1.41 1.50 1.50 1.43 1.50) | 1.503 (1.41 1.50 1.50 1.43 1.50) | 1.503 (1.41 1.51 1.50 1.50 1.51) |
| gray_cdc | clk2 | 1.267 | 1.572 | 1.736 (1.54 1.74 1.74 1.60 1.74) | 1.736 (1.54 1.74 1.74 1.60 1.74) | 1.615 (1.61 1.58 1.85 1.85 1.52) |
| lutram32 | clk | 1.894 | 1.814 | 2.267 (2.31 2.16 2.35 2.25 2.27) | 2.267 (2.31 2.16 2.35 2.25 2.27) | 2.205 (2.29 2.18 2.37 2.15 2.21) |
| mul18 | clk | 1.935 | 1.903 | 2.484 (2.48 2.27 2.51 2.57 2.47) | 2.484 (2.48 2.27 2.51 2.57 2.47) | 2.309 (2.31 2.30 2.53 2.40 2.29) |
| regfile_alu | clk | 5.511 | 5.797 | 9.142 (8.93 9.61 9.14 8.85 9.61) | 6.882 (6.92 6.88 7.05 6.73 6.75) | 6.395 (6.72 6.39 6.96 6.16 6.31) |
| sad | adc_sampleclk | 2.207 | 3.928 | 3.549 (3.55 3.48 3.38 3.67 3.81) | 3.512 (3.35 3.56 3.51 3.73 3.48) | 3.533 (3.53 3.41 3.74 3.52 3.75) |
| sad | clk_usb | 1.726 | 2.485 | 2.588 (2.52 2.75 2.59 2.62 2.57) | 2.633 (2.63 2.48 2.68 2.65 2.59) | 2.787 (3.79 2.79 2.54 2.79 2.52) |
| srl_delay | clk | 1.645 | 1.723 | 1.871 (1.87 1.97 2.03 1.81 1.87) | 1.871 (1.87 1.97 2.03 1.81 1.87) | 2.027 (2.02 2.09 1.95 2.03 2.09) |
| widemux | clk | 2.438 | 2.709 | 3.588 (3.40 4.41 3.59 3.45 3.84) | 3.588 (3.40 4.41 3.59 3.45 3.84) | 3.485 (3.48 3.62 3.53 3.28 3.47) |

**Over the twenty-one clocks, Reticle's median is 1.274 times Vivado's own figure,
against 1.437 with the legacy synthesis.** Vivado placing and routing Reticle's
netlist, the measure of the netlist alone, went from 1.279 times its own flow to
1.186: `adders` 2.24 ns against its own 2.04, `crc32` 2.33 against 2.23. Better by
more than a seed's spread: `adders`, `analyser_fifo` both clocks, `crc32`,
`regfile_alu` (9.14 to 6.40), `gray_cdc`'s `clk2`, `mul18`. Worse: `srl_delay`, 1.87
to 2.03 ns, on Vivado's own seven tables two deep, a placement's longer wires (0.83
and 0.60 ns on its two connections); `sad`'s `clk_usb`, 2.59 to 2.79, one seed at
3.79; `fir8`, 9.16 to 9.25 on three seeds of five.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--synthesis legacy`, beside
`--delay-model legacy` and `--memories legacy`, for the reason those are there: every
build of them measured on a board was made without it. Subtractions on the chain are
the one part of it that has been on a board, and the Husky Plus's captures went
wrong with them for a reason not found (`MapOptions::min_subtract_carry_width`). The
scripts' bitstreams are byte for byte those before: with this tree's release binary
`build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and
`build_plus.sh` seed 4's
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked.**

### Tests

`every_block_maps_to_the_logic_it_was_mapped_from` (`tests/ip_library.rs`) proves
every block of the IP library at LUT6 with the mapper's options as the 7 series sets
them, beside the four ways it proved them before. `tests/fpga_xray_synthesis.rs` maps
the fifteen designs of the corpus in this repository both ways, proves every mapping, and pins each one's tables, their depth
and its `CARRY4`s (the legacy figures are the ones before this section, so a leak
of the new mapping into the old fails it); it holds `adders`, `crc32`, `fsm` and
`regfile_alu` no deeper in tables than Vivado's worst path's levels, and
`netlist.txt` whole. `tests/fpga_carry.rs` simulates every input of the orderings
(each of four, signed and not, at 7, 8 and 9 bits on the chain and 7 to 16 against
constants) and of each shape with its carry ends on the pins, legalised and paired,
and proves them at 16 and 32 bits. `tests/fpga_xray_achieved.rs` holds `synthesis`
at least 5 % under `legacy` over the corpus and no clock more than 10 % over.
`the_analyser_fifo_is_as_fast_as_vivado_makes_it`, a target `#[ignore]`d since
2026-11-02, passes and is no longer ignored; with the legacy synthesis it still
fails, `wused_q[14]` at 3.72 ns against Vivado's 1.929 (checked). Unit tests hold the XOR rebuilding (a
chain to Golumbic's tree, shared chains collapsed, a pair cancelling, a parity too
large to collapse read with its polarity, Golumbic's depth), the linear proof (two
associations of a parity merged; a parity and its complement, an AND beside an XOR,
and an AND two levels over its inputs, not), and the duplicated complement.

**Mutations**, fourteen, each made by a script and put back, each failing its test:
`<` with its operands not swapped, `>=` started from zero, the sign bits not flipped
on the chain, the carry out read off the wrong lane, a signal carry in put on
`CYINIT` (`legalise_carries` refused it; since 2026-11-07 that is where it goes, the
generate on `O5`), the lowest constant lane's `>` taken for
`>=`, a rebuilt leaf's polarity dropped, Golumbic's first table taking a whole
table's worth, the leaves' depths ignored, the depth priority ignored, complements
not duplicated, an AND taken for a parity by the linear proof, the linear proof gone
(`crc32` unproved), and a hard block's output arriving at zero. Three of them first
passed, and the tests that catch them were written then: the polarity, the AND, and
the depths, which only the pinned figures catch — a late leaf's place in a parity
is visible to the mapper's cuts however the tree is drawn, and no unit test found
one that a single graph shows.

Two tests changed, each to what is now so: `fpga_xray_muxf.rs`'s `mux32` was a
cover unlike Vivado's (nine tables, four `MUXF7` and two `MUXF8` where Vivado has
none) and is now Vivado's slice for slice; `fpga_xray_lutram_vivado.rs`'s `mem32x8`
was Vivado's slices line for line, its two `RAM32X1D` in one slice, and is now a
slice each — nothing in the placer asks for the two in one, the anneal had put them
there, and the read word's parity two tables deep moved it. (Vivado's slice for slice
again since 2026-11-07, by the anneal again: "Packing against Vivado's".)

### Not done

- **`fsm`'s area**: 72 tables to Vivado's 33, the same flip-flops. Vivado re-encodes
  the machine (`inferred FSM for state register`); this flow does not here.
  *That reading was wrong*: Vivado's log infers the machine and does not re-encode
  it, and its five flip-flops keep their codes. The 33 tables are the next-state
  logic as a tree of multiplexers, and this flow's is 38 now; "Area against
  Vivado's" has both.
- **`regfile_alu`'s and `sad`'s area**: 504 tables to 351, 1,847 to 1,414, and the
  chains' buffer tables above. `sad`'s 64 orderings of eight and nine bits went onto
  the chain as Vivado's do, and it is larger for it and no faster.
- *Done since, in "Packing against Vivado's" (change 2): `esad` 77 `CARRY4`s, `sad` 142.*
  **A signal carry in on `CYINIT`**, for the shared adders: `legalise_carries` would
  need the lane's generate on `O5` first. `esad`'s 256 counters are three `CARRY4`s
  each to Vivado's two.
- **`a + 1` in tables to eleven bits**, as Vivado keeps it; the adder threshold is
  still four.
- **A hard block's output arrives one table late whatever it is**; a chain's top sum
  is later than its bottom one.
- **The logic before a chain has no required time of its own**, which is why the
  required times from the deepest output were not kept.
- Ordering a table's members by their estimated depth, so that a late one is an
  XOR from the root, was measured and not kept: `bram_fifo` 3.95 to 4.17 ns and
  `regfile_alu` 6.40 to 6.57, every other design the same netlist.
- None of it has been on a part.

## Area against Vivado's

**Asked: where do the tables go, design by design, between Vivado's synthesised
netlist and Reticle's, and how much of the difference can be closed without losing
the depth the section before reached?** Its figures left `fsm` at 72 tables to
Vivado's 33, `regfile_alu` 504 to 351, `sad` 1,847 to 1,414 and `analyser_fifo` 253 to
131. Run on 2026-11-07, at `xc7a100tftg256-3`, with Vivado 2026.1. **Everything here
is Vivado's synthesis and Vivado's timer; nothing has been on a part.**

`tools/vivado-parity/netlist_blame.py` reads a structural netlist — Vivado's, written
with `write_verilog` after `synth_design`, or Reticle's `x.v` — and charges each table
to the registers its output reaches through tables, chains and wide multiplexers,
split evenly where it reaches several (`blame`); `count` has the tables by size and
the buffers in front of a chain, and `cone` what feeds a cell. Vivado's netlists were
written flattened (`-flatten_hierarchy full`) for that; their counts are the default
synthesis's, checked on `analyser_fifo`, `bram_fifo`, `esad` and `sad`.

### What Vivado's log says it did

**Measured**, from the full `synth_design` log and the netlist:

- **`fsm` is not re-encoded.** The log has `INFO: [Synth 8-802] inferred FSM for
  state register 's_reg' in module 'fsm'` and nothing after it: no encoding table and
  no `Synth 8-3354 encoded FSM ... using encoding`. The netlist keeps `s_reg` as five
  flip-flops holding the codes as written: `out_reg[0]` to `out_reg[4]` read
  `s_reg[0]` to `s_reg[4]` directly, which they could not if the codes had changed,
  since the design shifts the state itself out. *The section before read "inferred
  FSM" as the machine being re-encoded, and said that was where `fsm`'s 39 tables
  more went. That was wrong:* the 33 tables are the same codes' next-state logic, built
  as a tree of multiplexers over the state's bits (each bit's last table a choice on
  `s[4:3]` among three subtrees of the low bits and the inputs, and zero for the
  codes no state has).
- **When the state is only compared, it is.** The same machine with
  `out <= {out[14:0], s == 23 || s == 9}`, the state read nowhere else, gets the
  table and `encoded FSM with state register 's_reg' using encoding 'sequential'`:
  24 states renumbered in the order it reached them (`00111` is `00001`, `10011` is
  `00011`, and so on), still five bits, 37 tables. Reticle makes 39 of that one
  without re-encoding anything (below).
- No other design of the corpus has a `Synth 8-802` line.
- **`sad`'s reference samples are shift registers**: eight `SRLC32E` and eight
  flip-flops for the 32 bytes of `refsamples`, which Reticle had as 256 flip-flops,
  each behind a table; and 31 `Unused sequential element ... was removed` (`Synth
  8-6014`), the copies `nextrefsample_r[i]` of `nextrefsample[i+1]`, which Reticle's
  synthesis merges as well.

### Where the tables went

Charged by `netlist_blame.py blame`, the groups that differ; Reticle's `before` is
the default synthesis at the start of this section, seed 1's netlist, its tables
including the ones `xray::legalise_carries` adds in front of a chain:

| design | registers | Vivado | before |
|---|---|---|---|
| sad | `refsamples` | 9 | 249 |
| | `counter_incr` (32 of them) | 512 | 576 |
| | `sad_counter` | 352 | 384 |
| | `individual_trigger` | 65 | 96 |
| esad | `counter_incr` (16) | 256 | 288 |
| | `sad_counter` | 176 | 208 |
| | `individual_trigger` | 32 | 48 |
| analyser_fifo | `prog_empty_q` | 38.5 | 60.5 |
| | `stored` | 9.5 | 28.5 |
| | `wptr_inc` | 1.3 | 22.8 |
| | `dout` | 4 | 22 |
| | `wptr` | 0.3 | 15 |

And what each is, read off the cells (`cone`):

- **`refsamples`** is a 32-byte shift written as a `for` loop over part selects
  inside an `if` inside an `if`. Process lowering leaves the register's own bits
  rearranged by the loop's partial updates in the arm the inner `if` fails, and the
  enable lifted from the outer `if` already says that arm is never loaded; but it is
  not the register's `q`, so it was a table on every bit, and the chain was not a
  shift register.
- **`counter_incr`**, `decision ? rpr - ref : rmr + ref`, is one adder on the chain
  both ways. Vivado's is eight lanes, each a `LUT4` propagate and a `LUT3` generate,
  and `decision` on `CYINIT`. Reticle's is nine: the carry in a lane of its own
  (`{a, 1} + {~b, 1}`), whose propagate is a constant table — the signal carry in on
  `CYINIT` that "Not done" of the section before has. Two tables a counter.
- **`individual_trigger`** is `sad_counter <= threshold && triggerer && ready &&
  !(triggered && !multiple_triggers)`. Vivado computes the last term once for all 32
  and is two tables a counter; the cover here is three.
- **`stored`**, `wptr_r_next != rptr_next` on the chain, is a lane table and then a
  buffer before each `CARRY4` lane: the chain's generate is a constant zero, which
  `legalise_carries` takes from `O5` of a table of its own. Vivado's lanes have a
  constant one there and are the table alone. The same is `counter64`'s 22 `LUT2`s
  and most of `carry_counter`'s.
- **`wptr_inc`** is `wptr_inc + 1`, fifteen propagates that are flip-flops' outputs,
  so fifteen `LUT1` buffers here and none in Vivado's netlist, whose placer makes
  them route-throughs. **`wptr`** is `wptr <= wptr_next` with `wptr_next = do_write ?
  wptr_inc : wptr`, a net the flags read too: a hold, but behind a net, so a table a
  bit where Vivado has the clock enable.
- **`dout`**, the parity of the memory's word, is eight block RAMs four deep and a
  read multiplexer before it, where Vivado's nine are 16K x 2 each and need none.
- **`fsm`** is the `case` of 24 states: decoded, each arm under `s == K` and the
  arms ORed, which a cut mapper finds nothing to share in.
- **`regfile_alu`** by pieces, each `regfile_alu` with arms of its `case` replaced
  (tables, Vivado's and this flow's with this section's changes): the logic alone
  39 and 39; the arithmetic and the logic 169 and 239; the shifts and the logic 261
  and 346; `a << b` alone 110 and 126, `a >> b` 98 and 121; and **the two behind one
  select, 167 and 257: Vivado shares one shifter between them**.
- `counter64` and `carry_counter` map to 24 and 13 tables against Vivado's 23 and
  12; the rest, 85 and 42, are `legalise_carries`'s buffers and constants. So are
  57 of `analyser_fifo`'s, 28 of `bram_fifo`'s, 11 of `gray_cdc`'s, 58 of `esad`'s
  and 75 of `sad`'s. Vivado's netlist has none; its placer puts route-throughs in the
  same tables' sites.

Measured and not where the area is, **the mapper**: with no depth bound at all on
its area recovery, case trees on, `regfile_alu` was 450 tables against 469 bounded,
`esad` 1,092 against 1,107 and every other design within three; 16 cuts a node
rather than 8 saved 15 on `regfile_alu` and 4 on `esad`, 32 saved 24 and 24 (before
the case trees); area flow over estimated references (ABC's `(2 * old + refs) / 3`,
from each node's fanout) saved 6 on `regfile_alu` and 3 on `crc32`. None of it was
kept: what is left is in what reaches the mapper.

### What was changed

Both behind `--synthesis vivado`, the 7 series' default; `--synthesis legacy`, the
ECP5 and the Gowin flows build what they did (the seventeen designs' legacy
netlists checked byte for byte, and the Husky's bitstreams below).

1. **What a register loads, as Vivado reads it** (`synth::opt::RegisterLoads`,
   `SynthOptions::register_loads`), two rules `FfOpt` does not have:
   - *A hold behind a net is an enable*: `d` a net that is `c ? v : q` — a
     multiplexer cell or a whole-net assignment — with `q` the register's own, is
     `en = c, d = v`. `analyser_fifo`'s `wptr`.
   - *What a register loads with its enable low is a don't-care*: a multiplexer in
     `d`, through concatenations and slices, whose select a conjunct of the enable
     decides, is that arm. `sad`'s reference samples: the arm goes, the chain is a
     shift register (`srl::extract`, as before), eight of them from 248
     flip-flops, as Vivado's. `sad` from 1,847 tables and 1,273 flip-flops to 1,607
     and 1,025; Vivado's 1,018.
2. **A `case` as a tree of multiplexers** (`aig::BlastOptions::case_trees`,
   `MapOptions::case_trees`): a `pmux` each of whose selects is `x == K`, one `x`
   of up to eight bits and distinct constants of its width naming at least half of
   its codes, is blasted as a tree over `x`'s bits with the default at every code no
   arm names, rather than as each arm under its decoded select. `fsm` from 72 tables
   to 38, three deep as before; `regfile_alu` from 504 to 470, and seven tables deep
   on its worst path to six, with no `CARRY4` on it, Vivado's own worst path. A
   sparse `case` (fewer arms than half the codes) stays decoded: with every `case` a
   tree, `sad` and `esad`, whose register reads are eight bits and nine arms, were
   five and three tables more.

**What is proved.** Neither changes what any register holds on any cycle, and the
proofs are of exactly that:

- *Register loads*: the next value of a register with an enable is `en ? d : q`,
  and both rules leave that function of the inputs and `q` as it was. That is how
  the post-synthesis check (`synth::verify`, `SynthOptions::verify_equivalence`)
  reads a register — sequentially, its registers matched by name, by induction from
  reset — and `tests/synth_register_loads.rs` proves both shapes with it: the
  for-loop shift of `refsamples` and the analyser's `wptr`. It is not run on every
  build; the rules are structural (a conjunct of the enable implying a select, read
  through `and`, `or` and `not` cells and expressions), so the proof is of the rules
  on their shapes, and the techmap proof of every build's mapping is against the
  netlist they leave.
- *Case trees*: combinational. The graph with trees has the same inputs and outputs
  as the graph blasted without, and whenever the mapping is proved
  (`--verify`, `FpgaOptions::verify_mapping`) the trees are proved against the
  `pmux` as blasted before (`verify::check_graphs`), beside the mapping's own proof,
  so the proof reaches back past the blaster. All seventeen designs prove;
  `every_block_maps_to_the_logic_it_was_mapped_from` maps the IP library with case
  trees and register loads in its 7-series variant and proves every block.
- No state is re-encoded, so there is no sequential argument about encodings to
  make: Vivado did not re-encode `fsm` either.

Tried and not kept, **measured** on `regfile_alu` over five seeds (Vivado's timer):

- *A `case`'s two-level trees kept for the slice's `MUXF7`*
  (`muxtree::MIN_LEVELS` at two for a `case`): a bit of the eight operations is a
  table for each half and a `MUXF7` between them, Vivado's cover in the arithmetic
  piece (169 tables there, 177 this way). 470 tables to 423, the period 6.336 to
  6.569 ns (6.82 6.99 6.57 6.55 6.55).
- *The two shifts sharing one shifter*, as Vivado does: the left arm reads `a`
  reversed into the right shift and reverses its output, the input chosen by the
  left arm's select. 423 tables to 302 with the `MUXF7`s, 365 without — fewer than
  Vivado's 351 — but eight tables deep where it was six, and 6.794 ns (6.79 7.13
  7.15 6.60 6.65) and 6.795 (6.59 6.70 6.83 6.83 6.79). And it was not proved: the
  graph check left 1,460 AND nodes of `regfile_alu`'s miter, the shared shifter
  equal to the two only under the select. A proof would split on the `case`'s
  subject, where each half folds to the same graph.

### Before and after

Each design's cells (tables / `CARRY4` / flip-flops) and, per clock, the most levels
of logic on its paths and its worst path's levels as tables + `CARRY4`s, all by
Vivado's counting (`netlist_qor.sh`); `before` and `after` are seed 1's netlists,
before this section and with it. The last column is `after`'s tables less
`legalise_carries`'s buffers and constants, which Vivado's netlist does not have:

| design | Vivado | before | after | after, less the chain's |
|---|---|---|---|---|
| adders | 231 / 56 / 481, 9 (1+8) | 231 / 56 / 481, 9 (1+8) | 231 / 56 / 481, 9 (1+8) | 231 |
| analyser_fifo | 131 / 26 / 221 | 253 / 26 / 213 | 238 / 26 / 213 | 181 |
| `rd_clk` | 7 (6+1) | 8 (5+3) | 9 (5+4) | |
| `wr_clk` | 5 (2+2) | 5 (3+2) | 5 (3+2) | |
| barrel | 103 / 0 / 70, 3 | 87 / 0 / 70, 3 | 87 / 0 / 70, 3 | 87 |
| bram_fifo | 49 / 3 / 68, 7 (3) | 55 / 12 / 68, 7 (3) | 55 / 12 / 68, 7 (3) | 27 |
| carry_counter | 12 / 11 / 65, 8 (0+8) | 55 / 11 / 65, 9 (2+3) | 55 / 11 / 65, 9 (2+3) | 13 |
| counter64 | 23 / 22 / 129, 16 (1+6) | 109 / 22 / 129, 17 (2+6) | 109 / 22 / 129, 17 (2+6) | 24 |
| crc32 | 129 / 0 / 65, 3 | 191 / 0 / 65, 2 | 191 / 0 / 65, 2 | 191 |
| esad | 1046 / 68 / 1078 | 1187 / 93 / 1085 | 1187 / 93 / 1085 | 1129 |
| fir8 | 7 / 0 / 23, 5 | 7 / 0 / 23, 5 | 7 / 0 / 23, 5 | 7 |
| fsm | 33 / 0 / 30, 3 | 72 / 0 / 30, 3 | **38** / 0 / 30, 3 | 38 |
| gray_cdc | 15 / 3 / 37 | 26 / 3 / 37 | 26 / 3 / 37 | 15 |
| lutram32 | 7 / 0 / 70, 2 | 8 / 0 / 70, 2 | 8 / 0 / 70, 2 | 8 |
| mul18 | 7 / 0 / 36, 2 | 7 / 0 / 36, 2 | 7 / 0 / 36, 2 | 7 |
| regfile_alu | 351 / 20 / 20, 12 (6+0) | 504 / 20 / 20, 15 (7+7) | **470** / 20 / 20, **12 (6+0)** | 470 |
| sad | 1414 / 132 / 1018 | 1847 / 174 / 1273 | **1607** / 174 / **1025** | 1532 |
| srl_delay | 7 / 0 / 34, 2 | 7 / 0 / 34, 2 | 7 / 0 / 34, 2 | 7 |
| widemux | 275 / 0 / 1049, 4 (2) | 276 / 0 / 1047, 4 (2) | 276 / 0 / 1047, 4 (2) | 276 |

`esad`'s and `sad`'s clocks, and `gray_cdc`'s, are their levels before
(`adc_sampleclk` 5 (3+0), `clk_usb` 4 (3+1); `gray_cdc` 4 (1+3) and 2) both before and
after. **No clock's worst path has more tables than before.** `analyser_fifo`'s read
clock has one more `CARRY4` on its worst path: that nine-level path, from `rptr`
through `rptr + fetch`, the subtraction and the addition into `prog_empty_q`, is in
`before`'s netlist too, the same cells, at a slack of −1.533 ns where its worst
path to that flip-flop, from `wgray_r2`, was eight levels at −1.984; `after`'s worst
there is the nine at −1.872, the eight having gone faster (Vivado's timer on the
netlists unplaced, `get_timing_paths -from`). Its median period, below, is 0.10 ns
longer, inside its seeds' spread both ways.

The period each clock achieves, medians over five seeds, each seed's figure beside
it (ns); `synthesis` is the `synthesis` build of the section before, rebuilt with
this tree's starting commit (the same figures, to the picosecond), and `area` with
this section's changes. "Vivado's, Reticle's netlist" is Vivado's placement and
routes of `area`'s seed-1 netlist:

| design | clock | Vivado | Vivado's, Reticle's netlist | synthesis | area |
|---|---|---|---|---|---|
| adders | clk | 2.042 | 2.238 | 2.337 (2.34 2.33 2.45 2.31 2.53) | 2.337 (2.34 2.33 2.45 2.31 2.53) |
| analyser_fifo | rd_clk | 3.887 | 5.002 | 5.248 (5.53 5.20 5.62 5.24 5.25) | 5.352 (5.67 5.18 5.35 5.31 5.42) |
| analyser_fifo | wr_clk | 2.417 | 2.970 | 3.036 (3.26 2.97 3.04 2.95 3.26) | 2.983 (2.91 3.17 3.19 2.91 2.98) |
| barrel | clk | 2.376 | 2.780 | 2.948 (2.96 2.98 2.75 2.95 2.85) | 2.948 (2.96 2.98 2.75 2.95 2.85) |
| bram_fifo | clk | 3.388 | 3.598 | 3.945 (3.95 3.93 4.02 3.91 3.94) | 3.945 (3.95 3.93 4.02 3.91 3.94) |
| carry_counter | clk | 1.855 | 2.352 | 2.385 (2.38 2.38 2.32 2.38 2.44) | 2.385 (2.38 2.38 2.32 2.38 2.44) |
| counter64 | clk | 3.220 | 3.721 | 4.040 (3.98 4.14 4.04 3.83 4.06) | 4.040 (3.98 4.14 4.04 3.83 4.06) |
| crc32 | clk | 2.234 | 2.331 | 2.676 (2.75 2.84 2.62 2.65 2.68) | 2.676 (2.75 2.84 2.62 2.65 2.68) |
| esad | adc_sampleclk | 2.131 | 3.506 | 3.033 (3.16 3.12 2.87 2.96 3.03) | 3.033 (3.16 3.12 2.87 2.96 3.03) |
| esad | clk_usb | 1.727 | 2.422 | 2.443 (2.43 2.51 2.54 2.44 2.31) | 2.443 (2.43 2.51 2.54 2.44 2.31) |
| fir8 | clk | 9.142 | 9.164 | 9.247 (9.25 9.16 9.25 9.25 9.16) | 9.247 (9.25 9.16 9.25 9.25 9.16) |
| fsm | clk | 1.903 | 2.057 | 2.887 (2.96 2.62 2.80 2.89 2.95) | **2.340** (2.37 2.34 2.32 2.35 2.30) |
| gray_cdc | clk | 1.466 | 1.447 | 1.503 (1.41 1.51 1.50 1.50 1.51) | 1.503 (1.41 1.51 1.50 1.50 1.51) |
| gray_cdc | clk2 | 1.267 | 1.572 | 1.615 (1.61 1.58 1.85 1.85 1.52) | 1.615 (1.61 1.58 1.85 1.85 1.52) |
| lutram32 | clk | 1.894 | 1.814 | 2.205 (2.29 2.18 2.37 2.15 2.21) | 2.205 (2.29 2.18 2.37 2.15 2.21) |
| mul18 | clk | 1.935 | 1.903 | 2.309 (2.31 2.30 2.53 2.40 2.29) | 2.309 (2.31 2.30 2.53 2.40 2.29) |
| regfile_alu | clk | 5.511 | 5.525 | 6.395 (6.72 6.39 6.96 6.16 6.31) | 6.336 (6.34 6.31 6.39 6.28 6.41) |
| sad | adc_sampleclk | 2.207 | 3.877 | 3.533 (3.53 3.41 3.74 3.52 3.75) | **3.150** (3.48 3.15 3.18 3.07 3.14) |
| sad | clk_usb | 1.726 | 2.635 | 2.787 (3.79 2.79 2.54 2.79 2.52) | 2.563 (2.40 2.40 2.74 2.63 2.56) |
| srl_delay | clk | 1.645 | 1.723 | 2.027 (2.02 2.09 1.95 2.03 2.09) | 2.027 (2.02 2.09 1.95 2.03 2.09) |
| widemux | clk | 2.438 | 2.709 | 3.485 (3.48 3.62 3.53 3.28 3.47) | 3.485 (3.48 3.62 3.53 3.28 3.47) |

**Over the twenty-one clocks, Reticle's median is 1.249 times Vivado's own figure,
against 1.274 before.** Vivado placing and routing Reticle's netlist went from 1.187
times its own flow to 1.175: `regfile_alu` 5.525 ns against its own 5.511, where
Reticle's netlist had been 5.797; `fsm` 2.444 to 2.057. Better by more than a seed's
spread: `fsm` (2.89 to 2.34 ns), `sad`'s `adc_sampleclk` (3.53 to 3.15). Worse:
`analyser_fifo`'s read clock by 0.10, inside the spread. Thirteen designs are the
same netlist, byte for byte, and so the same figures.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--synthesis legacy`, so neither
change reaches them, and their bitstreams are byte for byte those before: with this
tree's release binary `build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and
`build_plus.sh` seed 4's
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked.** That
`sad` and `esad` here are the Husky Plus's own pieces is what the measurements above
say of what the default would do to it; nothing has built it that way.

### Tests

- `tests/synth_register_loads.rs` proves both rules on the shapes they were written
  for, with the post-synthesis check, and that the pass does not run unless asked;
  unit tests in `synth::opt::ff_opt` hold each rule (a hold behind a cell and behind
  an assignment, on either arm; a select a conjunct decides on either side, one an
  `or` around it does not, and one nothing decides; an `or` select decided by a term
  of it and not by another) and leave a kept register alone.
- `aig::blast`'s tests hold a dense `case`, and one over a signed subject, the
  same function as its `pmux` on every input pattern, and a sparse one, one with a
  constant narrower than its subject and one naming a code twice blasted as before,
  node for node; `techmap`'s that a case tree is proved against the graph without
  trees, a different graph, and only when proving.
- `tests/fpga_xray_synthesis.rs` pins `fsm` at 38 tables, `regfile_alu` at 470 and
  six deep and `analyser_fifo` at 180 (the mapper's own counts, without
  `legalise_carries`'s), every design proved, and the legacy figures as they were.
  `netlist.txt` and `achieved.txt` gain this section's `area` build, and
  `tests/fpga_xray_achieved.rs` holds it no slower over the corpus than `synthesis`
  and no clock more than 5 % over.

**Mutations**, twelve, each made by a script and put back, each failing its test:
a decided select taking the other arm (the unit tests and the post-synthesis
proof both), a hold on the then arm read as `c` rather than `!c`, an `or` select
read as implied whatever its terms, a kept register rewritten, register loads
not asked for by the flow (the pinned figures), a case tree's leaves paired the
wrong way round (the blaster's tests, and every design's proof in
`fpga_xray_synthesis.rs`), a sparse `case` made a tree, a code named twice made
one, a constant narrower than its subject taken, the trees not proved, the trees
proved against themselves, and case trees not asked for by the flow. Two first
passed, and the tests that catch them were written then: the `or` select, whose
tests had only single-term selects, and the subject's signedness, which the
constant's own check made redundant — the rule now asks for a constant of the
subject's width and nothing about signedness, which is what the comparison
depends on, and no design maps differently for it.

### Not done

- **A signal carry in on `CYINIT`**, still: two tables of each of `sad`'s and
  `esad`'s 48 shared adders, 96 tables, and Vivado's lane count. *Taken since, in
  "Packing against Vivado's" (change 2): the lanes, `esad` 93 `CARRY4`s to 77 and
  `sad` 174 to 142.*
- **A constant generate of one on the slice's bypass input**, so that an equality's
  lanes on the chain need no buffer: Vivado's chains compare `!=` with a generate of
  one, which a bypass input left unrouted reads. Here every constant generate takes
  a table's `O5` (`legalise_carries`), and whether the bypass may be left to read one
  is the placer's packing to answer, which is not touched here.
- **Route-throughs as Vivado's netlist has them**: a propagate that is a
  flip-flop's output is a `LUT1` cell here and nothing in Vivado's synthesised
  netlist; the sites are the same. `counter64`'s 85.
- ~~**`analyser_fifo`'s memory** as Vivado arranges it, nine 16K x 2 block RAMs and no
  read multiplexer; that is the memory mapping's.~~ *Done in "Memories as Vivado
  arranges them"*:
  nine `RAMB36E1` of 16,384 by 2, 26 tables fewer by the mapper's count.
- **Resource sharing** of the shifts, which is most of `regfile_alu`'s remaining
  gap, is measured above and not kept: slower on the board's timer and unproved.
- `crc32`'s 191 tables two deep to Vivado's 129 three deep is the section before's
  choice, and stays.
- `individual_trigger`'s shared term and `sad_counter`'s table a counter: not found.
- None of it has been on a part.

## Packing against Vivado's

**Asked: what does each slice hold when Vivado places a design, what does it hold
when Reticle does, and where Vivado's packing is the better one, what does making
Reticle's the same do to the clock?** The open items the sections before left:
a ripple of tables four to a slice where Reticle had two to a tile, a carry in that
is a signal costing a lane, a delay of the fabric into a serialiser refused, two
`RAM32X1D` a slice each where Vivado has one, and `srl_delay`'s wires. Run on
2026-11-07 at `xc7a100tftg256-3` with Vivado 2026.1. **Everything here is Vivado's
placement and Vivado's timer, or Vivado's decoded bitstream; nothing has been on a
part.**

`tools/vivado-parity/slices.tcl` writes what each site holds and every net's pins,
of Vivado's own flow on a design (`own`, at the second set of periods of
`achieved.sh`) or of Reticle's placement loaded into Vivado (`placed`, a
`--vivado-placement` export, unrouted); `slices.py table` sets two side by side and
`slices.py connections` counts each kind of connection inside a slice, inside a
tile and how far.

### What each slice holds

Seed 1 of the `synthesis` build of "Synthesis against Vivado's" (b8bf631c, before
"Area against Vivado's" reached `master`), against Vivado's own flow. Flip-flops a slice is over the slices holding any; "fed inside"
is Vivado's own `report_utilization` line, *Register driven from within the Slice*,
of each placement. **Measured**:

| design | slices, Vivado | Reticle | flip-flops a slice, Vivado | Reticle | slices of more than four, Vivado | Reticle | fed inside the slice, Vivado | Reticle |
|---|---|---|---|---|---|---|---|---|
| adders | 83 | 165 | 5.8 | 3.0 | 52 | 7 | 293 of 481 | 226 of 481 |
| analyser_fifo | 71 | 136 | 3.4 | 1.9 | 14 | 5 | 108 of 221 | 110 of 213 |
| barrel | 36 | 29 | 2.2 | 2.9 | 0 | 5 | 33 of 70 | 40 of 70 |
| bram_fifo | 21 | 36 | 4.0 | 2.3 | 7 | 1 | 34 of 68 | 33 of 68 |
| carry_counter | 24 | 28 | 3.1 | 2.3 | 3 | 0 | 25 of 65 | 33 of 65 |
| counter64 | 44 | 56 | 3.9 | 2.6 | 11 | 0 | 27 of 129 | 70 of 129 |
| crc32 | 36 | 71 | 2.0 | 1.8 | 0 | 0 | 37 of 65 | 38 of 65 |
| esad | 289 | 431 | 4.0 | 2.8 | 87 | 56 | 584 of 1084 | 540 of 1085 |
| fir8 | 12 | 22 | 2.6 | 1.1 | 0 | 0 | 1 of 23 | 2 of 23 |
| fsm | 11 | 33 | 3.0 | 1.7 | 2 | 0 | 6 of 30 | 7 of 30 |
| gray_cdc | 9 | 16 | 4.1 | 2.5 | 2 | 0 | 36 of 37 | 36 of 37 |
| lutram32 | 17 | 36 | 5.0 | 2.2 | 10 | 1 | 34 of 70 | 2 of 70 |
| mul18 | 10 | 22 | 4.0 | 1.6 | 2 | 0 | 4 of 36 | 2 of 36 |
| regfile_alu | 104 | 164 | 1.8 | 1.8 | 0 | 0 | 8 of 20 | 8 of 20 |
| sad | 309 | 537 | 3.5 | 2.6 | 46 | 18 | 618 of 1019 | 784 of 1273 |
| srl_delay | 9 | 13 | 3.8 | 2.6 | 4 | 1 | 3 of 34 | 4 of 34 |
| widemux | 282 | 358 | 3.9 | 2.9 | 94 | 13 | 17 of 1049 | 19 of 1047 |

What it says:

- **Vivado fills slices: about half as many for the same logic.** A slice of eight
  flip-flops is common in Vivado's (`adders`' shift register, 33 slices of eight) and
  all but absent in Reticle's. Vivado's eight are four through the bypass inputs and
  four through a table used as a wire, which no cell names and `report_utilization`
  does not count. **Decoded** in `shared8`'s bits (below): a slice of eight
  flip-flops and one table cell (a `LUT4` on `A6LUT`) has `BFFMUX.O6`, `CFFMUX.O6`
  and `DFFMUX.O6` with the `B`, `C` and `D` tables holding an `INIT` and no cell,
  `AFFMUX.O5` beside the `LUT4` on `O6`, and each five-input flip-flop on its bypass
  input (`<L>5FFMUX.IN_B`). That `adders`' slices of eight are the same is read off
  their cells, not decoded. Reticle has no route through a table, so a flip-flop fed
  from the fabric takes the bypass input, four a slice.
  Its tables are spread the same way: `regfile_alu`'s are in 153 slices to Vivado's
  93, `crc32`'s in 71 to 35. **Not changed**; see "Not done".
- **"Fed inside" is about the same in both but in `lutram32`**: 2 of 70 to Vivado's
  34, its read word's registers a slice away from the `RAM32M`s they register.
  Vivado has six of each slice's flip-flops reading the slice's own RAM. Changed.
- **The earlier reading of `adders`, a ripple of tables four to a slice where Reticle
  had two to a tile, is of the legacy netlist**, fifteen tables deep; with the default
  synthesis it is Vivado's cell for cell, a table and eight `CARRY4`s deep, and its
  tables are four to a slice in both (56 slices of four each). What is left there is
  the flip-flops above.
- **`srl_delay`** is Vivado's netlist (two `SRLC32E`, 32 flip-flops, seven tables two
  deep), in 13 slices to Vivado's 9: its 32 flip-flops two or three a slice where
  Vivado's are four to six beside the tables that read them, the same route through
  a table. Its latest path is a flip-flop, a table and a table: 0.83 and 0.60 ns of
  wire here (`SLICE_X86Y93` to `X87Y93` to `X85Y93`, the last table in another tile)
  to Vivado's 0.61 and 0.49 (the two tables in one slice). Not changed.
- **`esad`'s and `sad`'s counters**: 93 `CARRY4`s to Vivado's 68, 174 to 132, a
  carry in that is a signal (`s ? c - d : a + b`) being a lane of its own. Changed.

### What was changed

1. **A distributed RAM's registered outputs go in its slice**
   (`place::registered_tables`, `shares_a_slice_with_ram`). A RAM's output is taken
   as a table's is: the flip-flop it alone feeds joins the RAM's group on the site
   the output reaches inside the tile (`O6` to `<L>FF`, `O5` to `<L>5FF`), when it is
   on the RAM's write clock and has the RAM's write enable or none, which a `SLICEM`'s
   flip-flops share with it. `lutram32`'s registers fed inside go from 2 of 70 to 36
   (Vivado 34), its slices from 36 to 28; its latest path was that connection, 0.51 ns
   of route where Vivado's is none. Behind `PlaceOptions::memory_packing`, which is on
   unless `--memories legacy`.
2. **A carry in that is a signal goes on `CYINIT`, and bit 0's generate on the lane's
   `O5`** (`primitives::shared_low_bit`, `xray::legalise_carries`' `SharedAx`).
   `CYINIT` is reached from `AX` alone, and a generate from the fabric by the bypass
   input, which on bit 0 is `AX` too; `legalise_carries` refused the two, so the
   mapping kept such a carry in as a lane of its own. Vivado puts the generate on
   `O5` — `shared8` (`s ? c - d : a + b`, eight bits, `testdata/fpga/vivado/carry_in/`)
   is a `LUT3` on `A5LUT`, the propagate's `LUT5` on `A6LUT`, `PRECYINIT.AX` and
   `CARRY4.ACY0`, two `CARRY4`s — and so does this flow now: bit 0's propagate and
   generate become one `LUT6_2` (the generate's own table folded in when nothing else
   reads it, passed through from an input otherwise). When the two read more than five
   signals between them the carry in goes on a `CARRY4` of its own below the chain,
   its top lane's carry out (propagate a constant zero, generate the signal on `DX`)
   into the chain's `CI`, which costs what the lane did. `esad` 93 `CARRY4`s to 77
   (Vivado 68) and `sad` 174 to 142 (132), sixteen and thirty-two chains on `O5`,
   none below. Under `--synthesis vivado`; `--synthesis legacy` never writes one.
   **Feature-compared** with Vivado's bits (`tests/fpga_xray_carry_in_vivado.rs`):
   both slices of `shared8` the same `PRECYINIT` and `CARRY4` features, but the top
   lane's `DCY0`, set here and not in Vivado's: a generate nothing reads (the lane's
   sum is `S ^ CO` of the lane below, its carry out goes nowhere), taken from `O5`
   because the lane's two tables are paired as every lane's are.
3. **A delay of the fabric into a serialiser's `DDLY` is built** (`ddly_datain`), the
   two in one tile, every feature of it Vivado's (`tests/fpga_xray_idelay.rs` compares
   it in the gate now, and its `STRICTER` list no longer has it). Two things held the
   two apart. The placer's dedicated relation `DDLY ← DATAOUT` was none: a `_SING` IO
   tile is declared with both halves' `ILOGIC`s on its one half's wires
   (`xray::iologic`), so two sinks reached one delay, which `find_relation` reads as a
   connection fed from two places. Two sinks on one wire are one site seen twice now,
   and left out of the relation (the tile cannot hold such a pair). And
   `place_io_delay_controllers` then moved the delay off any site whose `ILOGIC` was
   taken, which is the serialiser it feeds; that one is allowed now.

**Tried and not kept: two `RAM32X1D` of one address in one slice**, as Vivado puts
`mem32x8`'s (the `C` and `D` tables' upper halves and lower), written as a rule that
made each such pair one group. Over ten seeds, the RAM designs with and without it
(medians, ns):

| design | clock | with the pairs | without |
|---|---|---|---|
| esad | adc_sampleclk | 3.029 | 3.066 |
| esad | clk_usb | 2.455 | 2.425 |
| lutram32 | clk | 2.323 | 2.140 |
| regfile_alu | clk | 6.527 | 6.352 |
| sad | adc_sampleclk | 3.401 | 3.422 |
| sad | clk_usb | 2.641 | 2.532 |

1.381 against 1.347 of Vivado's own figure over the six; `lutram32` and
`regfile_alu` worse by more than their seeds' spread. Taken out. `mem32x8` is
Vivado's slice for slice without it all the same, by the anneal, after change 1
moved its placement, and `tests/fpga_xray_lutram_vivado.rs` compares it again; its
note says a placement that moves may part the two, as one did on 2026-11-06.

### Before and after

Medians over five seeds, each seed's figure beside it (ns). `before` is the `area`
build of "Area against Vivado's", the default before this section (f8625a1d), and
`after` this section's on top of it; "Vivado's, Reticle's netlist" is Vivado's
placement and routes of `after`'s seed-1 netlist. Only the clocks that moved: every
other design built the same, seed for seed, as `achieved.txt`'s `area`, which says
the `area` figures are this tree's too:

| design | clock | Vivado | Vivado's, Reticle's netlist | before | after |
|---|---|---|---|---|---|
| esad | adc_sampleclk | 2.131 | 3.479 | 3.033 (3.16 3.12 2.87 2.96 3.03) | 3.084 (3.08 3.14 3.21 3.00 2.89) |
| esad | clk_usb | 1.727 | 2.389 | 2.443 (2.43 2.51 2.54 2.44 2.31) | 2.426 (2.77 2.37 2.43 2.76 2.24) |
| lutram32 | clk | 1.894 | 1.792 | 2.205 (2.29 2.18 2.37 2.15 2.21) | 2.164 (2.16 2.03 2.38 2.17 2.02) |
| sad | adc_sampleclk | 2.207 | 4.545 | 3.150 (3.48 3.15 3.18 3.07 3.14) | 3.192 (3.14 3.09 3.38 3.19 3.32) |
| sad | clk_usb | 1.726 | 2.413 | 2.563 (2.40 2.40 2.74 2.63 2.56) | 2.546 (2.48 2.66 2.48 2.55 2.56) |

**Over the twenty-one clocks, Reticle's median is 1.248 times Vivado's own figure,
against 1.249 before: the same.** `lutram32` is better by about a seed's spread;
`esad`'s and `sad`'s clocks move both ways, each inside its spread. The packing is in
`achieved.txt` as the build `packing`, and
`the_packing_costs_no_clock_and_reaches_only_its_designs` holds it.

*Measured first on the build before "Area against Vivado's"* (`synthesis`, b8bf631c),
before that section reached `master`: the geometric mean went from 1.274 to 1.263,
`sad`'s clocks 3.533 to 3.320 ns and 2.787 to 2.491, `esad`'s `adc_sampleclk` 3.033
to 3.084. That reading, that the carry in on `CYINIT` made `sad` faster, does not
hold on top of the area changes: `area` alone took `sad`'s `adc_sampleclk` to 3.150,
and this section's changes leave it there. Kept visible because the first table is
the one the commits were made on.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` are byte for byte as before: with this
tree's release binary `build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and
`build_plus.sh` seed 4's
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked.**
Change 1 is off under `--memories legacy` and change 2 under `--synthesis legacy`,
which both pass; change 3 is not behind either: the Husky's delays are of pads
(`DELAY_SRC("IDATAIN")`), placed by their pads, and the builds being byte for byte
the same says the twins it leaves out of a relation reached neither.

### Tests

`a_ram_takes_the_flip_flops_it_registers_when_memories_are_packed` and
`two_sinks_on_one_wire_are_one_site_seen_twice` (`place`'s unit tests) hold changes 1
and 3 on small fabrics; `a_signal_carry_in_takes_cyinit_and_puts_bit_0s_generate_on_o5`
(`tests/fpga_carry.rs`) simulates change 2 on every input of the shared adders and of
`ChosenXor`, whose two tables need six inputs (the `CARRY4` below), legalised, paired
and not, and `carry_ins_have_ax_to_themselves` checks after every legalisation in that
file that a carry in on `CYINIT` has `AX` to itself. `tests/fpga_xray_carry_in_vivado.rs`
compares `shared8`'s carry features with Vivado's, `tests/fpga_xray_idelay.rs`
`ddly_datain`'s, and `tests/fpga_xray_lutram_vivado.rs` `mem32x8`'s slices.

**Mutations**, sixteen, each made by a script and put back, each failing a test: the RAM taken
without the option, not taken with it, its clock unchecked; the twins kept in the
relation, refusing it, and any two sinks of one driver taken for twins; the delay's
site rule not allowing its serialiser (`ddly_datain` unrouted); the `LUT6_2`'s halves
swapped; the `CARRY4` below with a propagate of one, its own `CYINIT` kept beside the
`CI`, and its carry passed up from lane 0; always below, never below; the signal low
bit not taken (three `CARRY4`s); and the shared `AX` not seen. **Three runs passed a
test they were meant for, and the tests say so:** the twins refusing the relation
passes `ddly_datain`, whose anneal puts the two together on this placement anyway
(the unit test catches it); the shared `AX` not seen passes the Vivado comparison,
because the lane pairing (`pack_lut_pairs`) makes the same `LUT6_2` when the generate
is a table (the `AX` check catches it); and the `RAM32X1D` pairs taken out passed
`mem32x8`, which is why that test does not stand for the pairing, and the pairing is
not kept.

### Not done

- **Flip-flops through an empty table.** It is most of the difference in the table
  above: Vivado packs eight flip-flops a slice where it can, four through the bypass
  inputs and four through a table used as a wire, and Reticle four. The route through
  a table would be the router's and the bitstream's (a table's `INIT` written for a
  route, which the decoded arcs would have to account for), or the placer's (a buffer
  table it may put in front of a flip-flop, as `legalise_carries` puts one in front of
  a chain's propagate); neither is written, and what it would do to the clock is not
  measured. `srl_delay`'s wires are this. *Done since, the router's and the
  bitstream's way, in "Eight flip-flops a slice" below.*
- **Tables four to a slice.** Reticle's tables are spread over half again as many
  slices as Vivado's (`regfile_alu` 153 to 93); nothing in the placer asks for a
  full slice, and what it would cost or save is not measured. *Measured since, in
  "Filling slices": most of that is the netlist; Vivado's own placement of Reticle's
  netlist takes 152 slices for `regfile_alu`, and Reticle's 130 now.*
- **`a + b + c` and a counter that counts on a comparison** (`cin8`, `cnt8`, probes
  of this section not committed) are four and three `CARRY4`s here to Vivado's two
  and one: synthesis writes the first as two adders and the second's carry in as an
  operand, so neither reaches the chain's carry in. That is synthesis's to do.
- **A `_SING` IO tile's second `ILOGIC`**, declared on the first's wires, is left
  out of a dedicated relation rather than taken out of the fabric: an `IDDR` or a
  serialiser placed there by its pad is as before.
- None of it has been on a part.

## Memories as Vivado arranges them

**Asked: how does Vivado lay an inferred memory out over the 7 series' block and
distributed RAMs, design by design, and where its arrangement is no worse, does
Reticle's default (`--memories vivado`) now choose the same?** "Area against Vivado's"
left `analyser_fifo`'s memory as eight `RAMB36E1` of 4,096 by 9, four deep, and a read
multiplexer, where Vivado has nine of 16,384 by 2 and none. Run on 2026-11-07, with
Vivado 2026.1, at `xc7a100tftg256-1` for the arrangements and the bitstreams and
`-3` for the periods. **Everything here is Vivado's synthesis, its bitstreams, its
simulator and its timer; nothing has been on a part.**

`testdata/fpga/vivado/memarr/` has 108 memories to infer, written here
(`tools/vivado-parity/gen_memarr.py`), each fed by a shift register from one pin and
its word folded to one bit: one write port and one read port at 52 sizes from 32 by 31
to 65,536 by 1, either side of distributed RAM's line, of 18 and 36 kbit, of the
parity widths and of a power of two (1,000, 1,536, 3,000, 6,000, 12,000, 20,000
words); at three sizes, a single port read first, written first and unchanged on a
write, a true dual port, a second read, an enabled read and a read registered twice; a
read on no clock; read-only memories and memories with initial contents (written as a
loop over a hash of the address); two written narrow and read wide in the vendor's
template; and each `ram_style`. Those 96 fold the word read to its parity; twelve
more, `bits_`, fold it under a mask that changes every cycle, so that a bit read from
the wrong place shows (two bits swapped leave a parity as it was), for the arrangements
whose word travels on more than one pin or block. `FIXTURE=memarr
tools/vivado-parity/lutram.sh` runs them; `lutram_probe.tcl` now writes each block's parameters too (`params`: `RAM_MODE`,
the four widths, `DO<x>_REG`, the write modes, `RAM_EXTENSION_<x>`), and `vivado.txt`
keeps them with the cells after routing and each block RAM tile decoded through this
repository's database. `xsim.txt` is what each does as Vivado built it, under its own
simulator (`gen_lutram_vectors.py`), and `memarr_table.py` sets Vivado's arrangement
beside Reticle's from a build's `--report`. Only answers are kept.

### What Vivado does

**Measured**, read off `vivado.txt`'s parameters and the netlists Vivado wrote:

- **Every block holds the whole depth, where one block can.** The depth rounded up to
  a power of two is each block's, from 1,024 words (a `RAMB18E1` 18 bits wide, a
  `RAMB36E1` 36) to 32,768 (a `RAMB36E1` one bit wide), and the width is split over
  blocks side by side: nothing is stacked and nothing is multiplexed on the read.
  `sdp_16384x18` is nine `RAMB36E1` of 16,384 by 2, which is one block more than the
  eight of 4,096 by 9 four deep that cost by capacity chose here, and 4 tables against
  31; `sdp_20000x2` two of 32,768 by 1.
- **The width is `RAMB36E1`s, low bits first, and a `RAMB18E1` for what they leave
  when it fits one at that depth**: `sdp_1024x40` is a `RAMB36E1` of 36 and a
  `RAMB18E1` holding 4, `sdp_2048x24` 18 and 6, `sdp_16384x9` four `RAMB36E1` of 2
  and a `RAMB18E1` of 1, `sdp_3000x12` 9 and 3; `sdp_1024x20` is one `RAMB36E1`.
  A width with parity is filled to it: `sdp_4096x16` is 9 bits and 7, bit 8 on the
  first block's `DIPADIP[0]` (its netlist).
- **512 words or fewer, written on one port and read on another, are simple dual
  port**: a `RAMB18E1` read 36 wide (`sdp_512x36`, and `sdp_32x32`, 1,024 bits as
  Vivado counts them), a `RAMB36E1` 72 wide (`sdp_512x64`, `sdp_512x72`); but what
  fits 18 bits is a `RAMB18E1` of 1,024 by 18 in true dual port (`sdp_512x18`,
  `sdp_256x4`, `sdp_128x8`). The word's bits 15:0 are `DIADI`, 31:16 `DIBDI`, 33:32
  `DIPADIP` and 35:34 `DIPBDIP`, each read alike, and in 72 bits 32 a side; both write
  modes `READ_FIRST`.
- **A single port is port A alone**: `READ_WIDTH_B` and `WRITE_WIDTH_B` zero,
  `WRITE_MODE_A` `READ_FIRST`, `WRITE_FIRST` or `NO_CHANGE` as the description reads
  (`sp_rf_`, `sp_wf_`, `sp_nc_`), and **`RDADDR_COLLISION_HWCONFIG` `PERFORMANCE`**,
  which no block Vivado uses both ports of sets. A true dual port is a read and a
  write at one address on each of A and B, both `READ_FIRST` (`tdp_`). Two reads and a
  write are a copy each, as here before (`r2_`), and Vivado puts the two `RAMB18E1`s
  of one in one tile.
- **A register after the read is the block's own** (`DOB_REG`, `reg_`): 41 flip-flops
  where this flow had 59. A read-only memory's address register and read go into the
  block too, `DOA_REG` on port A alone (`rom_16384x2`, the cell named `sr_reg_rep`).
- **Initial contents do not change the arrangement**: `init_1024x36` is one
  `RAMB36E1` 36 wide, its parity bits in `INITP_`, where this flow had given a memory
  with contents only modes without parity, three `RAMB18E1` of 16.
- **Deeper than 32,768**, two `RAMB36E1` cascade (`RAM_EXTENSION_A` `LOWER` and
  `UPPER`, `sdp_65536x1`).
- **Read-only memories of 512 by 36 to 4,096 by 8 are lookup tables**, 174 to 321
  of them and the multiplexers, where one block holds each (`rom_*`); 16,384 by 2 is a
  block.
- **Under a tight clock, Vivado's synthesis chooses distributed RAM for memories it
  puts in block RAM at 5 ns**: in the timing runs below, at periods of 0.85 of what it
  had reached, `sdp_32x32`, `sdp_512x36`, `sdp_512x64` and `sp_wf_1024x18` were
  `RAM32M`s and `RAM64M`s, with `INFO: [Synth 8-5584] The signal ... is implemented as
  distributed LUT RAM for the following reason(s): The timing constraints suggest that
  the chosen mapping will yield better timing results.` *The threshold of "Block or
  distributed" above, 1,024 bits, was measured with a relaxed clock and is what
  Vivado does there; it is not the whole rule.* This flow does not choose by timing.
- `ram_style = "registers"` is flip-flops, 1,059 of them for 128 by 8.

### What was changed

All behind `--memories vivado`, the default on the 7 series (`MapOptions::vivado_memories`);
`--memories legacy`, the ECP5, the Gowin and the iCE40 map as before:

1. **Vivado's arrangement** (`primitives::vivado_columns`, read off the answers
   above): columns of `RAMB36E1`s and a `RAMB18E1` at the depth that covers the
   memory, or simple dual port at 512 words; deeper than 32,768, rows of blocks of
   32,768 and the read multiplexed, as before. A memory's blocks can now be two
   primitives in two modes (`BramPlan::columns`); the report says so (`2 x RAMB36E1 in
   36x1024 mode (1 wide, 1 deep, and 1 x RAMB18E1 in 18x1024 mode beside)`).
2. **Simple dual port inferred** (`Mapper::sdp_wiring`): the mode is the device's true
   dual port of half the width and twice the depth, its word's address a pin higher,
   with Vivado's parameters, the read on A and the write on B.
3. **Contents in every mode, parity included** (`linear_contents`): bit `j` of word
   `a` at `a * data + j` of the `INIT_`s, a parity bit at `a * parity + j` of the
   `INITP_`s, the mode's top bits being its parity. That is UG473's layout,
   **quoted**; the database's layouts of the modes without parity are the same bit for
   bit (`contents_by_rule_are_the_databases_layouts`), and Vivado's bitstreams of the
   ten memories with contents and a write are the check, below.
4. **A read and a write at one address on one port** (`Mapper::pair_ports`): a read on
   the write's clock that always reads is read first; one enabled by the write's
   enable complemented is no change; and a read on no clock chosen against the write's
   data by its enable into a register on its clock (`if (we) q <= d; else q <=
   mem[a];`, which reaches the mapper so) is written first, the register being the
   block's read. A port nothing uses has widths of zero and the faster collision mode.
5. **The register after a read in the block** (`Mapper::output_register`): a
   flip-flop on the read's clock with no reset, starting at zero, loading the read's
   whole word with nothing else reading it, is `DO<x>_REG`, its enable `REGCE`, when
   the read is not multiplexed after the block.

### What it is now

Each design: Vivado's arrangement (`36` a `RAMB36E1`, `18` a `RAMB18E1`, by the
port's width; `A` one port; `reg` the output register in the block), this flow's
before (`73334987`, the same as `f8625a1d`) and after, `=` where it is Vivado's; the design's tables, Vivado's,
before and after; and for the 28 clocks timed (below) the period, Vivado's own flow,
then the medians of five seeds before and after (ns). **Measured**:

| design | Vivado | before | after | tables V / b / a | period V / b / a |
|---|---|---|---|---|---|
| sdp_16384x18 | 9 36 TDP 2 | 8 36 TDP 9 four deep | = | 4 / 31 / 5 | 3.41 / 4.60 / 3.81 |
| sdp_16384x9 | 4 36 TDP 2, 1 18 TDP 1 | 4 36 TDP 9 four deep | = | 2 / 19 / 2 | 3.04 / 4.08 / 3.58 |
| sdp_8192x9 | 2 36 TDP 4, 1 18 TDP 2 | 2 36 TDP 9 two deep | = | 2 / 9 / 2 | 3.11 / 3.60 / 3.47 |
| sdp_20000x2 | 2 36 TDP 1 | 3 18 TDP 2 three deep | = | 1 / 8 / 1 | 2.55 / 3.19 / 2.75 |
| sdp_12000x3 | 1 36 TDP 2, 1 18 TDP 1 | 3 18 TDP 1 | = | 1 / 1 / 1 | 2.58 / 2.76 / 2.77 |
| sdp_6000x6 | 1 36 TDP 4, 1 18 TDP 2 | 3 18 TDP 2 | = | 2 / 2 / 2 | 2.94 / 3.13 / 3.21 |
| sdp_3000x12 | 1 36 TDP 9, 1 18 TDP 4 | 3 18 TDP 4 | = | 3 / 3 / 3 | 3.03 / 3.48 / 3.40 |
| sdp_2048x24 | 1 36 TDP 18, 1 18 TDP 9 | 3 18 TDP 9 | = | 5 / 5 / 5 | 3.38 / 3.59 / 3.71 |
| sdp_1024x40 | 1 36 TDP 36, 1 18 TDP 18 | 3 18 TDP 18 | = | 8 / 9 / 9 | 3.53 / 4.03 / 4.05 |
| sdp_512x72 | 1 36 SDP 72 | 2 36 TDP 36 | = | 15 / 16 / 16 | 3.83 / 4.67 / 4.36 |
| sdp_512x64 | 1 36 SDP 72 | 2 36 TDP 36 | = | 13 / 14 / 14 | 2.73\* / 4.28 / 4.42 |
| sdp_512x36 | 1 18 SDP 36 | 1 36 TDP 36 | = | 8 / 8 / 8 | 2.64\* / 4.02 / 4.07 |
| sdp_32x32 | 1 18 SDP 36 | 1 36 TDP 36 | = | 7 / 7 / 7 | 1.79\* / 4.12 / 3.92 |
| init_1024x36 | 1 36 TDP 36 | 3 18 TDP 18, 16 bits each | = | 8 / 8 / 8 | 3.53 / 4.07 / 4.09 |
| init_4096x18 | 2 36 TDP 9 | 5 18 TDP 4 | = | 4 / 5 / 5 | 3.24 / 3.54 / 3.63 |
| init_16384x9 | 4 36 TDP 2, 1 18 TDP 1 | 9 18 TDP 1 | = | 2 / 2 / 2 | 3.04 / 3.43 / 3.58 |
| init_1024x40, init_2048x18, init_512x36, init_512x72 | as `sdp_` | modes without parity | = | | |
| reg_1024x18 | 1 18 TDP 18 reg | the register in a slice | = | 4 / 5 / 5 | 1.99 / 2.66 / 2.26 |
| reg_4096x18 | 2 36 TDP 9 reg | the same | = | 4 / 5 / 5 | 2.04 / 2.68 / 2.42 |
| reg_16384x4 | 2 36 TDP 2 reg | the same | = | 1 / 1 / 1 | 1.44 / 2.66 / 1.85 |
| sp_wf_1024x18 | 1 18 TDP 18 A | 96 `RAM64M` | = | 4 / 112 / 5 | 2.79\* / 4.48 / 3.55 |
| sp_wf_4096x18 | 2 36 TDP 9 A | 384 `RAM64M` | = | 4 / 421 / 5 | 3.36 / 7.96 / 3.64 |
| sp_wf_16384x4 | 2 36 TDP 2 A | 256 `RAM64M`, 256 `RAM64X1D` | = | 1 / 551 / 1 | |
| sp_rf_4096x18, sp_nc_4096x18 | 2 36 TDP 9 A | both ports | = | 4 / 5 or 6 / 5 | 3.17, 3.36 / 3.60, 3.58 / 3.63, 3.64 |
| sp_rf_, sp_nc_ at 1,024 by 18 and 16,384 by 4 | A | both ports | = | | |
| tdp_1024x18 | 1 18 TDP 18 | refused | = | 8 / - / 8 | 3.37 / - / 3.94 |
| tdp_4096x18 | 2 36 TDP 9 | refused | = | 8 / - / 8 | 3.35 / - / 4.13 |
| tdp_16384x4 | 2 36 TDP 2 | refused | = | 2 / - / 2 | |
| analyser_fifo | 9 36 TDP 2 | 8 36 TDP 9 four deep | = | 131 / 238 / 212, flip-flops 221 / 213 / 211 | `rd_clk` 3.89 / 5.35 / 5.26, `wr_clk` 2.42 / 2.98 / 3.00 |
| bits_ (eleven of the twelve) | as the design each is named for | as that design | = | | |
| the other 49, and `bits_sdp_4096x16` | | = | = | | |
| sdp_65536x1 | 2 36 TDP 1, cascaded | 2 36 TDP 1 two deep | the same | 4 / 4 / 4 | |
| rom_512x36 to rom_4096x8 (five) | lookup tables, 174 to 321 | one block | one block | | |
| rom_16384x2 | 1 36 TDP 2 A reg, its address in the block | 1 36 TDP 2 | 1 36 TDP 2 A | 1 / 1 / 1 | |
| async_1024x8 | 1 18 TDP 18, its address in the block | 48 `RAM64M` | the same | 2 / 51 / 51 | |
| asym_4096x8_r32, asym_2048x9_r36 | 4 36 TDP 9, 4 18 TDP 9 | 768, 384 `RAM64M` | the same | 7, 8 / 626, 366 | |
| style_registers_128x8 | 1,059 flip-flops | 6 `RAM64M` | the same | 410 / 13 / 13 | 3.11 / 2.47 / 2.47 |

\* Vivado's own flow had these in distributed RAM at the periods it was timed at (above);
its arrangement at 5 ns is the column beside. `analyser_fifo`'s tables are
`netlist_qor.sh`'s count of each netlist, as the sections before give them; the
others are the design's whole count of tables, the parity of the word read most of
them.

**97 of the 108 are Vivado's arrangement, where 50 were.** Of the eleven that are not:

- *`sdp_65536x1`*: no cascade is built (`b36_cascade` is refused, "Block RAM" above),
  so two blocks of 32,768 are stacked and their read multiplexed.
- *The five read-only memories Vivado makes of lookup tables* stay a block each: one
  block against 174 to 321 tables is not worse, and this flow does not time its choices.
- *`rom_16384x2` and `async_1024x8`*: Vivado takes the address register into the block,
  which this flow does not ("Block or distributed" above), so the first reads in the
  block's latch and the second is distributed RAM.
- *`asym_*`*: the template's loop of part-selects (`q[i * 8 +: 8] <= mem[{a, i}]`)
  reaches the mapper as four reads on no clock, the register a nest of its own bits,
  so the memory is distributed RAM; four reads on the clock would be four copies, which
  is what Vivado's four blocks are. That is the synthesis's, not looked into here.
- *`style_registers_128x8`*: Vivado's flip-flops are worse than this flow's distributed
  RAM by Vivado's own timer, 3.11 ns against 2.47 and 410 tables against 13, so the
  attribute keeps meaning logic of any kind here. Built as Vivado builds it, 4.30 ns
  (this flow's flip-flops); measured, and not kept.

**Checked, on all 108** (`tests/fpga_xray_memarr_vivado.rs`):

- every memory gets the cells Vivado gave it, block and distributed RAMs by kind and
  count, but the eleven;
- **every block RAM tile is Vivado's, site feature for site feature and contents bit
  for contents bit**: 177 tiles of the 97, the initial contents of the seven `init_`
  memories, the two `bits_` ones and `rom_r2_1024x16`'s two copies included, so the parity bits in
  `INITP_`, the halves of a simple dual port and which column holds which bits are
  Vivado's. Two `RAMB18E1` in one tile are compared as two blocks, since Vivado
  puts `r2_`'s copies in one tile and the placement here may not;
- every set bit decodes, none unexplained, and the arcs are the router's and the ties';
  every mapping is proved;
- **the routed netlist answers as Vivado's build does**, 2,000 cycles from power-up
  under Vivado's simulator on Vivado's models, bit for bit, on 106 of the 108: nine of
  the eleven Reticle arranges otherwise among them, read-only memories and contents
  included. The two `asym_*` are not simulated: Reticle's 768 `RAM64M` of one had not
  elaborated after forty minutes.

`tests/fpga_xray_bram_vivado.rs`'s inferred designs are all Vivado's now:
`ib_512x36` and `ib_512x72` in simple dual port, `ib_1024x36_reg2`'s register in the
block and `ib_1024x18_tdp`, which was refused, are no longer listed as differing, and
their tiles, pins and simulation are held as the rest. So are
`tests/fpga_xray_lutram_vivado.rs`'s `mem32x32` and `mem32x32r2`, a `RAMB18E1` read 36
wide each, as Vivado's.

### The period

`achieved.sh` on the 26 designs whose arrangement changed and `analyser_fifo`, five
seeds each, `achieved.txt` there (the table above has the medians). **Over the 26
clocks both builds have, Reticle's median is 1.193 times Vivado's own figure, against
1.293 before**; Vivado placing and routing Reticle's netlist, 1.068 against 1.137.
Faster by more than a seed's spread: `sdp_16384x18` (4.60 to 3.81 ns), `sdp_16384x9`,
`sdp_20000x2`, the three `reg_` (`reg_16384x4` 2.66 to 1.85, against Vivado's 1.44),
`sp_wf_` (7.96 to 3.64 at 4,096 by 18). Slower: `init_16384x9` 3.43 to
3.58 (four `RAMB36E1` and a `RAMB18E1` where it was nine `RAMB18E1`, the same tiles;
Vivado on the two netlists 2.94 and 3.00), `init_4096x18`, `sdp_2048x24`, `sdp_512x64`
and `sdp_6000x6` by 3 % or less, inside or at the edge of the seeds' spread, with
Vivado's placement of each new netlist as fast as or faster than of the old but for
`sdp_512x64`'s 3.89 to 3.91. `analyser_fifo`'s read clock is 5.35 to 5.26: its worst
path is the flags' arithmetic and not the memory, and Vivado's placement of the new
netlist was 5.00 to 5.25 on one seed.

The thirteen designs of `qor/` are the bitstream they were with the default, byte for
byte (their memories were already Vivado's: `bram_fifo` one `RAMB36E1`, `lutram32` and
`regfile_alu` distributed RAM), so their figures in the sections before stand.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--memories legacy`, which none of
this reaches: with this tree's release binary `build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and
`build_plus.sh` seed 4's
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked.** The
other families' mappings of five memory designs of `testdata/fpga/cynthion/` on the
ECP5, the Gowin and the iCE40, and the legacy mappings of three of `qor/`,
`analyser_fifo` and six of `memarr/`, are the files they were, byte for byte. (No
Trellis or Apicula database is on this machine, so those families' bitstreams were not
built; their mapped netlists are what changed nowhere.)

### Tests

- `tests/fpga_xray_memarr_vivado.rs`, above; with `RETICLE_MEMARR_BITS` and
  `UPDATE_EXPECT` it writes `vivado.txt` again from `lutram.sh`'s output. It also holds
  `--memories legacy` to the arrangement before (sixteen `RAMB18E1` for
  `sdp_16384x18`, two for `sdp_512x36`, distributed RAM for `sp_wf_1024x18`), and the
  quoted run of `achieved.txt`: every clock five seeds, the geometric mean 5 % lower
  after, no clock 5 % slower, and `sdp_16384x18` and `analyser_fifo`'s read clock
  faster.
- `vivado_arranges_a_memory_as_its_netlists_say` holds `vivado_columns` to one case
  per rule above, and `contents_by_rule_are_the_databases_layouts` the contents rule to
  the database's layouts in every mode without parity, and to `INITP_` in every mode
  with.
- `tests/fpga_xray_synthesis.rs` pins `analyser_fifo` at 165 tables under the legacy
  synthesis and 154 under the default, 29 and 26 fewer: both map memories as Vivado
  does.

**Mutations**, each made by a script in a copy of the tree and run against the
library's tests of the mapper and `tests/fpga_xray_memarr_vivado.rs` with Vivado's
simulator:

| mutation | what failed |
|---|---|
| the remainder always a `RAMB36E1`, never a `RAMB18E1` beside | the arrangement's unit test; the cells; the tiles; the legacy test's default rows; the exceptions' |
| no simple dual port | the cells; the tiles; the legacy test's default rows |
| a simple dual port's word address where a true dual port's is | the simulation only: the address pins are no tile feature |
| a simple dual port's halves written the wrong way round | **nothing**, at first: every design folded its word to its parity, which two halves swapped leave alone, and the contents are written by parameter, not by the pins. The twelve `bits_` designs were added for it, and with them the simulation fails, on the three in simple dual port |
| parity bits in `INIT_` | the contents' unit test; the tiles; the simulation |
| every depth slice's contents from word 0 | the contents' unit test only: no memory here with contents is two blocks deep |
| no change read as read first | the tiles; the simulation |
| written first given `READ_FIRST` | the tiles; the simulation |
| the register taken and `DO<x>_REG` left at 0 | the tiles; the simulation |
| an output register's `REGCE` tied low | the simulation only |
| an unused port's widths left as the mode's | the tiles; the simulation |
| one port's collision mode left at the default | the tiles; the simulation |

Three are caught by the simulation alone, which needs `RETICLE_VIVADO`: without
Vivado installed, a simple dual port's address or halves, and an output register that
never loads, pass every test here.

### Not done

- **A cascade** for a memory deeper than 32,768, and **the address register taken
  into the block** for a read on no clock or a read-only memory. *The cascade since
  built, and kept only where the read's register goes into it; the address register
  not, and for a read on no clock it is not exact ("Deeper than a block, and single
  ports, as Vivado builds them", below).*
- **Choosing by the clock**: Vivado's synthesis moves memories to distributed RAM
  when a tight constraint asks; this flow's choice is the same at any period. *Since
  done for up to 1,024 words, below.*
- **Read-only memories as lookup tables**, which Vivado does to 4,096 by 8; not
  wanted, at the measured cost.
- **A loop of part-selects reading a memory**, as the vendor's asymmetric template is
  written, is reads on no clock in the synthesis here. *Still so in the synthesis; the
  mapper now makes such reads reads on the register's clock, below.*
- ~~**Byte writes**, still refused by the frontend (`ib_1024x32_bytes`), and so not asked
  again here.~~ *Done in "Byte writes as Vivado arranges them", below.*
- **Placing a memory's two `RAMB18E1` copies in one tile**, as Vivado does: the placer's
  (not touched here).
- None of it has been on a part.

## Eight flip-flops a slice

**Asked: what does Vivado write for a flip-flop that reaches its data through a
lookup table, and does this flow building slices of eight that way cost the clock?**
"Packing against Vivado's" left it the largest difference in its table: Vivado's
slices hold eight flip-flops where they can, four on the bypass inputs `AX` to `DX`
and four through a table used as a wire, and this flow's four. Run on 2026-11-07 at
`xc7a100tftg256-3` with Vivado 2026.1. **Nothing here has been on a part.**

### What Vivado writes

`tools/vivado-parity/gen_ffpack.py` writes `testdata/fpga/vivado/ffpack/`: eight
flip-flops fed from elsewhere held to one slice by `LOC` and `BEL`, in a `SLICEL` and a
`SLICEM`, beside tables that hold nothing (`free8`, `free8m`), a function of two inputs
(`lut2x4`), of four (`lut4x4`, `lut4x4m`) or of five (`lut5x4`), of every reset kind
(`ctrl8`, `async8`), and two Vivado cannot hold there (`mixed_ce`, one of the eight with
an enable; `two_clocks`, one on another clock). `ffpack.sh` asks Vivado for each held
slice's cells, site pips, routes and data paths and a bitstream, and `vivado.txt` is
those bitstreams decoded, a line a slice and one a lane. **Decoded, so checked**:

- **A lane whose table holds nothing**: `<L>FF` reads `O6` (`<L>FFMUX.O6`, site pip
  `AFFMUX:O6`), the table's `INIT` is `FFFFFFFF00000000`, the upper half, which is `A6`
  passed on, and the route ends at the slice's `<L>6` pin (`CLBLL_LL_A6`). The same in
  the `SLICEM`.
- **A lane whose table holds a function of four inputs or of two**: `<L>FF` reads `O5`
  (`<L>FFMUX.O5`); the table's lower half passes on one of `A1` to `A5` the function does
  not read (`F0F0F0F0` for `A3`, `FFFF0000` for `A5`; which one is Vivado's router's,
  `A3`, `B4`, `C5`, `D2` in one build) and its upper half is the function. `A6` is not
  routed, and reads one, so `O6` is still the function.
- **The lane's other flip-flop, `<L>5FF`, takes the bypass input** (`<L>5FFMUX.IN_B`) in
  every lane of every design; a flip-flop alone in its lane takes it too (`AFFMUX.AX`,
  `two_clocks`). Vivado goes through a table only for a flip-flop the bypass cannot serve.
- **Each flip-flop keeps its own initial and reset value**, a five-input one as the other:
  `ZINI` set for `INIT` 0 and `ZRST` for a reset to 0 (`FDRE`, `FDCE`), the slice's
  `FFSYNC` set for the synchronous kinds and clear for the asynchronous, `CEUSEDMUX` and
  `SRUSEDMUX` once for all eight. Given one flip-flop of eight with an enable the others
  lack (`mixed_ce`) or on another clock (`two_clocks`), Vivado placed the others
  elsewhere against their `LOC`s: the eight share one clock, enable and reset, which this
  flow's placer already holds them to (the shared pins of `place::SiteRules`).
- **Four `LUT5`s beside eight flip-flops** (`lut5x4`): Vivado kept the flip-flops and put
  the tables elsewhere, so no way beside a table of five inputs; why it dropped those
  `LOC`s it did not say (`-quiet`), so that is read off the placement, not off a message.
- **Vivado's timer puts the table in the net.** Through `B6` the net to the flip-flop was
  0.211 ns where the same hops to the bypass were 0.238, and through `B4` and `O5` 0.413,
  about what the slower pin's wire costs in the fitted model (`CLBLL:LL_B4` 198 ps,
  `LL_B6` 2); the setup was -0.058 to -0.067 ns through a table against -0.018 to -0.039
  on the bypass. So a way costs nothing of its own here beyond its pin's wire. Read off
  the probes' `report_timing`, not committed. *That setup was one edge's: "Filling
  slices" has the means over every flip-flop of the corpus, 0.122 ns through a table in
  a `SLICEL` against 0.082 on the bypass.*

### What was built

`xray::thru`. Each lane has two wires of its own, `<L>6` to the upper half's way and on to
`<L>FF`'s `D` with `<L>FFMUX.O6`, and `<L>1`..`<L>5` to the lower half's and on with
`<L>FFMUX.O5`, each hop a pip carrying the `INIT` bits it needs, so the bitstream takes
them from the route. The placer counts a way as a wire into the lane that any one signal
may take, but only while its table lets it: the upper half's while no cell holds the
table or a site excluding it (a shift register, a RAM), the lower half's beside a table
of four inputs or fewer that does not read `A6` (`place::RouteThrough`,
`ControlGroup::gates`); the pool walk stops at a way rather than taking the table's six
inputs into the flip-flop's budget. After placement, a lane whose signals fit without its
way does not get it, as Vivado takes the bypass first (`place::route_throughs_needed`),
and every way not needed is closed to the router. A table beside a way keeps off `A6`
(it leaves the router's interchangeable inputs) and has only its upper half written.

**What accounts for a way's bits**: it is in no netlist, so after decoding,
`XrayDatabase::check_route_throughs` reads every table of the fabric back and holds it to
what was placed and routed there: a way's table exactly `FFFFFFFF00000000` from `A6`, or
its pass pattern under the cell's upper half, from the input the route arrived on, with
the lane's `<L>FFMUX` set to the half it took; a cell beside a way not reading `A6`; every
table cell its own truth table; and a table nothing holds or passes through, nothing. A
build that fails any of it writes nothing. The interconnect arcs are compared as before
and are still exactly the router's (a way's pips are a slice's own, which
`arc_differences` does not count). Run on the variants below before they were right, it
refused two placements that left a way open to the router the placer had not counted,
once through a table a `SLICEM`'s RAM held.

**Feature-compared** (`tests/fpga_xray_ffpack_vivado.rs`): `free16` and `lut2x16`,
sixteen flip-flops in one tile (Vivado's by `LOC`, this flow's by a region of the tile,
since it cannot fix a cell to a site) and in the second a two-input table in every lane,
so that both tools need a way for half of them. Every lane of the tile is Vivado's,
features for features (`<L>FFMUX`, `<L>5FFMUX`, `<L>OUTMUX`, the table's halves, `ZINI` and
`ZRST`), and so is each slice's own; which lane holds which flip-flop and which free input
a lower-half way takes are the placer's and the router's, and are left out.

**Mutations**, ten, each made by a script and put back: a way counted whatever its table
holds; the walk going on past a way; every open way given to the router; `room` read
for the upper half's way too; the upper half's way from `A5`; the two muxes swapped; a
cell's lower half left under a way; no table leaving room; a lower-half way's pattern
for the wrong input. All ten fail a test. **One passed the test it was meant for**: the
wrong input's pattern passes the Vivado comparison, which sees that a lower half passes
an input on and not which (the check compares with the same pattern), and fails
`thru`'s unit test, which holds each pattern to the one Vivado's bits decode to.

### Which ways, measured

Medians of Vivado's timer over the timing corpus, as `achieved.sh` measures it; `before`
is this tree without ways, which is `achieved.txt`'s `packing` (seeds 1 to 5, rebuilt and
the same to the picosecond on two designs) and seeds 6 to 10 run here; `empty` is the
upper half's way alone, the default now, over the same ten seeds; `all` both ways and
`lower` the lower half's alone, five seeds each (ns):

| design | clock | Vivado | before | empty | all | lower |
|---|---|---|---|---|---|---|
| adders | clk | 2.042 | 2.409 | 2.423 | 2.465 | 2.467 |
| analyser_fifo | rd_clk | 3.887 | 5.330 | 5.316 | 5.365 | 5.365 |
| analyser_fifo | wr_clk | 2.417 | 3.038 | 3.022 | 3.090 | 3.063 |
| barrel | clk | 2.376 | 2.923 | 2.946 | 2.873 | 2.989 |
| bram_fifo | clk | 3.388 | 3.981 | 4.054 | 4.154 | 4.130 |
| carry_counter | clk | 1.855 | 2.385 | 2.427 | 2.397 | 2.610 |
| counter64 | clk | 3.220 | 4.008 | 3.866 | 3.921 | 3.925 |
| crc32 | clk | 2.234 | 2.759 | 2.717 | 2.633 | 2.597 |
| esad | adc_sampleclk | 2.131 | 3.066 | 2.899 | 3.031 | 3.022 |
| esad | clk_usb | 1.727 | 2.425 | 2.430 | 2.701 | 2.541 |
| fir8 | clk | 9.142 | 9.161 | 9.161 | 9.161 | 9.161 |
| fsm | clk | 1.903 | 2.333 | 2.359 | 2.299 | 2.299 |
| gray_cdc | clk | 1.466 | 1.503 | 1.503 | 1.503 | 1.503 |
| gray_cdc | clk2 | 1.267 | 1.694 | 1.694 | 1.615 | 1.615 |
| lutram32 | clk | 1.894 | 2.140 | 2.106 | 2.286 | 2.270 |
| mul18 | clk | 1.935 | 2.304 | 2.223 | 2.262 | 2.208 |
| regfile_alu | clk | 5.511 | 6.348 | 6.171 | 6.104 | 6.104 |
| sad | adc_sampleclk | 2.207 | 3.203 | 3.095 | 3.105 | 3.158 |
| sad | clk_usb | 1.726 | 2.553 | 2.542 | 2.612 | 2.612 |
| srl_delay | clk | 1.645 | 2.057 | 2.170 | 1.938 | 1.949 |
| widemux | clk | 2.438 | 3.574 | 3.517 | 3.393 | 3.466 |

**Over the twenty-one clocks, `empty` is 1.248 times Vivado's own figure against 1.256
before: 0.6% faster**, eleven clocks faster, seven slower, three the same. Over the first
five seeds alone it was 1.242 against 1.249. `all`, as Vivado has it, was 1.252 over five
seeds and `lower` 1.255: a table beside a way keeps off `A6`, its fastest input, and its
lane's flip-flop on it, and the placer has no reason to weigh that. So the lower half's
way is built, decoded and compared but not the default (`--route-throughs all`). The
worst of `empty` is `srl_delay`, 0.11 ns slower, the design whose wires "Packing against
Vivado's" read as this; under `all` it was 0.12 faster. Not followed further.

The table was measured on 73334987, before "Memories as Vivado arranges them" reached
`master`. **Measured again on top of it** (seeds 1 to 5, `--route-throughs off` and the
default): every design but `analyser_fifo` built the same, seed for seed, as above, with
ways and without, and `off` the same as `packing`, which says `off` is the flow before.
`analyser_fifo`, whose memories that section rearranged, went from 5.262 and 3.000 ns
without ways to 5.360 and 3.090 with them, about a seed's spread. The geometric mean on
that tree is 1.243 with ways against 1.248 without. `achieved.txt` keeps the run on
73334987.

Slices used, medians over seeds 6 to 10 of the same builds (every slice holding a cell),
beside Vivado's own from the first table of "Packing against Vivado's":

| design | Vivado | before | empty | flip-flops a slice, before | empty |
|---|---|---|---|---|---|
| adders | 83 | 175 | 153 | 2.8 | 3.2 |
| analyser_fifo | 71 | 131 | 125 | 2.0 | 2.2 |
| barrel | 36 | 28 | 29 | 2.7 | 2.8 |
| bram_fifo | 21 | 33 | 34 | 2.5 | 2.4 |
| carry_counter | 24 | 26 | 24 | 2.6 | 3.0 |
| counter64 | 44 | 65 | 62 | 2.2 | 2.4 |
| crc32 | 36 | 67 | 64 | 1.9 | 1.9 |
| esad | 289 | 397 | 385 | 3.0 | 3.2 |
| fir8 | 12 | 21 | 21 | 1.2 | 1.2 |
| fsm | 11 | 22 | 19 | 1.8 | 2.1 |
| gray_cdc | 9 | 15 | 15 | 2.5 | 2.5 |
| lutram32 | 17 | 24 | 19 | 3.0 | 3.7 |
| mul18 | 10 | 14 | 12 | 2.6 | 3.0 |
| regfile_alu | 104 | 137 | 140 | 2.0 | 1.8 |
| sad | 309 | 443 | 455 | 2.6 | 2.5 |
| srl_delay | 9 | 9 | 13 | 4.2 | 2.6 |
| widemux | 282 | 347 | 257 | 3.0 | 4.1 |

**1,954 slices to 1,827 over the corpus (Vivado's 1,367), 6.5% fewer**, most of it
`widemux` (347 to 257, under Vivado's 282) and `adders`; `sad`, `regfile_alu` and
`srl_delay` took more. The placer packs by wirelength and legality alone, and a way only
widens what is legal; nothing asks it for full slices, which is why most designs moved
little and Vivado is still a third smaller. `all` used 1,821 over its own five seeds,
1 to 5, which are not these.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--route-throughs off`, with a comment
why: a build with ways packs and routes differently and has not been on a board. With
this tree's release binary they are byte for byte as before: `build.sh`
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd`, `build_plus.sh` seed 4
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked.** Under
`off` no way is declared and the fabric, the placement and the routes are the ones
before.

### Not done

- **A placer that asks for full slices**, or weighs a way against the `A6` it takes from
  the table beside it: the density above is what legality alone gave. *Measured since,
  in "Filling slices": median moves took the corpus from 1,815 slices to 1,588, under
  Vivado's 1,668 for the same netlists, with no cost for full slices.*
- **The lower half's way as a default.** Built and Vivado's, and slower here; whether a
  cost for the `A6` it takes would change that is not measured. *Measured again, in
  "Filling slices": with the placer since, it is the faster, and the default.*
- **A way through a table to anything but its lane's main flip-flop**: Vivado also
  passes a table's inputs out of the slice, and a five-input flip-flop could take
  `O5` (`<L>5FFMUX.IN_A`) from a way; in these designs it never did.
- **Reticle's own timing of a way**: its model charges the flip-flop the bypass input's
  setup, 0.079 ns, where Vivado's timer had -0.06 through a table, so its estimate of
  such a path is about 0.13 ns pessimistic. Vivado's timer, not this, is what was
  measured above. *Measured since, and wrong: the -0.06 was four probes at one edge;
  over 1,790 flip-flops through a way Vivado's setup is 0.122 ns in a `SLICEL` and
  0.083 in a `SLICEM`, the model's 0.079 short of the first, not over it. Each class
  has its own figure now; "Filling slices".*
- None of it has been on a part.

## Filling slices

**Asked: what does Vivado's placement do that Reticle's does not, and what does
making Reticle's do the same do to the clock?** The sections before left four
things open: the placer has no cost that asks for full slices, nor one that weighs a
way through a table against the `A6` it takes; `--route-throughs all` matched Vivado
and was slower, for a reason not known; this flow's own model charged a flip-flop
through a way the bypass's setup, about 0.13 ns too much; and Vivado's placement and
routes of Reticle's netlist were better than Reticle's, which was read as the
placer's to close. Run on 2026-11-07 at `xc7a100tftg256-3` with Vivado 2026.1, on
the timing corpus of "Placement and routing against Vivado's own flow" and with its
`achieved.sh`. **Everything here is Vivado's timer, Vivado's placement or Vivado's
interconnect delays; nothing has been on a part.**

`tools/vivado-parity/fill.tcl` loads one Reticle build into Vivado three ways:
`placed`, Reticle's placement and routes; `netlist`, Reticle's netlist placed and
routed by Vivado; `connections`, Reticle's placement routed by Vivado (or, as
`connections_routes`, with Reticle's routes). It writes what each slice holds, the
200 worst paths of each clock with every pin's site, every flip-flop's data input
with the site pin it arrives on and its setup, or every connection between two
slices with its interconnect delay; `fill.py` reads them (`paths`, `setups`,
`distance`).

### Where the difference is

Seed 1 of the tree before this section (38f812a0), each clock's achieved period
(ns): Reticle's placement and routes; the same placement routed by Vivado with each
table's pins free (`achieved.tcl`'s `placement_free`); and Vivado's placement and
routes of the same netlist (`netlist`), all three Vivado's timer:

| design | clock | Reticle's routes | Vivado's routes, Reticle's placement | Vivado's placement and routes |
|---|---|---|---|---|
| adders | clk | 2.353 | 2.133 | 2.294 |
| analyser_fifo | rd_clk | 5.168 | 5.105 | 5.256 |
| analyser_fifo | wr_clk | 3.163 | 2.980 | 2.867 |
| barrel | clk | 3.012 | 2.564 | 2.684 |
| bram_fifo | clk | 4.042 | 3.856 | 3.629 |
| carry_counter | clk | 2.442 | 2.236 | 2.258 |
| counter64 | clk | 4.543 | 4.137 | 3.730 |
| crc32 | clk | 2.727 | 2.286 | 2.390 |
| esad | adc_sampleclk | 2.929 | 3.906 | 3.972 |
| esad | clk_usb | 2.345 | 2.454 | 2.454 |
| fir8 | clk | 9.161 | 9.161 | 9.164 |
| fsm | clk | 2.420 | 2.137 | 1.986 |
| gray_cdc | clk | 1.414 | 1.414 | 1.447 |
| gray_cdc | clk2 | 1.615 | 1.623 | 1.572 |
| lutram32 | clk | 2.073 | 1.879 | 1.788 |
| mul18 | clk | 2.420 | 1.998 | 1.925 |
| regfile_alu | clk | 5.935 | 5.926 | 5.610 |
| sad | adc_sampleclk | 3.110 | 3.925 | 4.358 |
| sad | clk_usb | 2.395 | 2.592 | 2.497 |
| srl_delay | clk | 2.237 | 1.888 | 1.644 |
| widemux | clk | 3.514 | 2.703 | 2.800 |

Against Vivado's own flow, the geometric means of seeds 1, 2 and 3 are 1.253, 1.247
and 1.266 with Reticle's routes, 1.198, 1.189 and 1.197 with Vivado's on the same
placements, and 1.185 for Vivado's placement and routes of the netlist. **So of
what was read as the placer's, about one part in six is the placement and five the
routes**: Reticle's placements routed by Vivado are within 0.4 to 1.2 % of Vivado's
own placement of the same netlist, and Vivado's routes are 4 to 6 % faster than
Reticle's on the same placement, up to 23 % on one clock (`widemux`). The rest of
the distance to Vivado's own flow, 18 %, is the netlist. The reading in "Placement
and routing against Vivado's own flow", that the routes are within 0.25 ns of
Vivado's on the same placement, was of six designs with the legacy netlist and the
pins locked or free; with today's netlists it is up to 0.81 ns (`widemux`), 0.45
(`barrel`, `crc32`) and 0.42 (`mul18`).

What the routes lose is at least partly visible. `srl_delay`'s latest path is a
flip-flop, a table and a table in one tile: Reticle's route from the second table to
the third goes `LOGIC_OUTS10`, `SE2`, `WL1`, `NL1`, `IMUX2`, 0.80 ns, where Vivado's
on the same placement goes `LOGIC_OUTS10`, `FAN_ALT5`, `FAN_BOUNCE5`, `IMUX_L11`
onto another input of the same table, 0.43. And over every connection between two
slices of fanout four or less in eight of the designs (`fill.py distance`), a
column of interconnect costs 33 ps on Vivado's routes and 56 on Reticle's, on the
same placements, where a row costs 34 and 42. Neither is changed here: it is the
router's, and this section is the placer's.

### What Vivado's placement does

Slices, seed 1 of the same tree: Vivado's own flow (from "Packing against Vivado's"),
Vivado's placement of Reticle's netlist, and Reticle's, before this section and
after it (the medians of ten seeds), with the flip-flops a slice holding any:

| design | Vivado | Vivado, Reticle's netlist | before | after | flip-flops a slice, Vivado on the netlist | before | after |
|---|---|---|---|---|---|---|---|
| adders | 83 | 84 | 152 | 113 | 5.9 | 3.2 | 4.4 |
| analyser_fifo | 71 | 101 | 115 | 107 | 2.4 | 2.2 | 2.4 |
| barrel | 36 | 33 | 30 | 24 | 2.3 | 2.8 | 3.3 |
| bram_fifo | 21 | 25 | 34 | 27 | 3.2 | 2.5 | 3.4 |
| carry_counter | 24 | 37 | 28 | 22 | 1.9 | 2.7 | 3.4 |
| counter64 | 44 | 48 | 62 | 43 | 3.6 | 2.4 | 3.4 |
| crc32 | 36 | 52 | 65 | 56 | 1.6 | 1.9 | 1.9 |
| esad | 289 | 388 | 386 | 356 | 3.4 | 3.2 | 3.4 |
| fir8 | 12 | 12 | 22 | 17 | 2.6 | 1.2 | 1.5 |
| fsm | 11 | 15 | 20 | 14 | 2.5 | 2.0 | 2.7 |
| gray_cdc | 9 | 21 | 12 | 12 | 2.3 | 3.0 | 3.2 |
| lutram32 | 17 | 21 | 20 | 16 | 3.7 | 3.6 | 4.5 |
| mul18 | 10 | 9 | 12 | 8 | 4.0 | 2.9 | 4.8 |
| regfile_alu | 104 | 152 | 140 | 130 | 1.5 | 2.0 | 2.0 |
| sad | 309 | 394 | 448 | 404 | 3.0 | 2.6 | 3.0 |
| srl_delay | 9 | 10 | 10 | 10 | 3.4 | 3.2 | 3.4 |
| widemux | 282 | 266 | 258 | 229 | 4.1 | 4.1 | 4.6 |
| all | 1,367 | 1,668 | 1,815 | 1,588 | | | |

**Measured**: given Reticle's netlist, Vivado's placer takes 1,668 slices, not
Vivado's own 1,367; most of that difference is the netlist too. Reticle took 1,815,
8 % more than Vivado on the same netlist, most of it `adders` (152 to 84) and `sad`
(448 to 394). Where Vivado's is denser the connections say how: on `adders`, 14 % of
the connections from a flip-flop to a table are inside one slice and 59 % inside one
tile in Vivado's, none and 42 % in Reticle's, 0.34 tiles long against 0.98
(`slices.py connections`). The worst paths say the rest (`fill.py paths`, the 20
worst of each, tiles a hop and tiles a path):

| design | Vivado on the netlist | before | after |
|---|---|---|---|
| adders | 0.82, 8.0 | 0.83, 7.9 | 0.83, 7.1 |
| counter64 | 1.21, 10.9 | 4.14, 29.0 | 1.42, 12.8 |
| crc32 | 1.55, 4.7 | 2.30, 6.9 | 1.97, 5.9 |
| regfile_alu | 3.27, 25.6 | 3.40, 23.1 | 2.25, 25.8 |

`counter64`'s is what nothing else showed. Its latest path began at one of `limit`'s
sixty-four flip-flops, at `SLICE_X14Y108`, forty tiles from both its neighbours in
the shift register and from the table that reads it; on four seeds of five one of
them was so stranded. The anneal had sent it there at a hot temperature and its
window had shrunk to a tile or two before it came back. And the placement's own
estimate of `counter64` was 10.42 ns where its routes were 4.07: it charged the
carry chain's cascade from one `CARRY4` to the next, and every table into its carry
or flip-flop, the line's 0.414 ns, where Vivado's timer has nothing. Every
criticality of every timing round was against that, and the chains looked critical
while the real worst paths did not.

The clock is the last of it. Vivado puts `sad` inside one clock region row (its own,
309 slices in the row of `Y100` to `Y149`; on Reticle's netlist 361 of 394), and
Reticle spread it over two on most seeds. **Measured**, `clock_arrival.tcl`: one
global buffer's clock reaches the outer two region rows 120 to 150 ps later than the
inner two, and the columns past the outermost block RAM columns 40 to 80 ps later
than the rest (from `BUFGCTRL_X0Y0`, `X0Y16` and `X0Y31` alike, within 3 ps), so a
path from the top row to the one below loses 0.15 ns of its period. Two seeds of ten
of `sad`'s adc clock had their latest path across the rows, Vivado's skew -0.245 and
-0.247 ns where a path inside a row had -0.03 to -0.05; Reticle times none of it.

### What was changed

All four under the fitted delay model, the default; `--delay-model legacy` builds as
before (below).

1. **A flip-flop's setup by how its data arrives** (`xray::place_timing_on_sites`).
   Vivado's setup over the 13,224 flip-flops of three seeds of the corpus, classed by
   the site pin it names (`fill.py setups`):

   | slice | data | flip-flops | setup, mean (ns) |
   |---|---|---|---|
   | `SLICEL` | inside the slice | 3,564 | -0.039 |
   | `SLICEL` | the bypass, `<L>X` | 4,867 | 0.082 |
   | `SLICEL` | through the table, `<L>6` | 1,317 | 0.122 |
   | `SLICEM` | inside the slice | 1,400 | -0.082 |
   | `SLICEM` | the bypass | 1,603 | 0.058 |
   | `SLICEM` | through the table | 473 | 0.083 |

   The model had -0.050 and 0.079, measured before there were ways, for both slices.
   It charges each class its mean now, a way's from the ways the router is given.
   *The reading this replaces*, in "Eight flip-flops a slice", that a flip-flop through
   a way was charged 0.13 ns too much: that was Vivado's setup on four probes at one
   edge, -0.06 ns. A pin's setup is one of two figures about 0.1 ns apart (by
   something not read; the histograms are two humps), and the way's mean is 0.04 over
   the bypass's in a `SLICEL`, not 0.13 under it. The lower half's way, rarer (54
   flip-flops in seed 1 of `--route-throughs all`), had 0.11 in a `SLICEL`: the
   upper half's figure is used for both. Neutral by Vivado's timer, 1.244 against
   1.243 over five seeds; kept because it is the measured figure.
2. **A dedicated group's own wires cost nothing in the estimate**
   (`PlaceTiming::local_wire`, `PlacementReport::local`). `build_clusters` records
   the sink of each link it makes, and such a connection costs nothing in the timing
   rounds and in the router's first pass. `counter64`'s first estimate is 5.22 ns.
   Over five seeds 1.241 against 1.244, inside a seed's spread; kept for the estimate.
3. **Median moves** (`PlaceOptions::median_moves`, `median_region`). One draw in
   eight is VPR's median move: the destination drawn from between the lower median of
   the low edges of the boxes of the cell's signals and the upper median of their
   high edges, the cell left out of them, wherever that is; for a macro, of its
   members' signals; signals of more than 64 sinks left out. `counter64`'s worst
   path is 12.8 tiles long, Vivado's 10.9. Over five seeds 1.242 against 1.241, and
   1,612 slices against 1,824. A first version, which took the median of both edge
   lists the same way, was 1.236 and 1,576 slices over five seeds, with `sad`'s adc
   clock 3.39 ns against 3.11 and four seeds of five across two region rows; the
   difference between the two is a seed's (`sad`'s adc clock 3.06 in the second).
4. **When the clock reaches each register** (`xray::timing::CLOCK_ARRIVAL_PS`, now `XC7A100T_CLOCK`,
   `PlaceTiming::clock`, `PlaceTiming::skew`). The probe's 2,100 flip-flops by region
   row and column, every one within 6 ps of the table
   (`tests/fpga_xray_clock_arrival.rs`); a register launches and must settle that much
   later, in the rounds and in the router's analysis, and the pull charges a critical
   connection from a later site to an earlier one the difference, in tiles of wire, on
   top of its distance. Only the `xc7a100t` had figures (the `xc7a35t` has since:
   [The clock's arrival on the XC7A35T](#the-clocks-arrival-on-the-xc7a35t)). Over
   five seeds 1.244 against
   1.242; over ten, with the three before, none of `sad`'s ten latest paths crosses a
   region row (the worst skew -0.11 ns), and Reticle's routed estimate is 0.981 of
   Vivado's figure at the geometric mean, 0.935 to 1.015 from the 5th to the 95th
   percentile, against 0.979 and 0.927 to 1.022.

Tried and not kept, each over five seeds against the third change's build (1.242):
a fully critical connection's weight doubled, 1.243; six timing rounds rather than
three, 1.234, for up to twice the rounds' time; and, against the fourth change's
build (1.244), keeping the round whose estimate was best rather than the last, 1.241,
which twelve builds of the 85 took. A
horizontal tile being cheaper than a vertical one was measured and not tried: over
connections of fanout four or less a grid column costs 14 ps and a row 33 on Vivado's
routes, 25 and 40 on Reticle's, where the fitted line charges 36 for either. In
columns of interconnect the two are the same, 33 and 34, so an isotropic cost in
those columns would be the way to it.

### Before and after

Medians over ten seeds (ns); `before` is 38f812a0, `after` the four changes above,
and `default` those with `--route-throughs all` (below), which is the default now:

| design | clock | Vivado | before | after | default |
|---|---|---|---|---|---|
| adders | clk | 2.042 | 2.423 | 2.375 | 2.390 |
| analyser_fifo | rd_clk | 3.887 | 5.204 | 5.293 | 5.375 |
| analyser_fifo | wr_clk | 2.417 | 2.995 | 3.102 | 2.984 |
| barrel | clk | 2.376 | 2.946 | 2.929 | 2.942 |
| bram_fifo | clk | 3.388 | 4.054 | 4.060 | 4.046 |
| carry_counter | clk | 1.855 | 2.427 | 2.357 | 2.404 |
| counter64 | clk | 3.220 | 3.866 | 3.958 | 3.869 |
| crc32 | clk | 2.234 | 2.717 | 2.588 | 2.556 |
| esad | adc_sampleclk | 2.131 | 2.899 | 2.994 | 3.009 |
| esad | clk_usb | 1.727 | 2.430 | 2.455 | 2.523 |
| fir8 | clk | 9.142 | 9.161 | 9.161 | 9.161 |
| fsm | clk | 1.903 | 2.359 | 2.397 | 2.391 |
| gray_cdc | clk | 1.466 | 1.503 | 1.413 | 1.413 |
| gray_cdc | clk2 | 1.267 | 1.694 | 1.554 | 1.554 |
| lutram32 | clk | 1.894 | 2.106 | 2.101 | 2.186 |
| mul18 | clk | 1.935 | 2.223 | 2.220 | 2.220 |
| regfile_alu | clk | 5.511 | 6.171 | 6.269 | 6.223 |
| sad | adc_sampleclk | 2.207 | 3.095 | 3.068 | 3.021 |
| sad | clk_usb | 1.726 | 2.542 | 2.530 | 2.488 |
| srl_delay | clk | 1.645 | 2.170 | 2.085 | 1.898 |
| widemux | clk | 2.438 | 3.517 | 3.639 | 3.503 |

**Over the twenty-one clocks, Reticle's median is 1.231 times Vivado's own figure
by default now, 1.239 after the four changes and 1.247 before**; after against
before, eleven clocks faster and nine slower. One median is past the
quartile of its own before (`regfile_alu`, 6.171 to 6.269 ns, 1.6 %), and three are
better past theirs (`gray_cdc`'s two clocks, `sad`'s adc clock). Routed by Vivado (seeds 1 to 3, `placement_free`), the placements after are 1.189
of Vivado's own flow against 1.194 before, and 1.001 of Vivado's placement and routes
of the same netlist: on this corpus the placement is Vivado's, and what is left
between the two tools' implementations of one netlist is the routes. And 1,588
slices against 1,815.

**`--route-throughs all`, measured again** on the tree after, ten seeds: 1.231 against
1.239, slices the same (1,589); eleven clocks faster, seven slower, none past its
quartile and three better past it (`analyser_fifo`'s write clock, `sad`'s adc clock,
`srl_delay` 2.085 to 1.898 ns). On the tree before "Filling slices" it was 1.252
against 1.248 over five seeds; why the two differ is not known. Vivado has both halves; so `all` is the default now
(`xray::RouteThroughMode`), and `empty` and `off` are as they were.

Runtime: counted where it could be, and timed two runs each side by side,
seed 1. The Husky's builds are the legacy ones and place and route as before (below).
`counter64` placed and routed in 19.9 and 19.4 s against 26.1 and 22.0 before, its
anneal 685 thousand moves against 756 (medians of ten seeds). The larger pieces cost
more: `sad` 51.6 and 57.6 s against 35.2 and 36.2, `esad` 40.2 and 44.5 against 32.6
and 35.8, `widemux` 29.4 and 30.3 against 27.2 and 24.5; `sad`'s anneal 14.9 million
moves and 896 million cost pin reads against 11.6 and 597. Any one of the three
placement changes alone brings most of that (13.5 to 13.9 million moves with one of
them off, 11.6 with all three): `sad`'s third timing round found nothing to move on
the estimate before and stopped, and every round moves something now. Routing is
faster on every one of them (`sad` 3.3 to 4.3 s against 4.8 to 4.9).

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--delay-model legacy` and
`--route-throughs off`, and build as before: with this tree's release binary
`build.sh` writes `3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd`
and `build_plus.sh` seed 4's
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked.**
Under the legacy model the setups are its one figure, no connection is local
(`local_wire` is none), there are no median moves and no clock arrival.

### Tests

`a_flip_flops_setup_is_the_one_for_how_its_data_arrives` (`xray::timing`) holds the
setups on a fabric of two tiles and three flip-flops, each way open and shut;
`a_dedicated_connection_costs_its_own_figure_in_the_estimate` the marks and the
estimate, with `a_flip_flop_that_registers_a_carry_joins_its_macro` and
`a_lookup_table_takes_the_flip_flops_it_feeds_on_one_control_set` the marks of the
registered groups; `a_median_move_brings_a_stranded_cell_back` a cell ninety-one
tiles from its nearest neighbour brought back by a cold anneal, which without median moves
walks it a tile at a time; `a_later_clock_launches_later_and_the_pull_charges_its_fall`
the skew in the analysis and the pull; `the_clock_reaches_a_site_when_the_table_says`
the table's lookup, and `tests/fpga_xray_clock_arrival.rs` every flip-flop the probe
timed within 6 ps of it. `tests/fpga_xray_achieved.rs` reads this section's runs
(`tree`, `fill` and `fillall` in `achieved.txt`, ten seeds each, and each build's
slices): the default no slower than the tree before over the corpus, no clock more
than 6 % over its own before, and a tenth fewer slices.

**Mutations**, seventeen, each made by a script and put back, each failing its test:
the ways ignored; every open way taken for every flip-flop; the two slices' figures
swapped; a dedicated link's, a registered table's and a registered carry's sink not
recorded; the estimate ignoring the mark, and applying it with no figure; median moves
never drawn; the cell's own pin kept in its boxes; both edges by one median; the
capture's skew signed wrong; skew given to a cell in no clock's domain; the pull
charging the difference both ways; the growth from the `HCLK` row left out; the
region row read from the wrong `HCLK` row; the table applied to any part. **One passed
the test it was meant for at first**: skew given to a cell in no domain, because the
fixture's other sites were at zero; the test gives every site 0.1 ns now and catches
it. And the fixture's builds swapped, `tree` for `fillall` and the slices of `tree`
for `fill`'s, each fail `tests/fpga_xray_achieved.rs`.

### Not done

- **The router.** Five sixths of the difference between Reticle's implementation and
  Vivado's placement and routes of the same netlist, by the first table: a column of
  interconnect is 56 ps on its routes and 33 on Vivado's, and a local route inside a
  tile goes out and back where Vivado's takes a bounce. Not looked at further here;
  taken up in [Routing against Vivado's routes](#routing-against-vivados-routes),
  where `srl_delay`'s latest connections at seed 1 ended on a slow input of their
  table whose fast one another connection's route had taken, rather than wanting a
  bounce.
- **A cost in columns of interconnect** rather than in grid tiles, which the
  connection delays above suggest; not tried.
- **The clock's arrival on any part but the `xc7a100t`**, which needs the probe run
  on it, and of the block RAM and `DSP48E1` columns, which take their nearest slice
  column's figure, not measured. **The `xc7a35t` done since**, in both packages
  ([The clock's arrival on the XC7A35T](#the-clocks-arrival-on-the-xc7a35t)); the
  block RAM and `DSP48E1` columns are still not measured on either.
- **Why the setup has two figures** a pin, about 0.1 ns apart.
- None of it has been on a part.

## Byte writes as Vivado arranges them

**Asked: does a memory written a byte at a time (`mem[a][8*i +: 8] <= d[8*i +: 8]`
under `we[i]`, the vendor's byte-write template, and its other spellings) simulate
as xsim simulates it, and is it built as Vivado builds it?** Before, the frontend
refused it ("Block RAM", the correction there), having first written the wrong
element or nothing. Run on 2026-11-07, Vivado 2026.1, `xc7a100tftg256-1`.
**Everything here is Vivado's synthesis, bitstreams and simulator; nothing has been
on a part.**

### Simulation

The IR has a target for part of an element, `Lvalue::MemPart` (`docs/ir.md`), and
both simulators take it. **Measured**: `testdata/sim/mem_part_tb.v` and
`mem_part_sv_tb.sv`, testbenches written here, print every case (a select partly and
wholly outside the element, an `x` in the address, the index or an enable, elements
ascending, signed, not counted from zero and packed arrays of bytes, the clocked
byte-write loop, two non-blocking writes of one word in one step) and Reticle's event
simulator prints what xsim printed, line for line; `mem_part_tb.vhd` holds itself to
xsim's values for VHDL's `mem(a)(15 downto 8) <= d`. `docs/simulation.md` has the rest.
This ended "and what is left: a *read* partly outside an element reads all `x` here
where xsim reads the bits inside, as for any vector"; a read reads the bits inside now,
as xsim does (`docs/simulation.md`, *The language, against xsim*).

### What Vivado does

`testdata/fpga/vivado/memarr/` has 53 more designs, `bw_` (`gen_memarr.py`): lanes of
8 and 9 bits, widths 16 to 72, depths 16 to 65,536, simple, single (read first, written
first, unchanged) and true dual port, the loop, a process per lane out of a `generate`
and constant bounds, with and without contents, and lanes of 4 and of 16. **Measured**,
read off Vivado's netlists (`vivado.txt`'s `params`, and the pins of its `.cells`):

- **The arrangement is a word's, counted in lanes of nine bits, where a block at the
  depth is 9 bits wide or more.** A lane of eight leaves its ninth, parity, pin unused;
  a lane of nine puts bits 0 to 7 on `DIADI[8k+7:8k]` and bit 8 on `DIPADIP[k]`
  (`bw_sdp_1024x36_b9`). So `bw_sdp_1024x32_b8` is one `RAMB36E1` 36 wide, as
  `sdp_1024x32`; but `bw_sdp_4096x16_b8` is two `RAMB36E1` of 9 a byte each where
  `sdp_4096x16` is 9 bits and 7, `bw_sdp_2048x24_b8` a `RAMB36E1` of 18 and a
  `RAMB18E1` of 9, and `bw_sdp_1024x72_b8` two `RAMB36E1` and a `RAMB18E1` for the ninth
  byte where `sdp_1024x72` is two `RAMB36E1`. Simple dual port at 512 words as before,
  a lane a bit of `WEBWE`.
- **Each lane's enable is a write enable**, `WEA[k]` or `WEBWE[k]`; a lane of sixteen
  drives two (`bw_sdp_1024x32_b16`). Where a block holds one lane (9 bits or fewer)
  Vivado ties `WEA[0]` high and the others low and puts the lane's enable on `ENARDEN`.
- **Deeper, each lane is arranged alone**: `bw_sdp_8192x16_b8` four `RAMB36E1` of 4,
  `bw_sdp_16384x16_b8` eight of 2; at 65,536 words, 32 `RAMB36E1` cascaded in pairs.
- **Lanes of four are not byte enables**: `bw_sdp_1024x16_b4` is a `RAMB18E1` a lane.
- **Distributed RAM, up to 32 words, is a `RAM32X1D` a bit** (`bw_sdp_16x32_b8`,
  `bw_sdp_32x16_b8`, `bw_sdp_32x24_b8`), where a memory written a word at a time is
  `RAM32M`s; the line between block and distributed is the whole memory's 1,024 bits
  (`bw_sdp_32x32_b8` and `bw_sdp_64x16_b8` are a `RAMB18E1`).
- **A single port read only while no lane is written is `NO_CHANGE`**
  (`bw_sp_nc_*`); read first and true dual port as before.
- **A single port read first lane by lane is single-port distributed RAM**,
  `RAM256X1S`, 128 of them for `bw_sp_wf_1024x32_b8`, 32 kbit in lookup tables.
- **A single port of 32 by 32 is both ports of a `RAMB18E1`, a half each**
  (`bw_sp_rf_32x32_b8`).
- The constant-bounds spelling builds; `bram/ib_1024x32_bytes`, the same writes on a
  shift register's bits directly, crashes `synth_design` ("Block RAM").

### What was changed

1. **The frontends**: `mem[a][j]`, `mem[a][7:0]`, `mem[a][j +: 8]`, `mem[a][j -: 8]` and
   SystemVerilog's selects of a packed element, and VHDL's static slices and indices of
   an element, are `MemPart`s. A continuous assignment to part of an element and a VHDL
   slice whose bounds move with a loop's index are refused.
2. **Synthesis**: a `MemPart` is a write port whose enable has a bit per bit of the
   element; ports of one address and clock that follow one another are merged into one
   (`synth::proc::write_ports`), the later winning a bit two write.
3. **The mapper**: a memory whose lanes are bytes is mapped with the byte write enables
   (`primitives::vivado_lane_columns`), its contents through the same pins; one read
   only when no lane is written is a no-change pair; a lane in distributed RAM is a
   `RAM32X1D` a bit up to 32 words. Anything else, and the other families and
   `--memories legacy`, are built a memory per lane (`fpga::lanes`), exactly.

### What it is now

**Measured** (`tests/fpga_xray_memarr_vivado.rs`): of the 53, **48 get Vivado's cells**;
**all 75 of their block RAM tiles are Vivado's**, site feature for site feature and
contents bit for contents bit, the four with contents and the nine-bit lanes' parity
included; every set bit decodes, none unexplained, and every mapping is proved; and
**every one of the 53 routed netlists answers as Vivado's build does**, 2,000 cycles
under xsim on Vivado's models. One thing on the pins is not Vivado's and no tile shows
it: a block holding one lane has the lane's enable on its write enable and the port's
on `ENARDEN` here, where Vivado ties the write enable high; the simulation is what says
the two compute alike. The five that are not Vivado's arrangement, each held
to what it is instead:

- `bw_sp_rf_32x32_b8`: one `RAMB36E1` on port A, where Vivado splits the word over both
  ports of a `RAMB18E1`. No single port is split here.
- `bw_sp_wf_1024x32_b8`, `bw_sp_wf_2048x18_b9`: 192 `RAM64M`, where Vivado has 128 and
  144 `RAM256X1S`; the lane-by-lane choice leaves a read on no clock, and this flow has
  no single-port distributed RAM.
- `bw_sdp_65536x16_b8`, `bw_sp_rf_65536x16_b8`: the same 32 `RAMB36E1`, stacked two deep
  with a read multiplexer, where Vivado cascades them; no cascade is built, as for
  `sdp_65536x1`. These two are what simulates byte writes into blocks stacked, whose
  enables, a depth slice's own, are what holds the address there.

`bram/ib_1024x32_bytes` is one `RAMB36E1`, as `bw_sdp_1024x32_b8_const`, and its routed
netlist answers as its own source does under xsim (memory and registers zeroed as a
part's are), for all 2,000 cycles.

The 108 designs of the memarr corpus before these, and the rest of `bram/`, are as they
were: Vivado's cells, tiles and simulation, held by the same tests.

**Nothing else changes.** Every design of `memarr/` before these, of `bram/` but
`ib_1024x32_bytes`, of `qor/` and `analyser_fifo`, built by this tree's release binary
and by `origin/master`'s (70f566ad), is the same file byte for byte: 161 bitstreams with
the default memories, 121 with `--memories legacy`, and 57 mapped netlists each on the
ECP5, the iCE40 and the Gowin, 453 builds (**checked**; the script is not kept, the
answer is). No Trellis or Apicula database is here, so those families' bitstreams were
not built. A byte-write design on them is a memory per lane, mapped as any memory is
(`bw_sdp_1024x32_b8` is four `DP16KD` on the ECP5 and eight `SB_RAM40_4K` on the
iCE40, `mapping: equivalent`); on the Gowin, which declares no block RAM, 32 kbit is
left a memory and refused, as a memory of that size written a word at a time is.

### The Husky's bitstreams

With this tree's release binary (rebased on 70f566ad) `examples/husky/build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and
`build_plus.sh` seed 4's `e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`,
the files they were. **Checked.**

### Tests

- `tests/fpga_xray_memarr_vivado.rs`, above; `regenerate` now keeps what `vivado.txt`
  says of a design a run of `lutram.sh` did not reach, and `KEEP=1
  gen_lutram_vectors.py` the answers `xsim.txt` has, so that a run of the designs
  added is enough.
- `vivado_arranges_byte_lanes_as_its_netlists_say` holds `vivado_lane_columns` to one
  case per rule above, pins included; `write_ports`' tests the merge (lanes merge, a
  port at another address between keeps them apart, the later port wins a bit, a whole
  word written in part is a word written); `fpga::lanes`' tests the split, with a
  bounded formal check that a memory split into lanes computes what it did and that the
  same split with its reads crossed does not.
- `tests/fpga_xray_bram_vivado.rs` holds `ib_1024x32_bytes` to one `RAMB36E1` and to its
  source's answer.
- `tests/cli.rs` holds the simulation to xsim's answers, and the refusals.

**Mutations**, each made by a script in a copy of the tree and run against the unit
tests named, `tests/fpga_xray_memarr_vivado.rs` with Vivado's simulator and
`tests/fpga_xray_bram_vivado.rs`'s simulation (the mapper's), or `tests/cli.rs`,
`tests/sim_compiled.rs`, `tests/synth_golden.rs` and the self-check (the rest):

| mutation | what failed |
|---|---|
| the lanes' write enables in the other order | the simulation only: a write enable is no tile feature |
| a lane's ninth bit on a data pin | the arrangement's unit test; then no build at all |
| lanes arranged as a word's | the cells; the tiles; the simulation |
| no merging of one address's ports | the merge's unit tests; the cells; the tiles |
| the no-change pair of lanes not taken | the tiles only (`bw_sp_nc_*`) |
| a lane in distributed RAM not a bit at a time | the cells only |
| contents laid out as a word's, not through the pins | the tiles; the simulation (the two with lanes of nine and contents) |
| merged, the earlier port winning a bit | the merge's unit test only: no design here writes a bit twice at once |
| the event simulator writing part of an element at bit 0 | the xsim tests of `tests/cli.rs`; the two engines' agreement |
| the compiled engine's mask taken from the data | the two engines' agreement only |
| the frontend's offset on a descending element reversed | the xsim tests of `tests/cli.rs` |
| synthesis enabling the whole element for part of it | the synthesis golden files; the self-check |

A first run had three more designs failing under two of these, which a rerun did not:
the machine was building the Husky beside it, and the largest design's build (32 blocks)
did not finish. A write enable a depth slice's address gated, tried as a mutation,
changed nothing that any test sees, and rightly: the block's enable already holds the
address. It is not built.

### Not done

- ~~**A single port split over both ports of a block** (`bw_sp_rf_32x32_b8`), and
  **single-port distributed RAM** (`RAM256X1S`, `bw_sp_wf_*`).~~ *Both done, in "Deeper
  than a block, and single ports, as Vivado builds them", below.*
- **A cascade**, as before. *Built since, below, where the read is registered again;
  stacked otherwise, which Vivado's timer finds faster.*
- **Byte write enables on the ECP5** (`DP16KD` has them in its 18 and 36 bit modes) and
  on the other families: a memory written a lane at a time is a memory per lane there,
  which is exact but is more blocks.
- ~~**A VHDL slice whose bounds move with a loop's index**~~, which the VHDL
  byte-write loop is written with: the loop is unrolled now, and builds what one `if`
  per lane builds.
- ~~**A read partly outside an element** reads all `x`, as of any vector~~: it reads
  the bits inside now, as xsim does.
- Distributed RAM deeper than 32 words for a lane is built by the rule for a whole
  word, which is not measured for one: every byte-write memory measured deeper than 32
  words is over 1,024 bits, Vivado's block RAM.
- None of it has been on a part.

## Deeper than a block, and single ports, as Vivado builds them

**Asked: of what "Memories as Vivado arranges them" and "Byte writes as Vivado arranges
them" left undone (a cascade deeper than 32,768 words, single-port distributed RAM, a
single port split over both ports of a block, the address register taken into a
block, Vivado's choice by the clock, the asymmetric template), which does Vivado do
better, and where it does, is it built so now?** Run on 2026-11-08, Vivado 2026.1,
`xc7a100tftg256-1` for the arrangements, bitstreams and simulation, `-3` for the
periods (`achieved.sh`, five seeds). **Everything here is Vivado's synthesis,
bitstreams, simulator and timer; nothing has been on a part.**

`gen_memarr.py` writes 42 more designs, last so that the contents of every design before
are what they were: deeper than a block (40,000 to 131,072 words; single, true dual
port, a register after the read, contents, read-only), a single port written on the
clock and read on none (`sp_as_`, 16 to 512 words) and single ports read first,
written first or unchanged of up to 1,024 words, and single ports of up to 512 words
18 to 72 bits wide. `FIXTURE=memarr lutram.sh`'s probe answered each, `vivado.txt` and
`xsim.txt` keep the answers. One more, `sdp_65536x4`, was answered and is not kept: at
the test's flags it does not build here, with this tree or with `origin/master`'s
(`` `$lut4` has two signals routed to one input ``, `xray::lutswap`), a fault of the
lookup tables' pin swap and not of the memory, found and not followed; with
`achieved.sh`'s timing-driven flags it builds.

**Followed since (2026-11-09), and the reading was half right.** It is kept now,
answered as above (eight `RAMB36E1` cascaded by Vivado; stacked two deep here, so it
is in the test's `NOT_AS_VIVADO` with the other cascades). Two things met in it,
**measured** by printing the table the error named. The read multiplexer's select
is the read address's top bit registered, and the parity it is folded into reads
the next bit of the shift register, the same `D` on the same clock: `srl::merge`
made the two flip-flops one *after* the mapper had given the table both, so the
table read one signal on `I4` and `I5` (`0xf00f99990ff06666`). That is not wrong,
but `lutswap` offered both inputs to the router, which ended `I4`'s sink on `A6`,
`I5`'s own input, not yet in the tree, and then routed `I5` to `A6` as well. So the
memory's arrangement made the table and the pin swap could not follow it. Both are
changed: a table reading one bit twice after the merge is folded to read it once,
the other input tied to zero (`srl::fold_repeated_inputs`), and the second of two
inputs carrying one signal is never offered to the router
(`lutswap::movable_inputs`). Either alone builds it; both undone, it fails again.
**Checked** by undoing them: both undone, the memarr test fails on `sdp_65536x4`
(`$lut4`) and on `sdp_4096x9` under `--memories legacy` (`$lut10`, now routed by
`legacy_memories_map_as_before`); the fold undone, or made to read the folded input
as zero, fails `srl`'s unit test; the second input offered again fails `lutswap`'s.

What else it touched, **measured** by building every design of `memarr/`, `bram/`,
`qor/` and `lutram/` at the test's flags, `memarr/` again under `--memories legacy`,
and the language corpus's 25 synthesisable modules (default parameters, and
`gadd`'s `W` 8 and 12) for the XC7A35T, with the old behaviour and a probe for
tables reading one signal twice: at the default flags seven `memarr` designs had
such a table (`sdp_65536x1`, `sdp_65536x2`, `sdp_40000x1`, `init_65536x1`,
`rom_65536x1`, `bw_sdp_65536x16_b8` and this one), and only this one failed; under
`--memories legacy` 28 had one and six failed the same way (`sdp_65536x2`,
`sdp_4096x9`, `sdp_3000x12`, `bits_sdp_16384x9`, `bits_sdp_16384x18`,
`bw_sdp_4096x32_b8`). No `bram/`, `qor/`, `lutram/` or language-corpus design had
one. With the change all 35 build, no table reads a signal twice, and every bit
decodes. The seven default ones' routed netlists answer as Vivado's builds do under
xsim (`a_routed_netlist_answers_as_vivados_build_does`), and a fold that read the
folded input as zero made all seven differ, so that test sees the fold; the legacy
builds were not simulated. The Husky builds pass `--srl-min 0`, so nothing is
merged and nothing folded, and the Husky design has no table reading a signal twice
(probed): its bitstream is the one from before byte for byte.

### What Vivado does

**Measured**, read off `vivado.txt`'s parameters, the probes' cells and Vivado's
netlists:

- **Deeper than 32,768 words, a bit is two `RAMB36E1`s cascaded**: `RAM_EXTENSION_<x>`
  `LOWER` on the block holding words 0 to 32,767 and `UPPER` on the other, the lower's
  `CASCADEOUT<x>` on the upper's `CASCADEIN<x>`, the read off the upper alone, both
  blocks given all 16 address bits, bit 15 picking the block (`sdp_40000x1`,
  `sdp_65536x1`, `sdp_65536x2`'s four, `bw_sdp_65536x16_b8`'s 32). A single port
  cascades port A alone, `RAM_EXTENSION_B` `NONE` (`sp_rf_65536x1`, `sp_wf_65536x1` and
  `sp_nc_65536x1` in their write modes); a true dual port both (`tdp_65536x1`); a
  register after the read is both blocks' `DOB_REG` (`reg_65536x1`); 131,072 words are
  two cascaded pairs and a read multiplexer, one pair's enables through their
  inverters (`sdp_131072x1`). In the bitstream the cascade is one feature,
  `RAMB36.RAM_EXTENSION_<x>_LOWER` in the lower block's tile; `UPPER` and `NONE` are the
  same clear bit (the database's `RAM_EXTENSION_<x>_NONE_OR_UPPER`), and the route is
  `CASCADEOUT<x> -> CASCADEOUT<x>_1`, an `always` pip, to the tile five rows up, no bit
  at all. Vivado's model of the cell names the halves the same way: a block reads when
  `ADDR[15]`, inverted on the `UPPER`, is 0, and the upper's output is `CASCADEIN` when
  the address was the lower's (`RAMB36E1.v`, read and not kept). A read-only memory of
  65,536 by 1 is not a cascade but 60 lookup tables and their multiplexers
  (`rom_65536x1`).
- **A single port in distributed RAM is a single-port primitive a bit**: `RAM32X1S` up
  to 32 words, `RAM64X1S` at 64, `RAM128X1S` at 128, `RAM256X1S` at 256 and banks of
  them deeper with a multiplexer (`sp_as_16x8` eight `RAM32X1S`, `sp_as_64x8` eight
  `RAM64X1S`, `sp_as_512x1` two `RAM256X1S`), read on no clock or on the clock
  (`sp_rf_64x8`, `sp_nc_64x8`, `sp_wf_32x8` to `sp_wf_256x2`): never the four-port
  `RAM32M` and `RAM64M` a memory of two addresses is. `bw_sp_wf_1024x32_b8`'s 128 and
  `bw_sp_wf_2048x18_b9`'s 144 `RAM256X1S` are four and eight banks of each lane's bits.
- **A single port of 512 words or fewer wider than a port is split over both ports of
  one block**: a `RAMB18E1` for 19 to 36 bits, port A the word's bits 0 to 17 (16 data,
  2 parity) and port B the rest, a `RAMB36E1` for 37 to 72 bits, 36 a side
  (`sp_rf_32x32`, `sp_rf_256x24` with 6 bits on B, `sp_rf_512x32`, `sp_rf_512x36`,
  `sp_wf_512x36` and `sp_nc_512x36` in their write modes on both ports, `sp_rf_512x64`,
  `sp_rf_512x72`, `bits_sp_rf_512x36`); written a byte at a time, two lanes a side of a
  `RAMB18E1` (`bw_sp_rf_32x32_b8`, `bw_sp_rf_512x32_b8`). Port A addresses the lower
  half of the block's words and B the upper: the top address bit 0 on A and 1 on B, the
  bits between it and the word's address ones. What one port holds is not split
  (`sp_rf_512x18`), nor 1,024 words (`sp_rf_1024x32`, a `RAMB36E1` on port A).
- **The address register in the block, for a read on no clock**, is not what the source
  says when the word read is written in the same cycle. `async_1024x8`'s write is on
  port A, `READ_FIRST`, and the read on port B at the address register's input; a read
  on no clock after the edge sees the word just written, and Vivado's block the word
  before. Measured: under a stimulus whose shift register repeats every 18 cycles, so
  that the read's address is the write's every cycle, Vivado's own netlist of it, on
  its own models under xsim, answers otherwise than its source from cycle 78 (memory
  and registers zeroed in both; the script is not kept, the answer is). Where the read
  register's input *is* the write's address Vivado makes one port of the two,
  `WRITE_FIRST`, which is exact: the same stimulus, 2,000 cycles the same.
- **Under a tight clock**, measured with Vivado's synthesis of 21 of the designs at
  periods from 5 to 1.4 ns (`Synth 8-5584`): one write and one read, the read not
  registered again, 32 kbit or less: up to 1,024 words a block at 4 ns and distributed
  RAM at 3.5 (`sdp_32x32`, `sdp_64x16`, `sdp_128x8`, `sdp_512x8` to `sdp_512x64`,
  `sdp_1024x18`, `sdp_1024x32`, `sp_wf_1024x18`, `sp_rf_1024x18`); 2,048 words at 3.2
  (`sdp_2048x9`); and a block at every period for a read registered again
  (`reg_1024x18`) or over 32 kbit (`sdp_1024x36`, `sdp_2048x18`, `sdp_4096x9`,
  `sdp_4096x16`, `sdp_8192x4`, `sdp_1024x72`).
- **The asymmetric template**, four reads at `{a, i}` into a register's quarters, is
  four reads on the clock to Vivado, a copy of the memory each: four `RAMB36E1` of
  4,096 by 9 for `asym_4096x8_r32`, four `RAMB18E1` for `asym_2048x9_r36`.

### What was built, and what was not

All behind `--memories vivado`, the default on the 7 series (`MapOptions::vivado_memories`);
`--memories legacy`, the ECP5, the Gowin and the iCE40 are what they were.

1. **The cascade** (`vivado_columns`, `Mapper::emit_bram`, `xray::bram`): a column of
   65,536 words is a pair of blocks a row, the lower first in the contents, the upper's
   cascade input on the lower's output net, `RAM_EXTENSION_<x>` on each used port, the
   address one bit wider; the site's `CASCADEIN<x>` and `CASCADEOUT<x>` are pins of the
   `RAMB36E1` bel, so the placer holds the pair as dedicated wiring
   (`place::build_clusters`, read off the graph) and the router takes the fixed pip, and
   `ramb36_features` writes `RAMB36.RAM_EXTENSION_<x>_LOWER`, refusing a cascaded port
   not one bit wide as Vivado's model does. An unconnected cascade input is left alone,
   and the timer follows no path through a cascade's pins. A cascade named by a design
   is built too (`bram/b36_cascade`, refused before).

   **But the cascade is slower than the stack it replaces, by Vivado's own timer**, and
   is built only where it is not:

   | design | Vivado's own (cascaded) | stacked, as before | cascaded here |
   |---|---|---|---|
   | sdp_65536x1, sdp_40000x1 | 3.265 | 2.724 | 3.355 |
   | sp_rf_65536x1 | 3.268 | 2.756 | 3.355 |
   | tdp_65536x1 | 3.428 | 2.979 | 3.535 |
   | sdp_131072x1 | 3.413 | 3.248 | 3.683 |
   | sdp_65536x4 | 3.686 | 3.548 | 3.901 |
   | bw_sdp_65536x16_b8 | 4.587 | 4.535 | 5.852 |
   | bw_sp_rf_65536x16_b8 | 4.522 | 4.511 | 5.245 |
   | reg_65536x1 | 1.726 | 2.747 | **1.907** |

   The stack's multiplexer costs less than the path from the lower block through the
   upper; Vivado's placement and routes of this flow's stacked netlists are faster than
   its own cascade too (2.53 ns against 3.27 for `sdp_65536x1`). Only a read whose
   register goes into the blocks gains, because a stack's multiplexer keeps the register
   out of them. **So a memory is cascaded only when it is one row of pairs and every
   read's register goes into the blocks** (`reg_65536x1`, `reg_init_65536x1`), and
   stacked otherwise, which builds exactly the bitstream it did before (checked on all
   twelve deep designs). Built for every deep design first, the cascade was Vivado's
   tile for tile on all of them; `sdp_131072x1`'s rows were then each enabled by the
   address bit on the pin, through its inverter for one, as Vivado's are, which went
   with the cascade it was for.
2. **Single-port distributed RAM** (`Mapper::single_port_lutram`,
   `emit_vivado_single_lutram`, `vivado_single_lutram_words`): a memory of one write and
   one read at the same address, bit for bit, is a `RAM32X1S`, `RAM64X1S`, `RAM128X1S` or
   `RAM256X1S` a bit, in banks of 256 deeper. The two larger ones are written for
   Vivado's placement too, by their site (`xray::vivado`: placed leaf by leaf, Vivado
   cannot expand them, 12-1409, on `sp_rf_256x24`'s build).
3. **A single port split over both ports** (`vivado_split_port`, `Mapper::split_ports`,
   `split_contents`): the one pair becomes two, port A's share at `{0, 1.., a}` and port
   B's at `{1, 1.., a}` of a `RAMB18E1` or `RAMB36E1` in true dual port, the contents
   laid out to match, the read joined after.
4. **A register loaded from reads on no clock** (`Mapper::clock_registered_reads`): a
   register with no enable or reset starting at zero, each bit of which some read on no
   clock is the only reader of, makes those reads reads on its clock, as `q <= mem[a]`
   reads the memory before the edge's writes too. The asymmetric template's four reads
   are then four copies of the memory, as Vivado's four blocks. The synthesis still
   writes the loop as four reads on no clock and a nest of the register's bits; the
   frontend is not touched.
5. **The clock** (`Mapper::lutram_by_clock`): one write and one read, not registered
   again, 32 kbit or less, up to 1,024 words, under a declared clock of 3.5 ns or less,
   is distributed RAM. Not at 2,048 words, where Vivado moves it at 3.2 ns: built so here
   `sdp_2048x9` reached 3.82 ns where its block reaches 3.29.
6. **Not built: the address register in the block.** For a read on no clock it is not
   exact (above), and this flow's distributed RAM is; for a read-only memory it is exact
   only when the word at the address register's initial value is the output register's
   initial value (`rom_16384x2`'s word 0 is 0), which is not built either.

### What it is now

Each design: Vivado's memory cells, this flow's before (`6783ba68`) and after, the
design's lookup tables (Vivado's, before, after), and the period, Vivado's own flow and
the medians of five seeds before and after, each at 0.85 of Vivado's own period as
`achieved.sh` asks (ns). **Measured**:

| design | Vivado | before | after | tables V / b / a | period V / b / a |
|---|---|---|---|---|---|
| reg_65536x1, reg_init_65536x1 | 2 36 cascaded, `DOB_REG` | 2 36 two deep | = | 4 / 5 / 1 | 1.73 / 2.75 / 1.91 |
| sdp_65536x1, sdp_40000x1 | 2 36 cascaded | 2 36 two deep | the same | 4 / 4 / 4 | 3.27 / 2.72 / 2.72 |
| sdp_65536x2, sdp_131072x1, init_, sp_rf_, sp_wf_, sp_nc_, tdp_65536x1 | cascaded | stacked | the same | | 3.41 / 3.25 / 3.25 (`sdp_131072x1`) |
| bw_sdp_65536x16_b8, bw_sp_rf_65536x16_b8 | 32 36 cascaded | 32 36 two deep | the same | 71, 38 / 23, 21 / 23, 21 | 4.59, 4.52 / 4.54, 4.51 / the same |
| rom_65536x1 | 60 tables | 2 36 two deep | the same | 61 / 2 / 2 | |
| sp_as_16x8 to sp_as_512x1 (eleven) | `RAM32X1S` to `RAM256X1S` a bit | `RAM32M`, `RAM64M`, `RAM*X1D` | = | `sp_as_64x8` 2 / 3 / 2 | `sp_as_64x8` 2.20 / 2.22 / **2.38**; `sp_as_256x2` 2.30 / - / 2.13; `sp_as_512x1` 2.27 / - / 2.15 |
| sp_rf_64x8, sp_nc_64x8, sp_rf_256x2 | 8, 8, 2 X1S | 3 `RAM64M`, 3, 4 `RAM128X1D` | = | 2 / 3 / 2 | `sp_rf_64x8` 1.59 / 1.51 / 1.55 |
| sp_wf_32x8 to sp_wf_256x2 (four) | X1S a bit | M and X1D | = | `sp_wf_32x8` 10 / 11 / 10 | `sp_wf_32x8` 1.61 / 1.98 / 1.89 |
| bw_sp_wf_1024x32_b8, bw_sp_wf_2048x18_b9 | 128, 144 `RAM256X1S` | 192 `RAM64M` each | = | 97, 80 / 244, 238 / 131, 81 | 3.13, 3.09 / 3.91, 4.18 / 3.75, 4.02 |
| sp_rf_32x32, sp_rf_512x32, sp_rf_512x36, sp_wf_512x36, sp_nc_512x36, sp_rf_256x24, bits_sp_rf_512x36 | 1 18, both ports | 1 36, port A | = | 7 / 7 / 7 (`sp_rf_32x32`) | see the clock, below |
| sp_rf_512x64, sp_rf_512x72 | 1 36, both ports | 2 36, port A | = | 13, 15 / 14, 16 / 14, 16 | |
| bw_sp_rf_32x32_b8, bw_sp_rf_512x32_b8 | 1 18, both ports | 1 36, port A | = | 17 / 19 / 19 | 2.25, 2.66 / 3.88, 3.99 / **4.06, 4.05** |
| sp_rf_512x18, sp_rf_1024x32 | port A | = | = | | |
| asym_4096x8_r32, asym_2048x9_r36 | 4 36, 4 18 | 768, 384 `RAM64M` | = | 7, 8 / 626, 366 / 7, 8 | 3.54, 3.59 / 6.21, 5.24 / 3.90, 4.01 |

And under the clock `achieved.sh` asks for, the memories item 5 moves to distributed
RAM, as Vivado's own flow does at those periods (Vivado, before, after, ns):

| design | Vivado | before | after |
|---|---|---|---|
| sdp_32x32 | 1.79 | 3.74 | 2.21 |
| sdp_512x8 | 2.27 | 3.24 | 2.70 |
| sdp_512x36 | 2.64 | 3.93 | 3.12 |
| sdp_1024x18 | 2.57 | 3.49 | 3.15 |
| sdp_1024x32 | 2.69 | 3.74 | 3.38 |
| sp_wf_1024x18 | 2.79 | 3.50 | 3.30 |
| sp_rf_32x32 | 1.83 | 3.68 | 2.14 |
| sp_rf_256x24 | 2.18 | 3.54 | 2.29 |
| sp_rf_512x36 | 2.94 | 4.19 | 3.23 |
| sp_rf_512x64 | 2.73 | 4.23 | 3.41 |
| sp_wf_512x36 | 2.70 | 4.19 | 3.25 |
| sdp_2048x9 (not moved) | 3.23 | 3.29 | 3.29 |

The split alone, measured with item 5 left out: `sp_rf_32x32` 3.68 to 3.74,
`sp_rf_256x24` 3.54 to 3.50, `sp_rf_512x36` 4.19 to 4.07, `sp_rf_512x64` 4.23 to 4.17,
`sp_wf_512x36` 4.19 to 4.05: the same within the seeds' spread, on half the block for
all but the 72-bit ones.

**Over the 30 designs timed, this flow's median is 1.113 times Vivado's own figure,
where it was 1.287.** Four are slower: `sp_as_64x8` by 7 % (eight `RAM64X1S` where three
`RAM64M` were, two tables against three, its seeds 2.34 to 2.46 against 2.14 to 2.43),
`bw_sp_rf_32x32_b8` by 4 %, `sp_rf_64x8` by 3 % and `bw_sp_rf_512x32_b8` by 2 %, the last
three inside the seeds' spread; they are kept for the area, Vivado's arrangement, and
measured here so. (`sp_as_256x2` and `sp_as_512x1` have no figure before: Vivado's
placement of this flow is not written for a `RAM128X1D`, which they were.)

**Checked** (`tests/fpga_xray_memarr_vivado.rs`, 203 designs): every memory but the 20
listed is Vivado's cells, 183 designs; **every block RAM tile of those is Vivado's,
site feature for site feature and contents bit for contents bit**, 277 tiles, the
cascades' `RAM_EXTENSION_*_LOWER`, `DOB_REG` on both blocks and the lower and upper
halves of `reg_init_65536x1`'s contents among them, and `bram/b36_cascade`'s named
pair; every set bit decodes, none unexplained, the arcs are the router's and the ties'
(a cascade's route is one fixed pip of no bit, `CASCADEOUTA -> CASCADEOUTA_1 ->
CASCADEINA`, read off its build's `--vivado-placement` routes); every mapping is
proved; and **all 203 routed netlists answer as Vivado's builds do**, 2,000 cycles under
xsim on Vivado's models, the two `asym_*` among them now that they are blocks. The 20
not Vivado's are each held to what they are: the deep memories stacked, the read-only
memories in blocks, `async_1024x8` in distributed RAM, `style_registers_128x8`.

**Nothing else changes.** Built by this tree's release binary and by `origin/master`'s (`5717255d`, and before
the rebase `6783ba68`'s),
every design of `memarr/`, `bram/`, `qor/` and `lutram/` with `--memories legacy`, those
of `bram/`, `qor/` and `lutram/` with the default, and the mapped netlists of all of them
on the iCE40 (`ice40-hx8k-ct256`), the ECP5 (`ecp5-25f-CABGA381`) and the Gowin
(`gw2a-18-pg256`) are the same files byte for byte: 1,327 builds, 306 designs with `--memories
legacy`, 103 with the default and 918 mapped netlists (**checked**; the script is not
kept, the answer is). So are the twelve deep memories of `memarr/` with the default,
stacked as before. No Trellis or Apicula database is here, so those
families' bitstreams were not built.

### The Husky's bitstreams

With this tree's release binary `examples/husky/build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and `build_plus.sh`
seed 4's `e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`, the files
they were. **Checked.**

### Tests

- `tests/fpga_xray_memarr_vivado.rs`, above; `what_is_not_vivados_says_so` holds the
  stacked memories to their two and four rows, the registered ones to a cascade with
  its output register, `reg_65536x1`'s routed netlist to the lower block's `CASCADEOUT*`
  on the upper's `CASCADEIN*`, the split ports and the asymmetric copies to their
  reports; `a_tight_clock_makes_a_small_memory_distributed_ram` builds seven of the
  designs at the periods Vivado was measured at, and holds each to a block or not.
- `tests/fpga_xray_bram_vivado.rs`: `b36_cascade` is built and Vivado's, tile, cells
  and simulation, and its routed netlist has the cascade's two nets.
- Unit tests: `vivado_arranges_a_memory_as_its_netlists_say` the cascade's columns,
  `vivado_splits_a_single_port_as_its_netlists_say` one case per split measured,
  `a_single_port_in_distributed_ram_is_a_bit_a_primitive` the primitives by depth,
  `a_cascade_sets_the_lower_blocks_extension` the feature and the refusals.

**Mutations**, each made by a script in a copy of the tree and run against the library's
tests, `tests/fpga_xray_memarr_vivado.rs` and `tests/fpga_xray_bram_vivado.rs` with
Vivado's simulator:

| mutation | what failed |
|---|---|
| the lower block's `RAM_EXTENSION_<x>_LOWER` not written | the feature's unit test; the tiles (`reg_65536x1`, `reg_init_65536x1`, `bram/b36_cascade`) |
| the upper block's cascade input left open | the simulation; the routed netlist's cascade nets |
| a cascade's halves' contents the other way round | the tiles (`reg_init_65536x1`); the simulation |
| a cascade's address bit 15 not given | **the simulation only**: the address pins are no tile feature, and each half alone reads the right word |
| never cascaded | the tiles; the exceptions' test |
| cascaded where the read is not registered again | the exceptions' test only: those designs are in its list, Vivado's cascade there being slower |
| no single-port distributed RAM | the cells |
| a split port's two sides reading one half | **the simulation only** |
| no split | the split's unit test; the cells; the tiles; the exceptions' test |
| reads on no clock left on no clock | the cells; the tiles; the exceptions' test |
| the clock's line at 4 ns | the clock's test only |

Three are caught by the simulation alone, which needs `RETICLE_VIVADO`: without Vivado
installed, a cascade's address bit, a cascade input left open where its routed netlist is
not read, and a split port's sides on one half pass every other test here.

### Not done

- **The address register taken into the block**: not exact for a read on no clock
  (above), and for a read-only memory only under a condition on the initial values;
  `async_1024x8` and `rom_16384x2` stay as they were.
- **A cascade where the read is not registered again**: measured slower, so stacked.
- **Read-only memories as lookup tables**, `rom_65536x1` now among them: not wanted, at
  the measured cost.
- **Choosing by the clock at 2,048 words**, measured slower; and between the periods
  measured (3.5 and 4 ns) the line is a guess. A clock the constraints do not declare on
  the port's net (one out of a PLL) decides nothing.
- **A split port wider than 72 bits**, or than eight lanes, and **single-port
  distributed RAM with two reads**: not measured, not built.
- **The asymmetric template in the synthesis**: the loop is still four reads on no
  clock and a nest of bits there; it is the mapper that makes them reads on the clock.
  A frontend that wrote `q[i*8 +: 8] <= mem[{a, i}]` as four clocked reads would make
  this unnecessary (`src/synth`, not touched here).
- ~~**`sdp_65536x4`'s build at the default flags**: `xray::lutswap` refuses it, on master
  too; not followed.~~ Done (2026-11-09): a table reading a flip-flop and its merged
  copy, and the pin swap offering both inputs; see the start of this section.
- None of it has been on a part.

## Routing against Vivado's routes

**Asked: on the same placement, which connections does Vivado route faster than
Reticle, how, and why does Reticle's router not?** "Filling slices" left the routes
as most of what stands between Reticle's implementation and Vivado's: Reticle's
placements routed by Vivado were 4 to 6 % faster than with Reticle's routes. Run on
2026-11-07 and 08 at `xc7a100tftg256-3` with Vivado 2026.1, on the timing corpus and
with `achieved.sh`, ten seeds where it says so, on the tree of 70f566ad; with
"Byte writes as Vivado arranges them" under it, seed 1 of seven of the designs
(those with memories, `sad`, `esad`, `crc32`) builds the same bitstreams. **Everything here is Vivado's timer
of Vivado's or Reticle's routes; nothing has been on a part.**

`tools/vivado-parity/routes.tcl` loads one build's placement and routes it twice:
with the lookup tables' pins free and Vivado's router (`vivado`), and with Reticle's
routes fixed (`reticle`); each writes every net's route, every sink's interconnect
delay, and each clock's hundred worst paths. `routes.py` sets the two side by side a
connection at a time: `summary` and `show` over the connections on Reticle's paths
within 0.3 ns of each clock's latest, `pins` which input of a table each tool ends
them on, and `connections` the lines `fill.py distance` reads. Since the placement
does not depend on anything changed here, Vivado's routes of the tree before are
Vivado's routes of every build after.

### Where the time went

Seed 1, the sixteen designs that time a path between slices, before this section
(70f566ad): over the 448 connections on Reticle's latest paths, Reticle's routes are
**539 ps at the mean and Vivado's of the same connections 477**. Of the difference,
324 connections end on the same input of their table in both and cost Reticle 22 ps
more each; the other **124 end on another input than Vivado's and cost 164 ps more
each** (`routes.py summary`). The inputs (`routes.py pins`), connections and mean ps:

| input | Reticle | Vivado |
|---|---|---|
| 1 | 53 (834) | 42 (659) |
| 2 | 48 (806) | 28 (682) |
| 3 | 23 (520) | 31 (539) |
| 4 | 47 (658) | 22 (642) |
| 5 | 36 (696) | 70 (623) |
| 6 | 19 (511) | 33 (423) |

`A1` and `A2` cost about 390 ps past their `IMUX` where `A6` costs none, and
Reticle put 101 of the critical connections on them where Vivado put 70. On
`crc32`, `c9__lut194/I1` took `B2` over `NN6BEG SR1BEG SE2BEG SL1BEG SS2BEG SR1BEG_S
IMUX`, 1,224 ps, where Vivado went `EL1BEG BYP_ALT BYP_BOUNCE IMUX` to `B5`, 481; on
`widemux`, the select's fanout of 257 reached `c136__lut352` over ten wires in 1,724
ps where Vivado's took `NN6BEG NW2BEG IMUX` to `C6`, 719.

**Why, read from the router** (a trace of every pass of those signals, not
committed). Three things, each measured:

1. **The connections that end on the latest path were routed as slack ones.** At
   `xray::CRITICALITY_EXPONENT`'s 4, a connection a tenth short of its domain's
   latest path weighs delay at 0.66 and one a third short at 0.20; `crc32`'s
   `lut194` input was routed at 0.12 to 0.21 in every pass, and `crc[7]`'s branch to
   `lut214` at 0.16 in the first pass and, at 0.50 to 0.70 in every pass after, never
   again: a signal is rerouted when it is congested or when one of its connections
   is past `reroute_above` (0.9 after the exponent, 0.974 before it), and an
   incremental reroute keeps every branch that is clear. The latest path is a set of
   such connections.
2. **A connection off the latest path for one pass is rerouted as slack.** The
   latest path moves as connections are rerouted. `crc32`'s branch from `$lut23` to
   `$lut246`, at 0.50 in pass 2, was 0.22 in pass 4, was rerouted for congestion then
   from 0.41 ns to 1.04, was 0.70 at the next pass, and was on the latest path when
   the route ended.
3. **What a picosecond costs against a wire depended on the design's ties, sixty
   times over.** `route::delay_scale` turns a delay into a cost: the mean base cost
   of the timed nodes over their mean delay. It counted the ground fans, which the
   flow closes with a base cost of a million, and so one unit of cost was, in
   picoseconds:

   | design | one unit |
   |---|---|
   | `adders`, `barrel`, `counter64`, `crc32`, `fsm`, `lutram32`, `widemux` | 170 |
   | `carry_counter` | 172 |
   | `bram_fifo` | 41 |
   | `regfile_alu`, `srl_delay` | 28 |
   | `sad` | 21 |
   | `esad` | 17 |
   | `mul18` | 10 |
   | `gray_cdc` | 7.9 |
   | `analyser_fifo` | 4.1 |
   | `fir8` | 2.7 |

   On `fir8` a critical connection weighed delay sixty times over congestion against
   `crc32`, and its searches' estimate (`ROUTE_TILE_DELAY_PS` scaled the same way)
   was as far off. Left out, the timed wires of the part make it 170 ps on every
   design (169.7 to 172.3). The open item "ground-fan tie wires still weigh 1e6 in
   `route::delay_scale`" of earlier sections is this; under the legacy model, which
   the Husky builds with, it still is.

### What was changed

All under the fitted delay model, the default; `--delay-model legacy` routes as
before (below).

1. **The criticality exponent is 2** (`xray::FITTED_CRITICALITY_EXPONENT`), and the
   thresholds measured at 4 move with it: `reroute_above` 0.949 and the claim of a
   held input 0.548 (`FITTED_REROUTE_ABOVE`, `FITTED_CLAIM_INPUTS`), 0.9 and 0.3 to
   the power a half, so that the same connections are ripped up whole and may take a
   held input as before.
2. **A delay is weighed against the fabric's wires, twice** (`route::Timing::delay_weight`,
   `xray::FITTED_DELAY_WEIGHT`): `delay_scale` leaves out a node of base cost
   `route::TIE_BASE` (1,000) or more and multiplies what is left by the weight, so a
   node of base one weighs as 85 ps on every design.
3. **A connection's criticality is never lowered** (`route::Timing::keep_criticality`):
   each pass's analysis is the larger of what it finds and what the passes before
   found.

Each over ten seeds, geometric mean against Vivado's own flow (1.231 before):

| exponent | weight | kept | mean |
|---|---|---|---|
| 4 | none | no | 1.231 |
| 4 | none | yes | 1.214 |
| 1, thresholds not moved | none | no | 1.217 |
| 1 | none | no | 1.205 |
| 1 | none | yes | 1.200 |
| 4 | 1 | no | 1.259 |
| 4 | 2 | no | 1.240 |
| 4 | 4 | no | 1.221 |
| 1 | 1 | no | 1.206 |
| 1 | 1 | yes | 1.201 |
| 1 | 2 | yes | 1.194 |
| 2 | 1 | yes | 1.202 |
| 2 | 1.5 | yes | 1.200 |
| **2** | **2** | **yes** | **1.195** |
| 2 | 2 | no | 1.207 |
| 2 | 3 | yes | 1.198 |
| 3 | 1 | yes | 1.216 |

At exponent 1 with the thresholds moved, `sad`'s route visited 46 to 60 million
nodes against 5.3 before while the scale still counted the ties, and 7.4 million
with them left out; the weight alone, at exponent 4, is worse than nothing until it
is 4. Exponent 1 with a weight of 2 was as good as 2 with 2 and visited up to four
times the nodes (`crc32` 6.5 million against 1.6).

### Before and after

Medians (ns); `before` and `after` over ten seeds, and Vivado's routes of the same
placements (`achieved.tcl`'s `placement_free`) and `after` over seeds 1 to 5, from
figures rounded to 10 ps:

| design | clock | Vivado | Vivado's routes, seeds 1-5 | before | after | after, seeds 1-5 |
|---|---|---|---|---|---|---|
| adders | clk | 2.042 | 2.210 | 2.390 | 2.228 | 2.230 |
| analyser_fifo | rd_clk | 3.887 | 5.210 | 5.375 | 5.144 | 5.210 |
| analyser_fifo | wr_clk | 2.417 | 3.030 | 2.984 | 3.061 | 3.060 |
| barrel | clk | 2.376 | 2.510 | 2.942 | 2.677 | 2.690 |
| bram_fifo | clk | 3.388 | 3.870 | 4.046 | 3.996 | 3.980 |
| carry_counter | clk | 1.855 | 2.270 | 2.404 | 2.373 | 2.380 |
| counter64 | clk | 3.220 | 3.730 | 3.869 | 3.821 | 3.810 |
| crc32 | clk | 2.234 | 2.290 | 2.556 | 2.345 | 2.320 |
| esad | adc_sampleclk | 2.131 | 3.270 | 3.009 | 3.013 | 3.040 |
| esad | clk_usb | 1.727 | 2.590 | 2.523 | 2.471 | 2.470 |
| fir8 | clk | 9.142 | 9.160 | 9.161 | 9.161 | 9.160 |
| fsm | clk | 1.903 | 2.210 | 2.391 | 2.266 | 2.310 |
| gray_cdc | clk | 1.466 | 1.410 | 1.413 | 1.413 | 1.410 |
| gray_cdc | clk2 | 1.267 | 1.610 | 1.554 | 1.582 | 1.570 |
| lutram32 | clk | 1.894 | 1.970 | 2.186 | 2.122 | 2.110 |
| mul18 | clk | 1.935 | 2.110 | 2.220 | 2.200 | 2.300 |
| regfile_alu | clk | 5.511 | 5.850 | 6.223 | 6.050 | 6.060 |
| sad | adc_sampleclk | 2.207 | 4.050 | 3.021 | 2.989 | 2.990 |
| sad | clk_usb | 1.726 | 2.590 | 2.488 | 2.510 | 2.500 |
| srl_delay | clk | 1.645 | 1.760 | 1.898 | 1.906 | 1.910 |
| widemux | clk | 2.438 | 2.710 | 3.503 | 2.878 | 2.850 |
| geometric mean | | 1 | 1.194 | 1.231 | 1.195 | 1.198 |

**Over the twenty-one clocks Reticle's median is 1.195 times Vivado's own figure,
against 1.231**; fourteen clocks faster, five slower, the worst `analyser_fifo`'s
write clock by 2.6 %. **The geometric means of Reticle's routes and Vivado's on the
same placements are now within 0.4 %, but that is two clocks' doing**: on `sad`'s
and `esad`'s ADC clocks Vivado's routes are 1.06 and 0.23 ns slower than Reticle's,
as they were in "Filling slices". Over the other nineteen, Vivado's routes are 1.152
of Vivado's own flow, Reticle's 1.203 before and 1.180 after: **this closes 45 % of
the distance between the two routers**, and Vivado's are still faster on most clocks
(`barrel`, `bram_fifo`, `lutram32`, `srl_delay`, `widemux` by 0.11 to 0.18 ns).

**Per connection, against Vivado's routes**, seed 1 as above: over the 367
connections on Reticle's latest paths, Reticle's 454 ps and Vivado's 442 (539 and
477 before); 43 end on another input than Vivado's (124 of 448), and those cost 55 ps
more each (164). Inputs 1 and 2 take 35 of the 367 in both. Over every
connection between slices of fanout four or less (`fill.py distance`, the same
sixteen designs), a column of interconnect costs 26 ps on Vivado's routes, 47 on
Reticle's before and 44 after, a row 37, 43 and 42: what is left between the two is
mostly in connections with slack, which Vivado routes longer (404 ps where the
distance is nothing, Reticle 375 and 369) and Reticle shorter. *The reading of
"Filling slices"*, 33 against 56 ps a column, was of eight designs over every seed;
this is of sixteen at seed 1.

### Runtime

Two runs each of seed 1, side by side on the same machine, the route stage (which
includes the costs the flow gives the router) and the nodes its searches visited:

| design | before | after | visits before | after |
|---|---|---|---|---|
| `counter64` | 1.94, 1.95 s | 2.13, 1.94 s | 0.75 million | 5.16 |
| `sad` | 3.45, 3.40 s | 2.43, 2.30 s | 5.36 | 5.60 |
| `widemux` | 2.99, 3.42 s | 2.73, 3.30 s | 6.57 | 10.68 |

The searches do more (`widemux` 63 % more visits, `counter64` seven times and 31
passes against 13), and the stage is no slower because the commit before this
section's asks each wire name of the clock patterns once (`XrayFabric::node_costs`,
0.89 s of `counter64`'s 2.7 s before the router was called, 0.21 s after), which is
byte for byte the same build.

### Tried and not kept

Each over five or ten seeds against the build it was tried on, by the geometric
mean:

- **Rerouting more signals whole**: `reroute_above` 0.3 and 0.1 at exponent 4, 1.228
  and 1.249 against 1.221 (five seeds); four clean-ups rather than two, 1.221. On the
  build kept, ten seeds: `reroute_above` 0.8, 1.196 against 1.195; four clean-ups,
  1.192, seven clocks faster and five slower, 3 % more visits. That is inside a
  seed's spread and was left out: one more setting for a third of a percent.
- **A connection forced to reroute** when it is critical (from 0.1, 0.2 or 0.4) and
  slower by a tenth and 30 ps than a search for delay alone finds: 1.234, 1.238 and
  1.221 against 1.221 at five seeds (from 0.4, 1.228 against 1.231 at ten), and from
  0.4 with the criticality kept, 1.210 against 1.214. It does what it says (`crc32`'s
  `lut214` input from 872 ps to 470) and something else is then the latest path. On
  the build kept, from 0.3: 1.191 against 1.195, for 2.4 times the visits on `sad`
  and 2.2 on `esad`; not worth them.
- **A cheaper estimate of a critical search's congestion**, the base cost charged a
  tile at a half and a quarter: 1.221 and 1.226 against 1.221.
- **A lookahead table** of the least delay and base cost to a sink, by wire class
  and offset within eight tiles, from the searches back from eight of the design's
  sinks: at exponent 1, the critical searches visited two to six times the nodes
  (`sad` 276 million against 54) with the estimate weighed at 1 to 2, and Reticle's
  own estimate of the routes was no better on the three designs tried. The per-tile
  estimate overstates the way near the sink and the table did not, and the search
  spent the difference.
- **A critical search's estimate weighed at 1.5** rather than 1 at exponent 1: 1.206
  against 1.201, fewer visits.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--delay-model legacy`: the
exponent, the thresholds, the scale and the criticality are as before, and with this
tree's release binary `build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and `build_plus.sh`
seed 4's `e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`.
**Checked.**

### Tests

`a_weighed_delay_leaves_the_ties_out_and_takes_the_weight` (`route`) holds the
scale with a tie and without, and a priced node under `TIE_BASE` counted;
`a_kept_criticality_never_falls` the rule; `tests/fpga_xray_achieved.rs` this
section's runs (`routed`, ten seeds, and `vroutes`, Vivado's routes of seeds 1 to 5):
no slower than `fillall` by at least 2 % over the corpus, no clock more than 4 %
over its own, and seeds 1 to 5 within 1 % of Vivado's routes.

**Mutations**, made by a script and put back: the ties counted when weighed, the
weight left out, the threshold moved to 100, and the kept criticality taking the
old figure over a higher new one or ignoring it, each failing its test. **One passed:
the route not calling the rule**, which no unit test here holds; the corpus under
Vivado's timer measured it (1.207 without, 1.195 with). In the fixture, `routed`
swapped with `fillall` and a seed of `routed` lost each fail the corpus's test;
`vroutes` given `routed`'s own figures passes it, since it only bounds `routed` from
above.

### Not done

- **The rest of the routers' difference**, 2.3 points of the 5.1 left out of the two
  ADC clocks. What is left per connection is small on the latest paths (12 ps); a
  path's other connections, and which paths end up latest, are where to look.
  *Read again in [A table's inputs as Vivado gives them](#a-tables-inputs-as-vivado-gives-them)*:
  on the five designs furthest behind, at seeds 1 to 3, it was 116 ps a connection,
  most of it which signal a table's pins were given, and that is closed there.
- **Why Vivado's routes lose a nanosecond on `sad`'s and `esad`'s ADC clocks** on
  Reticle's placements: not looked at.
- **The legacy model's scale**, which still counts the ties: changing it changes the
  Husky's bitstreams, and is the user's to decide once the board is free.
- **A lookahead that pays for itself**: one that gives the search a tighter estimate
  than the per-tile figure where it underestimates, without being looser where it
  overestimates.
- None of it has been on a part.

## A table's inputs as Vivado gives them

**Asked: on the designs still furthest behind — `barrel`, `bram_fifo`, `lutram32`,
`srl_delay` and `widemux` — what does Vivado's router write, in full, for the nets on
their latest paths, and why does Reticle's not?** "Routing against Vivado's routes"
left Vivado's routes of Reticle's placements 0.11 to 0.18 ns faster than Reticle's on
these five. Run on 2026-11-09 at `xc7a100tftg256-3` with Vivado 2026.1, on the timing
corpus with `achieved.sh`, ten seeds, on aa1420e5, whose own build reproduced
`routed` of `achieved.txt` seed for seed (all 210 figures); rebased onto a7220e86, the
change built the same bitstreams at seeds 1 and 5 of all seventeen designs. The placements do not
depend on anything changed here, so Vivado's routes of the tree before are Vivado's
routes of the build after. **Everything here is Vivado's timer of Vivado's or
Reticle's routes; nothing has been on a part.**

### What Vivado writes

`routes.tcl` and `routes.py` (`summary`, `show`, `pins`) over seeds 1 to 3 of the five
designs, the connections on Reticle's paths within 0.3 ns of each clock's latest:
**211 connections, Reticle's routes 710 ps at the mean and Vivado's of the same
connections 594.** 103 end on the same input of their table in both and cost Reticle
60 ps more each; **108 end on another input and cost 169 ps more each**. The inputs,
connections and mean ps:

| input | Reticle | Vivado |
|---|---|---|
| 1 | 43 (832) | 21 (747) |
| 2 | 46 (878) | 19 (790) |
| 3 | 36 (667) | 42 (648) |
| 4 | 44 (603) | 48 (580) |
| 5 | 26 (546) | 53 (506) |
| 6 | 15 (527) | 27 (448) |

Read a table at a time, every input it reads in both routes (a script, not
committed), the difference is not in the wires to the table but in **which signal is
given which pin.** `lutram32`'s latest table at seed 1, `c14__lut8`, `SLICE_X89Y97`'s
`B` table, each input with the pin and the interconnect delay to it:

| input | Reticle | Vivado |
|---|---|---|
| `I0` (`q[7]`) | `B1`, 846 ps, `EE4BEG WL1BEG IMUX` | `B2`, 586, `SR1BEG IMUX` |
| `I1` (`q[6]`) | `B2`, 838, `SE2BEG WL1BEG IMUX` | `B6`, 192, `SL1BEG IMUX` |
| `I2` (`q[4]`) | `B3`, 351, `IMUX` | `B1`, 587, `NL1BEG_N IMUX` |
| `I3` (`q[3]`) | `B4`, 300 | `B4`, 300 |
| `I4` (`q[2]`) | `B5`, 277 | `B5`, 277 |
| `I5` (`q[5]`) | `B6`, 95, `IMUX` | `B3`, 453, `SR1BEG IMUX` |

**Vivado gives a table's inputs as a matching**: `q[6]`, on the latest path, gets the
fast `B6`, and `q[5]` and `q[4]`, which have time to spare, are moved to slower pins
and longer ways — `q[5]` from 95 ps to 453. The table's latest input is 587 ps in
Vivado's routes and 846 in Reticle's. `srl_delay` at seed 2 (`c11__lut12`: the latest
input on `B1` over `NE2BEG SL1BEG WL1BEG_N IMUX`, 779 ps, where Vivado put it on
`B4`, 401, and moved the one there to `B1`, 692) and `bram_fifo` at seed 1
(`c23__lut15`: a block RAM's `DOBDO23` on `D2`, 1,032 ps, Vivado's on `D4`, 745) are
the same.

**Why Reticle's router does not**, read from it: PathFinder routes a table's inputs one
signal at a time, and the first signal to reach a table takes whichever free pin is
cheapest for it, whatever its slack. A critical search may end on an input another
signal holds (`Timing::claim_inputs`), but the holder is as near critical — the six
inputs of a table on a latest path come from registers at much the same distance — and
takes it back; nothing in the negotiation asks which of them should have it.

### What was changed

**A legal route ends with a refinement** (`route::refine`, `Timing::refine_rounds` and
`refine_above`, under the fitted delay model only; `--delay-model legacy` routes as
before). Each round:

1. Analyses the routing as each pass does, and takes every lookup table one of whose
   inputs is at least `xray::FITTED_REFINE_ABOVE` (0.7) critical before the exponent,
   the latest first, and every other sink that critical on its own.
2. For a table: lifts off the routing, for every input it reads, the branch of that
   signal's tree that serves the input alone, and searches from what is left of each
   tree, for delay alone and over wires no signal holds, to every pin of the table that
   is free (one search a signal, all six pins, the least slack first). Each input's new
   slack is its old one less what its new way costs over its old one, since the table's
   required time is one for all its inputs.
3. Gives the pins as the assignment that leaves the table's least slack largest
   (`best_assignment`, at most 720 of them), and routes them, the tightest first, on the
   ways found while those are clear. A table whose least slack is not at least 1 ps
   larger keeps its routes; a sink of one pin is the same with one pin.
4. Analyses the whole design again, and puts the round back and stops if any domain's
   latest path is later for it.

Nothing is taken from another signal, so the routing stays legal and needs no
negotiation after. `xray::FITTED_REFINE_ROUNDS` is 3. Each round still finds tables to
improve: what one round moves changes which paths are latest for the next.

Each over ten seeds, geometric mean against Vivado's own flow (1.195 before), and the
route stage of the seventeen designs (the median over the seeds, summed; 39 s before):

| rounds | from | mean | route stage |
|---|---|---|---|
| 1 | 0.7 | 1.173 | 57 s |
| 2 | 0.6 | 1.164 | 79 s |
| **3** | **0.7** | **1.165** | **60 s** |
| 4 | 0.8 | 1.167 | 50 s |
| 4 | 0.85 | 1.170 | 46 s |
| 6 | 0.8 | 1.167 | 74 s |

The rows' route stages were not run side by side and the machine was shared, so they
are a guide: those at 57, 79 and 74 s were the first version of the searches, before
the first two changes under "Runtime" below, which kept every route as it was. The searches' estimate is the
distance to the table at `xray::ROUTE_TILE_DELAY_PS`'s 25 ps a tile and the least of
the pins not yet reached's own delay, which any way to one of them pays: without that
term a search for `A1` (394 ps past its `IMUX`) settled every node of the neighbourhood
nearer than that first. At 10 ps a tile, which is a bound on every way, three times as
many searches ran out of their budget and none found better routes.

### Before and after

Medians (ns) over ten seeds, the lowest and highest seed in brackets, Vivado's timer;
`before` is `routed` of `achieved.txt`, rebuilt the same:

| design | clock | Vivado | before | after |
|---|---|---|---|---|
| adders | clk | 2.042 | 2.228 [2.19-2.37] | 2.212 [2.13-2.37] |
| analyser_fifo | rd_clk | 3.887 | 5.144 [4.97-5.48] | 5.152 [4.90-5.40] |
| analyser_fifo | wr_clk | 2.417 | 3.061 [2.90-3.39] | 2.936 [2.73-3.16] |
| **barrel** | clk | 2.376 | 2.677 [2.55-2.81] | **2.479** [2.40-2.62] |
| **bram_fifo** | clk | 3.388 | 3.996 [3.86-4.15] | **3.878** [3.82-4.00] |
| carry_counter | clk | 1.855 | 2.373 [2.32-2.64] | 2.367 [2.22-2.49] |
| counter64 | clk | 3.220 | 3.821 [3.71-3.97] | 3.768 [3.64-3.88] |
| crc32 | clk | 2.234 | 2.345 [2.29-2.59] | 2.188 [2.11-2.25] |
| esad | adc_sampleclk | 2.131 | 3.013 [2.79-3.82] | 3.013 [2.78-3.82] |
| esad | clk_usb | 1.727 | 2.471 [2.30-2.69] | 2.486 [2.30-2.69] |
| fir8 | clk | 9.142 | 9.161 [9.16-9.16] | 9.161 [9.16-9.16] |
| fsm | clk | 1.903 | 2.266 [2.14-2.50] | 2.179 [2.09-2.24] |
| gray_cdc | clk | 1.466 | 1.413 [1.41-1.51] | 1.413 [1.41-1.51] |
| gray_cdc | clk2 | 1.267 | 1.582 [1.51-1.72] | 1.514 [1.47-1.57] |
| **lutram32** | clk | 1.894 | 2.122 [2.00-2.48] | **2.064** [1.93-2.11] |
| mul18 | clk | 1.935 | 2.200 [2.01-2.48] | 2.049 [2.01-2.10] |
| regfile_alu | clk | 5.511 | 6.050 [5.91-6.47] | 5.860 [5.73-6.55] |
| sad | adc_sampleclk | 2.207 | 2.989 [2.93-3.17] | 2.987 [2.90-3.17] |
| sad | clk_usb | 1.726 | 2.510 [2.40-2.67] | 2.491 [2.40-2.67] |
| **srl_delay** | clk | 1.645 | 1.906 [1.69-2.08] | **1.808** [1.69-1.96] |
| **widemux** | clk | 2.438 | 2.878 [2.80-3.05] | **2.769** [2.69-2.88] |
| geometric mean | | 1 | 1.195 | **1.165** |

**Over the twenty-one clocks Reticle's median is 1.165 times Vivado's own figure,
against 1.195**: sixteen clocks faster, two slower (`esad`'s `clk_usb` by 0.6 % and
`analyser_fifo`'s read clock by 0.2 %, each inside its seeds' spread), three the same.
The five designs furthest behind are 0.06 to 0.20 ns faster. **Against Vivado's routes
of the same placements**, seeds 1 to 5 (`vroutes`): 1.163 against Vivado's 1.194, and
over the nineteen clocks without the two ADC clocks, where Vivado's routes lose a
nanosecond, 1.141 against 1.152. Reticle's routes are now faster than Vivado's on these
placements by the geometric mean; Vivado's are still faster on `lutram32` (1.974 ns
against 2.056 at seeds 1 to 5), `widemux` (2.710 against 2.836) and, within 0.04 ns,
on `counter64` and `fsm`.

**Per connection, against Vivado's routes**, the same five designs at seeds 1 to 3: over
the 121 connections on Reticle's latest paths, Reticle's 574 ps and Vivado's 521 (710
and 594 before); 37 end on another input than Vivado's (108 of 211), costing 134 ps
more each, and inputs 1 and 2 take 27 of the 121 (89 of 211 before) where Vivado's
take 19. What is left is ways Vivado reaches over wires another of Reticle's signals
holds: `barrel`'s `c74__lut139/I2` at seed 2 is `NW6BEG EL1BEG ER1BEG IMUX` to `A6`, 613
ps, where Vivado's is `NN2BEG NN2BEG BYP_ALT BYP_BOUNCE IMUX` to the same pin, 439, a
bounce through the bypass wires; the `NN2BEG`s are another signal's in Reticle's
routes, and the refinement takes nothing that is held.

*The reading of "Routing against Vivado's routes"*, that what was left per connection
on the latest paths was small (12 ps), was of the sixteen designs at seed 1. On these
five at seeds 1 to 3 it was 116 ps, most of it the inputs a table's signals were given.

### Runtime

Two runs each of seed 1, side by side on the same machine, the place and route stages
(s), and the bitstream the same in both runs of each:

| design | place before | after | route before | after |
|---|---|---|---|---|
| `sad` | 43.2, 50.6 | 44.6, 50.3 | 2.44, 2.51 | 5.36, 5.68 |
| `widemux` | 24.0, 23.3 | 24.7, 22.2 | 2.70, 2.94 | 10.0, 10.5 |
| `regfile_alu` | 9.36, 9.32 | 9.39, 9.37 | 9.68, 10.06 | 13.65, 13.52 |
| `barrel` | 5.17, 5.22 | 5.24, 5.19 | 1.41, 1.46 | 2.31, 2.24 |
| `crc32` | 5.64, 5.72 | 5.66, 5.76 | 1.08, 1.10 | 1.61, 1.68 |
| `counter64` | 15.6, 15.6 | 15.8, 15.6 | 1.81, 1.82 | 2.00, 2.06 |

**The refinement costs time**: the route stage of `widemux` is 3.6 times what it was and
its build 25 % longer, `sad`'s 7 %, `regfile_alu`'s 17 %. `widemux`'s select reaches 257
tables, most of them near its latest paths, and each is searched for again three
times. Placement is unchanged. Two changes to the searches kept every route the same
and made them cheaper: the table's members are searched for the least slack first and
the group given up as soon as one has no way that would leave it more slack than the
group had (no assignment could then be better), and the ways found are kept for the
routing that follows rather than searched for again while they are clear (the nodes
visited fell by 7 to 47 % on the seven designs tried at seed 1); and the searches read the router's own
table of each node's pips and the nodes they reach rather than the graph's pips
(`widemux`'s route 16.7 and 18.2 s to 9.9 and 10.7, the same bitstream).

**The placement of `sad`, which "Filling slices" made 50 % slower, was looked at and
not made faster.** A profile has no one place to take it from: the legality checks of
the anneal's moves (`SiteRules::judge`, 20 to 25 % of the build) are most of it, and
the anneal makes 12.6 million moves (11.6 before that section, 14.9 just after it). Three
changes that kept every placement the same were tried and are not kept, since two
builds side by side were no faster by more than the machine's noise (`sad`'s place stage
42.8 to 47.4 s against 42.7 to 46.7): the legality checks' sites of a kind no instance
has left out of what is asked (the latches of a flip-flop's bels; 1.48 billion steps to
1.20), a macro's member's site looked up by tile rather than by a scan of every site of
its kind, and each instance's signal on each shared pin in one table rather than a
search of its pins (1 % fewer instructions).

### Tried and not kept

- **Routing again whole each late signal of at most eight sinks**, its trunk with it,
  for delay alone and over free wires, kept when its sinks' least slack was larger:
  `barrel`'s case above wanted a new trunk. It changed the routed estimate of none of
  seven designs at seed 1 by more than 0.01 ns: the wires a faster trunk wants are held.
- **A cap on how much slower a slack input's new way may be** (300 ps), to search less:
  1.172 against 1.165 over ten seeds, `widemux` 2.857 ns against 2.769.
- **A smaller search budget** (8,000 nodes against 30,000): `widemux`'s and
  `bram_fifo`'s routed estimates 0.11 and 0.13 ns later at seed 1.

### The Husky's bitstreams

`examples/husky/build.sh` and `build_plus.sh` pass `--delay-model legacy`, under which
`Timing::refine_rounds` is zero, and with this tree's release binary `build.sh` writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and `build_plus.sh`
seed 4's `e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`.
**Checked.**

### Tests

`the_refinement_gives_a_tables_fast_input_to_its_latest_input` (`route`) routes two
registers into one table whose pins delay nothing and 300 ps, the second register over
a wire of 100: in order the first takes the fast pin and the second ends at 400 ps; the
refinement swaps them, the latest input then 300, every node one signal's and every
moved sink recorded where its route ends; with the wire free, swapping gains nothing
and nothing is refined. `the_latest_input_takes_the_fastest_pin` and
`a_pin_no_way_reaches_is_never_given` (`route::refine`) hold the assignment.
`tests/fpga_xray_achieved.rs` reads this section's run (`refined`, ten seeds): at
least 2 % under `routed` over the corpus, no clock more than 2 % over its own, and seeds
1 to 5 no slower than Vivado's routes of the same placements.

**Mutations**, each made by a script and put back, each failing a test: the assignment
blind to slack (the least total delay), a moved sink not recorded, a group kept that is
no better, and the refinement never run. **Two passed**: the group kept that is no better
by either one of the two checks, before its routes are made and after, since the other
then refuses it; with both gone it fails. In the fixture, `refined` swapped with
`routed` and a seed of `refined` lost each fail the corpus's test.

### Not done

- **Taking a held wire from a slack signal.** What is left between the two routers on
  the latest paths is ways over wires another of Reticle's signals holds; the
  refinement takes only free ones. Ripping up the holder and routing it again is the
  next step, and the route stage is already the cost above.
- **The route stage's cost** on designs whose late tables are many, `widemux`'s above.
- **Why Vivado's routes lose a nanosecond on `sad`'s and `esad`'s ADC clocks**: not
  looked at.
- **`sad`'s placement time**, above.
- None of it has been on a part.

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
| 28 designs inferring one | 21 | 26 (28 since the simple dual port, below) |

The seven slices that differ are Vivado packing more than one cell into a slice: two
read ports' `RAM32X1D` into one slice's four tables, two `RAM64X1D` likewise, and a
`RAM64M` whose unread `C` port shares `D`'s data input. Reticle places one cell a
slice, but for the two halves of a pair of `RAM32X1D`. The two whose cells differ are
`mem32x32` and `mem32x32r2`, which Vivado puts in one `RAMB18E1` each port where
Reticle used two, its 36-bit mode, which the next section is for. Reticle uses one
`RAMB36E1` now, the same tile, in true dual port: inferring the simple dual port is
not done. *Since done ("Memories as Vivado arranges them"): both are a `RAMB18E1` read
36 wide each port, as Vivado's, and the cells of all 28 are Vivado's.*

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

**And not the whole rule:** the sweep asked at a relaxed clock. At a tight one Vivado's
synthesis moves memories it would put in block RAM to distributed RAM, saying the
timing constraints suggest it (`Synth 8-5584`): `sdp_512x36`, 18 kbit, among them
("Memories as Vivado arranges them"). This flow's line does not move with the clock.

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
stopped on. *Four of the six are Vivado's since "Memories as Vivado arranges them":
the ports that read and write are paired, the second register is the block's
`DOB_REG`, and the 512 words are simple dual port; their tiles, pins and simulation are
held with the rest. The cascade and the byte writes are not.*

**Correction, 2026-11-06: `ib_1024x32_bytes` is refused now, and what was built of
it before was wrong.** Its byte writes, `mem[a][7:0] <= d`, assign a select of an
array element, and the frontend's name resolution saw through `mem[a]` to `mem`:
it found no net to assign and dropped all four writes without a word, so the block
it built held a memory nothing wrote. Every check below passed on it, because none
compares what that design computes with an answer Vivado built (Vivado built none)
and its mapping was proved against a source already without its writes. The
frontend refuses the assignment now (`tests/cli.rs` holds both shapes), so the
counts of 38 below, measured while it was "built", are one too many. A target for
part of an element is what it would take to build it. *Since built, with that
target ("Byte writes as Vivado arranges them"): one `RAMB36E1` and its byte write
enables, its routed netlist held to what its own source does under xsim, since
Vivado still builds nothing of it. "Vivado's synthesis stopped on it" was a crash:
`synth_design` aborts with an assertion in its block RAM generator
(`HARTGRamGen::genDPBRamBWWE`), measured again on 2026-11-07, where the same memory
written through wires for its address and data (`memarr/bw_sdp_1024x32_b8_const`)
builds.*

Ten of the named designs read an `x` from the first cycle under `xsim`,
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
  *Corrected below: no wire of it mattered. What cost the route was how the wires
  were closed, a base cost of a million each, which also set what a delay costs.*
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

~~**Not fixed.** A remedy has to keep the ties and give back the wires: closing a tie's
wires only once the router has been shown to want them, or choosing each tie's wires
after routing among those still free, are the two shapes it could take, and neither
was built here. Until one is, the default mapping is not one for this design, and the
scripts' `--memories legacy` stays for that reason as much as for the board's.~~
*Neither was needed: the wires were never what cost the route (below).* With
`--memories legacy` the Plus is the bitstream before byte for byte, checked again on
this commit (`sha256` `e57fca47...`).

**Fixed: the wires stay closed, and the price that closed them no longer sets what a
delay costs.** The flow closed a wire by making it a clock network node
(`RouteOptions::network`, which only a clock may enter) with a base cost of a million
(`RouteOptions::node_base`). The base is also half of what the timing-driven router
converts a delay into a cost with (`route::delay_scale`: the mean base of the nodes
that have a delay over their mean delay), and nothing left the million out of that
mean. Measured on the Plus at seed 4, with a copy of the flow that printed it:

| build | closed by a million | of them timed | delay scale | the fabric's own |
|---|---|---|---|---|
| `--memories legacy` | 1,127 | 1,127 | 2.918 | 0.00504 |
| the default, before | 32,630 | 9,914 | 25.63 | 0.00503 |
| the default, now | 266 | 266 | 0.695 | 0.00504 |

So the default made every picosecond about 5,000 times dearer, against congestion,
than the fabric's wires say, and nine times dearer than the legacy build already did:
a connection of any criticality negotiated for time alone, and the overuse stopped
falling. Every subset in the measurements above that still closed thousands of timed
nodes did not help, and closing the ground fans alone (266) routed: that is what the
narrowing was seeing, and why it never ended at a set of wires.

The block RAM model's tied wires are now closed outright, by `RouteOptions::closed`,
which no signal enters, a clock included, and which `delay_scale` leaves out
(`XrayFabric::block_ram_ground_fans` returns them as `TieWires::held`, apart from the
ground fans and a shift register's enable, `TieWires::fans`, which are closed as they
always were). Every wire that was closed is closed, so every tie Vivado makes is
still made; no wire was given back, because none had to be. The default build of the
Plus, the script with `--memories legacy` taken out and nothing else changed:

| build | pass 10 | pass 20 | pass 40 | end |
|---|---|---|---|---|
| the default, before | 21,260 | 5,488 | 2,763 | 14 left at pass 100: not routed |
| the default, now | 10,939 | 1,257 | 19 | 0 overused from pass 65, and through two clean-ups to 67; route 1,935 s, build 44 min |
| `--memories legacy` (unchanged) | 11,612 | 1,087 | 8 | 0 at pass 46, kept when a clean-up did not converge |

Checked on that build: 122 block RAMs configured, 2,450 inputs tied to zero over
4,168 pips and 3,968 left at the interconnect's one; 2,610,894 bits set, every one
decoding (0 unexplained), and the arcs they select exactly the router's and the ties'.
The routed estimate against the legacy build's:

| clock | default, now | legacy |
|---|---|---|
| `clk_usb` (10.4 ns) | 16.52 ns | 16.55 ns |
| `ADC_clk_fb` (4 ns) | 10.18 ns | 6.74 ns |
| the glitch MMCM's `out0` (100 ns) | 18.41 ns | 17.42 ns |
| the logic analyser's `out0` (4.17 ns) | 5.68 ns | 5.65 ns |

`ADC_clk_fb` is the one that differs, and it differed before the route: the placement's
last estimate had it at 15.84 ns against legacy's 11.07, the round that grew the
wirelength above. That is the placement of this netlist and not the router, and is
not followed further here. **Nothing of this build has been on a part**, and the
scripts keep `--memories legacy` for the board's reason, which stands.

**What it changes elsewhere.** With `--memories legacy` nothing: `held` is empty, and the
Plus is `e57fca47...` byte for byte, built with this commit. Of `qor/`, twelve are the
bitstream before byte for byte; `bram_fifo`, the one with a block RAM, routes in 16 passes
where it took 9, with 3,892 pips against 3,939 and the same 5.06 ns; its bits all
decode and its arcs are the router's and the ties'. The block RAM and FIFO fixtures
pass as before, the simulation under Vivado's models included.

**What is left as it was, on purpose.** The ground fans are still closed by a million
that `delay_scale` counts, in both models: on the legacy Plus that makes a delay 580
times dearer than the fabric's own wires would, and on the default 138 times. Every
build of the Plus that has been on a part was routed with it, so taking it out is a
new build to measure and not part of this. It also means the weight a timing-driven
route gives delay depends on how many ground fans a design's block RAMs need, which
is not a thing anyone chose.

**Mutations of this change.** Each made by hand, against the router's unit tests, the
block RAM's, and (5 to 8) `fpga_xray_bram_vivado`, `fpga_xray_fifo_vivado`,
`fpga_xray_bram` and `fpga_xray_slice_cells` with `--include-ignored` and Vivado's
simulator. Ten of twelve failed a test; of the other two, one was the flow's own
wiring, since moved where a test sees it, and one is the code it mutates.

| mutation | what failed |
|---|---|
| the search ignores `closed` | `a_closed_node_is_entered_by_no_signal_an_allowed_one_included` |
| `delay_scale` counts the closed nodes | `a_closed_node_is_left_out_of_what_a_delay_costs` |
| the per-node delays scaled with the closed nodes counted | the same |
| `Geometry::closed` always false | both |
| the held wires not closed at all | 8 of `fpga_xray_bram_vivado`'s 9 and 6 of `fpga_xray_fifo_vivado`'s 9: the ties fail or a signal reaches a tied pin |
| the held wires closed by the fans' million, as before (in the flow) | **nothing** of the fixtures, which route either way; this is what the Husky Plus measured. Moved into `TieWires::close` and given a unit test, `held_wires_are_closed_outright_and_fans_by_a_million`, which fails for it there |
| the held wires given the million as well, in `TieWires::close` | `held_wires_are_closed_outright_and_fans_by_a_million` |
| a held wire not set in `closed` | the same |
| a fan closed by `closed` instead of the million | the same |
| a wire both held and a fan given the million | the same |
| a zero's chain not held, in `block_ram_ground_fans` | 8 of `fpga_xray_bram_vivado`'s 9 |
| the held wires chained into the fans' loop in `TieWires::close` | **nothing**, and rightly: a held wire is skipped there, so the mutant is the code |

`a_block_ram_fifo_ties_its_resets_with_the_router_kept_off_the_ground_fans`
(`tests/cli.rs`, ignored) failed on master before this change: it asked for sixteen
block RAMs, and the default has made the FIFO eight `RAMB36E1` since they were
inferred. It builds both now, sixteen with `--memories legacy` and eight without.

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

**Not done.** A cascade, error correction, ~~inference of simple dual port, of a register into the
block, and of a port that both reads and writes~~ (those three since done, "Memories as
Vivado arranges them"). A block's output register named
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
the simulation above runs on. (A `RAMB18E1` and a `RAMB36E1` have one now, in the
simulation library, [Simulating the hard blocks](#simulating-the-hard-blocks); the FIFOs
have one on one clock (`EN_SYN TRUE`) and none on two, for the reason given.)

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
**That reading was wrong**, and it builds beside both since: Vivado's zero was the
rule above, the code of the frame with the shift register's contents left out, in
frames that held nothing else. See "The configuration primitives".

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

The `DSP48E1` model, `dsp48e1_model.v` here until the simulation library
(`src/fpga/devices/xc7_sim.v`) took it in with the global reset added, is written
here from UG479's diagrams and holds none of the vendor's model. `tests/fpga_dsp_vivado.rs` holds it to the 52 settings of `vectors/`;
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
**Fixed since** (the event simulator kept a stored element's own unsigned flag, not
the memory's): `fir8`'s source now simulates to `answers/fir8.xsim` line for line,
and the test holds it to that rather than taking `xsim`'s answer in its place
(`testdata/sim/signed_memory_tb.v` is the small case, checked against `xsim`).

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

**The XC7A35T settles both readings, and refutes neither** (2026-11-06,
`tools/vivado-parity/clock_rules.py`, which recomputes each rule from a bitstream's
own buffers, rows and `BUFHCE`s and holds its enables to it). Stated exactly, as it
was checked:

- **`GCLK<k>_ENABLE_*`**: a global track's rebuffers cut it into segments; the live
  segments are the buffer's own and every one between it and each clock row whose
  `CLK_HROW_*_CK_GCLK<k>_ACTIVE` is set; each live segment `[lo, hi]` sets `lo`'s
  `ENABLE_BELOW` and `hi`'s `ENABLE_ABOVE`, and nothing else is set.
- **`HCLK_CMT_CK_BUFHCLK<n>_USED`**: set in the side's `HCLK_CMT` (`X0`, the left) or
  `HCLK_CMT_L` (`X1`) of the row for every `BUFHCE_X<side>Y<n>` in use there, and
  nowhere when that side of the row has no such tile.

No disagreement on **all 528** of Vivado's bitstreams of the probes on both XC7A35T
packages (its own flow's, and its routes and bitstream of Reticle's placements), **all
335** of the XC7A100T's (141 of its own flow, 194 of Reticle's placements), and **eleven
designs made to ask** (`gen_clock_rules.py`): a buffer in the top half (`BUFGCTRL_X0Y20`, `Y31`, from a pad of
bank 15) driving row 0, 1 or 2, every row and both sides at once, and a `BUFHCE` placed
on each side of each row. The XC7A35T has what the XC7A100T could not show: its top
clock row has **no `HCLK_CMT_L` on its right** (a transceiver's column), and a
`BUFHCE_X1` used there sets no `_USED` bit anywhere; and its global buffers sit between
rows 0 and 1 rather than 1 and 2, so the far end of the segment its top row hangs from
is `CLK_BUFG_REBUF_X60Y142`, the same tile name as the XC7A100T's case for another
reason. **Still not applied**, for the reason above: the Husky's and the Husky Plus's
bitstreams change with either, and neither rule has been measured on a part.

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
**130 since 2026-11-06**, the ten below on frame ECC and options read from an XDC among
them: Reticle builds all 119 Vivado builds, the frame ECC compared too, and refuses the
same 11.

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

**Built since, and the reading above was wrong** (2026-11-06). Eight more designs
asked it: a `SRLC32E`, an `SRL16E` and a `RAM64X1S` with contents, and a `RAM64X1S` of
zeros, each with three lookup tables of logic placed in its frames (on another table of
its slice, ten rows up its column, in the other slice of its tile), with a `FRAME_ECCE2`
and without (`ecc_*_logic`, `noecc_*_logic`). **Every frame of all eight, and of the 111
other bitstreams of this fixture, has the ECC `xc7::wanted_ecc_without` gives it: the code
of the frame with a distributed RAM's or a shift register's contents left out, and
nothing else** (`ecc 0 wrong` in `vivado.txt`, `--decode`'s count). So a `FRAME_ECCE2`
changes nothing in the frames: `ecc_srl`'s zero was that code, in frames with nothing
else in them, and the frames with logic beside have ECCs that are not zero. What was
wrong was this flow's: `--decode` had the rule for both, and the writer left a shift
register's contents in the code (`XrayFabric::dynamic_table_bits` took the RAMs alone),
so every frame of a shift register with contents had the whole frame's ECC, with or
without a `FRAME_ECCE2` — four frames of a plain `SRLC32E`, built and decoded. The
writer leaves them out now, and the pair builds: all eight and `ecc_srl` agree, the ECC
of every frame compared (`ecc` against Vivado's), which the comparison did not take
before.

**This changes existing bitstreams** with a shift register whose contents are not all
zero: the ECC bits of the frames holding them, and nothing else. The Husky Plus's build
(`build_plus.sh`, whose `--srl-min 0` makes no shift register) is the same bitstream, byte
for byte.

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
  on both; what the bits are there has not been asked. **Since asked of one design
  each:** the nine probes of the configuration primitives on both XC7A35T packages set
  in `CFG_CENTER_*` what Vivado's do (`tests/fpga_xray_probes.rs`). The designs of
  `config/` have not been run there.
- **`ICAPE2`'s `DEVICE_ID` and `SIM_CFG_FILE_NAME`, `FRAME_ECCE2`'s
  `FRAME_RBT_IN_FILENAME`, `STARTUPE2`'s `SIM_CCLK_FREQ`** set nothing, as measured,
  and are read nowhere here.
- **`POST_CRC`**, and what a readback through `ICAPE2` reads.
- **Timing.** These pins are timed as any other cell's: no delay of Vivado's for them.

## The XC7A35T, in both its packages

Reticle's device list has three 7-series parts: `xc7a100t-ftg256`, everything above,
and `xc7a35t-ftg256` (the Husky's) and `xc7a35t-cpg236` (the Basys 3's). Project X-Ray
builds both XC7A35T packages from one die, the XC7A50T's grid (`artix7/xc7a50t`), and
Vivado 2026.1's installed edition targets all three (`get_parts`: `xc7a35tcpg236-1`,
`xc7a35tftg256-1`, `xc7a100tftg256-1`, and others Reticle has no device for). `xczu7ev`
is an UltraScale+ part and not asked here. Measured 2026-11-06; **nothing here has
been on a board**, and none of the Vivado bitstreams is to be loaded.

### What was asked, and how

- **The probes**, the 146 designs of [Which primitives build](#which-primitives-build).
  The FTG256's balls are the same sites on both dies, so the XC7A35T's FTG256 takes the
  XC7A100T's constraints as they are; `gen_primitives.py <vivado> xc7a35t-cpg236` writes
  the CPG236's into `primitives/xc7a35t-cpg236/`, each ball the CPG236's bonded to the
  site the FTG256's is on the same die, so both packages ask the same of the same pads:

  | role | FTG256 | CPG236 | site |
  |---|---|---|---|
  | `clk` | N11 | N17 | `IOB_X0Y24`, `IO_L13P_T2_MRCC_14` |
  | `si` | L4 | R2 | `IOB_X1Y48` |
  | `so` | L5 | U1 | `IOB_X1Y49` / `IOB_X1Y43`: the CPG236 bonds no `Y49` |
  | single pins | M1 M2 M4 M5 | T3 R3 T2 V3 | the third differs likewise |
  | a pad's clock, and its neighbour | N14, N12 | L17, P17 | `IOB_X0Y26`, `IOB_X0Y23` |
  | a pair, and `SSTL18_I` | A5, A4 | H1, J1 | `IOB_X1Y94`, `IOB_X1Y93` |

- **Vivado's own flow** on each (`run_all.sh`), which says what it builds:
  `primitives/<part>/vivado.txt`, and the ledger `index.txt` beside it.
- **The same implementation**: `run_placed.sh` with `STAGES="placement routes"` gives
  Vivado Reticle's placement and routes, and `decode_placed.sh` compares the two
  bitstreams feature by feature, as [The same placement and routes, the same
  bits](#the-same-placement-and-routes-the-same-bits) does for `qor/`. Run on all three
  parts, the XC7A100T's too, since its probes had not been through this.
- **The tiles the constraints pin**, Vivado's own bitstream against Reticle's own:
  every pad, IO logic, bank clock, configuration and XADC tile, reduced to what does not
  depend on where either tool put the logic (`probe_pads.py`, `pads.txt` beside each
  ledger). This is the one held by a test of the gate, since neither tool's placement
  moves those tiles.

### Which build

**Vivado builds 141 of the 146 on each XC7A35T package, and refuses the same five it
refuses on the XC7A100T**: the four `DCIEN` buffers (`synth_design`, `opt_design` or
the placer: no DCI on high-range banks) and `ZHOLD_DELAY` (the placer). **Reticle
builds all 141 on both**, fully routed.

That was 141 on the FTG256 and **125 on the CPG236** at first: sixteen refused by
Reticle's own device file, `F0205`, "`xc7a35t-cpg236` does not offer the IO standard
`LVDS_25`" (and `SSTL18_I`): every probe of a pair and both `IBUFDISABLE` inputs. The
file listed the Basys 3's balls and its five standards, one bank at 3.3 V, and said so
("the Basys 3 has no differential connector"); it described the board and not the part.
It now has the die's 37 standards and all 106 balls with their banks and clock and
`VREF` functions, from `package_pins.csv`, as the FTG256's has. The 125 that built
before are the same bitstream, byte for byte, and so are the Basys 3 examples that build
(`examples/basys3/`, ten), since a constraint names their pins and standards.

And **"builds" had not meant "routed"**. The `XADC` probe built on all three parts with
its `CONVST` unrouted, "the design is NOT FULLY ROUTED", which no ledger looked at. The
fabric loaded around an XADC (`main.rs`, `region_with_site_type`) stopped at its tile's
own row, and `MONITOR_IMUX43_1`, `CONVST`'s way in, is a row above: its `VFRAME` and the
interconnect beside it were outside. It reaches two rows up and three columns right now.
`the_ledger_says_what_builds` asks for a fully routed build on every part since.

### The same implementation

Of the 141 on each part, **133** go through: Reticle writes its implementation for
Vivado, Vivado takes every route and writes a bitstream. On the XC7A35T's FTG256 the two
bitstreams name 31,954 features of Reticle's and 31,985 of Vivado's, 31,864 of them in
common; on its CPG236 32,171, 32,200 and 32,082; and on the XC7A100T 33,645, 33,661 and
33,547. **On each of the three parts 82 of the 133 are the same bitstream feature for
feature, and 120, 121 and 119 differ by nothing but the two clock rules.** What the
other eight are:

| | probes | why |
|---|---|---|
| `RAM128X1D`, `RAM128X1S`, `RAM256X1S` | 3 | macros whose leaves `--vivado-placement` does not name yet |
| `BUFMRCE` | 1 | `--regional-clocks auto` builds it on a global buffer, with its `BUFR` there too, which has no `BUFGCTRL` spelling |
| `RAMB18E1`, `RAMB36E1`, `FIFO18E1`, `FIFO36E1` | 4 | Vivado: a ground or a one it ties conflicts with a fixed route (`CONFLICTS`), or, on the `FIFO18E1`, `REQP-35`/`REQP-36` on the probe's `RSTREG`/`REGCE` with `EN_SYN` false; the class [the `qor/` run](#vivados-routing-graph-took-every-route) found |

`--vivado-placement` had no spelling for 32 of them at first, on every part: the
configuration primitives, `IDELAYE2`, the serialisers, every IO buffer but `IBUF`,
`OBUF` and `OBUFT`, any global buffer not a `BUFG` (`BUFGCE` was put on `BUFG`'s bel,
which Vivado refuses for the `BUFGCTRL` it links), pulls, and a pair's second ball. Each
is written now as Vivado's own placement of the probe spells it (`get_property BEL` of
each leaf after `place_design`); a buffer of two directions or two pads is a macro and
takes its site alone.

What differs, by class, the same on all three parts unless said:

| class | probes | what it is | |
|---|---|---|---|
| Reticle only: `CLK_BUFG_REBUF GCLK<n>_ENABLE_*` | 43 | the rebuffer rule [above](#vivados-bits-beside-a-clock-answered) | **settled, not applied** (the Husky) |
| Vivado only: `HCLK_CMT*_CK_BUFHCLK<n>_USED` | 50 | likewise | **settled, not applied** |
| Vivado only: the far `GCLK0_ENABLE_ABOVE` | 2 to 4 | likewise | **settled, not applied** |
| Vivado only: `IOI_OCLKM_<n>` from the clock leaf | `ISERDESE2`, `OSERDESE2` | Vivado drives an output register's `OCLKM` from its `OCLK`'s clock; Reticle leaves it, as this document already said of an `ODDR` | **open**: closing it changes the Husky, which has `ODDR`s |
| Vivado only: `? RIOB33 30_34 31_33` and `? RIOI3 30_34 31_33` | `ODDR` | two bits each of the pad's tile and its IO logic's that the database names nothing; also in NewAE's Husky image on every `ODDR` pad. Reticle does not set them | **open**, and Husky-changing |
| Vivado only: `CLK_BUFG_BUFGCTRL<n>_I1.CLK_BUFG_IMUX<k>_<j>` | `BUFGCE`, `BUFGCE_1` | Vivado ties a `BUFGCE`'s unused `I1` from the interconnect; Reticle leaves it | **open**, and Husky-changing (NewAE's design has eight `BUFGCE`s) |
| Vivado only: a clock manager's `CLKIN2` and DRP ties, and `CLKOUT1`..`3` output enables on a `_BASE` | the four managers | the ties are [known](#vivados-bits-beside-a-clock-answered); the enables come of Reticle's netlist naming the `_ADV` with those outputs' dividers, and Vivado's own flow on the `_BASE` sets none | the enables an export artefact; the ties **open** |
| Vivado only: `? HCLK_CMT_L 24_173`, `? HCLK_L 24_13` | the four managers | a clock manager's output into the row, through bits no `segbits` names | **open** |
| Reticle only: `ILOGIC.CLKDIVP_USED` | `ISERDESE2` | the export leaves `CLKDIVP` off the cell it writes | export artefact |
| Vivado only: `VREF_BALL` on bank 35's `VREF` balls; Reticle only: `VREF.V_900_MV` | `IBUF_IBUFDISABLE`, `IBUF_INTERMDISABLE` | an `SSTL18_I` input with no `INTERNAL_VREF`: Vivado's `link_design` flow takes the reference from the balls, **its own synthesis flow does not** (a design asking only that, `vref_ext`, on both dies: no `VREF_BALL`, `VREF.V_900_MV`), and Reticle does as the second | Vivado's two flows disagree; unexplained |
| Vivado only: `PULLTYPE`, `IN` on the `KEEPER` probe's pad | `KEEPER` | Reticle's netlist has an input buffer nothing reads there, which Vivado's own flow does not | export artefact |
| Reticle only: `HCLK_L ENABLE_BUFFER` where a route crosses | `BUFH`, `BUFHCE`, XC7A100T only | one tile of a horizontal clock's way Reticle enables and Vivado does not, on one placement | **open**, one data point |
| Reticle only: an XADC frame's ECC (`--decode`'s `!`) | `XADC` | [below](#a-drp-blocks-frame-ecc) | **open**, Husky-changing |

**Fixed in Reticle**, each found here:

- **A serialiser's other clocks stayed on the pad's net** when its `CLK` moved onto the
  global buffer: `OSERDESE2` had `CLKDIV` on the interconnect beside `CLK` on the
  network, which Vivado refuses (`REQP-1975`: "should be driven by the same source
  through the same buffer type"), and `ISERDESE2` had `CLKB` and `OCLK` unrouted. The
  clock buffer pass (`primitives::clock_buffers`) moves `CLKB`, `CLKDIV`, `CLKDIVP`,
  `OCLK` and `OCLKB` with `CLK` now, on all three parts. The XC7A100T's `iologic/`
  fixtures, which compare every IO logic setting, pass as before.
- **An `IDELAYCTRL` no delay needs** is taken away and its `RDY` is a one, as Vivado
  has it. That one was an assignment, made after mapping had built every constant's
  driver, so the flip-flop reading it took a constant `D` with neither input of its
  data mux chosen; Vivado chose the bypass input (`B5FFMUX.IN_B`), which reads a one
  unrouted. A lookup table of ones drives it now, as mapping drives any other.
- **The XADC**: its `CONVST`, above; `INIT_42`, whose library default is `16'h0800`,
  written as zero when the design did not set it (Vivado writes bit 11); and
  `XADC.IN_USE`, read off NewAE's image as three bits, is two, with a bit of its own for
  each clock that is a signal: `27_1928` for `DCLK`, `26_1928` for `CONVSTCLK`. Four of
  Vivado's bitstreams say so, the probe with every pin a signal and three with its DRP
  tied (`gen_drp.py`); **the old reading is refuted**, and the Husky's bits are the same,
  its `DCLK` being a signal and its `CONVSTCLK` tied.
- **A pair's `_N` half's disables**: Vivado drives the `IBUFDISABLE` and
  `INTERMDISABLE` of both halves of a pair from the one pin, and Reticle left the second
  unrouted, where it reads a one. Six probes; both halves are roles of the leading
  buffer now (`sites::bel_pins`, `xc7.dev`).

### The tiles the constraints pin

**Equal on 138 of 141 probes, on each part**, line for line after the reduction (with
`--regional-clocks on`, which gives a `BUFMRCE` and its `BUFR` real sites as Vivado
does), and the other three differ by one known line, `OCLKM`, which
`tests/fpga_xray_probes.rs` allows by name. The test builds the 60 probes of the groups that configure those tiles on all three parts in
the gate, and every probe when ignored tests are run; it also asks of each probe it
builds that every flip-flop in use has its data mux chosen, that an IO logic site's
clock pins arrive on one kind of road, and that the build is fully routed. **Checked by
breaking it**: undoing the serialiser clocks, the `RDY` driver, the XADC's region or its
`INIT_42` each fails it, and the CPG236's standards fail
`the_primitives_the_ledger_calls_ok_map`. Two fixes it cannot see: a pair's second
disables are reached by the interconnect, which it leaves out
(`sites::a_pairs_leading_buffer_has_the_other_halfs_disables` holds them), and the XADC's
`DCLK` bit folded back into `IN_USE` decodes to the same names on a design whose `DCLK`
is a signal (`xadc::in_use_and_each_clocks_bit_are_apart`).

### A DRP block's frame ECC

**Vivado leaves a clock manager's configuration out of its frames' ECC when its `DWE`
is a signal, and an XADC's registers out always when there is an XADC**, as it leaves a distributed RAM's
contents out ([Frame ECC](#frame-ecc)). Read off its frames 28 and 29 of the block's
tile, recomputed with and without the block's bits (`drp_ecc.py`, `drp_ecc.txt`,
`a_drp_blocks_frames_ecc_is_vivados`):

| design | its frames' code |
|---|---|
| `MMCME2_ADV` or `PLLE2_ADV`, `DWE` a signal (with `DEN` or `DCLK` tied or not) | without the manager's bits |
| the same with `DWE` tied low, or a `_BASE` | the whole frame's |
| `XADC`, its DRP a signal or tied | without its `INIT_*` registers; the `IN_USE` bits are in the code |
| no `XADC`, and `XADCPOWERDOWN` (`INIT_42[5:4]`) | the whole frame's (`options_probe`) |

The `HCLK_CMT` bits that share those frames are in the code either way. Measured on
seven designs of each manager on the XC7A35T, its probe and six with `DEN`, `DWE` or
`DCLK` tied low alone or in pairs, three of the XADC's (`gen_drp.py`), and the five
probes on the XC7A100T.

The last line was found by the gate: the first version of the rule left the XADC's
registers out whether or not the design had one, and the option's frames, which Vivado
writes whole, decoded as wrong.

`--decode` reads by that rule now, a clock manager's frame either way, so Vivado's
bitstreams of every probe here decode with no frame wrong; NewAE's Husky image went from
ten wrong to four, which are frames of the left clock managers holding bits the
database names nothing (`? CMT_TOP_R_LOWER_B 28_1060` and others), so the decode
cannot leave them out. **Reticle still writes the whole frame's code** for both blocks:
the Husky has an XADC and clock managers whose `DWE` is a signal, so writing Vivado's
code changes its bitstreams, and nothing measured on a part says which a part wants.
`--decode` of Reticle's own XADC build says so, one frame wrong.

### Not done

- **The fixture sections above** (`io/`, `iologic/`, `idelay/`, `clocks/`, `config/`,
  the memories, `dsp/`, `muxf/`, `slice_cells/`) have not been run on the XC7A35T. Their
  designs name the FTG256's balls, which the XC7A35T's FTG256 has, so the next step is
  only `PART=xc7a35tftg256-1` and a test that reads two `vivado.txt`s; the CPG236 needs
  constraints of its own for each.
- **Applying the two clock rules and the DRP blocks' ECC**, which the measurements here
  support and the Husky's byte-identical builds forbid until a board can say.
- `OCLKM`, a `BUFGCE`'s `I1`, the `ODDR` pad's unnamed bits, the managers' ties: each
  changes the Husky.
- **A clock below the global-buffer threshold** (eight pins) stays on the interconnect,
  where Vivado buffers every pad clock; the `IDDR` and `ODDR` probes, whose clocks drive
  fewer than eight pins, are clocked from the interconnect. Deliberate in Reticle, and a
  difference.
- The three distributed RAM macros and the `BUFMRCE` in `--vivado-placement`.

## The clock's arrival on the XC7A35T

**Asked: does the fitted delay model time the XC7A35T as it times the XC7A100T, and
what does the one table that existed only for the XC7A100T, when the global clock
reaches each register ([Filling slices](#filling-slices), change 4), give it?** Run
on 2026-11-09 with Vivado 2026.1 at `xc7a35tftg256-3`, on the timing corpus of
[Placement and routing against Vivado's own flow](#placement-and-routing-against-vivados-own-flow).
**Everything here is Vivado's timer; nothing has been on a part.**

### What was measured

`clock_arrival.tcl` takes a part and its three balls now, and its rows from the
part's slices (three a region row of fifty, as the XC7A100T's were). Four runs of
700 flip-flops, 297 of them held to every even slice column on three rows of each
region row: from `BUFGCTRL_X0Y0`, `X0Y16` and `X0Y31` at `xc7a35tftg256-3`, and from
`X0Y0` at `xc7a35tcpg236-3` (balls `N17`, `R2`, `U1`). Vivado's `xc7a35t` has the
whole of Project X-Ray's `xc7a50t` grid, 8,150 slices in three region rows of two
columns, which is the grid Reticle loads for both packages
(`testdata/fpga/vivado/clock/arrival_xc7a35t.txt`):

- **The table** (`xray::timing::XC7A35T_CLOCK`) is the medians of the three FTG256
  runs, `clock_arrival.py table arrival_xc7a35t.txt xc7a35tftg256`: every one of
  their 2,100 flip-flops within 6.1 ps of it against their own insertion delay. The
  top region row is 150 ps later than the middle and the bottom one 23 ps; the
  columns left of the leftmost block RAM column and right of the rightmost are about
  60 ps later than the rest; the growth away from a region's `HCLK` row is the
  XC7A100T's, 0.83 ps a row. **Measured.** The top region row has no slice, block
  RAM or `DSP48E1` right of grid column 91 (the die's corner is empty), so the
  table's figures there are column 91's filled in and time nothing.
- **The package moves nothing.** The CPG236's 700, which the table was not fitted
  to, are within 5.9 ps of it, and their insertion delay is the FTG256's to 1 ps
  (1,196 against 1,197 ps). One table serves both, keyed by the die
  (`clock_table`). **Measured**, on one buffer.
- **The XC7A100T's table would not do**: against it, 876 of the 2,800 are more than
  7 ps off, up to 77 ps, in every column of the die. The two grids share their
  columns up to grid column 56 and not past it, and the XC7A35T's bottom region row,
  grid row 130, is the die's edge where the XC7A100T's is an inner one.
- **Speed grade 1 is about 1.26 times slower** to every clock pin: the same probe
  at `xc7a35tftg256-1`, 332 flip-flops on the same sites as the grade-3 run, 1.254
  to 1.276 times, and its spread over the die 278 ps against 236. **Measured**, and
  not applied: the table is grade 3's, as every figure of the fitted model is
  ([The delay model, fitted to Vivado's timer](#the-delay-model-fitted-to-vivados-timer)),
  while `xc7.dev` declares both XC7A35T packages speed 1 (the Basys 3's part is
  `-1`). On a grade-1 part every figure of the fitted model is about a quarter
  under Vivado's for that part, the clock's as much as the rest; the comparisons
  below are all at grade 3 so that the table is what differs. **Applied since**, and
  not as a factor: the probe run again from all three buffers at `-1` and `-2` on
  both dies is a table a grade, and grade 3's table times 1.26 is not it (the spread
  grows by 1.13 on this die and 1.25 on the XC7A100T, the outer rows by more than the
  inner): [Speed grades](#speed-grades).

### The fit, before and after

The same comparison as [How it came out](#how-it-came-out), `run_placed.sh` with
`TIMING=1` and `timing_table.py compare --part xc7a35tftg256-3`, seed 1 of each
build on the seventeen designs, the tree before (`before`, no clock figures on this
part) and with the table (`after`). Each estimate / Vivado, median, 5th and 95th
percentile, lowest and highest:

| | before | after | the XC7A100T, fitted ([above](#how-it-came-out)) |
|---|---|---|---|
| connections | 15,654: 1.000 / 0.924 / 1.192, 0.696 to 2.896 | 15,709: 1.000 / 0.924 / 1.105, 0.702 to 2.924 | 18,166: 1.003 / 0.949 / 1.179 |
| endpoints | 4,856: 0.997 / 0.840 / 1.105, 0.467 to 1.483 | 4,854: 0.989 / 0.849 / 1.097, 0.630 to 1.387 | within 3 % at each design's median |
| each clock's worst endpoint | 0.88 (`esad` adc) to 1.08 (`srl_delay`) | 0.89 (`esad` adc) to 1.06 (`srl_delay`) | 0.91 to 1.05 |

**The fitted model times the XC7A35T as it times the XC7A100T**, connection by
connection: its median connection is 1.000 of Vivado's on both builds, which is a
statement about the class table and not the clock (a connection's delay has no
clock in it). The two builds are different placements, so the two connection rows
differ by that and not by the table. **What the table changes is the endpoints of
designs across two region rows**: `esad`'s highest endpoint ratio went from 1.403 to
1.186 and its 95th percentile from 1.107 to 1.067, `sad`'s highest from 1.483 to
1.363, and the lowest of all from 0.467 to 0.630; the designs inside one row move
by their placement's noise.

And Reticle's routed estimate of each clock against Vivado's figure on the same
routes (the measure "Filling slices" used, over ten seeds): **0.970 at the geometric
mean, 0.919 to 1.015 from the 5th to the 95th percentile, before; 0.968, 0.911 to
1.014, after.** The XC7A100T's was 0.981 and 0.935 to 1.015. The clock it changes is
the one that crosses rows: `esad`'s adc clock, median 0.945 before and 0.969 after.

### What it does to placement

`achieved.sh` on the XC7A35T, both builds at ten seeds, Vivado's own flow asked three
times as before (ns; Vivado's figure is its best, Reticle's the median, the ten
beside it in `clk35`'s runs and not committed):

| design | clock | Vivado | Vivado, Reticle's netlist | before | after |
|---|---|---|---|---|---|
| adders | clk | 2.152 | 2.165 | 2.345 | 2.223 |
| analyser_fifo | rd_clk | 3.840 | 5.198 | 5.172 | 5.276 |
| analyser_fifo | wr_clk | 2.306 | 3.098 | 2.987 | 2.974 |
| barrel | clk | 2.311 | 2.646 | 2.502 | 2.486 |
| bram_fifo | clk | 3.420 | 3.480 | 3.930 | 3.875 |
| carry_counter | clk | 1.808 | 2.179 | 2.274 | 2.258 |
| counter64 | clk | 3.198 | 3.722 | 3.735 | 3.774 |
| crc32 | clk | 2.130 | 2.231 | 2.179 | 2.181 |
| esad | adc_sampleclk | 2.214 | 3.769 | 3.144 | 3.135 |
| esad | clk_usb | 1.609 | 2.744 | 2.508 | 2.553 |
| fir8 | clk | 9.142 | 9.164 | 9.161 | 9.161 |
| fsm | clk | 1.822 | 1.878 | 2.137 | 2.187 |
| gray_cdc | clk | 1.466 | 1.447 | 1.413 | 1.413 |
| gray_cdc | clk2 | 1.402 | 1.578 | 1.514 | 1.514 |
| lutram32 | clk | 1.895 | 1.884 | 1.984 | 1.990 |
| mul18 | clk | 1.932 | 1.935 | 2.085 | 2.027 |
| regfile_alu | clk | 5.514 | 5.494 | 5.915 | 5.957 |
| sad | adc_sampleclk | 2.236 | 4.573 | 3.164 | 3.162 |
| sad | clk_usb | 1.644 | 2.476 | 2.460 | 2.543 |
| srl_delay | clk | 1.699 | 1.670 | 1.867 | 1.777 |
| widemux | clk | 2.491 | 2.705 | 2.942 | 2.872 |

**Over the twenty-one clocks, Reticle's median is 1.175 times Vivado's own figure
with the table and 1.179 without; ten clocks faster, eight slower, three the
same.** That is inside a seed's spread, as the same change was on the XC7A100T
(1.244 against 1.242 over five seeds there), and is reported as neutral, not as a
gain. The table is kept because it is Vivado's figure and the estimate is nearer
Vivado's with it where it matters, as above. Against Vivado's placement and routes
of the same netlist it is 0.994 and 0.997: on this corpus Reticle's implementation
of a netlist is Vivado's, and the 18 % between the two tools is the netlist, as on
the XC7A100T.

**`esad`'s adc clock still crosses region rows**: its latest path has Vivado's skew
under -0.15 ns on nine seeds of ten before and seven after, and `sad`'s on none and
two. On the XC7A100T the table kept `sad` inside a row. Why it does not here,
whether a pad's place pulls the logic across or the pull's charge is too small
against the rest, has not been looked at.

### What it changes

Under the fitted model only: `clock_arrival` gives an XC7A35T fabric, in either
package, its table, where it gave it none, so every timing-driven XC7A35T build
places and routes differently. `--delay-model legacy` has no clock figures on any
part, and the Husky's two builds pass it: with this tree's release binary
`build.sh` (the XC7A35T's FTG256) writes
`3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd` and
`build_plus.sh` seed 4's
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked**,
byte for byte.

### Which 7-series parts have a table

Every 7-series part in Reticle's device list: the `xc7a100t-ftg256`
(`XC7A100T_CLOCK`, [Filling slices](#filling-slices)) and the `xc7a35t-ftg256` and
`xc7a35t-cpg236` (`XC7A35T_CLOCK`, the CPG236 measured on one buffer). There is no
other. Another die would take a device file and Project X-Ray's grid first; then the
table is four runs of `clock_arrival.tcl`, a few minutes each, one of
`clock_arrival.py fixture` and `table`, and a `ClockTable` keyed by the die's name
in `clock_table`. A die whose region rows are not fifty slices tall, or whose rows
the probe's three per row do not describe, would need the probe looked at first.

### Tests

`every_flip_flop_vivado_timed_on_the_xc7a35t_is_within_its_tables_reach`
(`tests/fpga_xray_clock_arrival.rs`): all 2,800 within 7 ps of the table against one
insertion delay (6.8 ps the worst, a CPG236 flip-flop at the die's corner), three
FTG256 runs and one CPG236, and the CPG236's insertion delay within 2 ps of the
FTG256's. `the_clock_reaches_a_site_when_the_table_says` (`xray::timing`) now also
gives a fabric of the XC7A35T's three `HCLK` rows its table under both packages'
names, not the XC7A100T's, refuses the XC7A35T's table on the XC7A100T's four rows,
and gives a die nobody measured nothing.

**Mutations**, each made by a script and put back, against both tests:
the XC7A35T given the XC7A100T's table, or none, or the CPG236 none; the growth a row
left out; the region rows in the wrong order; the right-hand columns of the middle row
at the middle's figure; and one figure 10 ps off. Seven of the eight failed the
integration test, and the four about which table a fabric gets and its rows failed
the unit test too. **One survived**: the top row's right-hand
columns 60 ps later, as the lower rows' are. It cannot be caught, since nothing is
there to time, and it is how the reading that the top row's right-hand columns were
*not* later was found to be a reading of filler: the first draft of this section said
so.

### Not done

- **Speed grade 1.** The table and the whole fitted model are grade 3's, and both
  XC7A35T devices say speed 1. Scaling by the 1.26 measured here for the clock is a
  reading of one probe and nothing of the rest's; fitting the model at grade 1 is
  its own run. **Done since**, at grades 1 and 2: [Speed grades](#speed-grades).
- The block RAM and `DSP48E1` columns, which take their nearest slice column's
  figure, on this die as on the other.
- Why `esad`'s adc clock still crosses rows.
- None of it has been on a part.

## Speed grades

**Asked: the fitted delay model was Vivado's timer at speed grade 3, every figure of
it, and every XC7A35T device Reticle has declares grade 1; how far off is it on a
grade-1 part, would one factor a grade put it right, and what does timing at the
right grade do to a build?** Run on 2026-11-09 with Vivado 2026.1. **Everything here
is Vivado's timer; nothing has been on a part.**

### How the grade reaches the model

Before: a device file's `speed` (`1` for both XC7A35T packages, `3` for the
`xc7a100t-ftg256`) went into the part name a Vivado script was written for and
nowhere else. The fitted model had one set of figures, grade 3's, whatever the part.
There was no option for it.

Now: `xray::SpeedGrade` (`1`, `2`, `2L` or `3`) is read from the device's `speed`,
or from `--speed-grade`, which replaces it, part name included (a Vivado script then
names `xc7a35tcpg236-2L`). Under `--delay-model vivado` the build times at it: the
wire classes (`xray::delay::CLASSES`, a column a grade), the branch charge, the share
of prjxray's delay a wire of no class costs, every cell figure
(`xray::timing::VIVADO`, `VIVADO_2`, `VIVADO_1`), the carry chain's arcs, the
placement's line, a flip-flop's clock to `Q` by bel and its setups by way, an
`IDELAYE2`'s intercept, the clock's insertion delay for an input delay, the clock's
arrival tables, and the router's distance estimate a tile
(`xray::route_tile_delay_ps`). A timing-driven build says which: `note: timed at speed
grade 1 (the device's)`. A grade the model has no figures for is refused, not timed
as another. `--delay-model legacy` has no grade, and builds as before at any.

`-2L` is timed as `-2` and `-1L` as `-1`: **checked**, every figure of the corpus
below at `xc7a35tftg256-2L` is its `-2`'s and at `xc7a35tiftg256-1L` (the industrial
die, the only `-1L` Vivado has of these parts) its `-1`'s, connection, arc and
endpoint alike. `--speed-grade 1L` is refused, since no part Reticle names has it.

### What was measured

The same placements and routes timed again at each grade, so that a grade is the
only thing that differs: `tools/vivado-parity/regrade.sh` gives Vivado a
`run_placed.sh` run's routes at other parts and `timing_dump.tcl` each. Three runs:

- the corpus the model was fitted on, the legacy-built seventeen on
  `xc7a100tftg256`, at `-1` and `-2` (27,016 connections, 18,220 lookup table arcs);
- the same seventeen as the fitted model built them on `xc7a35tftg256` (the run of
  [The clock's arrival on the XC7A35T](#the-clocks-arrival-on-the-xc7a35t)), at
  `-1`, `-2`, `-2L` and the industrial `-1L`;
- an earlier fitted build on `xc7a100tftg256`, at `-1` and `-2`;

and `clock_arrival.tcl` from the same three buffers at `-1` and `-2` on both dies,
with a CPG236 run at each on the XC7A35T, and `idelay_sweep` and `ddly_datain` at all
three grades.

**A grade is the same factor on both dies, and on both builds**: a connection's
interconnect delay at `-1` is 1.398 times its `-3` at the median on the XC7A35T
build and 1.403 on the XC7A100T build, at `-2` 1.136 and 1.137; each class of cell arc
follows the same line from `-3` on both corpora to a few picoseconds. **But it is not
one factor for everything:**

| at grade 1, over grade 3, same cells and routes (median) | grade 2 | grade 1 |
|---|---|---|
| a connection (interconnect) | 1.136 | 1.398, 1.34 to 1.52 from the 5th to the 95th percentile |
| a slice's output wire (`LOGIC_OUTS`), by the fit | 1.14 to 1.28 | 1.43 to 1.84 |
| a lookup table's slow input wire (`A1`, `A2`), by the fit | 1.09 to 1.10 | 1.28 to 1.31 |
| a six-tile line (`NN6BEG`, `SS6BEG`), by the fit | 1.08 | 1.27 |
| a connection to a slice's bypass, `CE` or `SR` pin | 1.14 to 1.17 | 1.41 to 1.53 |
| a lookup table (0.097 ns arcs; the 0.21 to 0.25 ns quarter) | 1.08 | 1.28 (1.40) |
| a flip-flop's clock to `Q` | 1.11 | 1.34 |
| a carry chain | 1.11 | 1.34 |
| a block RAM's clock to output; its setups | 1.15; 1.11 | 1.33; 1.28 |
| a `DSP48E1`'s arcs | 1.14 to 1.17 | 1.35 to 1.43 |
| the input and output buffers | 1.06, 1.04 | 1.11, 1.12 |
| the clock's insertion (`qor/gray_cdc.v`'s registers; the probe's) | 1.10 (1.11) | 1.26 (1.28 on the XC7A35T, 1.32 on the XC7A100T) |
| the clock's spread over the die (the probe) | 1.06 to 1.10 | 1.13 to 1.25 |
| an `IDELAYE2` at no taps (`IDATAIN`; `DATAIN`) | 1.13; 1.30 | 1.36; 1.35 |

All **measured**, on the runs above; the taps of a delay are the same at every grade
(31 of them 2.975 ns at each).

### One factor a class, or a table a grade

Asked first: how near would grade 3's figures come, scaled by one factor for each
class (wires, cells, setup, the clock)? On the same routes, the factor that is each
class's median, against Vivado's figure at that grade (5th to 95th percentile):

| | grade 2 | grade 1 |
|---|---|---|
| a connection, Vivado's own grade-3 figure scaled | 0.979 to 1.033 | 0.957 to 1.088 |
| an endpoint, logic, route, setup and skew each scaled | 0.978 to 1.011 | 0.944 to 1.029 |
| an endpoint, one factor for all | 0.988 to 1.020 | 0.967 to 1.050 |
| a connection, grade 3's class table scaled, on the XC7A35T build | 0.925 to 1.049 | 0.900 to 1.057 |
| a connection, that grade's own class table, on the same | 0.927 to 1.043 | 0.933 to 1.036 |

(Grade 3's own table on the same is 0.923 to 1.047.) **At grade 2 a factor would do;
at grade 1 it would not**: the classes slow by 1.17 to 1.84, and the factor leaves the
slow ones a tenth under and the fast ones over. So each grade has a table of its own,
fitted as grade 3's was: `timing_table.py fit --part` of the legacy-built corpus at
`-1` and `-2`, the same 274 classes, 1.002 of Vivado's figure at the median at each,
0.960 to 1.035 from the 5th to the 95th percentile at grade 1 and 0.955 to 1.041 at
grade 2, against grade 3's 0.952 to 1.045 (`testdata/fpga/vivado/timing/classes_1.txt`,
`classes_2.txt`, and the sample's figures at each grade in `sample_grades.txt`). The
branch charge is 10.1, 9.1 and 8.4 ps at grades 1, 2 and 3, and the line for the
placement 0.594 + 0.0485 a tile, 0.474 + 0.0402 and 0.414 + 0.0358.

The cells are each grade's too, from `arcs_1.txt` and `arcs_2.txt`: a figure of grade
3's that is one of Vivado's arcs (a flip-flop's clock to `Q` on each bel, a block RAM's
clock to output, the `DSP48E1`'s cascade and its `M` and `P` to `P`, the buffers, a
shift register's and a distributed RAM's setups) is that arc's figure on the same cells
at the grade, **measured**; one that is a mean or a reading (a lookup table's 0.13 ns, a
flip-flop's setups, the carry's fallback, a block RAM's setups) goes through the line
its class's arcs follow from grade 3 to the grade, which every arc of the class is
within a few picoseconds of (a lookup table's 5th to 95th percentile -6 to +19 ps at
grade 1, a flip-flop's clock to `Q` ±1.5); the carry chain's arcs are each grade's own
medians. The figures grade 3's were derived for (`DSP48E1` settings no design used, a
shift register's address) are at grade 1 the constants read off Vivado's grade-1
`report_timing` that grade 3's were derived from, and at grade 2 derived from those; where
the corpus has the same arc, grade 1's measured figure is that constant (`M` to `P`
2.305 ns, `P` to `P` 0.434, the cascade 1.713). And the clock's arrival is a table a
grade on each die, every flip-flop within 3.5 to 6.1 ps of it; grade 3's table, set
against the same flip-flops, is 21 to 57 ps off, the outer region rows slowing by more
than the inner.

### The fit at each grade

`run_placed.sh` with `TIMING=1` on the XC7A35T FTG256, seed 1, each build timed by
Vivado at the grade it was built for (`timing_table.py compare --part`), and the tree
before this change timed at `-1`, which is what a Basys 3 build was. Estimate over
Vivado, median / 5th / 95th percentile (`testdata/fpga/vivado/timing/summary_grades.txt`):

| | connections | endpoints | each clock's worst endpoint |
|---|---|---|---|
| before, at grade 1 (grade 3's figures) | 15,709: 0.715 / 0.654 / 0.799 | 4,854: 0.723 / 0.597 / 0.780 | 0.66 to 0.76 |
| grade 1 | 15,553: 0.998 / 0.917 / 1.185 | 4,849: 0.998 / 0.862 / 1.120 | 0.88 (`widemux`) to 1.04 |
| grade 2 (`--speed-grade 2`) | 15,625: 1.002 / 0.930 / 1.228 | 4,856: 0.988 / 0.851 / 1.089 | 0.93 to 1.08 |
| grade 3 (`--speed-grade 3`) | 15,709: 1.000 / 0.924 / 1.105 | 4,854: 0.989 / 0.849 / 1.097 | 0.89 to 1.06 |

**Before, a grade-1 build was put 28 % under Vivado's figure for it**, connection
and endpoint alike, and every clock's worst by a quarter to a third; **at its grade
now, every design's median connection is within 2 % of Vivado's at every grade** and
the endpoints are where grade 3's were. The grade-3 row is the earlier section's
`after`, figure for figure (the same build: grade 3's figures are what they were). The
95th percentile of a connection is 1.19 and 1.23 at grades 1 and 2 against grade 3's
1.11; what makes those few percent, beyond that they are other builds, has not been
looked at.

**A design constrained at 10 ns on a grade-1 part** is timed against its 10 ns by
grade 1's figures: the period is the constraint's whatever the grade, and only the
figures move. `esad`'s and `sad`'s `clk_usb` are 10 ns clocks: at grade 1, Vivado's
worst 3.988 and 4.078 ns, Reticle's estimate of the same endpoints 4.066 and 4.243
(1.02, 1.04), where before it was 3.602 against 4.993 (0.72). `fpga_xray_timing`'s
`a_grade_one_part_is_timed_by_grade_ones_figures` holds the Basys 3's blink to it:
`of 10 ns` at both grades, and grade 1's first estimate 1.40 times grade 3's.

### What the placer and router are given

Nothing in the placer or the router compares a delay with a constant of its own but
three things, each now the grade's: the period, which is the constraint's
(`PlaceTiming::periods`), against which a domain's criticality is spanned when its
longest path is shorter, and an input's budget is reckoned from; the router's
distance estimate a tile, 25 ps at grade 3 (`route_tile_delay_ps`: 35 and 28 at grades
1 and 2, the same share of a connection's delay); and the branch charge. The rest is
ratios: criticalities are against the longer of the period and the longest path, and
the router weighs a picosecond against the mean delay of its timed nodes
(`route::delay_scale`), so a grade that slowed everything by one factor would route as
grade 3 does wherever the longest path is over the period. **What changes is a domain
whose period is longer than its paths at grade 3 and shorter at grade 1**: before, its
paths looked a quarter shorter than they were and were weighed against the period,
with less pressure than they needed.

### What it does to placement

`achieved.sh` on the XC7A35T FTG256 at `-1`, ten seeds, Vivado's own flow asked three
times as before, each clock at 0.85 of Vivado's best; the tree before (grade 3's
figures) and with this change (grade 1's), both timed by Vivado at `-1`. ns; Vivado's
figure is its best, Reticle's the median over the seeds (`sad`'s after, over nine:
seed 2's placement Vivado refused, below):

| design | clock | Vivado | Vivado, Reticle's netlist | before | after |
|---|---|---|---|---|---|
| adders | clk | 2.958 | 2.902 | 3.014 | 3.050 |
| analyser_fifo | rd_clk | 5.351 | 6.892 | 7.239 | 7.180 |
| analyser_fifo | wr_clk | 3.265 | 3.867 | 4.175 | 4.115 |
| barrel | clk | 3.283 | 3.511 | 3.442 | 3.444 |
| bram_fifo | clk | 4.583 | 4.763 | 5.220 | 5.261 |
| carry_counter | clk | 2.586 | 2.950 | 3.062 | 3.115 |
| counter64 | clk | 4.309 | 4.963 | 5.177 | 5.193 |
| crc32 | clk | 2.871 | 3.060 | 3.236 | 2.910 |
| esad | adc_sampleclk | 2.872 | 4.887 | 4.171 | 4.072 |
| esad | clk_usb | 2.155 | 3.343 | 3.551 | 3.476 |
| fir8 | clk | 12.437 | 12.460 | 12.458 | 12.458 |
| fsm | clk | 2.510 | 2.856 | 2.978 | 2.953 |
| gray_cdc | clk | 2.006 | 1.963 | 1.940 | 1.940 |
| gray_cdc | clk2 | 1.875 | 2.025 | 2.099 | 2.046 |
| lutram32 | clk | 2.318 | 2.419 | 2.720 | 2.735 |
| mul18 | clk | 2.437 | 2.580 | 2.709 | 2.780 |
| regfile_alu | clk | 7.690 | 7.411 | 8.056 | 8.101 |
| sad | adc_sampleclk | 2.938 | 5.906 | 4.373 | 4.356 |
| sad | clk_usb | 2.263 | 3.451 | 3.578 | 3.483 |
| srl_delay | clk | 2.101 | 2.204 | 2.532 | 2.492 |
| widemux | clk | 3.202 | 3.654 | 4.011 | 3.851 |

**Over the twenty-one clocks, Reticle's median is 1.193 times Vivado's own figure at the
geometric mean with grade 1's figures and 1.205 with grade 3's; 1.016 and 1.027 of
Vivado's placement and routes of the same netlist. Eleven clocks faster, eight slower,
two the same.** Most of that is inside a seed's spread, as the clock tables' change
was; `crc32`, 3.236 to 2.910 ns, is not (every one of the ten seeds after is under every
one before), and is the case above: at 0.85 of Vivado's best its paths fit the period by
grade 3's figures and not by grade 1's. Reported as a small gain on this corpus, not as
a general one.

**The XC7A100T builds nothing differently**: its device declares grade 3, whose figures,
tables and charges are what they were. **Checked**: every design of this corpus built
timing-driven on the `xc7a100t-ftg256` by the tree before and by this one writes the
same bitstream, byte for byte, so `qor/achieved.txt`, measured there at `-3`, stands.

### The Husky's bitstreams

They pass `--delay-model legacy`, which has no grade. With this tree's release binary
`build.sh` writes `3fd3e4c8df2e8c5ffe9f70c552a4c02a3df0b11eaaff69798b4ffdcad40d30dd`
and `build_plus.sh` seed 4's
`e57fca471db9f9ee965f35634770b95e9a53c9edab5aae6190ddba868e4d274f`. **Checked**, byte
for byte.

### Tests

- `xray::delay`'s `a_grade_reads_its_own_column`: the grades as Vivado writes them, `1L`
  refused, grade 1 the slowest column of every class of 20 ps or more, and each grade's
  branch charge.
- `tests/fpga_xray_delay.rs`: the source holds each grade's fit
  (`the_source_holds_the_table_the_fit_wrote`, now all three); over the sample, each
  grade's column is within 3 % of Vivado's figure at that grade at the median and 0.9
  to 1.1 from the 5th to the 95th, and grade 3's column against grade 1's figures is
  under 0.8 (`at_each_grade_a_connection_is_within_a_few_percent_of_vivados_figure`);
  and the quoted run of `summary_grades.txt` has every design within 3 % at each grade
  and every clock's worst within 15 %, and before under 0.8.
- `tests/fpga_xray_clock_arrival.rs`: every flip-flop at grades 1 and 2 within 7 ps of
  its grade's table, both dies and both packages, and grade 3's table more than 14 ps
  off on each.
- `xray::timing`: a site's clock by the grade asked and not the one in a part's name;
  a flip-flop's launch and setups at grade 1; an `IDELAYE2` at grades 2 and 1.
- `tests/fpga_xray_timing.rs`: a node costs its class at grade 1 as at grade 3; and
  `a_grade_one_part_is_timed_by_grade_ones_figures`, above.
- `tests/cli.rs`'s `fpga_takes_the_speed_grade_it_is_given`: the Vivado script's part
  follows `--speed-grade`, and `1L`, or a grade on another family, is a usage error.

**Mutations**, each made by a script and put back, against all of those:
the class table read in grade 3's column whatever the grade, or grade 1 read in
grade 3's; one branch charge for every grade; the share of prjxray's delay grade
3's at every grade; `node_delays` at grade 3 whatever it was asked; grade 1 given
grade 3's cell figures; the XC7A35T's grade-3 clock table at every grade; the
XC7A100T's grade-1 and grade-2 tables swapped; a flip-flop's launch looked up by
the wrong bel; an `IDELAYE2`'s two intercepts swapped, and grade 2's set to grade
3's; the build timed at grade 3 whatever the device says; the legacy model given
grade 1's figures; and `--speed-grade` not reaching the part's name. Fourteen of
fifteen failed a test. **One survived:** one figure of grade 1's cells off by
itself (a lookup table's 0.174 ns put back to grade 3's 0.13). Nothing in the gate
holds a cell figure of any grade to `arcs_1.txt`, `arcs_2.txt` or, for grade 3,
`arcs.txt`; only the quoted runs of `summary_grades.txt` do, which a regeneration
and not the source would move. That is as grade 3's figures were.

### Not done

- **Grade 1's and 2's figures are fitted on grade 3's corpus, re-timed**, not on
  builds made at those grades: the same routes, so the table sees what grade 3's saw. A
  fit on grade-1 builds would see other routes; the comparison above, on builds the
  tables had not seen, is the check that it does not matter much.
- The connections' 95th percentile at grades 1 and 2, 1.19 and 1.23 against grade 3's
  1.11.
- The clock managers, a pad's register and an IO delay's controller have no figures
  of Vivado's at any grade, as before.
- `sad` seed 2 at grade 1: Vivado refused its placement (`Cannot set LOC and BEL
  properties` on a `D5FF`, "Routing contention at pips element DOUTMUX"), a packing
  rule the placer broke on that seed. Not looked at; it is the placer's, not the
  timing model's.
- None of it has been on a part, at any grade.

## The language corpus, synthesised by both

### What was asked

Whether what Reticle builds from the language corpus of
`docs/simulation.md` ("The language, against xsim") computes what Vivado's
build of the same source computes. Sixteen of the corpus's testbenches have
a synthesizable design in them; their nineteen modules (with the
parameters the testbenches give them) were built for the XC7A35T by both
tools, and each routed netlist was run under xsim with the testbench the
source came from, its output held to what xsim printed for the source.
Vivado's netlists were also run under Reticle's simulator with `--device`.

This was a measurement made once, in a scratch directory, with the
binary of the commit that brought the corpus's Verilog to 71 of 103; the
netlists are Vivado's output and are not committed, and nothing here is a
test. The answers below are what those runs printed.

### What was compared, and how it came out (measured)

- **Every bit decodes** in all nineteen of Reticle's bitstreams: no bit
  unexplained, no tile without a segbits file.
- **Reticle's routed netlists compute what the source computes** under xsim
  for eighteen of the nineteen. The nineteenth, `vh_adder`'s `gadd`, built
  its sum as a constant 0: the VHDL frontend read a slice bounded by a
  generic, `v(8*i+7 downto 8*i)`, as zeros (`vhd/min_54.vhd`), and Reticle's
  own simulation of the source showed the same. That was fixed with the
  VHDL commits under this one ("unroll loops whose parameter bounds a
  slice"). Built again with this branch's binary, both widths' routed
  netlists now compute what the source does under xsim, and every bit of
  their bitstreams decodes.
- **Four builds exit 0 with a design that is NOT FULLY ROUTED**:
  `sv_pkg_alu_b`'s `alu`, both `gadd` widths and `vh_pipeline`'s `pipe`
  (`gadd` still, when built again with this branch's binary).
  In each the router gave up on two signals that both need one slice's `D`
  output multiplexer (`CLBLM_L_DMUX`): the last sum bit of a carry chain
  and its carry out. The flow says so in a warning; the netlist still
  matched under xsim, because a routed netlist carries no routing, which
  is why "every bit decodes" and the router's own account, not a netlist
  simulation, are the checks that see this. Not fixed here: it is the
  packer's and the router's, which other work owns.

  **Fixed since (2026-11-08), in both halves.** The build of a design that
  is not fully routed now fails: `reticle fpga` still writes the bitstream,
  as it always did, for `--decode` and the report to read, and then exits 1
  with "error: the design is NOT FULLY ROUTED", where it had been a warning
  on a build that exited 0. The cause was neither the router nor the
  placer: a lane's sum and its carry out leave a slice by the same `<L>MUX`,
  and the mapper takes the top bit of `{0, a} + {0, b}` off the last lane's
  carry out, beside the top sum, so a design reading both (`gadd`'s second
  adder, `+ c`) asks for one wire twice. What Vivado writes for that was
  measured (`docs/fpga-xray.md`, the carry chain section): the carry out off
  the lane above, a pass-through, and at a multiple of four bits a `CARRY4`
  of its own. `xray::legalise_carries` now does the same. Built again, all
  nineteen route, every bit of every bitstream decodes, and each of the four
  has one `CARRY4` and one lookup table more than the table below says
  (sv_pkg_alu_b 7, `gadd` 10 and 14, `pipe` 3). Under xsim each of their
  routed netlists still computes what its source does (**measured**, the
  same testbenches, `GSR` released at once).
- **Vivado's netlists under Reticle's simulator** compute what the source
  computes for the Verilog and SystemVerilog designs but one: `v_ram_b`,
  where Vivado put the byte-write memory in a `RAMB18E1` and Reticle's model
  of it read zeros. A separate run found Reticle's `DSP48E1` model giving 0
  for the pre-adder configuration Vivado chose for `vh_pipeline`
  (`INMODE = 5'b00100`, `USE_DPORT = "TRUE"`). Neither is fixed here; both
  are the vendor-cell models', not the language's. The VHDL testbenches
  were not run this way, since Reticle's simulator does not mix a VHDL
  testbench with a Verilog netlist.

  **That reading was wrong, and neither model was.** Reticle's hard-block
  models hold the global reset for the first 100 ns, as xsim's standard
  `glbl` does, and both testbenches did their work inside it (the
  `vh_pipeline` one, a Verilog transcription, finishes at 80 ns); the xsim
  runs they were held to had `GSR` released at once. Under xsim with the
  standard `glbl`, Vivado's `v_ram_b` netlist printed exactly the four
  wrong words Reticle's simulator did. Both configurations, as Vivado wrote
  them, are now testbenches in `testdata/sim/xc7/` (`ramb18e1_byte_write.v`,
  `dsp48e1_preadd.v`), driven through the first 100 ns and after, and
  Reticle prints what xsim with Vivado's `unisims_ver` and `glbl` prints, byte
  for byte; the two corpus testbenches started after 120 ns print what the
  source does under Reticle's simulator (**measured**, 2026-11-08). What is
  left is that Reticle's slice cells do not hold the reset at all
  (`docs/simulation.md`, *Vendor cells*).
- **Reticle's simulator on Reticle's own routed netlist** gave all `x` for
  `v_alu`: two lookup tables of one site form a structural loop the
  simulator does not settle. xsim runs the same netlist correctly.

  **Fixed since (2026-11-08); the loop was false.** It is one `LUT6_2`, not
  two tables: `O5` is bit 7's propagate, `a[7] ^ b[7]` on `I0` and `I3`, it
  feeds the `CARRY4` lane whose sum `O[3]` comes back on `I1` for `O6`, and
  `O5`'s truth table (`0x55aa55aa`) does not read `I1`. Nothing loops in the
  silicon. The cell library writes a table as `INIT[index]`, and an index
  with an unknown bit reads `x` by the language's rule, so `O5` never left
  `x`. xsim's tables answer what every entry an unknown index could name
  agrees on; the simulation library now has tables that do the same
  (`docs/simulation.md`, *Vendor cells*), held to xsim by
  `testdata/sim/xc7/lut_unknown_inputs.v`, and the netlist prints what the
  source does.
- With xsim's standard `glbl`, which holds the global set/reset for the
  first 100 ns, both tools' netlists lose what the testbenches drive before
  then; every comparison above holds `GSR` released from the start
  (`ROC` width 0), which is what the testbenches assume.

### Where the builds differ

Cell counts as each tool reported them (Reticle's LUTs are all `LUT6`
sites; Vivado's are its `LUT1`–`LUT6` cells):

| Design | LUTs, Reticle / Vivado | FFs | CARRY4 | Otherwise | Reticle routed |
|---|---|---|---|---|---|
| `v_alu/alu` | 156 / 124 | 10 / 19 | 8 / 13 | Reticle 7 `MUXF7` | yes |
| `v_counter/counter` | 11 / 6 | 4 / 4 | 1 / 0 |  | yes |
| `v_fsm/det1011` | 2 / 1 | 2 / 2 | 0 / 0 |  | yes |
| `v_ram_b/ram_bw` | 5 / 12 | 32 / 0 | 0 / 0 | Reticle 32 `RAM32X1D`, Vivado 1 `RAMB18E1` | yes |
| `v_shift/shreg` | 9 / 10 | 8 / 8 | 0 / 0 |  | yes |
| `v_shift/lfsr` | 1 / 1 | 16 / 32 | 0 / 0 |  | yes |
| `sv_axis_if_b/axis_inc` | 2 / 7 | 10 / 10 | 2 / 0 |  | yes |
| `sv_counter_gen/chain` | 8 / 4 | 6 / 6 | 0 / 0 |  | yes |
| `sv_counter_gen/up_counter` | 6 / 3 | 4 / 4 | 1 / 0 |  | yes |
| `sv_fifo_b/fifo` | 20 / 16 | 10 / 10 | 1 / 0 | Reticle 2 `RAM32X1D` and 1 `RAM32M`; Vivado 10 `RAMD32`, 2 `RAMS32` | yes |
| `sv_fsm/seq_detect` | 5 / 5 | 3 / 7 | 0 / 0 |  | yes |
| `sv_pkg_alu_b/alu` | 74 / 40 | 10 / 10 | 6 / 7 | Vivado 8 `MUXF7` | no (yes since, with one `CARRY4` more) |
| `sv_struct_pipe_b/pipe` | 3 / 1 | 51 / 50 | 6 / 3 |  | yes |
| `vh_adder/gadd_w8` (built again) | 11 / 9 | 10 / 10 | 9 / 3 |  | no (yes since, with one `CARRY4` more) |
| `vh_adder/gadd_w12` (built again) | 15 / 13 | 14 / 14 | 13 / 4 |  | no (yes since, with one `CARRY4` more) |
| `vh_counter_b/counter` | 6 / 3 | 5 / 5 | 1 / 0 |  | yes |
| `vh_fsm/seqdet` | 4 / 2 | 3 / 3 | 0 / 0 |  | yes |
| `vh_pipeline/pipe` | 165 / 0 | 77 / 3 | 2 / 0 | Vivado 1 `DSP48E1` | no (yes since, with one `CARRY4` more) |
| `vh_ram/ram` | 1 / 6 | 8 / 8 | 0 / 0 | Reticle 2 `RAM32X1D` and 1 `RAM32M`; Vivado 10 `RAMD32`, 2 `RAMS32` | yes |

Read off the table, not measured further: Reticle builds a counter's
increment with a carry chain where Vivado uses lookup tables alone; it put
`v_ram_b`'s 32-word byte-write memory in distributed RAM where Vivado used
a block; it built `vh_pipeline`'s 9-by-8 multiply in fabric where Vivado
used a `DSP48E1`; Vivado encoded `seq_detect`'s states one-hot (7
flip-flops to 3). The `gadd` rows are from the build with this branch's
binary; the first build, with the slice read as zeros, had two flip-flops
where the design has ten and fourteen.
