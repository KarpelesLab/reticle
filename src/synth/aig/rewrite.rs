//! DAG-aware AIG rewriting with 4-input cuts.
//!
//! This is the algorithm of Mishchenko, Chatterjee and Brayton, "DAG-aware
//! AIG rewriting: a fresh look at combinational logic synthesis" (DAC
//! 2006), as implemented by ABC's `rewrite`: for every node, enumerate its
//! cuts of up to four leaves, compute the function of each cut, look up a
//! good implementation of that function in a precomputed [`Library`], and
//! replace the node's maximum fanout-free cone by that implementation when
//! doing so removes more nodes than it adds. Existing nodes found through
//! the structural hash table cost nothing, which is what makes the
//! rewriting "DAG-aware": a candidate that shares logic with the rest of
//! the graph is preferred over one that does not.
//!
//! # The library
//!
//! ABC ships a table of the 222 NPN classes of 4-input functions with the
//! best AIG structures found by exhaustive enumeration. Reticle computes
//! its table at first use instead, and indexes it by the full 16-bit
//! truth table so no canonicalisation is needed on the hot path. It is
//! built in two stages:
//!
//! 1. **Waves.** A dynamic programme over all 65 536 functions finds, in
//!    order of increasing cost, the cheapest way to write each function as
//!    `g & h` (one node) or `g ^ h` (three nodes) of functions already
//!    known, with inversions free. This is exhaustive and therefore
//!    optimal up to the cost it runs to ([`MAX_COST`], bounded by a budget
//!    of combinations so generation stays well under a second).
//! 2. **Decomposition.** The functions that need more nodes than the waves
//!    bought are filled in by splitting on one variable: `f` is an AND
//!    with a literal when a cofactor is constant, an XOR with the variable
//!    when the cofactors are complementary, and a multiplexer otherwise.
//!    Relaxing this until it stops improving guarantees every function has
//!    an entry.
//!
//! Recorded costs are tree costs and so overestimate the multiplexer
//! entries, whose two arms usually share logic. That does not affect the
//! rewriting decision, which measures the real number of added nodes
//! against the hash table ([`Counter`]) and uses the recorded cost only to
//! confirm an entry exists. The table is deterministic and shared by every
//! pass in the process.

use std::sync::OnceLock;

use super::cut::{cone_truth, enumerate_cuts};
use super::mffc::{Builder, Counter, Synth, View};
use super::{Aig, Edge};

/// The 16-bit truth tables of the four variables.
const VARS: [u16; 4] = [0xAAAA, 0xCCCC, 0xF0F0, 0xFF00];

/// Cost of a function not reached by the generation.
const UNREACHED: u8 = u8::MAX;

/// Largest cost the exhaustive wave generation attempts; beyond it the
/// decomposition fill takes over.
pub const MAX_COST: usize = 10;

/// Budget of `(g, h)` combinations for the wave generation, which stops
/// before a wave that would exceed it.
const BUDGET: usize = 60_000_000;

/// How a function is built from cheaper ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// The constant false (the representative of both constants).
    Const,
    /// A variable.
    Var(u8),
    /// `a & b` for the two stored functions (either polarity of the result).
    And(u16, u16),
    /// `a ^ b`.
    Xor(u16, u16),
    /// `v ? t : e` for variable `v` and the two stored functions, three
    /// AND nodes.
    Mux(u8, u16, u16),
}

/// The cofactors of `f` with variable `v` set and cleared, as functions of
/// all four variables (they no longer depend on `v`).
fn cofactors(f: u16, v: u8) -> (u16, u16) {
    let mask = VARS[usize::from(v)];
    let shift = 1u32 << v;
    let hi = f & mask;
    let lo = f & !mask;
    (hi | (hi >> shift), lo | (lo << shift))
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    cost: u8,
    op: Op,
}

/// The table of good implementations for every 4-input function.
pub struct Library {
    entries: Vec<Entry>,
    /// Number of functions with an implementation.
    pub reached: usize,
    /// The largest cost assigned.
    pub max_cost: u8,
}

/// The representative of a function and its complement: the one whose
/// value for pattern 0 is false.
fn rep(tt: u16) -> u16 {
    if tt & 1 == 0 { tt } else { !tt }
}

