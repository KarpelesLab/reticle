//! Functional reduction: merging nodes that compute the same function.
//!
//! Structural hashing only merges nodes with identical fanins. A
//! *functionally reduced* AIG (FRAIG, Mishchenko, Chatterjee, Jiang,
//! Brayton, "FRAIGs: a unifying representation for logic synthesis and
//! verification", 2005) also merges nodes that are equal, or
//! complementary, as Boolean functions of the primary inputs. The
//! procedure has two halves:
//!
//! 1. **Simulation** with random input vectors sorts nodes into candidate
//!    equivalence classes by their signature (the vector of simulated
//!    values, normalised so a node and its complement land in one class).
//!    That is [`SimClasses`].
//! 2. **Proof**, for each node against the earlier members of its class,
//!    in topological order, merging a node as soon as one candidate is
//!    proved equal. That is [`sweep`], which asks a [`Prover`] about each
//!    candidate pair; a disproof that comes with a distinguishing input
//!    pattern is fed back into the simulation ([`SimClasses::refine`]),
//!    which splits every class the pattern separates, so a false
//!    candidate is not proposed again.
//!
//! The same two halves serve two masters. [`fraig`], the optimisation
//! pass, proves candidates with a cone-local [`Prover`]: when the union
//! of the two cones depends on at most [`FraigOptions::max_exhaustive`]
//! inputs it is checked exhaustively by simulating every pattern;
//! otherwise, with the `formal` feature, the two cones are encoded in CNF
//! and handed to a fresh SAT solver under a conflict limit. Without the
//! feature, large candidates are left alone. The equivalence checker's
//! SAT sweeping (`formal::sweep`, after Kuehlmann and Krohm, "Equivalence
//! checking using cuts and heaps", DAC 1997, and Mishchenko, Chatterjee,
//! Brayton, Eén, "Improvements to combinational equivalence checking",
//! ICCAD 2006) plugs in a prover built on one incremental solver instead,
//! whose learnt clauses carry over from one pair to the next.
//!
//! Merges are recorded in a [`Forward`] map; [`fraig`] applies them with
//! [`Aig::rebuild_with`] at the end.
//!
//! # Soundness
//!
//! A merge is only as good as its proof. [`sweep`] merges a node for
//! exactly two reasons: the prover returned [`Verdict::Equal`], which a
//! prover may only do on a complete argument (an exhaustive truth table
//! or an unsatisfiable SAT query, never simulation alone), or
//! [`Prover::visit`] found the node *structurally* identical to an
//! earlier one once the merges below it are applied. As a cheap check on
//! both, debug builds assert that every merged pair agrees on every
//! simulation vector seen so far, which a wrong merge usually does not.

use std::collections::{HashMap, HashSet};

use super::truth::TruthTable;
use super::{Aig, Edge, Forward, Rng};

/// Options for [`fraig`].
#[derive(Clone, Debug)]
pub struct FraigOptions {
    /// Seed for the random simulation vectors.
    pub seed: u64,
    /// Number of 64-bit words of random patterns (so `64 * words`
    /// patterns).
    pub words: usize,
    /// Largest number of inputs for which a candidate is checked
    /// exhaustively (at most 16).
    pub max_exhaustive: usize,
    /// Largest cone (in nodes) traversed for a candidate; larger ones are
    /// left to SAT, or skipped without the `formal` feature.
    pub max_cone: usize,
    /// Conflict limit for each SAT call.
    pub sat_conflicts: u64,
    /// Largest number of candidates from one class a node is checked
    /// against before it becomes a class member of its own; this bounds
    /// the proof effort per node.
    pub max_attempts: usize,
}

impl Default for FraigOptions {
    fn default() -> Self {
        FraigOptions {
            seed: 1,
            words: 8,
            max_exhaustive: 12,
            max_cone: 4000,
            sat_conflicts: 1000,
            max_attempts: 3,
        }
    }
}

// ---------------------------------------------------------------------------
// Simulation and candidate classes
// ---------------------------------------------------------------------------

/// Marks a node that belongs to no class (it has been merged away).
const NO_CLASS: u32 = u32::MAX;

