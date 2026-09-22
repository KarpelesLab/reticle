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

## Not yet here

The compiled 2-state fast mode, and inertial delay on continuous
assignments (transport delay is used).
