# Where synthesis spends its time

This is a measurement, not an estimate. Everything below was sampled or
timed on one machine against one design, and the method is written down so
the numbers can be disputed by repeating them.

It exists because a round of work was about to optimise the wrong thing.
The reasoning was good — technology mapping had become the slowest stage,
the cut enumerator had just been rewritten, and composing a truth table per
merged cut is exactly the shape that produces a quadratic-looking stage — and
the conclusion was wrong. Cut enumeration is **1.3%** of the run.

## The design and the build

`testdata/fpga/cynthion/usb_host_target.v` plus the nine IP sources it
instantiates: 7733 lines, mapping to **3168 `LUT4`** at depth 11 for an
ECP5. The command line is in `CLAUDE.md` under "Building a design".

**The build matters more than anything else here.** `cargo test` compiles
with the `dev` profile, so the gate — and therefore `tests/ip_library.rs`,
the slowest suite in it — runs unoptimised code with debug assertions on. A
release build of the same commit runs the same mapping about **ten times
faster** (0.79 s against 7.9 s for `reticle fpga` before this work). The
figures below are all `dev`, because that is the build whose speed the gate
pays for.

Timings are **child CPU time, best of several runs, interleaved between the
two builds being compared**. This machine is shared: a wall clock reading
taken while another round is placing and routing is worth nothing, and even
CPU time inflates under cache contention, so the only comparison that holds
is one where both builds see the same load within seconds of each other.

## Profiling without `perf`

`perf` is not installed, and `/proc/sys/kernel/yama/ptrace_scope` is `1`, so
a process may only be traced by an **ancestor**. `eu-stack -p` run from a
sibling shell gets `Operation not permitted`. Two ways round it:

- Launch the target through a three-line C program that calls
  `prctl(PR_SET_PTRACER, PR_SET_PTRACER_ANY)` and then `execvp`s it. Any
  tracer is then allowed, and a shell loop calling `eu-stack -p` collects a
  few hundred stacks over a multi-second run. That is what produced the
  tables below.
- Or run under `gdb`, which *is* the ancestor, and drive it from a Python
  script that alternates `continue &` with `interrupt`.

Symbols come back as Rust v0 mangled names; `c++filt` demangles them.

A few hundred samples is enough to tell 50% from 1%, which is all these
numbers are claimed to do.

## What the mapper was doing

`reticle synth --lut 4`, 225 samples, before any of this work. Inclusive
share of samples, so the entries nest:

| frame | share |
|---|---|
| `techmap::map_module_checked` | 96.0% |
| — `aig::optimize`, run *before* the covering | 93.8% |
| — — `aig::rewrite::rewrite` | 63.1% |
| — — — `cut::cone_truth` | 26.7% |
| — — — `rewrite::Library::build` (the per-cut memo) | 14.7% |
| — — — `cut::enumerate_cuts` | 8.9% |
| — — `aig::refactor::refactor` | 23.1% |
| — — — `refactor::isop` | 14.2% |
| — — `aig::fraig::fraig` | 5.8% |
| `techmap::PriorityCuts::compute` and the covering passes | **1.3%** |
| **anything inside a hash map or set** | **50.7%** |

The last line is the finding. Half the mapper's time went into SipHash-1-3
over keys four bytes wide. Attributing each hashing sample to the compiler
frame that called it:

| call site | share of the run |
|---|---|
| `cone_nodes`' visited set, `HashSet<u32>` | 13.3% |
| `cone_truth`'s node tables, `HashMap<u32, TruthTable>` | 10.7% |
| the rewriting library's memo, `HashMap<u16, Sig>` | 8.4% |
| the MFFC counter's memo, `HashMap<(Trial, Trial), Trial>` | 6.2% |
| the structural hash table, `HashMap<(Edge, Edge), u32>` | 4.9% |
| `enumerate_cuts`' seen set, `HashSet<Vec<u32>>` | 4.4% |