/// Bit-parallel simulation signatures of every node of an [`Aig`], and
/// the partition of the nodes into candidate equivalence classes they
/// induce.
///
/// Two nodes share a class when their signatures are equal *or
/// complementary*: each signature is normalised so that the first
/// simulated pattern reads 0, and [`SimClasses::phase`] remembers
/// whether that took a complement. The constant node has the all-zero
/// signature, so a node that simulates to a constant is a candidate for
/// merging into it.
///
/// Simulation uses [`Aig::simulate`], 64 patterns per `u64` word. The
/// random patterns come from a seeded [`Rng`], so the classes (and
/// everything decided from them) are deterministic. Patterns added later
/// by [`SimClasses::refine`] (counter-examples from failed proofs) are
/// packed 64 to a word as well.
#[derive(Clone, Debug)]
pub struct SimClasses {
    /// Simulated values per node, one `Vec` of words each.
    sigs: Vec<Vec<u64>>,
    /// The normalisation phase of each node: true when its first
    /// simulated pattern reads 1.
    phase: Vec<bool>,
    /// The class of each node, or [`NO_CLASS`] once it is merged.
    class: Vec<u32>,
    /// The members of each class, in increasing node order.
    members: Vec<Vec<u32>>,
    /// Patterns already packed into the last refinement word (0 when no
    /// refinement word is open).
    packed: usize,
    /// Number of refinement patterns added.
    refinements: usize,
}

impl SimClasses {
    /// Simulates `aig` over `64 * words` seeded random patterns and
    /// partitions its nodes by normalised signature. Classes are numbered
    /// in order of their smallest member.
    pub fn new(aig: &Aig, seed: u64, words: usize) -> SimClasses {
        let words = words.max(1);
        let mut rng = Rng::new(seed);
        let flat: Vec<u64> = (0..aig.inputs().len() * words)
            .map(|_| rng.next_u64())
            .collect();
        let vals = aig.simulate(&flat, words);
        let sigs: Vec<Vec<u64>> = (0..aig.len())
            .map(|i| vals[i * words..(i + 1) * words].to_vec())
            .collect();
        let phase: Vec<bool> = sigs.iter().map(|s| s[0] & 1 == 1).collect();
        let mut index: HashMap<Vec<u64>, u32> = HashMap::new();
        let mut class = Vec::with_capacity(aig.len());
        let mut members: Vec<Vec<u32>> = Vec::new();
        for (i, sig) in sigs.iter().enumerate() {
            let key: Vec<u64> = if phase[i] {
                sig.iter().map(|w| !w).collect()
            } else {
                sig.clone()
            };
            let next = u32::try_from(members.len()).expect("class count fits u32");
            let c = *index.entry(key).or_insert(next);
            if c == next {
                members.push(Vec::new());
            }
            members[c as usize].push(u32::try_from(i).expect("node index"));
            class.push(c);
        }
        SimClasses {
            sigs,
            phase,
            class,
            members,
            packed: 0,
            refinements: 0,
        }
    }

    /// Whether node `id`'s signature was complemented to normalise it.
    pub fn phase(&self, id: u32) -> bool {
        self.phase[id as usize]
    }

    /// The simulated values of node `id`, one word per 64 patterns.
    pub fn signature(&self, id: u32) -> &[u64] {
        &self.sigs[id as usize]
    }

    /// Number of refinement patterns added so far.
    pub fn refinements(&self) -> usize {
        self.refinements
    }

    /// Number of classes with more than one member: the candidate
    /// equivalences still standing.
    pub fn open_classes(&self) -> usize {
        self.members.iter().filter(|m| m.len() > 1).count()
    }

    /// The candidates for merging node `id`: the members of its class
    /// that come before it, in increasing order, each as the edge `id`
    /// would equal (complemented when the two signatures are).
    pub fn candidates(&self, id: u32) -> Vec<Edge> {
        let c = self.class[id as usize];
        if c == NO_CLASS {
            return Vec::new();
        }
        let p = self.phase(id);
        self.members[c as usize]
            .iter()
            .take_while(|&&m| m < id)
            .map(|&m| Edge::new(m, p ^ self.phase(m)))
            .collect()
    }

    /// True when `id` and `target` agree on every pattern simulated so
    /// far: a necessary condition for them to be equal.
    pub fn agree(&self, id: u32, target: Edge) -> bool {
        let mask = if target.is_complement() { !0 } else { 0 };
        self.sigs[id as usize]
            .iter()
            .zip(&self.sigs[target.index()])
            .all(|(&a, &b)| a == b ^ mask)
    }

