//! SAT sweeping: deciding a miter by proving its internal equivalences
//! first.
//!
//! Handing the whole miter of two circuits to a SAT solver is
//! exponentially hard when the two are structurally different
//! implementations of one function: the solver sees two unrelated
//! networks of gates and can only case-split its way to the conclusion
//! that their outputs agree. But two implementations of one function
//! usually share a great deal of *internal* structure even when their
//! outputs are computed differently — partial products, intermediate
//! sums, decoded selects — and every internal equivalence that is proved
//! and merged makes everything above it smaller. Proved bottom-up, each
//! such equivalence is a small, local SAT query, because the nodes below
//! it have already been merged; by the time the outputs are reached the
//! miter has often collapsed to a constant.
//!
//! That is SAT sweeping (Kuehlmann and Krohm, "Equivalence checking using
//! cuts and heaps", DAC 1997; in the FRAIG form of Mishchenko, Chatterjee, Jiang, Brayton, "FRAIGs:
//! a unifying representation for logic synthesis and verification", 2005,
//! and Mishchenko, Chatterjee, Brayton, Eén, "Improvements to
//! combinational equivalence checking", ICCAD 2006). [`sweep_cnf`] does
//! it in four steps:
//!
//! 1. **Read the circuit back out of the CNF.** A [`CnfBuilder`] records
//!    the gate that defines each of its variables, so the cone of the
//!    query is rebuilt as one [`Aig`] — both sides of a miter in a single
//!    graph, where structural hashing already merges whatever the two
//!    build identically.
//! 2. **Simulate** the graph over seeded random patterns, 64 per word,
//!    and partition its nodes into candidate classes (equal or
//!    complementary signatures): [`SimClasses`].
//! 3. **Sweep** the nodes in topological order ([`fraig::sweep`]). Each
//!    AND is first encoded into one incremental solver *over the
//!    representatives of its fanins*, with a hash table on the result, so
//!    a node whose fanins were merged into another node's is merged too,
//!    for free. Otherwise it is checked against the earlier members of its
//!    class with two assumption-based solves (`n ∧ ¬t`, `¬n ∧ t`), under
//!    [`SweepOptions::pair_conflicts`]. Both unsatisfiable: the pair is
//!    merged, and the two implications become permanent clauses. A model:
//!    its input values become a new simulation pattern, which splits
//!    every class it separates, so a false candidate is not tried twice.
//!    Out of conflicts: the pair is skipped, and one hard pair cannot
//!    stall the sweep.
//! 4. **Decide the query** on what is left: the target literals are
//!    resolved through the merges (a miter whose outputs were all merged
//!    resolves to the constant false and needs no solve at all) and one
//!    last solve, under the caller's conflict limit, settles the rest.
//!
//! The solver is the same one throughout, so everything learnt proving
//! one pair is there for the next, and the final query runs on a
//! database already full of proved equivalences.
//!
//! Between steps 1 and 2, a query whose cone is small enough is decided
//! by simulating every input pattern instead
//! ([`SweepOptions::exhaustive_budget`]). That is complete too, and for a
//! hard miter with few inputs it is far cheaper than any SAT call: two
//! multiplier architectures share almost no internal signal, so the
//! sweep has little to merge and the final query is as hard as the
//! whole miter was, while twenty-six inputs are only `2^20` words of
//! simulation.
//!
//! # Soundness
//!
//! A sweep that merged two nodes it had not proved equal would report
//! "equivalent" for circuits that are not, and nothing downstream would
//! notice. So a node is merged for exactly two reasons: both of its
//! solves came back unsatisfiable, or it is structurally redundant — the
//! AND of the same two (representative) edges as an earlier node, or one
//! that folds to a constant or a fanin. Simulation only ever proposes. Debug builds check every
//! merge against the simulation vectors, and every "different" answer is
//! a concrete input assignment the caller can replay on the original
//! formula ([`super::equiv`] does, and reports nothing it cannot replay).
//!
//! # Scope
//!
//! This is the combinational engine of [`super::equiv::check_equivalent`].
//! The sequential equivalence checks (bounded search and k-induction
//! over the miter) still solve their unrollings directly; sweeping the
//! combinational logic of each frame is the natural extension, and is
//! not done yet.
//!
//! The graph comes from [`crate::synth::aig`], so the engine needs the
//! `synth` feature as well as `formal`; without it, only the options and
//! statistics types exist and the equivalence checker uses the
//! monolithic path.

