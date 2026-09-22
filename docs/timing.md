# Timing and clock domain crossings

`reticle::timing` (Cargo feature `timing`) holds two analyses over a
module in cell form: **static timing analysis**, which says how much
slack each end point has, and **clock domain crossing analysis**, which
says where data moves between clocks and whether anything makes that
safe. Both work on the same view of the netlist and are meant to be read
together: a crossing the CDC report calls a synchroniser is exactly the
path the timing report should have been told to ignore.

The feature is empty (`timing = []`): the graph, the analyses and the
unit and FPGA delay models need nothing but the IR. With `asic` also on
there is a Liberty delay model; with `fpga` also on, clocks and path
exceptions can be read straight out of a `Constraints` value.

## The timing graph

`timing::graph::TimingGraph::build` turns a module into a graph whose
nodes are **pins** — a cell's input pins, a cell's output pins, the
module's ports, and the two sides of a continuous assignment — and whose
edges are of exactly two kinds:

| Edge     | From              | To                | Cost                 |
|----------|-------------------|-------------------|----------------------|
| cell arc | a cell input pin  | a cell output pin | delay through the cell |
| net arc  | the pin driving a net | each pin loading it | interconnect delay |

Sequential cells break the graph. A `dff` has a `clk` to `q` arc and no
`d` to `q` arc, so no path crosses it:

- **start points** are input ports and the outputs of sequential cells;
- **end points** are output ports and the data pins of sequential cells
  (`d`, `en`, and a synchronous `rst`), each checked against that cell's
  own clock pin;
- **clock pins** are where the clock network stops and data begins.

A black box gets the same treatment by default — its outputs start paths
and its inputs end them — but a delay model may describe it instead
(`DelayModel::cell_topology`), which is what lets a standard-cell netlist
of black boxes named after library cells time end to end.

Arrival times are propagated in topological order, so a combinational
loop has no answer. `TimingGraph::loops` lists every cycle the sort could
not place, and those pins simply get no arrival time; nothing spins.

A hierarchical design is flattened first with
`timing::graph::flatten_for_timing`, which works on a copy. A module that
still holds instances is analysed anyway, with the instances as black-box
boundaries, and the report says so.

## Delay models

`DelayModel` answers four questions and knows nothing about the graph
that asks them: the delay of a cell arc, the delay of a net arc, the
capacitance an input pin presents, and the setup and hold of a sequential
data pin. Everything is a `Transition`, a pair of numbers for the rising
and the falling output edge, and every arc carries a `Sense` saying which
input edge produces which output edge.

| Model          | Feature | What it does                                                |
|----------------|---------|-------------------------------------------------------------|
| `UnitModel`    | —       | one unit per cell arc, nothing per net; the arithmetic is checkable by hand |
| `LibertyModel` | `asic`  | `cell_rise` / `cell_fall` / `rise_transition` / `fall_transition` through `LutTable::lookup`, with input transition on one axis and output capacitance on the other |
| `FpgaModel`    | —       | a table of per-primitive numbers keyed by primitive name, falling back to `UnitModel` |

The Liberty model works out which table axis is the transition and which
is the capacitance from the table's template, clamps both to the
characterised range rather than extrapolating out of it, and accumulates
each net's load from the `capacitance` of every pin it drives. A cell,
pin or arc the library does not describe falls through to the wrapped
unit model, so a partly mapped netlist still gets a report.

**The numbers in `FpgaModel::placeholders` are placeholders.** They are
not from a vendor datasheet or from the open timing databases; they are
ordered sensibly relative to each other, so a report built on them ranks
paths the way a real one would, but the absolute slack means nothing.
Install real ones with `FpgaModel::with_primitive`.

## The analysis

```rust
use reticle::timing::delay::UnitModel;
use reticle::timing::sta::{ClockSpec, TimingOptions, TimingSpec, analyze_with};

let spec = TimingSpec::new().with_clock(ClockSpec::new("sys", "clk", 10.0));
let report = analyze_with(&module, &TimingOptions::default(), &UnitModel::new(), &spec);
println!("{}", report.render());
```

With the `fpga` feature, `analyze(&module, &options, &model, &constraints)`
is the same thing with the spec read out of a `Constraints` value:
`create_clock` becomes a `ClockSpec`, `set_false_path` and
`set_multicycle_path` become `PathException`s.

What happens, in order:

1. Every arc is annotated by the delay model, in topological order, so
   each arc is asked with the transition time the previous stage
   produced.
2. The **clock network** is propagated from each clock source through
   combinational arcs until it reaches a sequential cell's clock pin. In
   `ClockMode::Ideal` the resulting latency is discarded; in
   `ClockMode::Propagated` it is kept, and the difference between the
   launch and the capture flop's latency is the clock skew.
3. **Arrival times** are propagated forwards from the start points: input
   ports at their input delay, and clock pins at their clock latency,
   from where the clock-to-output arc carries them to `q`.
4. **Required times** are propagated backwards from the end points. Both
   are kept *relative to the launching clock edge*, so a required time is
   an amount of budget rather than an instant, and the slack at any pin
   is `required - arrival`.
5. **Paths** are enumerated backwards from each end point with a
   best-first search whose priority is `arrival + suffix`, an exact bound
   on a directed acyclic graph, so the N worst paths come out in order.

Rise and fall are kept apart throughout. An arrival, a required time and
an arc delay are all `Transition`s, and an inverter's negative sense
means a rising output takes the *falling* arrival of its input.