None of those keys comes from outside the compiler and none of those maps is
ever iterated, so the collision resistance was paying for nothing. The
standard hasher also builds a fresh keyed hasher per lookup, which showed up
as 7.6% of self time in `RandomState::build_hasher` alone.

## What the mapping check was doing

`--verify` on `reticle synth` runs **two** different checks, and conflating
them is easy: `synth::verify::check_synthesis` proves the optimised netlist
equal to a minimal lowering of the source, and `techmap::verify` proves the
*mapped* network equal to the AIG it came from. Of the 594 samples of
`synth --lut 4 --verify`:

| part | share | of 18.5 s |
|---|---|---|
| `synth::verify::check_synthesis` (not mapping) | 49.5% | ~9.2 s |
| `techmap::map_module` (optimise + cover + its own check) | 48.7% | ~9.0 s |
| — of which `techmap::verify` | 9.3% | ~1.7 s |

So the mapping equivalence check costs **under two seconds** on this design,
not the fourteen a subtraction of `synth --lut 4 --verify` from
`synth --lut 4` suggests. `reticle fpga --verify` is the figure to read for
it, because that flow runs the mapping check and nothing else: 10.28 s
against 8.52 s without, so 1.76 s.

Within that 1.76 s (55 samples):

| frame | share of the check |
|---|---|
| `fraig::sweep` | 92.7% |
| — `prove_sat` | 61.8% |
| — `prove_exhaustive` | 29.1% |
| — `fraig::cones`, the cone traversal | 40.0% |
| `lut_miter`, building the miter | 1.8% |
| `SimClasses`, the random simulation | 7.3% |
| **inside a hash map or set** | **50.9%** |

Three answers to the three questions: building the miter is free, the
simulation tier is nearly free, and the cost is the proof — specifically the
cone traversal it repeats per candidate pair, which it was doing **twice**
per pair (once bounded for the exhaustive tier, then again unbounded for the
SAT tier).

## What was changed

All of it output-preserving; see the next section.

| change | where |
|---|---|
| `IntHasher`, a multiply-rotate hasher for dense integer keys | `aig::ihash` |
| the two smallest memos became association lists | `aig::rewrite`, `aig::mffc` |
| `TruthTable` keeps its word inline for six variables or fewer | `aig::truth` |
| `ConeScratch`: one allocation per pass, epoch-stamped marks, slot-indexed tables | `aig::cut` |
| `enumerate_cuts` keeps no second copy of a cut and no hash set | `aig::cut` |
| one cone traversal serves both proof tiers | `aig::fraig` |
| the prover's marks, tables and SAT literals are slot-indexed vectors | `aig::fraig` |
| `cofactor` and `depends_on` stop building tables to read a mask | `aig::truth` |
| `and_with`, `and_not_with`, `or_with`: in-place forms for `isop` | `aig::truth` |

Interleaved best-of-two child CPU time, `dev` profile:

| command | before | after | change |
|---|---|---|---|
| `check` (parse and elaborate) | — | — | untouched |
| `synth` (generic synthesis, no mapping) | 0.40 s | 0.38 s | −5% |
| `synth --lut 4` | 6.94 s | 3.46 s | **−50%** |
| `synth --lut 6` | 6.93 s | 3.70 s | **−47%** |
| `synth --lut 4 --verify` | 18.49 s | 14.15 s | −23% |
| `fpga ecp5` | 8.52 s | 4.44 s | **−48%** |
| `fpga ecp5 --verify` | 10.28 s | 5.44 s | **−47%** |
| the mapping check alone (the difference of the last two) | 1.76 s | 1.00 s | **−43%** |

`synth --lut 4 --verify` improves least because half of it is the generic
equivalence check, which none of this touches.

## Why the output is the same

A performance change to the mapper that produced a different cover would be
a different change, and the repository already has the checks to tell:

- `footprints_match_the_documentation`, run **without** `UPDATE_EXPECT`,
  compares a freshly measured footprint table for all 31 library blocks on
  four targets against the one in `docs/ip-library.md`. It passes.