#[cfg(feature = "synth")]
use std::collections::HashMap;

use super::sat::Var;
#[cfg(feature = "synth")]
use super::{
    cnf::{CnfBuilder, Gate},
    sat::{Lit, SolveResult, Solver},
};
#[cfg(feature = "synth")]
use crate::synth::aig::fraig::{self, Prover, SimClasses, Verdict};
#[cfg(feature = "synth")]
use crate::synth::aig::{Aig, Edge, Forward};

/// Options of [`sweep_cnf`] and of the sweeping engine of
/// [`super::equiv::check_equivalent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SweepOptions {
    /// Seed of the random simulation patterns; the same seed gives the
    /// same candidates, the same proofs and the same answer.
    pub seed: u64,
    /// Words of random patterns per node before sweeping (64 patterns a
    /// word). More words mean fewer false candidates to refute.
    pub words: usize,
    /// Conflicts allowed for each candidate-pair query. A pair that needs
    /// more is skipped and stays unmerged; the final query still decides
    /// the answer, just on a larger graph.
    pub pair_conflicts: u64,
    /// Candidates a node is checked against before it is left alone.
    pub max_attempts: usize,
    /// Conflicts the equivalence checker first gives the miter as one
    /// monolithic problem, before sweeping. Most miters, equivalent or
    /// not, are decided within a few thousand conflicts, and deciding them
    /// the old way keeps their counter-examples exactly what they have
    /// always been; `0` goes straight to the sweep.
    pub quick_conflicts: u64,
    /// The most simulation a query may cost to be decided by trying every
    /// input pattern instead of by the sweep, counted in node-words: one
    /// AND node evaluated over one word of 64 patterns. A cone of `k`
    /// inputs and `n` ANDs costs `n · 2^(k-6)`. Exhaustive simulation is a
    /// complete decision like any other, and for a cone with few inputs
    /// it is cheaper than a SAT call however hard the miter: a 13-bit
    /// multiplier pair (26 inputs, 1300 ANDs) is 1.4 G node-words, well
    /// under a second in an optimised build. Each further input doubles
    /// the cost, so it is never the answer for a wide datapath. The
    /// default is `2^31`; `0` never simulates exhaustively.
    pub exhaustive_budget: u64,
}

impl Default for SweepOptions {
    fn default() -> Self {
        SweepOptions {
            seed: 1,
            words: 16,
            pair_conflicts: 1000,
            max_attempts: 4,
            quick_conflicts: 2000,
            exhaustive_budget: 1 << 31,
        }
    }
}

/// The node-words [`exhaustive`] spends on `aig`, or `None` when its
/// inputs are too many to enumerate at all.
#[cfg(feature = "synth")]
fn exhaustive_cost(aig: &Aig) -> Option<u64> {
    let k = u32::try_from(aig.inputs().len()).ok().filter(|&k| k < 64)?;
    let words = 1u64 << k.saturating_sub(6);
    let ands = u64::try_from(aig.num_ands().max(1)).ok()?;
    Some(words.saturating_mul(ands))
}

/// What a sweep did, for reports and for tuning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepStats {
    /// Free variables of the formula in the query's cone: the inputs of
    /// the swept graph.
    pub inputs: usize,
    /// AND nodes of the swept graph, after structural hashing.
    pub ands: usize,
    /// Nodes merged because they became structurally identical to an
    /// earlier node (or folded) once their fanins were merged.
    pub structural: usize,
    /// Nodes merged on an unsatisfiable pair of queries.
    pub proved: usize,
    /// Candidate pairs refuted by a counter-example.
    pub disproved: usize,
    /// Candidate pairs skipped at the conflict limit.
    pub skipped: usize,
    /// Counter-examples added to the simulation.
    pub refinements: usize,
    /// AND nodes left in the cone of the final query after merging.
    pub remaining: usize,
    /// True when the query was decided by simulating every input pattern
    /// ([`SweepOptions::exhaustive_budget`]) rather than by sweeping; the
    /// merge counts are then zero.
    pub exhaustive: bool,
}

