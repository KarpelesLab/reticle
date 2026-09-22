# Simulation

`reticle::sim` is an event-driven, 4-state simulator over the IR's process
form. It flattens a hierarchical `Design` into an instance tree and runs it
under one scheduler that implements both the Verilog stratified event queue
and the VHDL delta cycle. It is sans-I/O: `$display` text, VCD data and
coverage come back as values, and `$readmemh` reads through a
caller-supplied `FileProvider`.

The API reference (`cargo doc --features sim`) has the per-item details; the
module docs of `sim`, `sim::assertion`, `sim::coverage` and
`sim::interactive` explain the designs. This document is the map.

## The model in one paragraph

Time is a `u64` count of the finest `timescale` precision in the design
(1 ps when none is given). One *time slot* runs the active, inactive and NBA
regions to quiescence, then the concurrent assertions, then the monitor
region (`$strobe`, `$monitor`). One pass through active / inactive / NBA is
exactly one VHDL delta cycle, so both languages share one loop; the only
per-language choice is whether the front end lowered an assignment as
blocking or non-blocking. Processes are run by an interpreter with an
explicit frame stack, so a `wait` saves a continuation without OS threads.

## Driving a simulation from Rust

```rust
let mut sim = Simulator::new(&design, SimOptions::default())?;
let clk = sim.net("top.clk").unwrap();
let q = sim.net("top.q").unwrap();
sim.set(clk, Logic::from_bool(true));
sim.run_for(sim.ticks(Delay::new(5, TimeUnit::Ns)));
assert_eq!(sim.get(q).to_u64(), Some(1));
```

`set` writes a net as a process would; `force` overrides it until `release`.
`run_until`, `run_for`, `step` and `run` advance time and return when the
target is reached, `$finish` runs, `$stop` pauses or a breakpoint fires.

## Waveforms

`enable_vcd` starts a VCD capture that `vcd` returns as text and
`disable_vcd` ends; `enable_fst` starts the binary equivalent, written by
`dump_fst` in GTKWave's FST format. Both record exactly the changes the
propagation path sees.

## Assertions

Immediate assertions (`assert (expr)`, VHDL `assert ... report`) are IR
statements and need nothing extra. *Concurrent* assertions are added to a
running simulation:

```rust
let id = sim.add_assertion_text(
    "no_push_when_full: assert property (@(posedge clk) full |-> !push);",
    span,
)?;
sim.run();
assert_eq!(sim.assertion_results()[id.index()].failures, 0);
```

A property is compiled to an automaton over boolean predicates: each
sequence becomes an NFA whose transitions consume one clock cycle, and the
property-level operators that an automaton cannot express (`not`, `and`,
`or`, `|->`, `|=>`) sit in a small tree above it. One attempt starts at
every clocking event and all live attempts step in lockstep, so
`a |-> ##2 b` restarted every cycle tracks several attempts at once and each
failure names its own start time and cycle.

Values are *sampled*: a cycle reads the values the nets had at the start of
the time slot the clock edge occurred in (the preponed region of IEEE 1800
§4.4). A boolean that evaluates to `x` or `z` is not a match.

### The supported subset

| Layer      | Supported |
|------------|-----------|
| Booleans   | `!` `~` `&&` `\|\|` `->`, `==` `!=` `===` `!==` `<` `<=` `>` `>=`, bit and part selects, literals, `$rose` `$fell` `$stable` |
| Sequences  | `##n` `##[m:n]` `##[m:$]` `##[*]` `##[+]` (including the `##0` fusion), `[*n]` `[*m:n]` `[*m:$]`, `[->n]` `[->m:n]`, `and`, `or`, `intersect`, `throughout`, `within`, PSL SERE braces `{a; b}` and `{a : b}` |
| Properties | `\|->`, `\|=>`, `not`, `and`, `or`, PSL `always` and `never`, `@(posedge clk)` / `@(negedge clk)` / `@(clk)`, `disable iff (expr)` |
| Directives | `assert`, `assume`, `cover`, with an optional `label:` |

