# Combinational equivalence: SAT sweeping

`formal::check_equivalent` proves two modules equal by building their
*miter*: shared inputs, both sets of outputs, and a signal that is true
when any output pair differs. For combinational modules the question
is whether that signal can ever be true. Until this engine, the whole
miter went to the SAT solver as one problem, and for two structurally
different implementations of the same function that is exponentially
hard: the solver sees two unrelated networks and can only case-split
its way to the conclusion that their outputs agree.

The fix is the one every industrial equivalence checker uses: **SAT
sweeping**, after Kuehlmann and Krohm, "Equivalence checking using cuts
and heaps" (DAC 1997), in the functionally reduced AIG form of
Mishchenko, Chatterjee, Jiang and Brayton, "FRAIGs: a unifying
representation for logic synthesis and verification" (2005) and
Mishchenko, Chatterjee, Brayton and Eén, "Improvements to combinational
equivalence checking" (ICCAD 2006). Two implementations of one function
usually share much of their internal structure even when their outputs
are computed differently. Proving those internal equivalences first,
bottom-up, and merging each proved pair shrinks everything above it,
until the final comparison is small or has vanished.

## How it runs

`EquivOptions::engine` is `EquivEngine::Sweep(SweepOptions)` by default.
A combinational miter goes through three stages, and the first one that
answers wins:

1. **A direct attempt.** The whole miter, as before, for
   `quick_conflicts` conflicts (2000). Most miters, equivalent or not,
   are settled here. The solver is deterministic and a conflict limit
   only stops it, so anything settled here gets the same verdict and
   the same counter-example it always had. The golden traces did not
   change.
2. **Exhaustive simulation**, when the cone is small enough that trying
   every input pattern costs at most `exhaustive_budget` node-words (an
   AND node evaluated over 64 patterns): `n · 2^(k-6)` for `k` inputs
   and `n` ANDs. The default of 2^31 covers, say, 26 inputs and 1300
   ANDs, a fraction of a second in an optimised build. This is a
   complete decision too, and for a hard miter with few inputs it is
   much cheaper than any SAT call.
3. **The sweep** (`formal::sweep::sweep_cnf`):
   - the miter's gates are read back out of the CNF (`CnfBuilder` now
     records the gate defining each variable) into one AIG, where
     structural hashing already merges whatever the two sides build
     identically;
   - the AIG is simulated over seeded random patterns, 64 per word, and
     its nodes are partitioned into candidate classes of equal or
     complementary signatures;
   - the nodes are visited in topological order. Each AND is encoded
     into one incremental solver over the *representatives* of its
     fanins, through a hash table, so a node whose fanins were merged
     into another node's is merged too, without a query. Otherwise it is
     checked against the earlier members of its class with two
     assumption-based solves (`n ∧ ¬t`, `¬n ∧ t`) under
     `pair_conflicts` (1000). Both unsatisfiable: merged, and the two
     implications become permanent clauses. Satisfiable: the model's
     inputs become a new simulation pattern, which splits every class
     it separates. Out of conflicts: skipped, so one hard pair cannot
     stall the sweep;
   - the miter's output is resolved through the merges and decided with
     one last solve under `EquivOptions::conflict_limit`, on a solver
     already holding every proved equivalence.

The simulation classes and the sweep loop are the ones the FRAIG
optimisation pass uses (`synth::aig::fraig::{SimClasses, sweep}`); the
optimiser and the checker differ only in the `Prover` that decides a
pair. The sweep needs the `synth` feature for the AIG. Without it,
`EquivEngine::Sweep` falls back to the monolithic path.
`EquivEngine::Monolithic` is the old engine, kept for comparison.

`EquivReport::sweep` carries the statistics when stage 2 or 3 decided:
inputs, ANDs, merges (structural and proved), refuted and skipped
candidates, refinements, the ANDs left in the final query, and whether
simulation decided.

## Soundness

A sweep that merges two nodes it has not proved equal reports
"equivalent" for circuits that are not, and nothing downstream would
notice. So:

- a node is merged only when both of its solves came back
  unsatisfiable, or when after the merges below it it is the AND of the
  same two edges as an earlier node (or folds to a constant or a
  fanin). Simulation only proposes candidates. A debug assertion checks
  that the count of proved merges matches the count of UNSAT-backed
  pairs, and another checks every merge against every simulation vector
  seen so far;
- a "different" answer from the sweep is a concrete input assignment,
  and `check_equivalent` replays it on the miter's own formula before
  reporting it. The trace comes from that replay. A model that does not
  replay trips a debug assertion, and in release the check falls back
  to the monolithic engine rather than report a difference it cannot
  show;
- `tests/synth_arith.rs` puts one wrong gate deep inside a 16-bit Dadda
  multiplier (the mutation the fewest random vectors notice among the
  ones sampled) and requires `Different`, with a counter-example on
  which the two netlists really disagree, from both the default engine
  and the sweep alone, against three references;
- the sweep alone, exhaustive simulation alone and the monolithic
  engine are required to agree on every equivalence question in the
  corpus the monolithic engine can finish: the formal goldens, the
  post-synthesis goldens, and the arithmetic architectures at small
  widths, with and without a wrong gate.

## Where it pays, and where it does not

Measured in an optimised build on one core.

| Miter | Monolithic | Sweep | Default engine |
|---|---|---|---|
| 16-bit Dadda multiplier against its own optimised netlist | 598 s (18 s at 12 bits, 257 s at 14) | 5 ms | 5 ms |
| the same, full 32-bit signed product | not attempted | 11 ms | 11 ms |
| 11-bit Dadda multiplier against the generic `mul` cell | 66 s | 53 s | 14 ms (simulation) |
| full 8-bit signed product, Dadda against the generic cell | 8.2 s | 11 s | 0.6 ms (simulation) |

The first two rows are what post-synthesis verification, the ASIC
flow's mapped-netlist check and `reticle verify` ask: a netlist against
a restructured version of itself. The internal signals mostly survive
restructuring, the sweep proves them, and ten minutes of monolithic
search become milliseconds.

The last two rows are the limit. A carry-save tree (Wallace, Dadda) or
a Booth recoding against a row-at-a-time array shares no internal
signal beyond the partial products and the low output bits, so there
is almost nothing to merge and the final query is the whole miter. The
sweep is then no faster than the monolithic engine, and both grow by
five to eight times per bit. That is a property of the problem, not of
the implementation: these are the standard hard instances for
SAT-based equivalence checking, and the tools that prove them at 32 bits
and beyond use algebraic reasoning over the adder structure instead,
which Reticle does not have. Below about 26 inputs exhaustive simulation
settles them; see `docs/arithmetic.md` for how the multiplier proofs
are arranged.

## Not yet

The sequential case (bounded search and k-induction over a miter with
state) still solves each unrolling directly. Sweeping the combinational
logic of each frame is the natural extension. Reset-matched frames
unrolled from reset would sweep well, since corresponding registers
compute equal functions of the inputs. The induction step needs more:
its state equalities are assumptions, which an unconditional sweep
cannot use. That is the register-correspondence problem (van Eijk,
"Sequential equivalence checking based on structural similarities",
2000). This is left for later.