/// The answer of [`sweep_cnf`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SweepOutcome {
    /// The formula, with the assumptions, is unsatisfiable.
    Unsat,
    /// It is satisfiable; the assignment gives every free variable in the
    /// query's cone a value, and the gates determine the rest. Variables
    /// outside the cone are unconstrained by it.
    Sat(Vec<(Var, bool)>),
    /// The final query hit its conflict limit.
    Unknown,
}

/// What [`sweep_cnf`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SweepResult {
    /// The answer.
    pub outcome: SweepOutcome,
    /// How it was reached.
    pub stats: SweepStats,
}

/// Decides whether `cnf` is satisfiable with every literal of
/// `assumptions` true, by SAT sweeping the circuit its gates describe
/// (see the module docs).
///
/// The gate clauses of `cnf` define the circuit; its other clauses
/// ([`CnfBuilder::constraints`]) and the assumptions are the query. The
/// sweep itself proves equivalences of the circuit alone, which hold
/// under any constraint; the constraints and assumptions enter only the
/// final solve, which runs under `final_conflicts` (`None` for no limit).
#[cfg(feature = "synth")]
pub fn sweep_cnf(
    cnf: &CnfBuilder,
    assumptions: &[Lit],
    options: &SweepOptions,
    final_conflicts: Option<u64>,
) -> SweepResult {
    let constraints: Vec<&[Lit]> = cnf.constraints().collect();
    let roots: Vec<Lit> = assumptions
        .iter()
        .copied()
        .chain(constraints.iter().flat_map(|c| c.iter().copied()))
        .collect();
    let circuit = Circuit::read(cnf, &roots);
    let aig = &circuit.aig;

    let (targets, clauses) = query(
        &circuit,
        assumptions,
        &constraints,
        &mut Forward::identity(aig.len()),
    );
    if targets.contains(&Edge::FALSE) || clauses.iter().any(Vec::is_empty) {
        // Folded to a contradiction while the circuit was read.
        return SweepResult {
            outcome: SweepOutcome::Unsat,
            stats: SweepStats {
                inputs: aig.inputs().len(),
                ands: aig.num_ands(),
                ..SweepStats::default()
            },
        };
    }
    if exhaustive_cost(aig).is_some_and(|c| c <= options.exhaustive_budget) {
        let stats = SweepStats {
            inputs: aig.inputs().len(),
            ands: aig.num_ands(),
            remaining: aig.num_ands(),
            exhaustive: true,
            ..SweepStats::default()
        };
        let outcome = match exhaustive(aig, &targets, &clauses) {
            None => SweepOutcome::Unsat,
            Some(pattern) => {
                SweepOutcome::Sat(circuit.inputs.iter().copied().zip(pattern).collect())
            }
        };
        return SweepResult { outcome, stats };
    }

    let mut classes = SimClasses::new(aig, options.seed, options.words);
    let mut fwd = Forward::identity(aig.len());
    let mut prover = SatProver::new(aig, options.pair_conflicts);
    let counts = fraig::sweep(
        aig,
        &mut classes,
        &mut fwd,
        &mut prover,
        options.max_attempts,
    );
    let mut stats = SweepStats {
        inputs: aig.inputs().len(),
        ands: aig.num_ands(),
        structural: counts.structural,
        proved: counts.proved,
        disproved: counts.disproved,
        skipped: counts.unknown,
        refinements: classes.refinements(),
        remaining: 0,
        exhaustive: false,
    };
    debug_assert_eq!(
        prover.unsat_pairs, counts.proved,
        "every proved merge is backed by two unsatisfiable queries"
    );

    // The query, on the swept graph.
    let (targets, clauses) = query(&circuit, assumptions, &constraints, &mut fwd);
    let mut roots: Vec<Edge> = targets.clone();
    roots.extend(clauses.iter().flatten().copied());
    stats.remaining = cone_size(aig, &mut fwd, &roots);

    let unsat = SweepResult {
        outcome: SweepOutcome::Unsat,
        stats,
    };
    if targets.contains(&Edge::FALSE) || clauses.iter().any(Vec::is_empty) {
        return unsat;
    }
    for clause in &clauses {
        let lits: Vec<Lit> = clause.iter().map(|&e| prover.lit(e)).collect();
        if !prover.solver.add_clause(&lits) {
            return unsat;
        }
    }
    let lits: Vec<Lit> = targets
        .iter()
        .filter(|&&e| e != Edge::TRUE)
        .map(|&e| prover.lit(e))
        .collect();
    prover.solver.set_conflict_limit(final_conflicts);
    let outcome = match prover.solver.solve_with_assumptions(&lits) {
        SolveResult::Unsat => SweepOutcome::Unsat,
        SolveResult::Unknown => SweepOutcome::Unknown,
        SolveResult::Sat => SweepOutcome::Sat(
            circuit
                .inputs
                .iter()
                .zip(prover.pattern(aig))
                .map(|(&v, value)| (v, value))
                .collect(),
        ),
    };
    SweepResult { outcome, stats }
}