    /// Takes `id` out of its class, once it has been merged into another
    /// node and can no longer be anyone's candidate.
    pub fn remove(&mut self, id: u32) {
        let c = std::mem::replace(&mut self.class[id as usize], NO_CLASS);
        if c != NO_CLASS {
            self.members[c as usize].retain(|&m| m != id);
        }
    }

    /// Adds the input pattern `pattern` (one value per primary input, in
    /// [`Aig::inputs`] order) to the simulation and splits every class
    /// whose members it tells apart.
    ///
    /// This is what makes a counter-example useful beyond the pair it
    /// refuted: every other candidate pair it separates is gone too.
    /// Patterns are packed 64 to a word; the open word is re-simulated
    /// over the whole graph each time, one pass of the simulator.
    pub fn refine(&mut self, aig: &Aig, pattern: &[bool]) {
        assert_eq!(pattern.len(), aig.inputs().len(), "one value per input");
        if self.packed == 0 || self.packed == 64 {
            for sig in &mut self.sigs {
                sig.push(0);
            }
            self.packed = 0;
        }
        let bit = 1u64 << self.packed;
        let words: Vec<u64> = aig
            .inputs()
            .iter()
            .zip(pattern)
            .map(|(&pi, &v)| {
                let w = *self.sigs[pi as usize].last().expect("an open word");
                if v { w | bit } else { w & !bit }
            })
            .collect();
        let vals = aig.simulate(&words, 1);
        for (sig, v) in self.sigs.iter_mut().zip(vals) {
            *sig.last_mut().expect("an open word") = v;
        }
        self.packed += 1;
        self.refinements += 1;
        self.split_on_last_word();
    }

    /// The normalised value of node `m` in the last word.
    fn last_word(&self, m: u32) -> u64 {
        let w = *self.sigs[m as usize].last().expect("a word");
        if self.phase(m) { !w } else { w }
    }

