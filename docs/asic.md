# The ASIC flow

`reticle::asic` (Cargo feature `asic`) takes a design from RTL to a
netlist of a real standard-cell library, reports its area and its
critical path, and writes the files a place-and-route tool is handed.
The library is the PDK's: Reticle reads Liberty, LEF and DEF and owns
none of them.

```
   .v / .vhd                    <pdk>.lib            <pdk>.lef
       │                            │                    │
   frontends                 liberty::Library        lef::Lef
       │                            │                    │
       ▼                            ▼                    │
      ir  ──► synth::run ──► library::StdCells           │
                  │               │                      │
                  ▼               ▼                      │
            flow::synthesize_asic (map, flops, drive)    │
                  │                                      │
     ┌────────────┼──────────────────┬───────────────────┘
     ▼            ▼                  ▼
  AsicReport   sdc::write_sdc    openroad::export_openroad
  (area,       (constraints)     (netlist, SDC, Tcl, DEF)
   timing)                                │
                                          ▼
                                       OpenROAD
```

| Module | What it does |
|--------|--------------|
| `asic::liberty` | reads `.lib`: cells, pins, functions, timing tables |
| `asic::lef` / `asic::def` | reads and writes the physical formats |
| `asic::library` | Liberty to a mapper gate library, and flip-flop matching |
| `asic::flow` | the whole flow, area and timing in one report |
| `asic::sdc` | the constraints, written as SDC |
| `asic::openroad` | the hand-off: netlist, SDC, Tcl script, DEF |

## What you must point it at

Reticle ships **no** process data. To map a design you supply, from a
PDK:

1. a **Liberty** file for the corner you want — the typical corner for a
   first look, `sky130_fd_sc_hd__tt_025C_1v80.lib` or
   `sg13g2_stdcell_typ_1p20V_25C.lib`. This is the only file synthesis
   needs;
2. a **technology LEF** and a **cell LEF** (open PDKs often ship them
   merged into one file), for the OpenROAD hand-off;
3. the names of a few cells the back end wants by name: the clock buffers
   for CTS and the filler cells, in `OpenRoadOptions`.

Nothing else is configured. The same code runs against SKY130, IHP
SG13G2, Nangate45 / FreePDK45 and GF180MCU unchanged; the reader was
shaped after those files' quirks (bus pins with a `type` group, bundles,
`\` line continuations, `when`-conditional arcs, `leakage_power` groups).

```rust
# #[cfg(all(feature = "asic", feature = "timing"))]
# fn go(design: &mut reticle::ir::Design, top: reticle::ir::ModuleId) {
use reticle::asic::flow::{AsicOptions, synthesize_asic};
use reticle::asic::liberty::Library;
use reticle::asic::sdc::{AsicConstraints, Clock};
use reticle::diag::Diagnostics;
use reticle::source::SourceMap;

let mut sources = SourceMap::new();
let text = std::fs::read_to_string("sky130_fd_sc_hd__tt_025C_1v80.lib").unwrap();
let file = sources.add("sky130.lib", text.clone()).unwrap();
let mut diags = Diagnostics::new();
let library = Library::parse(&text, file, &mut diags).unwrap();

