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
//! 2. **Proof**, for each node against the representative of its class,
//!    in topological order. When the union of the two cones depends on at
//!    most [`FraigOptions::max_exhaustive`] inputs it is checked
//!    exhaustively by simulating every pattern. Otherwise, with the
//!    `formal` feature, the two cones are encoded in CNF (Tseitin) and the
//!    crate's SAT solver is asked for a distinguishing input under a
//!    conflict limit; a counter-example is added to the simulation vectors
//!    so it also separates other false candidates, as in ABC's `fraig`.
//!    Without the feature, large candidates are left alone.
//!
//! Proven merges are recorded in a [`Forward`] map and applied with
//! [`Aig::rebuild_with`] at the end.

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

/// The signature of a node normalised so that its complement has the same
/// key; the flag says whether it was complemented.
fn normalise(sig: &[u64]) -> (Vec<u64>, bool) {
    if sig[0] & 1 == 1 {
        (sig.iter().map(|w| !w).collect(), true)
    } else {
        (sig.to_vec(), false)
    }
}

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

/// The outcome of trying to prove a candidate.
enum Verdict {
    /// The nodes are equivalent (with the candidate's phase).
    Equal,
    /// They are not: the candidate class splits here.
    Differ,
    /// Could not be decided within the limits.
    Unknown,
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
        Verdict::Differ
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
            SolveResult::Sat => return Verdict::Differ,
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

/// Functionally reduces `aig`; returns the number of nodes merged.
pub fn fraig(aig: &mut Aig, opts: &FraigOptions) -> usize {
    let n = aig.len();
    let num_inputs = aig.inputs().len();
    let words = opts.words.max(1);
    let mut rng = Rng::new(opts.seed);
    let patterns: Vec<Vec<u64>> = (0..num_inputs)
        .map(|_| (0..words).map(|_| rng.next_u64()).collect())
        .collect();
    let sig = simulate_all(aig, &patterns, words);
    let mut fwd = Forward::identity(n);
    // A candidate class holds every node with the same signature, not just
    // a representative: when a candidate is disproved the class has to
    // split, so the next node with that signature must still be able to
    // try the other members.
    let mut classes: HashMap<Vec<u64>, Vec<u32>> = HashMap::new();
    let mut merged = 0;

    for id in 0..n {
        let id = u32::try_from(id).expect("node index");
        if fwd.is_replaced(id) {
            continue;
        }
        let (key, phase) = normalise(&sig[id as usize]);
        let members: Vec<u32> = classes.get(&key).cloned().unwrap_or_default();
        if !aig.is_and(id) {
            // Inputs and the constant are always their own representative.
            classes.entry(key).or_default().push(id);
            continue;
        }
        let mut proved = false;
        for &m in members.iter().take(opts.max_attempts.max(1)) {
            let (_, m_phase) = normalise(&sig[m as usize]);
            let target = Edge::new(m, phase ^ m_phase);
            let verdict = match prove_exhaustive(aig, &mut fwd, id, target, opts) {
                Some(v) => v,
                None => prove_sat(aig, &mut fwd, id, target, opts),
            };
            match verdict {
                Verdict::Equal => {
                    fwd.set(id, target);
                    merged += 1;
                    proved = true;
                    break;
                }
                // The signature did not separate them but the function
                // does (or the proof gave up): try the next member.
                Verdict::Differ | Verdict::Unknown => {}
            }
        }
        if !proved {
            classes.entry(key).or_default().push(id);
        }
    }
    if merged > 0 {
        *aig = aig.rebuild_with(&mut fwd);
    }
    merged
}

/// Simulates the whole graph and returns one signature per node.
fn simulate_all(aig: &Aig, patterns: &[Vec<u64>], words: usize) -> Vec<Vec<u64>> {
    let flat: Vec<u64> = patterns.iter().flat_map(|p| p.iter().copied()).collect();
    let vals = aig.simulate(&flat, words);
    (0..aig.len())
        .map(|i| vals[i * words..(i + 1) * words].to_vec())
        .collect()
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