Not supported, and rejected with a diagnostic: `[=n]`, `first_match`,
`s_eventually`, `eventually`, `until` and its variants, `nexttime`,
`accept_on` / `reject_on`, `restrict`; `$past` and the other sampled value
functions; sequence and property *declarations* with formal arguments or
local variables; sequence method calls (`.triggered`, `.matched`);
multi-clocked properties; empty matches (`[*0]`); action blocks
(`else $error(...)`); `expect` and deferred assertions.

A failing `assert` is an error `Diagnostic` and a failing `assume` a
warning; both carry the property's span, the attempt's start time and
cycle, the cycle the failure happened in, and the sampled value of every net
the property reads. A `cover` reports nothing and counts its matches. No
concurrent assertion ends the run.

The Verilog front end keeps property text as `RawTokens` rather than
parsing it. `sim::assertion::property::from_assertion` is the bridge from a
parsed `ast::Assertion` to a directive, so the simulator gained SVA without
the front end changing.

## Coverage

`SimOptions::coverage` turns on collection; with it off nothing is
allocated and nothing is recorded.

- **Line coverage** counts executions per statement, keyed by span. Every
  statement of every instantiated module starts at zero, so the report
  tells "never ran" from "not in the design".
- **Toggle coverage** records, per net bit, whether it was seen going `0`
  to `1` and whether it was seen going `1` to `0`. Transitions through `x`
  or `z` are not toggles.

`Simulator::coverage()` returns a `CoverageReport`, which renders two ways.
Both take the `SourceMap` the design was parsed from, since the simulator
holds spans, not line numbers:

- `render(&map)` — percentages per file and per module, then the uncovered
  lines and the untoggled bits.
- `to_lcov(&map)` — the LCOV `.info` format every coverage viewer reads.
  Statements become `DA` records; toggles become `BRDA` branch records, two
  per net bit (branch `2 * bit` for the rise, `2 * bit + 1` for the fall)
  on the line the net is declared on.

Both outputs are sorted, so a golden file compares byte for byte.

## Interactive mode

`sim::interactive::Session` is a command interpreter over a `Simulator`.
It is sans-I/O like everything else: `Session::execute(&str)` returns a
`Response` (the lines to show) or a `SessionError`, and
`Session::complete(&str)` supplies tab candidates. The CLI is a loop around
it.

| Command                  | What it does                              |
|--------------------------|-------------------------------------------|
| `run [time]`             | run for `time`, or to the end             |
| `step`                   | run one time slot                         |
| `continue`               | run to the end, a breakpoint or `$stop`   |
| `break <net> [value]`    | stop when the net changes (to `value`)    |
| `delete [id]`            | remove one breakpoint, or all of them     |
| `force <net> <value>`    | override a net                            |
| `release <net>`          | drop the override                         |
| `print <net>`            | show a net's value                        |
| `watch <net>`            | report every change of a net              |
| `dump on` / `dump off`   | start or stop VCD capture                 |
| `memory <name>[<index>]` | show memory contents                      |
| `scope [path]`           | show or set the scope names resolve in    |
| `where`                  | scope, module, time and run state         |
| `time`                   | the current time in ticks                 |
| `help`                   | the command list                          |
| `quit`                   | end the session                           |

Times are ticks of the simulation precision or carry a unit (`100ns`);
values are Verilog literals (`1'b1`, `8'hff`, `42`); net names are
hierarchical or relative to the current scope.

Breakpoints belong to the simulator (`Simulator::add_breakpoint`), not to
the session: they are checked where a change is propagated and stop the run
at the *end* of that time slot, so the design has settled when the prompt
comes back.

## Compiled fast mode

`sim::compiled` is the other trade for the same designs. Where the event
simulator is general — 4-state, delays, a queue run to quiescence — the
compiled engine assumes the design is *synchronous* and *two-state safe*,
proves it, and lowers it once to a flat list of operations over a register
file of `u64` words. A cycle is then three straight-line passes and no
queue at all.

```rust
use reticle::sim::compiled::{CompileOptions, CompiledSim};

let mut sim = CompiledSim::new(&design, CompileOptions::default())?;
let rst = sim.net("counter.rst").unwrap();
let q = sim.net("counter.q").unwrap();
sim.set(rst, Logic::from_bool(true));
sim.step();                 // one clock edge
sim.set(rst, Logic::from_bool(false));
sim.run_cycles(20);
assert_eq!(sim.get(q).to_u64(), Some(20));
```