    /// Splits every class by its members' normalised values in the last
    /// word; the group of the smallest member keeps the class number, and
    /// new classes are numbered in member order, so the result is
    /// deterministic.
    fn split_on_last_word(&mut self) {
        let classes = self.members.len();
        for c in 0..classes {
            if self.members[c].len() < 2 {
                continue;
            }
            let first = self.last_word(self.members[c][0]);
            if self.members[c].iter().all(|&m| self.last_word(m) == first) {
                continue;
            }
            let old = std::mem::take(&mut self.members[c]);
            let mut groups: HashMap<u64, u32> = HashMap::new();
            groups.insert(first, u32::try_from(c).expect("class index"));
            for m in old {
                let v = self.last_word(m);
                let next = u32::try_from(self.members.len()).expect("class count fits u32");
                let g = *groups.entry(v).or_insert(next);
                if g == next {
                    self.members.push(Vec::new());
                }
                self.members[g as usize].push(m);
                self.class[m as usize] = g;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

/// The outcome of trying to prove a candidate pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The nodes are equivalent (with the candidate's phase). Only a
    /// complete argument may return this: an exhaustive check or an
    /// unsatisfiable SAT query, never simulation.
    Equal,
    /// They are not. The pattern, when there is one, is an input
    /// assignment (one value per primary input) on which they differ;
    /// [`sweep`] adds it to the simulation to split the classes.
    Differ(Option<Vec<bool>>),
    /// Could not be decided within the prover's limits.
    Unknown,
}

/// Decides candidate pairs for [`sweep`].
pub trait Prover {
    /// Called once for every AND node, in topological order, before its
    /// candidates are tried. Returns `Some(edge)` when the node is already
    /// known to equal `edge` without a proof: when, after the merges made
    /// so far, it is the AND of two edges that fold, or that an earlier
    /// node already combines. `edge` must be resolved through `fwd`. The
    /// default knows nothing.
    fn visit(&mut self, aig: &Aig, fwd: &mut Forward, node: u32) -> Option<Edge> {
        let _ = (aig, fwd, node);
        None
    }

    /// Decides whether `node` equals `target`, an edge to an earlier node
    /// that is not itself merged.
    fn prove(&mut self, aig: &Aig, fwd: &mut Forward, node: u32, target: Edge) -> Verdict;
}

/// What [`sweep`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweepCounts {
    /// Nodes merged because [`Prover::visit`] found them structurally
    /// equal to an earlier node.
    pub structural: usize,
    /// Nodes merged on a proof ([`Verdict::Equal`]).
    pub proved: usize,
    /// Candidate pairs refuted ([`Verdict::Differ`]).
    pub disproved: usize,
    /// Candidate pairs left undecided ([`Verdict::Unknown`]).
    pub unknown: usize,
}

impl SweepCounts {
    /// Every node merged, for either reason.
    pub fn merged(&self) -> usize {
        self.structural + self.proved
    }
}

/// Walks the AND nodes of `aig` in topological order and merges each into
/// the first candidate of its class that `prover` proves equal, trying at
/// most `max_attempts` candidates per node. Merges go into `fwd` (which
/// must cover every node of `aig`) and take the node out of `classes`.
///
/// A refuting counter-example refines `classes` on the spot, after which
/// the node's candidates are recomputed: the pattern separates the pair
/// it came from, so the refuted candidate is not proposed again, and a
/// candidate the prover could not decide is not retried for this node.
pub fn sweep(
    aig: &Aig,
    classes: &mut SimClasses,
    fwd: &mut Forward,
    prover: &mut dyn Prover,
    max_attempts: usize,
) -> SweepCounts {
    let mut counts = SweepCounts::default();
    let max_attempts = max_attempts.max(1);
    for id in 0..aig.len() {
        let id = u32::try_from(id).expect("node index");
        if !aig.is_and(id) {
            continue;
        }
        debug_assert!(!fwd.is_replaced(id), "nodes are merged when visited");
        if let Some(edge) = prover.visit(aig, fwd, id) {
            debug_assert!(
                classes.agree(id, edge),
                "structural merge of n{id} into {edge:?} contradicts simulation"
            );
            fwd.set(id, edge);
            classes.remove(id);
            counts.structural += 1;
            continue;
        }
        let mut tried: HashSet<Edge> = HashSet::new();
        'node: loop {
            let fresh: Vec<Edge> = classes
                .candidates(id)
                .into_iter()
                .filter(|e| !tried.contains(e))
                .collect();
            for target in fresh {
                if tried.len() >= max_attempts {
                    break 'node;
                }
                tried.insert(target);
                match prover.prove(aig, fwd, id, target) {
                    Verdict::Equal => {
                        debug_assert!(
                            classes.agree(id, target),
                            "proved merge of n{id} into {target:?} contradicts simulation"
                        );
                        fwd.set(id, target);
                        classes.remove(id);
                        counts.proved += 1;
                        break 'node;
                    }
                    Verdict::Differ(Some(pattern)) => {
                        counts.disproved += 1;
                        classes.refine(aig, &pattern);
                        debug_assert!(
                            !classes.agree(id, target),
                            "a counter-example must separate n{id} from {target:?}"
                        );
                        // The classes changed: start over from the
                        // node's new candidates.
                        continue 'node;
                    }
                    Verdict::Differ(None) => counts.disproved += 1,
                    Verdict::Unknown => counts.unknown += 1,
                }
            }
            break;
        }
    }
    counts
}

// ---------------------------------------------------------------------------
// The optimisation pass
// ---------------------------------------------------------------------------

/// The nodes of the cones of `roots` in topological order and the inputs
/// they depend on, or `None` when the traversal exceeds `max_nodes`.
fn cones(
    aig: &Aig,
    fwd: &mut Forward,
    roots: &[u32],
    max_nodes: usize,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut order = Vec::new();
    let mut inputs = Vec::new();
    let mut visited: HashSet<u32> = HashSet::new();
    visited.insert(0);
    for &root in roots {
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        while let Some((id, expanded)) = stack.pop() {
            if expanded {
                order.push(id);
                continue;
            }
            if !visited.insert(id) {
                continue;
            }
            if !aig.is_and(id) {
                inputs.push(id);
                continue;
            }
            if order.len() + stack.len() > max_nodes {
                return None;
            }
            let (a, b) = aig.fanins(id);
            let a = fwd.resolve(a);
            let b = fwd.resolve(b);
            stack.push((id, true));
            for f in [b.node(), a.node()] {
                if !visited.contains(&f) {
                    stack.push((f, false));
                }
            }
        }
    }
    inputs.sort_unstable();
    Some((order, inputs))
}

/// Simulates `order` (a topological cone) over the given input tables and
/// returns the table of every node in the cone, keyed by node.
fn simulate_cone(
    aig: &Aig,
    fwd: &mut Forward,
    order: &[u32],
    inputs: &HashMap<u32, TruthTable>,
    vars: usize,
) -> HashMap<u32, TruthTable> {
    let mut values: HashMap<u32, TruthTable> = inputs.clone();
    values.insert(0, TruthTable::constant(vars, false));
    for &id in order {
        let (a, b) = aig.fanins(id);
        let a = fwd.resolve(a);
        let b = fwd.resolve(b);
        let ta = values[&a.node()].clone();
        let tb = values[&b.node()].clone();
        let ta = if a.is_complement() { ta.not() } else { ta };
        let tb = if b.is_complement() { tb.not() } else { tb };
        values.insert(id, ta.and(&tb));
    }
    values
}

/// Checks `node` against `target` exhaustively when the cones are small
/// enough.
fn prove_exhaustive(
    aig: &Aig,
    fwd: &mut Forward,
    node: u32,
    target: Edge,
    opts: &FraigOptions,
) -> Option<Verdict> {
    let (order, inputs) = cones(aig, fwd, &[node, target.node()], opts.max_cone)?;
    let vars = inputs.len();
    if vars > opts.max_exhaustive.min(16) {
        return None;
    }
    let tables: HashMap<u32, TruthTable> = inputs
        .iter()
        .enumerate()
        .map(|(i, &pi)| (pi, TruthTable::var(vars, i)))
        .collect();
    let values = simulate_cone(aig, fwd, &order, &tables, vars);
    let tn = values[&node].clone();
    let tt = values[&target.node()].clone();
    let tt = if target.is_complement() { tt.not() } else { tt };
    Some(if tn == tt {
        Verdict::Equal
    } else {
        Verdict::Differ(None)
    })
}

/// Checks `node` against `target` with the SAT solver.
#[cfg(feature = "formal")]
fn prove_sat(
    aig: &Aig,
    fwd: &mut Forward,
    node: u32,
    target: Edge,
    opts: &FraigOptions,
) -> Verdict {
    use crate::formal::sat::{Lit, SolveResult, Solver};

    let Some((order, inputs)) = cones(aig, fwd, &[node, target.node()], usize::MAX) else {
        return Verdict::Unknown;
    };
    let mut solver = Solver::new();
    solver.set_conflict_limit(Some(opts.sat_conflicts));
    let mut vars: HashMap<u32, Lit> = HashMap::new();
    let zero = Lit::pos(solver.new_var());
    solver.add_clause(&[!zero]);
    vars.insert(0, zero);
    for &pi in &inputs {
        vars.insert(pi, Lit::pos(solver.new_var()));
    }
    let lit_of = |vars: &HashMap<u32, Lit>, e: Edge| -> Lit {
        let l = vars[&e.node()];
        if e.is_complement() { !l } else { l }
    };
    for &id in &order {
        let (a, b) = aig.fanins(id);
        let a = lit_of(&vars, fwd.resolve(a));
        let b = lit_of(&vars, fwd.resolve(b));
        let y = Lit::pos(solver.new_var());
        solver.add_clause(&[!y, a]);
        solver.add_clause(&[!y, b]);
        solver.add_clause(&[y, !a, !b]);
        vars.insert(id, y);
    }
    let n = lit_of(&vars, Edge::plain(node));
    let t = lit_of(&vars, target);
    for assumptions in [[n, !t], [!n, t]] {
        match solver.solve_with_assumptions(&assumptions) {
            SolveResult::Unsat => {}
            // A satisfying assignment is an input pattern on which the
            // two cones disagree, so they are not equivalent.
            SolveResult::Sat => return Verdict::Differ(None),
            SolveResult::Unknown => return Verdict::Unknown,
        }
    }
    Verdict::Equal
}

#[cfg(not(feature = "formal"))]
fn prove_sat(
    _aig: &Aig,
    _fwd: &mut Forward,
    _node: u32,
    _target: Edge,
    _opts: &FraigOptions,
) -> Verdict {
    Verdict::Unknown
}

/// The prover of [`fraig`]: exhaustive simulation of small cones, a
/// fresh SAT solver per pair for the rest (with the `formal` feature).
struct ConeProver<'o> {
    opts: &'o FraigOptions,
}