/// The recorded cost of `tt`, or `None` when it has no implementation.
fn cost_of(entries: &[Entry], tt: u16) -> Option<u8> {
    let c = entries[tt as usize].cost;
    (c != UNREACHED).then_some(c)
}

impl Library {
    /// The process-wide table, generated on first use.
    pub fn get() -> &'static Library {
        static LIB: OnceLock<Library> = OnceLock::new();
        LIB.get_or_init(|| Library::generate(BUDGET))
    }

    /// Generates the table with the given combination budget.
    pub fn generate(budget: usize) -> Library {
        let mut entries = vec![
            Entry {
                cost: UNREACHED,
                op: Op::Const,
            };
            1 << 16
        ];
        let mut waves: Vec<Vec<u16>> = vec![Vec::new(); MAX_COST + 1];
        let set =
            |entries: &mut Vec<Entry>, waves: &mut Vec<Vec<u16>>, tt: u16, cost: usize, op: Op| {
                let r = rep(tt);
                if entries[r as usize].cost == UNREACHED {
                    let c = u8::try_from(cost).expect("cost fits");
                    entries[r as usize] = Entry { cost: c, op };
                    entries[(!r) as usize] = Entry { cost: c, op };
                    waves[cost].push(r);
                }
            };
        set(&mut entries, &mut waves, 0, 0, Op::Const);
        for (v, &tt) in VARS.iter().enumerate() {
            set(
                &mut entries,
                &mut waves,
                tt,
                0,
                Op::Var(u8::try_from(v).expect("var")),
            );
        }
        let mut spent = 0usize;
        let mut max_cost = 0u8;
        for cost in 1..=MAX_COST {
            let pairs = |total: usize| -> usize {
                (0..=total)
                    .filter(|&i| i <= total - i)
                    .map(|i| waves[i].len() * waves[total - i].len())
                    .sum()
            };
            let mut planned = pairs(cost - 1);
            if cost >= 3 {
                planned += pairs(cost - 3);
            }
            if spent + planned > budget {
                break;
            }
            spent += planned;
            let mut new_reps: Vec<(u16, Op)> = Vec::new();
            // AND combinations: cost(g) + cost(h) + 1 == cost.
            for i in 0..cost {
                let j = cost - 1 - i;
                if i > j {
                    continue;
                }
                for (gi, &g) in waves[i].iter().enumerate() {
                    let start = if i == j { gi } else { 0 };
                    for &h in &waves[j][start..] {
                        for (pg, ph) in [(false, false), (false, true), (true, false), (true, true)]
                        {
                            let a = if pg { !g } else { g };
                            let b = if ph { !h } else { h };
                            let f = a & b;
                            if entries[rep(f) as usize].cost == UNREACHED {
                                new_reps.push((f, Op::And(a, b)));
                            }
                        }
                    }
                }
            }
            // XOR combinations: cost(g) + cost(h) + 3 == cost.
            if cost >= 3 {
                for i in 0..=(cost - 3) {
                    let j = cost - 3 - i;
                    if i > j {
                        continue;
                    }
                    for (gi, &g) in waves[i].iter().enumerate() {
                        let start = if i == j { gi } else { 0 };
                        for &h in &waves[j][start..] {
                            let f = g ^ h;
                            if entries[rep(f) as usize].cost == UNREACHED {
                                new_reps.push((f, Op::Xor(g, h)));
                            }
                        }
                    }
                }
            }
            for (f, op) in new_reps {
                set(&mut entries, &mut waves, f, cost, op);
            }
            max_cost = u8::try_from(cost).expect("cost fits");
        }
        // The waves stop at the budget, so the functions that need more
        // nodes than it bought are still missing. Decomposition on a
        // single variable closes the gap cheaply: writing `f` through its
        // cofactors `f|v=1` and `f|v=0`, which are functions of the other
        // three variables and are always reached, costs one AND node when
        // a cofactor is constant, three when the cofactors are
        // complementary (an XOR with the variable), and three plus both
        // cofactors otherwise (a multiplexer). Relaxing until nothing
        // changes also improves entries whose cofactors got cheaper in a
        // later round.
        loop {
            let mut changed = false;
            for f in 0..=u16::MAX {
                if f != rep(f) {
                    continue;
                }
                let mut best: Option<(u8, Op)> = None;
                let mut offer = |cost: Option<u8>, extra: u8, op: Op| {
                    let Some(total) = cost.and_then(|c| c.checked_add(extra)) else {
                        return;
                    };
                    if best.is_none_or(|(b, _)| total < b) {
                        best = Some((total, op));
                    }
                };
                for v in 0..4u8 {
                    let mask = VARS[usize::from(v)];
                    let (t, e) = cofactors(f, v);
                    if t == e {
                        // `f` does not depend on `v`; nothing to gain.
                        continue;
                    }
                    if t == !e {
                        // f = v ^ (f|v=0), three nodes on top.
                        offer(cost_of(&entries, e), 3, Op::Xor(mask, e));
                    } else if t == 0 {
                        // f = !v & (f|v=0).
                        offer(cost_of(&entries, e), 1, Op::And(!mask, e));
                    } else if t == !0 {
                        // f = v | (f|v=0) = !(!v & !(f|v=0)).
                        offer(cost_of(&entries, e), 1, Op::And(!mask, !e));
                    } else if e == 0 {
                        offer(cost_of(&entries, t), 1, Op::And(mask, t));
                    } else if e == !0 {
                        offer(cost_of(&entries, t), 1, Op::And(mask, !t));
                    } else {
                        let total = cost_of(&entries, t)
                            .zip(cost_of(&entries, e))
                            .and_then(|(a, b)| a.checked_add(b));
                        offer(total, 3, Op::Mux(v, t, e));
                    }
                }
                if let Some((cost, op)) = best
                    && cost < entries[f as usize].cost
                {
                    entries[f as usize] = Entry { cost, op };
                    entries[(!f) as usize] = Entry { cost, op };
                    max_cost = max_cost.max(cost);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let reached = entries.iter().filter(|e| e.cost != UNREACHED).count();
        Library {
            entries,
            reached,
            max_cost,
        }
    }

    /// The tree cost (AND count) of the stored implementation of `tt`, or
    /// `None` when the function was not reached.
    pub fn cost(&self, tt: u16) -> Option<u8> {
        let c = self.entries[tt as usize].cost;
        (c != UNREACHED).then_some(c)
    }

    /// Builds the stored implementation of `tt` over the leaves of `s`
    /// (variable `i` is leaf `i`).
    ///
    /// # Panics
    ///
    /// If `tt` was not reached; check [`Library::cost`] first.
    pub fn synthesize<S: Synth>(&self, tt: u16, s: &mut S) -> S::Sig {
        let mut memo: std::collections::HashMap<u16, S::Sig> = std::collections::HashMap::new();
        self.build(tt, s, &mut memo)
    }

    fn build<S: Synth>(
        &self,
        tt: u16,
        s: &mut S,
        memo: &mut std::collections::HashMap<u16, S::Sig>,
    ) -> S::Sig {
        let r = rep(tt);
        if let Some(&sig) = memo.get(&r) {
            return if tt == r { sig } else { s.not(sig) };
        }
        let entry = self.entries[r as usize];
        assert!(
            entry.cost != UNREACHED,
            "function {tt:#06x} is not in the library"
        );
        let sig = match entry.op {
            Op::Const => s.constant(false),
            Op::Var(v) => s.leaf(usize::from(v)),
            Op::And(a, b) => {
                let x = self.build(a, s, memo);
                let y = self.build(b, s, memo);
                let sig = s.and(x, y);
                if a & b == r { sig } else { s.not(sig) }
            }
            Op::Xor(a, b) => {
                let x = self.build(a, s, memo);
                let y = self.build(b, s, memo);
                let sig = s.xor(x, y);
                if a ^ b == r { sig } else { s.not(sig) }
            }
            Op::Mux(v, t, e) => {
                let sel = s.leaf(usize::from(v));
                let x = self.build(t, s, memo);
                let y = self.build(e, s, memo);
                let hi = s.and(sel, x);
                let lo = s.and(s.not(sel), y);
                s.or(hi, lo)
            }
        };
        memo.insert(r, sig);
        if tt == r { sig } else { s.not(sig) }
    }
}

/// Largest number of cuts examined per node.
const CUT_LIMIT: usize = 32;

/// One rewriting pass over every node, in topological order. With
/// `zero_cost`, replacements that neither add nor remove nodes are applied
/// too (ABC's `rewrite -z`), which changes the structure so later passes
/// find new opportunities. Returns the number of nodes rewritten.
pub fn rewrite(aig: &mut Aig, zero_cost: bool) -> usize {
    let lib = Library::get();
    let mut view = View::new(aig);
    let count = u32::try_from(view.old_len).expect("node count");
    let mut applied = 0;
    for id in 1..count {
        if !view.aig.is_and(id) || view.fwd.is_replaced(id) || view.aig.refs(id) == 0 {
            continue;
        }
        let cuts = enumerate_cuts(&mut |x| view.fanins(x), id, 4, CUT_LIMIT);
        let mut best: Option<(i64, Vec<u32>, u16)> = None;
        for cut in cuts {
            let tt = cone_truth(&mut |x| view.fanins(x), id, &cut, 4).as_u64();
            let tt = u16::try_from(tt & 0xFFFF).expect("16 bits");
            if lib.cost(tt).is_none() {
                continue;
            }
            let saved = view.deref_cone(id, &cut);
            let leaves: Vec<Edge> = cut.iter().map(|&l| Edge::plain(l)).collect();
            let mut counter = Counter::new(&view, &leaves, id);
            lib.synthesize(tt, &mut counter);
            let hit_root = counter.hit_root;
            let added = i64::from(counter.added);
            view.ref_cone(id, &cut);
            if hit_root {
                continue;
            }
            let gain = i64::try_from(saved).expect("count") - added;
            let threshold = best
                .as_ref()
                .map_or(if zero_cost { -1 } else { 0 }, |b| b.0);
            if gain > threshold {
                best = Some((gain, cut, tt));
            }
        }
        let Some((_, cut, tt)) = best else {
            continue;
        };
        view.deref_cone(id, &cut);
        let leaves: Vec<Edge> = cut.iter().map(|&l| Edge::plain(l)).collect();
        let mut builder = Builder::new(&mut view, leaves, id);
        let e = lib.synthesize(tt, &mut builder);
        if !view.replace(id, e) {
            // The new structure is the node itself; keep what is there.
            view.ref_cone(id, &cut);
            continue;
        }
        applied += 1;
    }
    let mut fwd = view.fwd.clone();
    *aig = aig.rebuild_with(&mut fwd);
    applied
}

#[cfg(test)]
mod tests {
    use super::super::truth::{TruthTable, next_permutation, npn_canonical};
    use super::*;

    #[test]
    fn library_covers_the_common_functions() {
        let lib = Library::get();
        // The Shannon fill guarantees every function has an entry.
        assert_eq!(lib.reached, 1 << 16);
        assert_eq!(lib.cost(0x0000), Some(0));
        assert_eq!(lib.cost(0xFFFF), Some(0));
        assert_eq!(lib.cost(0xAAAA), Some(0));
        assert_eq!(lib.cost(0x8888), Some(1)); // a & b
        assert_eq!(lib.cost(0x6666), Some(3)); // a ^ b
        assert_eq!(lib.cost(0x9696), Some(6)); // a ^ b ^ c
        assert_eq!(lib.cost(0x6996), Some(9)); // a ^ b ^ c ^ d
        assert_eq!(lib.cost(0xCACA), Some(3)); // mux(c, b, a)
        assert_eq!(lib.cost(0xE8E8), Some(4)); // majority
        // The waves are exhaustive up to their cost, so nothing below it
        // is missed, and no entry is unreasonably large.
        assert!(lib.max_cost <= 16, "max cost {}", lib.max_cost);
        let filled = (0..=u16::MAX)
            .filter(|&tt| lib.cost(tt).is_some_and(|c| usize::from(c) > MAX_COST))
            .count();
        assert!(filled < 1000, "{filled} functions above the wave cost");
    }

    /// The 4-input functions fall into 222 NPN equivalence classes, and
    /// the library covers every one of them. The classes are found by
    /// marking whole orbits rather than canonicalising each function,
    /// which keeps the check fast.
    #[test]
    fn library_covers_every_npn_class() {
        let lib = Library::get();
        let mut class_of = vec![u16::MAX; 1 << 16];
        let mut classes = 0u16;
        for tt in 0..=u16::MAX {
            if class_of[usize::from(tt)] != u16::MAX {
                continue;
            }
            let table = TruthTable::from_u64(4, u64::from(tt));
            let mut perm = [0usize, 1, 2, 3];
            loop {
                for neg in 0..16u32 {
                    let image = table.negate_inputs(neg).permute(&perm);
                    for out_neg in [false, true] {
                        let word = if out_neg {
                            image.not().as_u64()
                        } else {
                            image.as_u64()
                        };
                        let word = u16::try_from(word & 0xFFFF).expect("16 bits");
                        class_of[usize::from(word)] = classes;
                    }
                }
                if !next_permutation(&mut perm) {
                    break;
                }
            }
            assert!(lib.cost(tt).is_some(), "class of {tt:#06x} has no entry");
            classes += 1;
        }
        assert_eq!(classes, 222);
        // The canonical form agrees with the orbit partition.
        for tt in [0x8888u16, 0x6996, 0xE8E8, 0x1234] {
            let (c, perm, neg, out_neg) = npn_canonical(&TruthTable::from_u64(4, u64::from(tt)));
            let t = TruthTable::from_u64(4, u64::from(tt))
                .negate_inputs(neg)
                .permute(&perm);
            let t = if out_neg { t.not() } else { t };
            assert_eq!(t, c);
            let canon = u16::try_from(c.as_u64() & 0xFFFF).expect("16 bits");
            assert_eq!(class_of[usize::from(tt)], class_of[usize::from(canon)]);
        }
    }

    /// Evaluates a structure directly as 16-bit truth tables, so every
    /// entry of the library can be checked cheaply.
    struct TtSynth;

    impl Synth for TtSynth {
        type Sig = u16;

        fn leaf(&self, i: usize) -> u16 {
            VARS[i]
        }

        fn constant(&self, value: bool) -> u16 {
            if value { !0 } else { 0 }
        }

        fn not(&self, s: u16) -> u16 {
            !s
        }

        fn and(&mut self, a: u16, b: u16) -> u16 {
            a & b
        }
    }

    #[test]
    fn every_library_entry_computes_its_function() {
        let lib = Library::get();
        for tt in 0..=u16::MAX {
            assert_eq!(lib.synthesize(tt, &mut TtSynth), tt, "tt {tt:#06x}");
        }
    }

    #[test]
    fn library_structures_compute_their_function() {
        let lib = Library::get();
        for tt in [
            0x0000u16, 0xFFFF, 0x8888, 0x6666, 0x9696, 0xCACA, 0xE8E8, 0x1234, 0xFEDC, 0x0001,
        ] {
            if lib.cost(tt).is_none() {
                continue;
            }
            let mut g = Aig::new();
            let leaves: Vec<Edge> = (0..4).map(|_| g.add_input()).collect();
            let mut view = View::new(&mut g);
            let mut b = Builder::new(&mut view, leaves, 0);
            let e = lib.synthesize(tt, &mut b);
            g.add_output(e);
            for pat in 0..16u32 {
                let ins: Vec<bool> = (0..4).map(|i| (pat >> i) & 1 == 1).collect();
                assert_eq!(
                    g.eval(&ins)[0],
                    (tt >> pat) & 1 == 1,
                    "tt {tt:#06x} pattern {pat}"
                );
            }
        }
    }

    #[test]
    fn rewriting_shrinks_and_preserves_function() {
        // A redundant implementation of a ^ b ^ c: two xor3 structures
        // built differently and OR'd together.
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let c = g.add_input();
        let x1 = g.xor(a, b);
        let x1 = g.xor(x1, c);
        let t = g.xor(b, c);
        let x2 = g.xor(t, a);
        let y = g.or(x1, x2);
        let m = g.mux(a, b, c);
        let m2 = g.and(m, y);
        g.add_output(y);
        g.add_output(m2);
        let before = g.num_ands();
        let reference = g.clone();
        rewrite(&mut g, false);
        rewrite(&mut g, true);
        assert!(g.num_ands() < before, "{} -> {}", before, g.num_ands());
        for pat in 0..8u32 {
            let ins: Vec<bool> = (0..3).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(g.eval(&ins), reference.eval(&ins), "pattern {pat}");
        }
    }
}