/// The query as edges of the graph after the merges in `fwd`: the
/// assumptions, and the constraint clauses with constant literals folded
/// (a clause made true by a constant is dropped; an empty one is
/// unsatisfiable).
#[cfg(feature = "synth")]
fn query(
    circuit: &Circuit,
    assumptions: &[Lit],
    constraints: &[&[Lit]],
    fwd: &mut Forward,
) -> (Vec<Edge>, Vec<Vec<Edge>>) {
    let targets: Vec<Edge> = assumptions
        .iter()
        .map(|&a| fwd.resolve(circuit.edge(a)))
        .collect();
    let mut clauses: Vec<Vec<Edge>> = Vec::with_capacity(constraints.len());
    for clause in constraints {
        let edges: Vec<Edge> = clause
            .iter()
            .map(|&l| fwd.resolve(circuit.edge(l)))
            .collect();
        if edges.contains(&Edge::TRUE) {
            continue;
        }
        clauses.push(edges.into_iter().filter(|&e| e != Edge::FALSE).collect());
    }
    (targets, clauses)
}

/// Words simulated at a time by [`exhaustive`].
#[cfg(feature = "synth")]
const EXHAUSTIVE_CHUNK: usize = 256;

/// Decides the query by simulating every input pattern: returns one that
/// makes every target true and every clause hold, or `None` when there
/// is none. The cost is `2^inputs / 64` words of [`Aig::simulate`].
#[cfg(feature = "synth")]
fn exhaustive(aig: &Aig, targets: &[Edge], clauses: &[Vec<Edge>]) -> Option<Vec<bool>> {
    /// The first six inputs take every value within one word.
    const LANES: [u64; 6] = [
        0xAAAA_AAAA_AAAA_AAAA,
        0xCCCC_CCCC_CCCC_CCCC,
        0xF0F0_F0F0_F0F0_F0F0,
        0xFF00_FF00_FF00_FF00,
        0xFFFF_0000_FFFF_0000,
        0xFFFF_FFFF_0000_0000,
    ];
    let k = aig.inputs().len();
    assert!(k < 64, "exhaustive simulation of {k} inputs");
    let total_words: u64 = if k <= 6 { 1 } else { 1 << (k - 6) };
    let chunk = usize::try_from(total_words)
        .unwrap_or(usize::MAX)
        .min(EXHAUSTIVE_CHUNK);
    let chunk_bits = chunk.trailing_zeros() as usize;
    // Only the first `2^k` lanes are patterns when there are fewer than
    // six inputs.
    let valid = if k >= 6 {
        !0
    } else {
        (1u64 << (1u32 << k)) - 1
    };
    let chunks = total_words / chunk as u64;
    let mut words = vec![0u64; k * chunk];
    for c in 0..chunks {
        for i in 0..k {
            for w in 0..chunk {
                words[i * chunk + w] = if i < 6 {
                    LANES[i]
                } else if i < 6 + chunk_bits {
                    if (w >> (i - 6)) & 1 == 1 { !0 } else { 0 }
                } else if (c >> (i - 6 - chunk_bits)) & 1 == 1 {
                    !0
                } else {
                    0
                };
            }
        }
        let vals = aig.simulate(&words, chunk);
        for w in 0..chunk {
            let mut hit = valid;
            for &t in targets {
                hit &= Aig::sim_value(&vals, chunk, t, w);
            }
            for clause in clauses {
                hit &= clause
                    .iter()
                    .fold(0, |acc, &e| acc | Aig::sim_value(&vals, chunk, e, w));
            }
            if hit != 0 {
                let lane = u64::from(hit.trailing_zeros());
                let pattern = (c * chunk as u64 + w as u64) << 6 | lane;
                return Some((0..k).map(|i| (pattern >> i) & 1 == 1).collect());
            }
        }
    }
    None
}