impl Prover for ConeProver<'_> {
    fn prove(&mut self, aig: &Aig, fwd: &mut Forward, node: u32, target: Edge) -> Verdict {
        match prove_exhaustive(aig, fwd, node, target, self.opts) {
            Some(v) => v,
            None => prove_sat(aig, fwd, node, target, self.opts),
        }
    }
}

/// Functionally reduces `aig`; returns the number of nodes merged.
pub fn fraig(aig: &mut Aig, opts: &FraigOptions) -> usize {
    let mut classes = SimClasses::new(aig, opts.seed, opts.words);
    let mut fwd = Forward::identity(aig.len());
    let counts = sweep(
        aig,
        &mut classes,
        &mut fwd,
        &mut ConeProver { opts },
        opts.max_attempts,
    );
    let merged = counts.merged();
    if merged > 0 {
        *aig = aig.rebuild_with(&mut fwd);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_structurally_different_equivalents() {
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let c = g.add_input();
        // xor(a, b) written two ways, and a constant-false pair.
        let x1 = g.xor(a, b);
        let t = g.and(a, b);
        let u = g.or(a, b);
        let x2 = g.and(u, !t);
        let f = g.and(x1, !x2); // always false
        let out = g.or(x1, c);
        let out2 = g.or(x2, c);
        g.add_output(out);
        g.add_output(out2);
        g.add_output(f);
        let reference = g.clone();
        let merged = fraig(&mut g, &FraigOptions::default());
        assert!(merged >= 2, "merged {merged}");
        assert_eq!(g.outputs()[0], g.outputs()[1]);
        assert_eq!(g.outputs()[2], Edge::FALSE);
        for pat in 0..8u32 {
            let bits: Vec<bool> = (0..3).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(g.eval(&bits), reference.eval(&bits));
        }
    }

    #[test]
    fn false_candidates_are_refuted() {
        // Two functions equal on all but one of 2^4 patterns: random
        // simulation may put them in one class, proof must separate them.
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..4).map(|_| g.add_input()).collect();
        let all = g.and_n(&ins);
        let three = g.and_n(&ins[..3]);
        g.add_output(all);
        g.add_output(three);
        let reference = g.clone();
        let opts = FraigOptions {
            words: 1,
            ..FraigOptions::default()
        };
        fraig(&mut g, &opts);
        for pat in 0..16u32 {
            let bits: Vec<bool> = (0..4).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(g.eval(&bits), reference.eval(&bits));
        }
        assert_ne!(g.outputs()[0], g.outputs()[1]);
    }

    #[test]
    fn wide_cones_use_sat_or_are_skipped() {
        // 8-bit equality two ways: bit-wise comparison, versus "neither
        // operand is less than the other". Sixteen inputs is past the
        // exhaustive limit, and the two cones share no structure, so only
        // the SAT proof can merge them.
        let mut g = Aig::new();
        let a: Vec<Edge> = (0..8).map(|_| g.add_input()).collect();
        let b: Vec<Edge> = (0..8).map(|_| g.add_input()).collect();
        let e1 = g.eq_bits(&a, &b);
        let lt = g.lt_bits(&a, &b, false);
        let gt = g.lt_bits(&b, &a, false);
        let e2 = g.and(!lt, !gt);
        g.add_output(e1);
        g.add_output(e2);
        let reference = g.clone();
        let merged = fraig(&mut g, &FraigOptions::default());
        // Either way the small cones inside the two comparators merge;
        // what needs the SAT proof is the equivalence of the two roots,
        // whose cone spans all sixteen inputs.
        assert!(merged > 0, "nothing merged at all");
        if cfg!(feature = "formal") {
            assert_eq!(
                g.outputs()[0],
                g.outputs()[1],
                "the roots should have been proven equivalent"
            );
        } else {
            assert_ne!(
                g.outputs()[0],
                g.outputs()[1],
                "without a SAT solver the wide roots cannot be merged"
            );
        }
        let mut rng = Rng::new(9);
        for _ in 0..50 {
            let w = rng.next_u64();
            let bits: Vec<bool> = (0..16).map(|i| (w >> i) & 1 == 1).collect();
            assert_eq!(g.eval(&bits), reference.eval(&bits));
        }
    }
}