let options = AsicOptions {
    constraints: AsicConstraints::new().with_clock(Clock::new("sys", "clk", 10.0)),
    ..AsicOptions::new()
};
let report = synthesize_asic(design, top, &library, &options, &mut diags).unwrap();
println!("{}", report.to_text());
# }
```

## From Liberty to a gate library

`library::StdCells::from_library` is the translation between the two
vocabularies. A cell reaches the technology mapper when it is not
`dont_use`, has exactly one output with a `function` and no
`three_state`, mentions nothing but its own input pins, has an `area`,
and has at most four inputs (what the mapper's Boolean matching covers).
Its function becomes a truth table over the input pins in declaration
order.

Everything else is *reported*, not silently dropped:
`StdCells::skipped()` lists each cell with its reason — a physical-only
fill or tap cell, a tri-state buffer, a multi-output full adder, a wide
multiplexer past the width limit, a `dont_use` cell, a cell with no
area. "The library has 400 cells and the mapper used 19" has to be an
answerable question.

### Which delay

A Liberty cell has no single delay: it has a table per arc, per output
edge, sometimes per `when` condition, indexed by input transition and
output load. The mapper's cost function needs one number per input pin,
and the one taken is **the worst of `cell_rise` and `cell_fall` over
every delay arc from that pin to the output, at a nominal input slew and
output load**. Worst rather than typical because the mapper is
minimising a critical path; over every arc, `when`-conditional ones
included, because picking one condition would be a guess about the data.
Constraint arcs (setup, hold, recovery, removal) and the three-state
enable and disable arcs are not delays and are not considered.

The nominal point comes from the library itself, so two libraries are
compared on the same footing: the **load** is four times the input
capacitance of the library's smallest inverter (the usual FO4 point) and
the **slew** is what that FO4 inverter's own transition tables give.
Both can be overridden in `LibraryOptions`.

These numbers only order the mapper's choices. The timing in the report
is a full non-linear lookup through `timing::delay::LibertyModel`, not
this one number.

## Flip-flops

Sequential cells are matched by *features*, not by function.
`StdCells::plan_flop` takes what the IR inferred — clock edge, clock
enable, reset kind, polarity and value — and answers with the library
cell, which pin each signal goes to, and what needs an inverter.

The rules, and why:

- **A missing polarity gets an inverter.** Open PDKs ship active-low
  resets only (`sky130_fd_sc_hd__dfrtp_1` has `RESET_B`), so refusing an
  active-high reset would refuse most designs. This is exactly the gap
  the FPGA side leaves open, where an iCE40 flip-flop with an active-low
  reset is reported instead of being built; the ASIC side inserts the
  inverter and says so in the report (`inverters_inserted`). One
  inverter is shared by every bit of a register and by every register
  driven from the same net.
- **A pin the flip-flop does not use is tied off**, so a library whose
  only flip-flop has a reset can still hold a design that has none.
- **A clock enable the library has no cell for becomes a feedback
  multiplexer** on `d` (`en ? d : q`).
- **A synchronous reset always becomes a multiplexer** on `d`: Liberty
  hides a synchronous reset inside `next_state` and no open PDK ships
  one as a pin. The reset multiplexer wraps the enable one, which is the
  priority the IR's `dff` has.
- **A clock of the wrong edge is not fixed with an inverter.** Inverting
  a clock creates a second clock network with its own skew and
  duty-cycle distortion; that is a physical-design decision, not a
  mapper's. A negative-edge flip-flop with no negative-edge cell is
  reported (`A0301`) and left generic.

Both rewrites happen *before* technology mapping, so the multiplexers
are mapped into gates with the rest of the logic. A `next_state` of the
form `(D & E) + (IQ & !E)` is recognised as a clock enable by evaluating
it, so every way of writing that function works.

## The flow, step by step

`flow::synthesize_asic` runs:

1. **Generic synthesis** (`synth::run`): processes to cells, inference,
   optimisation.
2. **Flip-flop legalisation**: enables and synchronous resets into the
   data path.
3. **Standard-cell mapping** (`synth::techmap::map_module`) over the
   gate library.
4. **Clean-up** (`synth::opt::Dce`).
5. **Flip-flop mapping**: one library cell per bit, with the inverters
   and tie-offs the plan asked for.
6. **Drive strength** (optional): a cell driving more capacitance than
   its `max_capacitance` allows is swapped for a larger variant of the
   same cell. Variants are found by function, not by name, so `X1`/`X4`
   and `_1`/`_4` conventions both work. The pass repeats, because
   upsizing a cell loads its own driver.
7. **Timing** (with the `timing` feature): `timing::analyze_with` and
   `LibertyModel` over the mapped netlist, with the clocks and
   exceptions from the constraints.

So one call gives area *and* timing, which is the pair of numbers a
first look at a block is about.

### What the netlist holds

Every mapped cell is a black box named exactly as the library spells it,
carrying a `lib_cell` attribute, with the library's own pin names — even
when the cell is exactly an IR primitive, because a place-and-route tool
wants an instance of a library cell and nothing else.

That makes the netlist opaque to anything that wants to *evaluate* it.
`flow::logic_model` is the way back: it rewrites each standard cell into
the generic IR cell computing the same function (a `Lut` for a gate, a
`Dff` for a flip-flop), which is what makes

```text
check_equivalent(design, before_mapping, logic_model(after_mapping))
```

a proof that mapping did not change behaviour. The test suite runs that
over every mapped design.

### What the flow does not do

- **No buffer insertion.** The drive-strength pass only swaps a cell for
  a stronger variant of itself. A net no drive strength can carry is
  reported (`A0302`) and left to the physical flow, which has to do it
  anyway once it knows the wire load; OpenROAD's `repair_design` is in
  the generated script for exactly that, after placement, where the
  numbers are real. Buffering against a wire load of zero is guessing.
- **No clock tree**, no scan insertion, no power intent, no multi-corner
  analysis, no wire RC beyond a flat per-net load.

## Constraints

`sdc::AsicConstraints` is the timing constraints of a block, and
`write_sdc` renders `create_clock`, `set_input_delay`,
`set_output_delay`, `set_load`, `set_driving_cell`,
`set_clock_uncertainty`, `set_false_path` and `set_multicycle_path`.

It deliberately does not reuse `fpga::Constraints`: that type is half
FPGA-physical (pin assignment, IO standards, placement regions, all
checked against a device database) and has no use for `set_load` or
`set_driving_cell`. The overlap is the four timing constructs, and both
render them the same way. `AsicConstraints::from_timing_spec` converts
through `timing::sta::TimingSpec`, which is already the analyser's
feature-independent view of an SDC file, so a design that has an FPGA
constraints value or an `.rcf` file can cross over without the `asic`
feature depending on `fpga`.

## The OpenROAD hand-off

`openroad::export_openroad` returns the bytes and the command line;
Reticle spawns nothing.

| File | What it is |
|------|------------|
| `<top>.v` | the gate-level netlist, structural Verilog |
| `<top>.sdc` | the constraints |
| `<top>.tcl` | the generated script |
| `<top>.def` | the unplaced DEF, when a floorplan is given |

Run it with `openroad -no_init -exit <top>.tcl` from the directory
holding those files. The LEF and the Liberty are *not* generated — they
are the PDK's, and the script reads them from the paths you named.

The script targets the **OpenROAD app** (the `openroad` binary of the
OpenROAD project), command set of the 2.0 series, which is what
OpenROAD-flow-scripts has driven since 2023:

| Stage | Commands |
|-------|----------|
| read | `read_lef`, `read_liberty`, `read_verilog`, `link_design`, `read_sdc` |
| floorplan | `initialize_floorplan` (or `read_def -floorplan_initialize`), `make_tracks`, `place_pins` |
| placement | `set_wire_rc`, `global_placement`, `estimate_parasitics`, `repair_design`, `detailed_placement`, `check_placement` |
| clock tree | `clock_tree_synthesis`, `set_propagated_clock`, `repair_clock_nets` |
| routing | `set_routing_layers`, `global_route`, `detailed_route` |
| finishing | `filler_placement`, `check_placement` |
| reports | `report_design_area`, `report_checks`, `report_worst_slack`, `report_tns`, `report_clock_skew` |
| write | `write_def`, `write_verilog` |

The order is the part that is easy to get wrong: parasitics are
estimated after placement and again after global routing, because
anything before placement is a wire-load guess; `repair_design` runs
after the first estimate so it fixes real violations; the clock tree is
built after placement and before routing, with `set_propagated_clock`
after it so the reports stop pretending the clock is ideal.

`openroad::check_physical` is the last gate before the tool sees the
design: every cell a library cell, every one of them in both the Liberty
and the LEF, every connected pin a pin that macro has. A netlist that
fails it is one OpenROAD would reject with a worse message.

## The reference library

No open PDK is vendored here — they are large and their licences are
their own — so the tests run against `testdata/asic/reticle_sc.lib` and
`reticle_sc.lef`, **a synthetic library whose every number is
invented**. Its header says so at length. Delays come out of a
first-order RC model (`intrinsic + k * slew + R * load`) with
hand-picked constants; areas are a site width times a row height. They
are internally consistent and monotonic and they mean *nothing* about
any process.

What is real about it is its *shape*, which is what the reader and the
mapper are tested against: a units header, `delay_model : table_lookup`,
operating conditions, 2-D template tables, drive variants
(`INV_X1/X2/X4`, `BUF_X1/X2/X4`, `NAND2_X1/X2`, `NOR2_X1/X2`),
combinational cells (`NAND2/3`, `NOR2/3`, `AND2`, `OR2`, `XOR2`,
`XNOR2`, `MUX2`, `AOI21`, `OAI21`), flip-flops written the way a real
library writes them (`DFF_X1`, `DFFN_X1` on the negative edge,
`DFFR_X1` with `clear : "!RN"`, `DFFS_X1` with `preset : "!SN"`,
`DFFE_X1` with `next_state : "(D & E) + (IQ & !E)"`), and cells the
mapper must refuse (a tri-state `EBUFN_X1`, a multi-output `FA_X1`, a
six-input `MUX4_X1`, a `dont_use` `DLY_X1`, `FILL_X1` and `TAP_X1`).

**A real PDK must be supplied for real results.** Point
`Library::parse` at a real `.lib` and everything else is unchanged.

## Test data

`testdata/asic/<name>.rtl` with `<name>.map` (the area and timing
report), `<name>.mapped.rtl` (the netlist in the IR text format),
`<name>.v` (the same netlist as the Verilog OpenROAD reads),
`<name>.sdc`, `<name>.tcl` and `<name>.diag`, plus `reticle_sc.cells`
(the mapper's view of the library), driven by `tests/asic_flow.rs`;
`UPDATE_EXPECT=1` rewrites them. The cases are a combinational ALU, a
one-bit toggle with an active-high asynchronous reset (which forces an
inverter), a counter with a clock enable and a synchronous reset (which
forces the reset into the data path), and a shift register with an
enable and an asynchronous reset (which forces the enable into the data
path instead, since no cell has both).

Three tests are about behaviour rather than bytes:
`mapping_preserves_behaviour` proves each mapped netlist equivalent to
the pre-mapping design through `logic_model` and `formal::check_equivalent`
(with the expected verdict written down per design, as
`tests/synth_verify.rs` does); `exports_are_placeable` runs
`check_physical` over every export; and
`a_heavy_wire_load_upsizes_what_it_can` forces the drive-strength pass
to work. If `openroad`, `yosys` or `sta` happen to be installed, three
more tests run them over the export and say so when they are not.