/// The circuit of a [`CnfBuilder`], read back into an [`Aig`].
#[cfg(feature = "synth")]
struct Circuit {
    aig: Aig,
    /// The edge standing for each variable of the CNF, once reached.
    edges: Vec<Option<Edge>>,
    /// The CNF variable of each AIG input, in [`Aig::inputs`] order.
    inputs: Vec<Var>,
}

#[cfg(feature = "synth")]
impl Circuit {
    /// Rebuilds the cone of `roots`: a gate becomes its AND, XOR or
    /// multiplexer, a free variable an input, variable 0 the constant.
    fn read(cnf: &CnfBuilder, roots: &[Lit]) -> Circuit {
        let n = usize::try_from(cnf.num_vars()).expect("variable count fits usize");
        let mut c = Circuit {
            aig: Aig::new(),
            edges: vec![None; n],
            inputs: Vec::new(),
        };
        c.edges[0] = Some(Edge::TRUE);
        let index = |v: Var| usize::try_from(v.index()).expect("variable fits usize");
        for &root in roots {
            // Iterative post-order, so a deep circuit cannot overflow the
            // stack.
            let mut stack: Vec<(Var, bool)> = vec![(root.var(), false)];
            while let Some((v, expanded)) = stack.pop() {
                if c.edges[index(v)].is_some() {
                    continue;
                }
                let Some(gate) = cnf.gate(v) else {
                    c.edges[index(v)] = Some(c.aig.add_input());
                    c.inputs.push(v);
                    continue;
                };
                if expanded {
                    let e = match gate {
                        Gate::And(a, b) => {
                            let (a, b) = (c.edge(a), c.edge(b));
                            c.aig.and(a, b)
                        }
                        Gate::Xor(a, b) => {
                            let (a, b) = (c.edge(a), c.edge(b));
                            c.aig.xor(a, b)
                        }
                        Gate::Ite(s, t, e) => {
                            let (s, t, e) = (c.edge(s), c.edge(t), c.edge(e));
                            c.aig.mux(s, t, e)
                        }
                    };
                    c.edges[index(v)] = Some(e);
                } else {
                    stack.push((v, true));
                    let operands: &[Lit] = match &gate {
                        Gate::And(a, b) | Gate::Xor(a, b) => &[*a, *b],
                        Gate::Ite(s, t, e) => &[*s, *t, *e],
                    };
                    for op in operands.iter().rev() {
                        if c.edges[index(op.var())].is_none() {
                            stack.push((op.var(), false));
                        }
                    }
                }
            }
        }
        c
    }

    /// The edge standing for `lit`; its variable must have been reached.
    fn edge(&self, lit: Lit) -> Edge {
        let v = usize::try_from(lit.var().index()).expect("variable fits usize");
        self.edges[v]
            .expect("operand read before its gate")
            .xor(lit.is_neg())
    }
}

/// The number of AND nodes in the cone of `roots` after the merges in
/// `fwd`.
#[cfg(feature = "synth")]
fn cone_size(aig: &Aig, fwd: &mut Forward, roots: &[Edge]) -> usize {
    let mut seen = vec![false; aig.len()];
    let mut stack: Vec<u32> = roots.iter().map(|e| e.node()).collect();
    let mut count = 0;
    while let Some(id) = stack.pop() {
        if std::mem::replace(&mut seen[id as usize], true) || !aig.is_and(id) {
            continue;
        }
        count += 1;
        let (a, b) = aig.fanins(id);
        stack.push(fwd.resolve(a).node());
        stack.push(fwd.resolve(b).node());
    }
    count
}