`CompiledSim` keeps the event simulator's names and handles — it *is* built
on the same elaboration — so `net`, `memory`, `nets`, `memories`,
`net_name`, `get`, `set`, `get_mem`, `set_mem`, `mem_len`, `output`,
`messages`, `status` and `finished` mean exactly what they do there.
`run_cycles(n)` replaces `run_for(ticks)`, `step()` is one clock edge
rather than one time slot, `time()` counts cycles, and `get` takes
`&mut self` because it settles the combinational region first if an input
changed.

### One cycle

1. **Settle** — run the combinational operations in topological order. One
   pass, because the order was computed at compile time.
2. **Edge** — compute the next value of every register, the memory writes
   and the surviving side effects. Nothing is visible yet.
3. **Commit** — make the next state current in one `copy_within` and apply
   the memory writes. Committing all at once is what makes a cycle-based
   run agree with non-blocking semantics without modelling them.

An asynchronous reset is the exception that is applied *before* the settle,
since that is where it happens in the event simulator; its condition and
value therefore have to come from an input or a register, not from
combinational logic.

### Eligibility

`compiled::check(&design, options)` returns a `Plan` or one `Ineligible`
per obstacle, each naming the object and its span — "it was slow and I do
not know why" is the worst outcome a fast mode can have. A design has to
be:

| Requirement | What breaks it |
|-------------|----------------|
| every process combinational, clocked or a time-zero initialiser | a `free` process, a `wait`, a delayed assignment |
| one clock, driven from outside | two clock nets, both edges of one net, a clock the design generates |
| no level-sensitive storage | a `dlatch`, or a combinational unit that leaves a bit unassigned on some path |
| one driver per bit | two drivers of the same bit, `tristate`, an unresolved black box |
| no combinational loop | a cycle among the combinational units |
| no `x` or `z` that matters | a literal with unknown bits, a state element still unknown after time zero |

`CompileOptions::zero_init` answers the last one for registers and
memories that nothing initialises: without it `check` refuses rather than
guessing.

### What it does not do

No time (`time()` is cycles; `$time` and the other system functions are
refused at check time rather than returning something wrong), no `x` or
`z`, no wire resolution, no concurrent assertions, no coverage, and **no
waveform** — there is no VCD or FST from compiled mode, and a run that
needs one belongs on the event simulator. Three places where the 4-state
simulator yields `x` have no two-state answer and resolve as zero: a
select outside its operand, division or remainder by zero, and a `Pmux`
with several select bits set (which takes the lowest). Immediate
assertions, `$display`, `$write`, `$finish` and `$stop` do survive, and
`$display` is formatted by the same code the event simulator uses.

### Measured speed-up

`cargo test --release --features sim,verilog --test sim_compiled --
--ignored --nocapture` prints this table; the numbers below are from one
machine (a 2024 x86-64 laptop, `--release`, `lto = "thin"`), so read the
ratios, not the absolutes. Both loops include the cost of driving the
inputs through `Logic`, which a real testbench also pays.