- `every_block_maps_to_the_logic_it_was_mapped_from` proves all 68 mappings
  equivalent, and `every_cut_computes_its_node` checks every cut function
  against exhaustive simulation. Both pass.
- Every golden `.map` and `.rtl` under `testdata/` is unchanged.

The reasoning behind each change, rather than the test result:

- A **hasher** changes which bucket a key lands in, so it changes the order a
  map *iterates*. Every map switched over is a lookup table that nothing
  iterates; the ones that are read in order kept the standard hasher.
- `TruthTable`'s **inline word** is a second representation of the same
  bits, chosen by the table's size alone, with `PartialEq` and `Hash`
  written over the words rather than over the representation.
- `ConeScratch` performs the **same traversal**: the same order of pushes,
  the same cone, and the same refusal for a leaf set that does not separate
  the root.
- `enumerate_cuts`' linear scan asks the same membership question of the same
  set. The cuts it has seen are the trivial cut plus its result, because a cut
  joins the one exactly when it joins the other, so the result *is* the
  membership test and there is nothing to keep a second copy of; the scan is
  over at most `limit` lists of at most `k` leaves.
- The prover's **shared traversal** is sound because `max_cone` can only cut
  a walk short: a walk that finished under the bound visited exactly what an
  unbounded one would have. The SAT encoding is built in the same order, so
  the solver meets the same clauses under the same conflict limit and returns
  the same verdicts — which matters, because `fraig` is an optimisation pass
  as well as the checker's engine.
- `depends_on` compares the two cofactors where they lie instead of building
  them. A table's unused high bits are zero on both sides of that
  comparison, so they cannot claim a dependency of their own.

## What is left, and what would parallelise

Sampled again after the work, `synth --lut 4` at 3.46 s:

| frame | share |
|---|---|
| `aig::optimize` | 89.5% |
| — `rewrite::rewrite` | 47.5% |
| — `refactor::refactor` | 35.6% |
| — — `refactor::isop` | 22.8% |
| `rewrite::Library::get`, building the 65536-entry table once | 11.0% |
| `PriorityCuts::compute` and the covering | 2.7% |
| inside a hash map or set | 6.8% |
| inside the allocator or `Vec` | 35.2% |

Three things are still on the table and were not taken:

- **The rewriting library is built once per process** and costs about 0.4 s
  of every `reticle synth` invocation. A test binary pays it once and does
  not care; a command line pays it every time. It is a fixed-point
  relaxation over all 65536 four-variable functions and could be made
  cheaper, or serialised into the binary.
- **`opt-level` for the test profile.** A release build of the same code is
  about ten times faster. `[profile.test] opt-level = 1` would cut the gate's
  test time by much more than anything in this document, at the cost of
  compile time on every edit. That is a build-configuration decision, not a
  mapper change, so it is reported rather than taken.
- **`isop`'s three-way recursion** still allocates for cuts of seven leaves
  or more, where a truth table does not fit one word.

**What would parallelise**, reported only — nothing here was threaded:

- `tests/ip_library.rs`' two heavy tests are a loop over 31 independent
  blocks, each elaborated, synthesised and mapped on its own. That loop is
  embarrassingly parallel and is the single largest easy win in the gate; it
  needs the per-block results collected in block order so the footprint table
  stays byte-identical.
- `techmap::verify`'s miter is decided per candidate pair by a prover with no
  state the pairs share except the `Forward` map, which is written as pairs
  are proved. The pairs of one *class* must stay ordered, but different
  classes are independent.
- `PriorityCuts::compute` is a topological sweep: a node's cuts depend on its
  fanins', so it parallelises only by level, and at 1.3% of the run there is
  nothing there.
- `rewrite` and `refactor` cannot be parallelised as written: both mutate one
  graph through a shared `View` and decide each node against the reference
  counts the previous node left behind, so two threads would make different
  decisions and a different netlist.