/// The sweep's [`Prover`]: one incremental solver holding the swept graph,
/// each node encoded over the representatives of its fanins.
#[cfg(feature = "synth")]
struct SatProver {
    solver: Solver,
    /// The solver literal of each encoded node (the constant, the inputs,
    /// and every AND that was not merged structurally).
    lits: Vec<Option<Lit>>,
    /// Encoded ANDs by their (representative, ordered) fanin pair: the
    /// structural hash of the swept graph.
    strash: HashMap<(Edge, Edge), u32>,
    /// Conflicts per candidate query.
    limit: u64,
    /// Pairs shown equal by two unsatisfiable queries; the sweep's count
    /// of proved merges must match it exactly.
    unsat_pairs: usize,
}

#[cfg(feature = "synth")]
impl SatProver {
    fn new(aig: &Aig, limit: u64) -> SatProver {
        let mut solver = Solver::new();
        let mut lits = vec![None; aig.len()];
        let zero = Lit::pos(solver.new_var());
        solver.add_clause(&[!zero]);
        lits[0] = Some(zero);
        for &pi in aig.inputs() {
            lits[pi as usize] = Some(Lit::pos(solver.new_var()));
        }
        SatProver {
            solver,
            lits,
            strash: HashMap::new(),
            limit,
            unsat_pairs: 0,
        }
    }

    /// The literal of a (resolved) edge.
    fn lit(&self, e: Edge) -> Lit {
        let l = self.lits[e.index()].expect("a representative is encoded");
        if e.is_complement() { !l } else { l }
    }

    /// The input values of the last model, in [`Aig::inputs`] order.
    fn pattern(&self, aig: &Aig) -> Vec<bool> {
        aig.inputs()
            .iter()
            .map(|&pi| {
                let l = self.lits[pi as usize].expect("inputs are encoded");
                // A variable the model leaves open can take either value;
                // the gates are functions of the inputs, so the queried
                // literals do not depend on the choice.
                self.solver.value(l.var()).unwrap_or(false)
            })
            .collect()
    }
}

#[cfg(feature = "synth")]
impl Prover for SatProver {
    fn visit(&mut self, aig: &Aig, fwd: &mut Forward, node: u32) -> Option<Edge> {
        let (a, b) = aig.fanins(node);
        let (a, b) = (fwd.resolve(a), fwd.resolve(b));
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        // The folds of `Aig::and`, now that merges may have made the
        // fanins constant, equal or complementary.
        if a == Edge::FALSE || a == !b {
            return Some(Edge::FALSE);
        }
        if a == Edge::TRUE || a == b {
            return Some(b);
        }
        if let Some(&m) = self.strash.get(&(a, b)) {
            return Some(fwd.resolve(Edge::plain(m)));
        }
        let (la, lb) = (self.lit(a), self.lit(b));
        let y = Lit::pos(self.solver.new_var());
        self.solver.add_clause(&[!y, la]);
        self.solver.add_clause(&[!y, lb]);
        self.solver.add_clause(&[y, !la, !lb]);
        self.lits[node as usize] = Some(y);
        self.strash.insert((a, b), node);
        None
    }

    fn prove(&mut self, aig: &Aig, _fwd: &mut Forward, node: u32, target: Edge) -> Verdict {
        let n = self.lit(Edge::plain(node));
        let t = self.lit(target);
        self.solver.set_conflict_limit(Some(self.limit));
        // `n` without `t`, then `t` without `n`: both unsatisfiable is the
        // proof, and each half is a valid implication worth keeping.
        for (x, y) in [(n, t), (t, n)] {
            match self.solver.solve_with_assumptions(&[x, !y]) {
                SolveResult::Unsat => {
                    self.solver.add_clause(&[!x, y]);
                }
                SolveResult::Sat => return Verdict::Differ(Some(self.pattern(aig))),
                SolveResult::Unknown => return Verdict::Unknown,
            }
        }
        self.unsat_pairs += 1;
        Verdict::Equal
    }
}

#[cfg(all(test, feature = "synth"))]
mod tests {
    use super::*;

