# Simulation

`reticle::sim` is an event-driven, 4-state simulator over the IR's process
form. It flattens a hierarchical `Design` into an instance tree and runs it
under one scheduler that implements both the Verilog stratified event queue
and the VHDL delta cycle. It is sans-I/O: `$display` text, VCD data and
coverage come back as values, `$readmemh` / `$readmemb` read through a
caller-supplied `FileProvider` (the one synthesis reads through too), and
what `$writememh` / `$writememb` save comes back from
`Simulator::written_files`.

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

## What `x` costs

A cell's output should be `x` when the unknown bits of its inputs can change
it, and for no other reason. `sched::lut_output` states that rule exactly:

> the output is the value every input assignment consistent with the unknown
> inputs produces, and `x` when two such assignments disagree.

A lookup table is **not** one of the gate-level primitives IEEE 1364-2005 §7
tabulates, so there is no table to copy — but that rule, applied to the
primitives the standard does define, reproduces their tables: `0 & x` is `0`
and `1 & x` is `x` (§7.2's `and` table), `1 | x` is `1`, `x ^ 0` is `x`, and
a two-to-one multiplexer is the bit-by-bit merge of its two arms that
§5.1.13 gives the conditional operator. So the rule is not an extension of
the standard; it is what the standard already does everywhere it says
anything, written in a form that does not need a table per cell.

It is computed by restriction rather than by search. The known address bits
select a sub-cube of the truth table, which is a word mask over `init`, and
the surviving entries are known to agree when they are all ones or all zeros:
`k` word operations, no allocation and no enumeration, for any `k`. A fully
known address skips all of it and reads one bit of `init`, exactly as before,
so the hot path is unchanged.

**It used to answer `x` as soon as any address bit was `x`**, whether the
table's function read that bit or not, and that was measurably worse than
the cells a mapped netlist replaces. `CellKind::Mux` hands on the input its
select chose, so an unknown on the other arm is harmless; after covering,
that multiplexer is `lut` cells, and one unwritten byte of a packet buffer
used to silence a whole USB device. `tests/sim_lut_x.rs` holds the mapped
netlists that pin it, and `sched.rs`'s own tests check the restriction
arithmetic against an enumeration of the assignments for every three-input
function and every address over `{0, 1, x, z}`.

**What it costs: nothing measurable.** On one x86-64 box with `--release`,
an 8x8 multiplier mapped onto LUT4 (182 lookup tables), 2000 input vectors
through the event simulator, best of five runs, three samples of each build:

| Unknown bits of one operand | Before | After |
|-----------------------------|--------|-------|
| none (the hot path) | 7150–7270 vectors/s | 7230–7620 vectors/s |
| one | 18100–19310 | 18070–19260 |
| four | 19960–21320 | 20930–21260 |
| eight | 21050–21700 | 21490–21760 |

The two builds' spreads overlap at every row, so the restriction is below
this measurement's noise; the `--release` run of
`usb_descriptors_survive_lookup_table_mapping`, which simulates four mapped
USB devices at two lookup-table widths, took 3.71 s both before and after.
(The rows with unknown inputs are *faster* than the hot path because an `x`
that spreads stops nets toggling, which is scheduling the simulator then does
not do — they measure activity, not the rule.)

### Per-cell optimality is not network optimality

A covered netlist can still be more unknown than the logic it came from, and
no cell-by-cell four-state evaluation can fix that. Technology mapping
duplicates and merges cones, so the multiplexer whose select made the unknown
irrelevant may no longer exist as a cell: in `usb_device_fs` mapped onto
LUT4, two separate lookup tables each take the unwritten buffer's bit 7 and
`own_data` among their inputs and each genuinely depends on that bit, and the
dependence cancels only where they **reconverge** several cells later.
Restricting one table cannot see a correlation between two of them.

This was measured, not guessed: forcing that buffer's read to `0x00` and to
`0xFF` makes all eight of
`usb_descriptors_survive_lookup_table_mapping`'s mapped devices answer the
*same, right* descriptor bytes, so the descriptors do not depend on the
buffer at all and the `x` is pure pessimism — and the same run with the
buffer left unwritten still answers nothing. Deciding it would take ternary
analysis across the network (symbolic simulation, or one satisfiability
question per net), which is not what an event-driven simulator does. A
testbench whose design has a memory nothing wrote should **write it**:
`Simulator::memories`, `mem_len` and `set_mem` do that in four lines, and
`CompiledSim`'s `CompileOptions::zero_init` is the same answer for the
compiled engine.

### Where the simulator is still pessimistic, and where it is optimistic

Pessimism is safe and loses coverage; optimism invents a value and hides a
defect. Both are listed because the second is the one to be unhappy about.

| Cell or operator | What `x` does | Verdict |
|------------------|---------------|---------|
| `Not` `Buf` `And` `Or` `Xor`, the reductions | the IEEE §7.2–§7.3 tables, bit by bit | optimal |
| `Eq` `Ne` | `0` when some bit is known on both sides and differs, else `x` | optimal, and *better* than §5.1.8's letter, which says `x` whenever either operand has an `x` |
| `Mux` `Dlatch` | the chosen input; §5.1.13's merge when the select or enable is `x` | optimal |
| `Lut` | the rule above | optimal |
| `Lt` `Le` `Gt` `Ge` | `x` if either operand has any unknown bit | **pessimistic beyond the standard**: §5.1.7 says `x` when the relation is *ambiguous*, and `4'b1xxx > 4'b0111` is not. `Logic::relation` in `src/logic.rs` |
| `Add` `Sub` `Mul` `Div` `Mod`, `<<` `>>` `>>>` | all `x` if any operand bit is unknown | pessimistic, but §5.1 and §5.1.12 **require** it |
| `Pmux` | all `x` if any select bit is unknown | pessimistic, and the same shape of defect the lookup table had — though mostly unimprovable, since any assignment that sets a second select bit is `x` anyway. The one case it loses is a single unknown select bit over known zeros, which should merge like a `Mux`. Not a Verilog primitive; no mapped netlist in the corpus contains one |
| `MemRdPort` | all `x` for an unknown or out-of-range address | pessimistic; the same restriction would decide it when the reachable elements agree. An out-of-range select reading `x` is §5.2.1 |
| `Tristate` | all `x` when the enable is `x` | optimal without strengths; §7.11's `bufif` tables would give a *weak* `0` or `1`, which `Logic` does not model |
| `Dff` enable | an `x` enable **holds** `q` | **optimistic**: it might have loaded, so the answer is §5.1.13's merge of `d` and `q`, the way `Dlatch` already does it |
| `Dff` reset | an `x` reset counts as **inactive** | **optimistic**, same reason |
| `MemWrPort` enable | an `x` enable writes **nothing** | **optimistic**, same reason |
| `MemRdPort` enable (clocked) | an `x` enable **reads** | the opposite convention from `Dff`'s enable; one of the two is wrong |
| `Dff` clock | `x→1` and `0→x` both count as a positive edge | deliberately pessimistic, and what §9.7.1 asks for |

The three optimistic rows are the ones that could let a broken design pass,
and none of them is this round's: they are reported here so the next round
starts from a list rather than from a surprise.

## What the simulator refuses to run

A black box the design **declares** simulates: its outputs stay undriven and
one warning per instance says so, which is what an encrypted IP package or
an `.rtl` `blackbox module` is for. A module **nothing** declares does not.
Elaboration refuses it and names the instantiation, because running it
anyway would fill the box with `x` and let a testbench report a failure of a
design that is in fact fine — which is exactly what happened once, as
`FAIL: LED 1 is lit and no host has configured anything` from a healthy
design whose IP sources had been left off the command line. Reticle has no
library search path, so the fix is always to name the file that defines the
module. `docs/ir.md` has the rest of the line.

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

// The counter has no initial value, so say what uninitialised means.
let options = CompileOptions { zero_init: true, ..CompileOptions::default() };
let mut sim = CompiledSim::new(&design, options)?;
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
| no race between processes | a clocked blocking write another clocked unit reads, or one net a process assigns both blockingly and non-blockingly |
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
`$display` is formatted by the same code the event simulator uses — but
they all run once per `step()`, where the event simulator runs the ones
in a combinational process once per delta cycle the process is triggered
in, which may be more than once or not at all.

### Measured speed-up

`cargo test --release --features sim,verilog --test sim_compiled --
--ignored --nocapture` prints this table. Each timed loop runs five times
and the fastest is kept, because a slow repetition measures the machine
and not the engine; the numbers are still from one x86-64 box with
`--release` and `lto = "thin"`, so read the ratios, not the absolutes.
Both loops include the cost of driving the inputs through `Logic`, which
a real testbench also pays.

| Design | Program | Event sim | Compiled | Speed-up |
|--------|---------|-----------|----------|----------|
| `testdata/synth/counter_en.rtl` | 0 comb + 12 edge ops, 2 regs / 9 bits | 2.12 M cycles/s | 12.26 M cycles/s | 5.8x |
| `testdata/synth/fsm.rtl` | 1 comb + 26 edge ops, 1 reg / 2 bits | 2.39 M cycles/s | 7.01 M cycles/s | 2.9x |
| `testdata/synth/adder_tree.rtl` | 12 comb ops, no state | 0.49 M cycles/s | 20.30 M cycles/s | 41.6x |
| `testdata/synth/ram_regread.rtl` | 2 comb + 4 edge ops, 1 reg / 8 bits, 16x8 RAM | 1.77 M cycles/s | 11.28 M cycles/s | 6.4x |
| `testdata/synth/mux_tree.rtl` | 13 comb ops, no state | 2.47 M cycles/s | 9.23 M cycles/s | 3.7x |
| `testdata/synth/counter_en.cells.rtl` | 2 comb + 6 edge ops, 2 regs / 9 bits | 0.72 M cycles/s | 5.31 M cycles/s | 7.4x |
| `ip/uart_tx` | 7 comb + 40 edge ops, 4 regs / 31 bits | 1.22 M cycles/s | 4.07 M cycles/s | 3.3x |
| `ip/uart` | 18 comb + 126 edge ops, 13 regs / 72 bits | 0.42 M cycles/s | 1.65 M cycles/s | 3.9x |
| `ip/fifo_sync` | 18 comb + 21 edge ops, 3 regs / 18 bits, 16x8 RAM | 0.59 M cycles/s | 5.19 M cycles/s | 8.8x |
| `ip/spi_master` | 17 comb + 90 edge ops, 10 regs / 53 bits | 0.56 M cycles/s | 2.09 M cycles/s | 3.8x |
| `ip/i2c_master` | 21 comb + 220 edge ops, 14 regs / 41 bits | 0.40 M cycles/s | 1.07 M cycles/s | 2.7x |
| `ip/axil_gpio` | 26 comb + 97 edge ops, 13 regs / 148 bits | 0.32 M cycles/s | 1.82 M cycles/s | 5.6x |

So: three to ten times on the IP blocks, and forty on a design that is
nothing but combinational logic. Two caveats before the analysis. The two
`counter_en` rows swing between runs — 2.2x to 8x for the process form,
9x to 30x for the cell form — because those programs are six to twelve
operations long and what is really being timed is the stimulus; the rows
with more than about twenty operations repeat to within a few percent.
And "M cycles/s" here is a cycle of *this* harness, one input vector and
one clock edge, not a wall-clock claim about any particular design.

Where the rest of the time goes is worth saying plainly, because 3x is
not the order of magnitude a compiled simulator is supposed to be worth:

- **Driving the inputs is in the loop, and it allocates.** Every `set`
  takes a `Logic` by value, and cloning one out of the stimulus pool
  allocates two `Vec`s. On `counter_en` that is six allocations per cycle
  against twelve engine operations, which is both most of the compiled
  time and the reason that row is not repeatable. Both engines pay it, so
  it understates the ratio rather than inflating it — but it means the
  small rows measure the allocator.
- **These designs are tiny.** The event simulator's per-cycle cost is
  dominated by scheduling, which is roughly constant; the compiled
  engine's is proportional to the program. On a twelve-operation block the
  constant is most of the event simulator's time and the ratio is large
  (`adder_tree`); on a two-hundred-operation block it shrinks
  (`i2c_master`). The advantage should grow again with design size,
  because the event simulator re-evaluates a net every time one of its
  inputs changes while the compiled engine evaluates it exactly once — but
  that needs a design big enough to measure, and the corpus has none yet.
- **The lowering is not an optimiser.** A `case` becomes a chain of
  guarded merges, one `Mux` per arm per assigned signal, which is why
  `i2c_master` needs 220 operations per edge for 41 bits of state.
  Constant folding and common subexpression elimination run, and nothing
  else: no dead-value elimination, no mux-tree flattening, no merging of
  a chain of one-bit operations into one word-wide one. That is the first
  thing to do if the number has to be bigger.
- **It is an interpreter.** Each operation costs a `match` on the opcode
  and a few bounds-checked slice indexings into the register file.
  Generating Rust, or a threaded dispatch table, would remove that, and
  would be the step after the optimiser.

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
- `tests/sim_lut_x.rs` drives **mapped** netlists — Verilog synthesised and
  covered onto LUT4 and LUT6 inside the test — at the lookup-table rule
  above, including an unwritten memory behind a mapped multiplexer. Three of
  its four checks fail without that rule; the fourth, the parity of the
  unknown bits, fails if the rule is ever *optimistic*, which is what it is
  there for. Its `#[ignore]`d `lookup_table_evaluation_rate` prints the
  measurement in the table above.
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