### Clocks, setup and hold

A clock is a period and a waveform. For a check between two clocks the
launch and capture edges are the closest pair of active edges with the
capture edge after the launch edge, searched over enough periods to cover
their relationship; for one clock that is `launch = 0`,
`capture = period`. The hold check uses the same launch edge against the
capture edge one capture period earlier.

- `set_false_path -from A -to B` removes those paths. The analysis
  applies exceptions as paths are *completed*, so a false path skips to
  the next worst path at that end point rather than making the end point
  disappear — unless every path into it is cut, in which case it really
  does disappear.
- `set_multicycle_path N -from A -to B` moves the setup capture edge
  `N - 1` periods later and leaves the hold check alone.
  `set_multicycle_path N -hold` moves the hold capture edge `N` periods
  earlier.

A clock pin no `create_clock` reaches gets a clock of its own, named
after its net and with `TimingOptions::default_period`, so its flops are
still checked; the report says it was inferred.

### The report

`TimingReport::render_summary` is the clocks and the worst slack per
clock group; `render` is that plus each reported path in detail, in the
shape a real timing report has — an ordered list of pins with the
incremental and the cumulative delay, then the arrival, the capture edge,
the setup or hold time and the slack, each contributing to the column so
the arithmetic can be followed down the page. `render_sources` adds
`file:line:col` from the IR's spans. `TimingReport::pins` has the raw
arrival and required times per pin for callers that want the propagation
rather than the paths.

Output is deterministic: paths are sorted by slack then by check, end
point, start point and length, groups by clock pair, and nothing is
iterated out of a hash map.

## Clock domain crossings

`analyze_cdc_with(&module, &spec)` (or `analyze_cdc(&module, &constraints)`
with `fpga`) assigns every sequential cell to a domain by tracing its
clock pin back through buffers and assignments to a root net, names the
domain after the `create_clock` on that net, and then looks for every
flip-flop that captures data launched in another domain.

The important part of the report is which findings are **proved** and
which are **guesses it is honest about**.

| Finding | Severity | Proved? |
|---------|----------|---------|
| `Unsynchronised` — a crossing with no synchroniser | error | yes, structural |
| `Synchroniser { depth }` — a chain of two or more flops with no logic between them and no other fanout on the intermediate flop | note | yes, structural |
| `AsyncFifo` — a memory written in one domain and read in another | error, or warning with synchronised buses both ways | the shape, yes; the gray pointers, no |
| `GrayBus { bits, generator_found }` — several bits crossing together | warning | **no** |
| `Handshake` — synchronised crossings both ways between one pair of domains | note | **no** |
| `Reconvergence` — two synchronisers from one domain recombined by one cone of logic | warning | yes, structural |

Whether a bus is gray coded is a property of the *values* it takes, not
of its structure, and nothing in a netlist says so. The analysis does
look for the usual generator — an XOR of a value with a shifted copy of
itself, seen through wiring and shifts — and reports whether it found
one, but the finding stays unverified either way. Proving that at most
one bit changes per source clock is a job for `reticle::formal`.

Reconvergence is reported at the cell where the synchronised bits *first*
meet, not at every cell downstream: a cell counts only if no single one of
its input pins already carries two of them.

## What is not modelled

A timing report that hides its assumptions is worse than none.

- **Wire RC.** A net is a lumped capacitance and one flat interconnect
  delay. No resistance, no distributed tree, no Elmore delay, and no
  difference between a near and a far load on the same net.
- **On-chip variation** beyond two flat derating factors
  (`early_derate`, `late_derate`). No distance-dependent derating, no
  common-path pessimism removal, no statistical timing.
- **Crosstalk.** No coupling capacitance, no aggressor / victim
  analysis, no delta delay.
- **Design rules.** Maximum transition, capacitance, fanout and minimum
  pulse width are not checked.
- **Recovery and removal** on asynchronous resets, **clock gating
  checks**, and **latch time borrowing**: a latch is transparent with a
  setup check on its data pin.
- **Generated clocks.** A clock divided by a flip-flop is not recognised
  as derived; give it its own `create_clock` or it becomes a domain named
  after its net.
- **`when`-conditional Liberty arcs**, which are merged pessimistically
  rather than evaluated: safe for setup, possibly optimistic for hold.
- On the CDC side: **asynchronous resets crossing domains**, clock
  gating, latch-based synchronisers, and protocols where the destination
  is qualified by an enable the analysis cannot see is safe. Storage also
  has to be in the IR's own primitives: a netlist already mapped to
  library cells keeps its flip-flops in black boxes, so run the crossing
  analysis before technology mapping. The report says so when it meets a
  module in that state.

## Test data

`testdata/timing/<name>.rtl` (plus an optional `<name>.rcf` and
`<name>.lib`) with `<name>.timing` and `<name>.cdc` expectations, driven
by `tests/timing_report.rs`; `UPDATE_EXPECT=1` rewrites them. The cases
cover a register-to-register path whose slack is worked out by hand in
the test, two clocks, a false path, a multicycle path, a combinational
loop, a standard-cell netlist against a Liberty library, an
unsynchronised crossing, a correct two-flop synchroniser, a reconvergence
bug and an asynchronous FIFO. Where a number is checkable by hand the
test asserts it as well as snapshotting the report, so a golden file
cannot drift into being wrong but stable.