    /// The sweep proper: no exhaustive shortcut, whatever the input count.
    fn no_simulation() -> SweepOptions {
        SweepOptions {
            exhaustive_budget: 0,
            ..SweepOptions::default()
        }
    }

    /// Reads a vector of literals out of an assignment.
    fn value(assign: &[(Var, bool)], cnf: &CnfBuilder, bits: &[Lit]) -> u64 {
        // Evaluate by replaying the assignment on a fresh solver.
        let mut solver = cnf.clone().into_solver();
        let assumptions: Vec<Lit> = assign.iter().map(|&(v, b)| Lit::new(v, !b)).collect();
        assert_eq!(
            solver.solve_with_assumptions(&assumptions),
            SolveResult::Sat
        );
        bits.iter()
            .enumerate()
            .filter(|(_, l)| solver.value(l.var()) == Some(l.is_pos()))
            .map(|(i, _)| 1u64 << i)
            .sum()
    }

    /// A ripple-carry sum built bit by bit with fresh variables, so
    /// structural hashing cannot share it with `CnfBuilder::add`.
    fn other_adder(b: &mut CnfBuilder, x: &[Lit], y: &[Lit]) -> Vec<Lit> {
        let mut carry = b.false_lit();
        let mut out = Vec::new();
        for (&p, &q) in x.iter().zip(y) {
            // sum = !(!(p ^ q) ^ carry), carry = maj written with ands.
            let pq = b.xor(p, q);
            let s = b.xor(!pq, carry);
            out.push(!s);
            let g = b.and(p, q);
            let t = b.and(pq, carry);
            carry = b.or(g, t);
        }
        out
    }

    #[test]
    fn proves_two_adders_equal() {
        let mut b = CnfBuilder::new();
        let x = b.new_vars(12);
        let y = b.new_vars(12);
        let s1 = b.add(&x, &y);
        let s2 = other_adder(&mut b, &x, &y);
        let eq = b.eq_vec(&s1[..12], &s2);
        let r = sweep_cnf(&b, &[!eq], &no_simulation(), None);
        assert_eq!(r.outcome, SweepOutcome::Unsat, "{:?}", r.stats);
        assert!(r.stats.proved + r.stats.structural > 0, "{:?}", r.stats);
        assert_eq!(r.stats.remaining, 0, "the miter collapses: {:?}", r.stats);
        assert_eq!(r.stats.inputs, 24);
    }

    #[test]
    fn finds_a_real_difference() {
        let mut b = CnfBuilder::new();
        let x = b.new_vars(10);
        let y = b.new_vars(10);
        let s1 = b.add(&x, &y);
        let mut s2 = other_adder(&mut b, &x, &y);
        // Break one bit only when both top operand bits are set.
        let both = b.and(x[9], y[9]);
        s2[9] = b.xor(s2[9], both);
        let eq = b.eq_vec(&s1[..10], &s2);
        let r = sweep_cnf(&b, &[!eq], &no_simulation(), None);
        let SweepOutcome::Sat(assign) = r.outcome else {
            panic!("{:?}", r.outcome);
        };
        let (a, c) = (value(&assign, &b, &x), value(&assign, &b, &y));
        assert!(a >> 9 & 1 == 1 && c >> 9 & 1 == 1, "{a} {c}");
        assert_ne!(value(&assign, &b, &s1[..10]), value(&assign, &b, &s2));
    }

    #[test]
    fn constraints_enter_the_final_query() {
        // x & y is not always false, but it is once x is forced low.
        let mut b = CnfBuilder::new();
        let x = b.new_var();
        let y = b.new_var();
        let g = b.and(x, y);
        let r = sweep_cnf(&b, &[g], &no_simulation(), None);
        assert!(matches!(r.outcome, SweepOutcome::Sat(_)));
        b.add_unit(!x);
        let r = sweep_cnf(&b, &[g], &no_simulation(), None);
        assert_eq!(r.outcome, SweepOutcome::Unsat);
        // A contradictory pair of assumptions.
        let mut b = CnfBuilder::new();
        let x = b.new_var();
        let r = sweep_cnf(&b, &[x, !x], &no_simulation(), None);
        assert_eq!(r.outcome, SweepOutcome::Unsat);
    }

