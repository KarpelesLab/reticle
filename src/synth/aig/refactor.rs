//! Refactoring: collapse a large fanout-free cone and re-synthesise it.
//!
//! Where rewriting looks at cuts of four leaves, refactoring takes one
//! reconvergence-driven cut of up to a dozen leaves around each node,
//! computes the function of the cone as a truth table, derives an
//! irredundant sum of products with the Minato-Morreale ISOP algorithm
//! (S. Minato, "Fast generation of prime-irredundant covers from binary
//! decision diagrams", IEICE Trans. 1993), factors that cover
//! algebraically by repeatedly dividing out the most frequent literal, and
//! builds the factored form back into the graph. The replacement is kept
//! when it removes more nodes than it adds, counted through the hash table
//! like rewriting. This is a simpler cousin of ABC's `refactor`, which
//! uses the same cut and ISOP but a stronger factoring.

use super::cut::{cone_truth, reconvergent_cut};
use super::mffc::{Builder, Counter, Synth, View};
use super::truth::TruthTable;
use super::{Aig, Edge};

/// A product term: the variables appearing positively and negatively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cube {
    /// Bit `i` set: variable `i` appears as itself.
    pub pos: u32,
    /// Bit `i` set: variable `i` appears complemented.
    pub neg: u32,
}

impl Cube {
    /// The empty product (constant true).
    pub const ONE: Cube = Cube { pos: 0, neg: 0 };

    /// True when the cube contains literal `(v, negated)`.
    pub fn has(self, v: usize, negated: bool) -> bool {
        let mask = if negated { self.neg } else { self.pos };
        (mask >> v) & 1 == 1
    }

    /// The cube without literal `(v, negated)`.
    pub fn without(self, v: usize, negated: bool) -> Cube {
        if negated {
            Cube {
                pos: self.pos,
                neg: self.neg & !(1 << v),
            }
        } else {
            Cube {
                pos: self.pos & !(1 << v),
                neg: self.neg,
            }
        }
    }

    /// Number of literals.
    pub fn len(self) -> u32 {
        self.pos.count_ones() + self.neg.count_ones()
    }

    /// True for the empty product.
    pub fn is_empty(self) -> bool {
        self.pos == 0 && self.neg == 0
    }
}

/// An irredundant sum-of-products cover of `on` (Minato-Morreale ISOP),
/// as cubes over the table's variables.
pub fn isop(on: &TruthTable) -> Vec<Cube> {
    let vars = on.vars();
    assert!(vars <= 32, "too many variables for a cube mask");
    isop_rec(on, on, vars).0
}

fn isop_rec(on: &TruthTable, upper: &TruthTable, vars_left: usize) -> (Vec<Cube>, TruthTable) {
    if on.is_zero() {
        return (Vec::new(), TruthTable::constant(on.vars(), false));
    }
    if upper.is_ones() {
        return (vec![Cube::ONE], TruthTable::constant(on.vars(), true));
    }
    assert!(vars_left > 0, "on-set not covered by its upper bound");
    let x = vars_left - 1;
    let on0 = on.cofactor(x, false);
    let on1 = on.cofactor(x, true);
    let up0 = upper.cofactor(x, false);
    let up1 = upper.cofactor(x, true);
    let (c0, t0) = isop_rec(&on0.and(&up1.not()), &up0, x);
    let (c1, t1) = isop_rec(&on1.and(&up0.not()), &up1, x);
    let rest_on = on0.and(&t0.not()).or(&on1.and(&t1.not()));
    let (c2, t2) = isop_rec(&rest_on, &up0.and(&up1), x);
    let mut cubes = Vec::with_capacity(c0.len() + c1.len() + c2.len());
    for c in c0 {
        cubes.push(Cube {
            pos: c.pos,
            neg: c.neg | (1 << x),
        });
    }
    for c in c1 {
        cubes.push(Cube {
            pos: c.pos | (1 << x),
            neg: c.neg,
        });
    }
    cubes.extend(c2);
    let xv = TruthTable::var(on.vars(), x);
    let tt = t0.and(&xv.not()).or(&t1.and(&xv)).or(&t2);
    (cubes, tt)
}

/// The truth table of a cover, for checking.
pub fn cover_truth(cubes: &[Cube], vars: usize) -> TruthTable {
    let mut acc = TruthTable::constant(vars, false);
    for cube in cubes {
        let mut term = TruthTable::constant(vars, true);
        for v in 0..vars {
            if cube.has(v, false) {
                term = term.and(&TruthTable::var(vars, v));
            }
            if cube.has(v, true) {
                term = term.and(&TruthTable::var(vars, v).not());
            }
        }
        acc = acc.or(&term);
    }
    acc
}

/// Builds a factored form of the cover through `s`: the most frequent
/// literal is divided out recursively (`f = l · q + r`), products are
/// balanced AND trees, sums balanced OR trees.
pub fn build_cover<S: Synth>(cubes: &[Cube], vars: usize, s: &mut S) -> S::Sig {
    if cubes.is_empty() {
        return s.constant(false);
    }
    if cubes.len() == 1 {
        return build_cube(cubes[0], vars, s);
    }
    // Most frequent literal.
    let mut best: Option<(usize, usize, bool)> = None;
    for v in 0..vars {
        for negated in [false, true] {
            let count = cubes.iter().filter(|c| c.has(v, negated)).count();
            if count >= 2 && best.is_none_or(|(c, _, _)| count > c) {
                best = Some((count, v, negated));
            }
        }
    }
    match best {
        Some((_, v, negated)) => {
            let quotient: Vec<Cube> = cubes
                .iter()
                .filter(|c| c.has(v, negated))
                .map(|c| c.without(v, negated))
                .collect();
            let remainder: Vec<Cube> = cubes
                .iter()
                .filter(|c| !c.has(v, negated))
                .copied()
                .collect();
            let lit = s.leaf(v);
            let lit = if negated { s.not(lit) } else { lit };
            let q = build_cover(&quotient, vars, s);
            let t = s.and(lit, q);
            if remainder.is_empty() {
                t
            } else {
                let r = build_cover(&remainder, vars, s);
                s.or(t, r)
            }
        }
        None => {
            let terms: Vec<S::Sig> = cubes.iter().map(|&c| build_cube(c, vars, s)).collect();
            tree(&terms, s, |s, a, b| s.or(a, b))
        }
    }
}