| Design | Program | Event sim | Compiled | Speed-up |
|--------|---------|-----------|----------|----------|
| `testdata/synth/counter_en.rtl` | 0 comb + 12 edge ops, 2 regs / 9 bits | 2.06 M cycles/s | 12.32 M cycles/s | 6.0x |
| `testdata/synth/fsm.rtl` | 1 comb + 26 edge ops, 1 reg / 2 bits | 2.19 M cycles/s | 7.37 M cycles/s | 3.4x |
| `testdata/synth/adder_tree.rtl` | 12 comb ops, no state | 0.43 M cycles/s | 18.05 M cycles/s | 42.4x |
| `testdata/synth/ram_regread.rtl` | 2 comb + 4 edge ops, 1 reg / 8 bits, 16×8 RAM | 1.56 M cycles/s | 10.65 M cycles/s | 6.8x |
| `testdata/synth/mux_tree.rtl` | 13 comb ops, no state | 2.19 M cycles/s | 7.92 M cycles/s | 3.6x |
| `testdata/synth/counter_en.cells.rtl` | 2 comb + 6 edge ops, 2 regs / 9 bits | 0.59 M cycles/s | 5.21 M cycles/s | 8.8x |
| `ip/uart_tx` | 7 comb + 40 edge ops, 4 regs / 31 bits | 1.21 M cycles/s | 4.31 M cycles/s | 3.6x |
| `ip/uart` | 16 comb + 126 edge ops, 13 regs / 72 bits | 0.42 M cycles/s | 1.74 M cycles/s | 4.2x |
| `ip/fifo_sync` | 17 comb + 21 edge ops, 3 regs / 18 bits, 16×8 RAM | 0.58 M cycles/s | 5.55 M cycles/s | 9.5x |
| `ip/spi_master` | 17 comb + 90 edge ops, 10 regs / 53 bits | 0.54 M cycles/s | 2.21 M cycles/s | 4.1x |
| `ip/i2c_master` | 20 comb + 222 edge ops, 14 regs / 41 bits | 0.40 M cycles/s | 1.11 M cycles/s | 2.8x |
| `ip/axil_gpio` | 25 comb + 97 edge ops, 13 regs / 148 bits | 0.31 M cycles/s | 1.80 M cycles/s | 5.8x |

So: three to ten times on real blocks, and forty on a design that is
nothing but combinational logic. Where the rest of the time goes is worth
saying plainly, because 3x is not the order of magnitude a compiled
simulator is supposed to be worth:

- **These designs are tiny.** The event simulator's per-cycle cost is
  dominated by scheduling, which is roughly constant; the compiled
  engine's is proportional to the program. On a 20-operation block the
  constant is most of the event simulator's time and the ratio is large
  (`adder_tree`); on a 200-operation block the ratio shrinks
  (`i2c_master`). The advantage would grow again with design size, because
  the event simulator also re-evaluates a net every time one of its inputs
  changes while the compiled engine evaluates it exactly once — but that
  needs a design big enough to measure, and the corpus has none yet.
- **Driving the inputs is in the loop.** Every `set` builds a `Logic`,
  which allocates. Both engines pay it, so it understates the ratio rather
  than inflating it, but on a block with a handful of narrow inputs it is
  a real share of the compiled engine's time.
- **The lowering is not an optimiser.** A `case` becomes a chain of
  guarded merges, one `Mux` per arm per assigned signal, which is why
  `i2c_master` needs 222 operations per edge for 41 bits of state.
  Constant folding and common subexpression elimination run, but nothing
  else: no dead-value elimination, no mux-tree flattening, no
  specialisation of a whole word-wide operation into one machine
  instruction beyond the one-word fast path. Those are the next thing to
  do if the number has to be bigger.
- **It is an interpreter.** Each operation costs a `match` and two
  bounds-checked slice indexings. Generating Rust or a threaded dispatch
  table would remove that, and would be the step after the optimiser.

## Tests

- Unit tests next to the code: the scheduler, the process interpreter, the
  sequence automaton (every operator, including overlapping attempts),
  coverage accounting and every session command.
- `tests/sim_rtl.rs` drives `testdata/sim/*.rtl` from a `.expect` script;
  its command list is in the file's module docs and covers assertions
  (`assert`, `expect-assert`) and coverage (`coverage`, `expect-coverage`,
  `expect-lcov`) as well as nets, memories, output and VCD.
- `tests/sim_interactive.rs` replays the transcripts in
  `testdata/sim/interactive/`.
- `tests/sim_assert.rs` covers the Verilog `RawTokens` bridge and
  overlapping attempts end to end.
- `tests/sim_cosim.rs` and `tests/sim_fst.rs` cover the Rust API and the
  FST writer and reader.
- `tests/sim_compiled.rs` is the compiled engine's real deliverable: it
  runs the *same* design through both engines for thousands of seeded
  random input vectors, comparing every net and every memory element
  before and after every clock edge, over every eligible module of
  `testdata/sim/`, `testdata/synth/` and the `ip/` library. An `x` on the
  event side counts as a divergence, not an excuse. It also keeps
  `testdata/sim/compiled_eligibility.txt`, a golden census of which
  corpus module qualifies and, for the rest, exactly why not.

## Not yet here

Inertial delay on continuous assignments (transport delay is used), and a
waveform from compiled fast mode.