    #[test]
    fn is_deterministic() {
        let mut b = CnfBuilder::new();
        let x = b.new_vars(8);
        let y = b.new_vars(8);
        let p1 = b.mul(&x, &y);
        let p2 = b.mul(&y, &x);
        let eq = b.eq_vec(&p1[..8], &p2[..8]);
        let opts = no_simulation();
        let r1 = sweep_cnf(&b, &[!eq], &opts, None);
        let r2 = sweep_cnf(&b, &[!eq], &opts, None);
        assert_eq!(r1, r2);
        assert_eq!(r1.outcome, SweepOutcome::Unsat);
    }

    #[test]
    fn small_queries_are_simulated_exhaustively() {
        let exhaustive = SweepOptions::default();
        // Equal: every pattern is tried and none separates them.
        let mut b = CnfBuilder::new();
        let x = b.new_vars(7);
        let y = b.new_vars(7);
        let s1 = b.add(&x, &y);
        let s2 = other_adder(&mut b, &x, &y);
        let eq = b.eq_vec(&s1[..7], &s2);
        let r = sweep_cnf(&b, &[!eq], &exhaustive, None);
        assert_eq!(r.outcome, SweepOutcome::Unsat);
        assert_eq!((r.stats.inputs, r.stats.proved), (14, 0), "{:?}", r.stats);
        // Different on exactly one pattern out of 2^14, which random
        // simulation would almost surely miss.
        let mut s3 = s2.clone();
        let all_x = b.and_n(&x);
        let all_y = b.and_n(&y);
        let corner = b.and(all_x, all_y);
        s3[3] = b.xor(s3[3], corner);
        let eq = b.eq_vec(&s1[..7], &s3);
        let r = sweep_cnf(&b, &[!eq], &exhaustive, None);
        let SweepOutcome::Sat(assign) = r.outcome else {
            panic!("{:?}", r.outcome);
        };
        assert_eq!(value(&assign, &b, &x), 127);
        assert_eq!(value(&assign, &b, &y), 127);
        // The sweep proper finds the same corner.
        let r = sweep_cnf(&b, &[!eq], &no_simulation(), None);
        let SweepOutcome::Sat(assign) = r.outcome else {
            panic!("{:?}", r.outcome);
        };
        assert_eq!(value(&assign, &b, &x), 127);
        // Fewer than six inputs: only the first `2^k` lanes are patterns.
        let mut b = CnfBuilder::new();
        let x = b.new_vars(2);
        let g = b.and(x[0], !x[1]);
        let r = sweep_cnf(&b, &[g], &exhaustive, None);
        let SweepOutcome::Sat(assign) = r.outcome else {
            panic!("{:?}", r.outcome);
        };
        assert_eq!(assign, [(x[0].var(), true), (x[1].var(), false)]);
        let h = b.and(g, x[1]);
        let r = sweep_cnf(&b, &[h], &exhaustive, None);
        assert_eq!(r.outcome, SweepOutcome::Unsat);
        // Constraints are honoured pattern by pattern.
        b.add_clause(&[!x[0], x[1]]);
        let r = sweep_cnf(&b, &[g], &exhaustive, None);
        assert_eq!(r.outcome, SweepOutcome::Unsat);
    }

    #[test]
    fn a_conflict_limit_leaves_the_answer_open() {
        // A 12-bit multiplier against its commuted twin, with no room to
        // prove anything and a final query allowed one conflict.
        let mut b = CnfBuilder::new();
        let x = b.new_vars(12);
        let y = b.new_vars(12);
        let p1 = b.mul(&x, &y);
        let p2 = b.mul(&y, &x);
        let eq = b.eq_vec(&p1[..12], &p2[..12]);
        let opts = SweepOptions {
            pair_conflicts: 1,
            ..no_simulation()
        };
        let r = sweep_cnf(&b, &[!eq], &opts, Some(1));
        assert_eq!(r.outcome, SweepOutcome::Unknown, "{:?}", r.stats);
        assert!(r.stats.skipped > 0, "{:?}", r.stats);
    }
}