fn build_cube<S: Synth>(cube: Cube, vars: usize, s: &mut S) -> S::Sig {
    let mut lits = Vec::new();
    for v in 0..vars {
        if cube.has(v, false) {
            lits.push(s.leaf(v));
        }
        if cube.has(v, true) {
            let l = s.leaf(v);
            lits.push(s.not(l));
        }
    }
    if lits.is_empty() {
        return s.constant(true);
    }
    tree(&lits, s, |s, a, b| s.and(a, b))
}

fn tree<S: Synth>(items: &[S::Sig], s: &mut S, op: fn(&mut S, S::Sig, S::Sig) -> S::Sig) -> S::Sig {
    match items {
        [x] => *x,
        _ => {
            let (l, r) = items.split_at(items.len() / 2);
            let a = tree(l, s, op);
            let b = tree(r, s, op);
            op(s, a, b)
        }
    }
}

/// One refactoring pass over every node with cuts of at most `k` leaves
/// (`4 ..= 12`). Returns the number of nodes refactored.
pub fn refactor(aig: &mut Aig, k: usize) -> usize {
    let k = k.clamp(4, 12);
    let mut view = View::new(aig);
    let count = u32::try_from(view.old_len).expect("node count");
    let mut applied = 0;
    for id in 1..count {
        if !view.aig.is_and(id) || view.fwd.is_replaced(id) || view.aig.refs(id) == 0 {
            continue;
        }
        let leaves = reconvergent_cut(&mut |x| view.fanins(x), id, k);
        if leaves.len() < 2 {
            continue;
        }
        let saved = view.deref_cone(id, &leaves);
        if saved < 3 {
            view.ref_cone(id, &leaves);
            continue;
        }
        let tt = cone_truth(&mut |x| view.fanins(x), id, &leaves, leaves.len());
        let (tt, negate) = if tt.count_ones() * 2 > tt.len().try_into().unwrap_or(u32::MAX) {
            (tt.not(), true)
        } else {
            (tt, false)
        };
        let cubes = isop(&tt);
        let leaf_edges: Vec<Edge> = leaves.iter().map(|&l| Edge::plain(l)).collect();
        let mut counter = Counter::new(&view, &leaf_edges, id);
        build_cover(&cubes, leaves.len(), &mut counter);
        let added = usize::try_from(counter.added).expect("count");
        if counter.hit_root || added >= saved {
            view.ref_cone(id, &leaves);
            continue;
        }
        let mut builder = Builder::new(&mut view, leaf_edges, id);
        let e = build_cover(&cubes, leaves.len(), &mut builder);
        let e = e.xor(negate);
        if !view.replace(id, e) {
            // The cover rebuilt the node itself; keep what is there.
            view.ref_cone(id, &leaves);
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
    use super::*;

    #[test]
    fn isop_covers_exactly() {
        for vars in [3usize, 5, 8] {
            let mut rng = super::super::Rng::new(42);
            for _ in 0..20 {
                let words: Vec<u64> = (0..super::super::truth::words_for(vars))
                    .map(|_| rng.next_u64())
                    .collect();
                let tt = TruthTable::from_words(vars, words);
                let cubes = isop(&tt);
                assert_eq!(cover_truth(&cubes, vars), tt, "{vars} vars");
            }
        }
        let zero = TruthTable::constant(4, false);
        assert!(isop(&zero).is_empty());
        let one = TruthTable::constant(4, true);
        assert_eq!(isop(&one), [Cube::ONE]);
        assert!(Cube::ONE.is_empty());
        assert_eq!(
            Cube {
                pos: 0b101,
                neg: 0b010
            }
            .len(),
            3
        );
        assert_eq!(
            Cube {
                pos: 0b101,
                neg: 0b010
            }
            .without(2, false),
            Cube {
                pos: 0b001,
                neg: 0b010
            }
        );
    }

    #[test]
    fn refactoring_preserves_function() {
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..6).map(|_| g.add_input()).collect();
        // A wasteful implementation of a 3-bit equality plus a sum-of-products.
        let mut eq = Edge::TRUE;
        for i in 0..3 {
            let d = g.xor(ins[i], ins[i + 3]);
            let n = g.and(!d, Edge::TRUE);
            eq = g.and(eq, n);
        }
        let t1 = g.and(ins[0], ins[1]);
        let t2 = g.and(ins[0], ins[2]);
        let t3 = g.and(ins[0], ins[3]);
        let s = g.or(t1, t2);
        let s = g.or(s, t3);
        let out = g.or(eq, s);
        g.add_output(out);
        g.add_output(eq);
        let reference = g.clone();
        let before = g.num_ands();
        refactor(&mut g, 10);
        assert!(g.num_ands() <= before);
        for pat in 0..64u32 {
            let bits: Vec<bool> = (0..6).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(g.eval(&bits), reference.eval(&bits), "pattern {pat}");
        }
    }
}
