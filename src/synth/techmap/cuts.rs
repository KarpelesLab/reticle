//! Priority cut enumeration.
//!
//! Every AND node keeps a bounded list of cuts, computed in one
//! topological sweep: the cuts of a node are the *merges* of one cut of
//! each fanin (the union of their leaves, kept only when it still fits
//! `k`), plus the trivial cut `{node}`. Keeping all of them is exponential,
//! so the list is truncated to [`CutOptions::per_node`] of them, as in
//! Mishchenko, Chatterjee and Brayton, "Improvements to technology mapping
//! for LUT-based FPGAs" (FPGA 2007), which bounds both time and memory at
//! a small cost in quality.
//!
//! Which cuts to keep matters more than it looks. Ranking them by size and
//! keeping the smallest is the obvious choice and the wrong one: a node
//! has many more small cuts than large ones, so the large cuts — the whole
//! point of mapping to a big LUT — are crowded out, and a 6-LUT mapping
//! comes out no better than a 4-LUT one. The selection here instead gives
//! each cut size its own quota, largest first, and only then fills what is
//! left with the smallest cuts, so every size survives into the mapper.
//!
//! Each cut carries the function of its root over its leaves as a
//! [`TruthTable`], computed once here, so the mapper can price it (a LUT
//! takes any function; a gate library must match it) and write it out as a
//! LUT `init` or a cell.
//!
//! Dominated cuts are removed: a cut whose leaves are a subset of
//! another's is never worse, so the superset is dropped. Every cut is
//! also reduced to the leaves its function actually depends on, because a
//! leaf that cancels out (which reconvergent paths routinely produce)
//! would otherwise inflate the cut and hide a cheaper match.

use super::super::aig::Aig;
use super::super::aig::cut::cone_truth;
use super::super::aig::truth::TruthTable;

/// Limits on cut enumeration.
#[derive(Clone, Copy, Debug)]
pub struct CutOptions {
    /// Largest number of cuts kept per node, not counting the trivial one.
    pub per_node: usize,
}

impl Default for CutOptions {
    fn default() -> Self {
        CutOptions { per_node: 8 }
    }
}

/// One cut: the leaves that feed it and the function they compute.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// The leaves, ascending. Leaf `i` is variable `i` of `function`.
    pub leaves: Vec<u32>,
    /// The root's function over the leaves.
    pub function: TruthTable,
}

impl Cut {
    /// True for the cut `{node}`, which exists so a fanin can appear as a
    /// leaf of its parents' cuts. It is a merge base, never an
    /// implementation: a cell whose only input is its own output is not a
    /// thing, so the mapper skips it.
    pub fn is_trivial(&self, node: u32) -> bool {
        self.leaves == [node]
    }

    /// Number of leaves.
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// True when the cut has no leaves (a constant).
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// True when every leaf of `self` is a leaf of `other`, so `self` is
    /// at least as good and `other` can be dropped.
    pub fn dominates(&self, other: &Cut) -> bool {
        self.leaves.len() <= other.leaves.len()
            && self.leaves.iter().all(|l| other.leaves.contains(l))
    }
}

/// The cut lists of every node of an AIG.
pub struct PriorityCuts {
    per_node: Vec<Vec<Cut>>,
}

impl PriorityCuts {
    /// Computes cuts of at most `k` leaves for every node of `aig`.
    pub fn compute(aig: &Aig, k: usize, options: &CutOptions) -> PriorityCuts {
        let n = aig.len();
        let mut per_node: Vec<Vec<Cut>> = vec![Vec::new(); n];
        for id in 0..u32::try_from(n).expect("node count") {
            let index = id as usize;
            if !aig.is_and(id) {
                // Inputs and the constant have only the trivial cut.
                per_node[index] = vec![trivial(id)];
                continue;
            }
            let (a, b) = aig.fanins(id);
            let mut merged: Vec<Cut> = Vec::new();
            for ca in &per_node[a.index()] {
                for cb in &per_node[b.index()] {
                    let Some(leaves) = merge(&ca.leaves, &cb.leaves, k) else {
                        continue;
                    };
                    if leaves.len() > k {
                        continue;
                    }
                    // A cut that just re-derives the node itself is the
                    // trivial cut, added below.
                    if leaves.len() == 1 && leaves[0] == id {
                        continue;
                    }
                    let Some(cut) = build(aig, id, leaves) else {
                        continue;
                    };
                    if merged.iter().any(|c| c.dominates(&cut)) {
                        continue;
                    }
                    merged.retain(|c| !cut.dominates(c));
                    merged.push(cut);
                }
            }
            // Order deterministically, then select across the sizes.
            merged.sort_by(|x, y| (x.leaves.len(), &x.leaves).cmp(&(y.leaves.len(), &y.leaves)));
            merged = select(merged, k, options.per_node.max(1));
            // The trivial cut always stays, as the merge base for the
            // parents of this node.
            merged.push(trivial(id));
            per_node[index] = merged;
        }
        PriorityCuts { per_node }
    }

    /// The cuts of `node`.
    pub fn cuts(&self, node: u32) -> &[Cut] {
        &self.per_node[node as usize]
    }

    /// Total number of cuts stored.
    pub fn total(&self) -> usize {
        self.per_node.iter().map(Vec::len).sum()
    }
}

/// Keeps at most `limit` cuts, giving each cut size a quota so the large
/// cuts are not crowded out by the many small ones (see the module docs).
///
/// `cuts` must already be sorted by size; the result keeps that order.
fn select(cuts: Vec<Cut>, k: usize, limit: usize) -> Vec<Cut> {
    if cuts.len() <= limit {
        return cuts;
    }
    let quota = limit.div_ceil(k.max(1)).max(1);
    let mut taken = vec![false; cuts.len()];
    let mut count = 0usize;
    // Largest sizes first, so they claim their quota before the small
    // cuts fill the list.
    for size in (1..=k).rev() {
        let mut from_size = 0usize;
        for (i, cut) in cuts.iter().enumerate() {
            if count >= limit || from_size >= quota {
                break;
            }
            if cut.leaves.len() == size && !taken[i] {
                taken[i] = true;
                from_size += 1;
                count += 1;
            }
        }
    }
    // Fill the remaining slots with the smallest cuts left over.
    for slot in &mut taken {
        if count >= limit {
            break;
        }
        if !*slot {
            *slot = true;
            count += 1;
        }
    }
    cuts.into_iter()
        .zip(taken)
        .filter(|(_, keep)| *keep)
        .map(|(cut, _)| cut)
        .collect()
}

/// Builds the cut of `node` with the given leaves, reduced to the support
/// of its function. `None` when the function is constant, which leaves
/// nothing for a cell to compute.
fn build(aig: &Aig, node: u32, leaves: Vec<u32>) -> Option<Cut> {
    let function = cone_truth(
        &mut |x| aig.is_and(x).then(|| aig.fanins(x)),
        node,
        &leaves,
        leaves.len().max(1),
    );
    let support = function.support();
    if support == 0 {
        return None;
    }
    let full = if leaves.is_empty() {
        0
    } else {
        (1u32 << leaves.len()) - 1
    };
    if support == full {
        return Some(Cut { leaves, function });
    }
    Some(Cut {
        leaves: leaves
            .into_iter()
            .enumerate()
            .filter(|(i, _)| (support >> i) & 1 == 1)
            .map(|(_, l)| l)
            .collect(),
        function: function.restrict(support),
    })
}

/// The trivial cut `{node}`, whose function is the identity.
fn trivial(node: u32) -> Cut {
    Cut {
        leaves: vec![node],
        function: TruthTable::var(1, 0),
    }
}

/// The cut a node falls back on when nothing else fits: its two fanins.
/// Every target can implement it, since it is a single AND.
pub(super) fn fanin_cut(aig: &Aig, node: u32) -> Cut {
    let (a, b) = aig.fanins(node);
    let mut leaves: Vec<u32> = [a.node(), b.node()]
        .into_iter()
        .filter(|&l| l != 0)
        .collect();
    leaves.sort_unstable();
    leaves.dedup();
    build(aig, node, leaves).expect("an AND node is not constant")
}

/// The union of two ascending leaf lists, or `None` when it exceeds `k`.
fn merge(a: &[u32], b: &[u32], k: usize) -> Option<Vec<u32>> {
    let mut out: Vec<u32> = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if out.len() > k {
            return None;
        }
        match (a.get(i), b.get(j)) {
            (Some(&x), Some(&y)) if x == y => {
                out.push(x);
                i += 1;
                j += 1;
            }
            (Some(&x), Some(&y)) if x < y => {
                out.push(x);
                i += 1;
            }
            (Some(_), Some(&y)) => {
                out.push(y);
                j += 1;
            }
            (Some(&x), None) => {
                out.push(x);
                i += 1;
            }
            (None, Some(&y)) => {
                out.push(y);
                j += 1;
            }
            (None, None) => break,
        }
    }
    // The constant node is never a leaf: it contributes no variable.
    out.retain(|&l| l != 0);
    if out.len() > k { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::super::super::aig::Edge;
    use super::*;

    /// The leaves a cut would store for these nodes.
    fn sorted(nodes: &[u32]) -> Vec<u32> {
        let mut v = nodes.to_vec();
        v.sort_unstable();
        v
    }

    /// A cut is reduced to the leaves its function depends on.
    #[test]
    fn cuts_drop_leaves_that_cancel_out() {
        // `(a & b) & !(a & c)` seen through the cut {a, b, c}; every leaf
        // matters. But `(a & b) | (a & !b)` reduces to `a`.
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let ab = g.and(a, b);
        let anb = g.and(a, !b);
        let root = g.or(ab, anb);
        g.add_output(root);
        let cuts = PriorityCuts::compute(&g, 4, &CutOptions::default());
        // The cut over {a, b} computes `a`, so `b` is dropped and it
        // becomes a one-leaf cut. Cut functions are the function of the
        // *node*, and `or` builds `!(!ab & !anb)`, so the node itself
        // computes `!a` and the output edge carries the complement.
        let reduced = cuts
            .cuts(root.node())
            .iter()
            .find(|c| c.leaves == vec![a.node()])
            .expect("the reduced cut");
        assert_eq!(reduced.function.vars(), 1);
        assert_eq!(reduced.function.as_u64(), 0b01);
        assert!(root.is_complement());
    }

    /// The selection keeps large cuts even when small ones are plentiful.
    #[test]
    fn selection_spreads_across_cut_sizes() {
        let cut = |leaves: Vec<u32>| Cut {
            function: TruthTable::constant(leaves.len().max(1), false),
            leaves,
        };
        let mut cuts = Vec::new();
        for i in 0..10u32 {
            cuts.push(cut(vec![i]));
        }
        for i in 0..10u32 {
            cuts.push(cut(vec![i, i + 20]));
        }
        cuts.push(cut(vec![1, 2, 3, 4]));
        cuts.sort_by(|x, y| (x.leaves.len(), &x.leaves).cmp(&(y.leaves.len(), &y.leaves)));
        let kept = select(cuts, 4, 8);
        assert_eq!(kept.len(), 8);
        // The single four-leaf cut survives, and so do some of each size.
        assert!(kept.iter().any(|c| c.leaves.len() == 4));
        assert!(kept.iter().any(|c| c.leaves.len() == 2));
        assert!(kept.iter().any(|c| c.leaves.len() == 1));
        // The order is still by size.
        assert!(
            kept.windows(2)
                .all(|w| w[0].leaves.len() <= w[1].leaves.len())
        );
        // A list already within the limit is returned unchanged.
        let small = vec![cut(vec![1]), cut(vec![2])];
        assert_eq!(select(small.clone(), 4, 8).len(), small.len());
    }

    #[test]
    fn merges_and_bounds_cut_lists() {
        assert_eq!(merge(&[1, 3], &[2, 3], 4), Some(vec![1, 2, 3]));
        assert_eq!(merge(&[1, 2, 3], &[4, 5], 4), None);
        assert_eq!(merge(&[0, 2], &[0, 3], 4), Some(vec![2, 3]));
        assert_eq!(merge(&[], &[7], 2), Some(vec![7]));
    }

    #[test]
    fn enumerates_cuts_with_functions() {
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let c = g.add_input();
        let ab = g.and(a, b);
        let root = g.and(ab, c);
        g.add_output(root);
        let cuts = PriorityCuts::compute(&g, 3, &CutOptions::default());
        let list = cuts.cuts(root.node());
        // Leaves are stored ascending, and `ab` was created after `c`.
        let two_leaf = sorted(&[ab.node(), c.node()]);
        // The trivial cut, {c, ab} and {a, b, c}.
        assert!(list.iter().any(|x| x.leaves == vec![root.node()]));
        assert!(list.iter().any(|x| x.leaves == two_leaf));
        let full = list
            .iter()
            .find(|x| x.leaves == vec![a.node(), b.node(), c.node()])
            .expect("the input cut");
        assert_eq!(full.function.as_u64(), 0x80);
        assert_eq!(full.len(), 3);
        assert!(!full.is_empty());
        assert!(cuts.total() >= list.len());
        // A cut of two leaves dominates one of three that contains it.
        let two = list.iter().find(|x| x.leaves == two_leaf).unwrap();
        assert!(!two.dominates(full));
        assert!(two.dominates(&Cut {
            leaves: sorted(&[ab.node(), c.node(), a.node()]),
            function: full.function.clone(),
        }));
        // With k = 2 the three-leaf cut is gone.
        let small = PriorityCuts::compute(&g, 2, &CutOptions::default());
        assert!(small.cuts(root.node()).iter().all(|x| x.leaves.len() <= 2));
        // The per-node cap is honoured.
        let capped = PriorityCuts::compute(&g, 3, &CutOptions { per_node: 1 });
        // One merged cut plus the trivial one.
        assert_eq!(capped.cuts(root.node()).len(), 2);
        assert!(
            capped
                .cuts(root.node())
                .iter()
                .any(|c| c.is_trivial(root.node()))
        );
        // The fanin cut is always available as a fallback.
        let fallback = fanin_cut(&g, root.node());
        assert_eq!(fallback.leaves, two_leaf);
        // Constants are never leaves.
        let mut h = Aig::new();
        let x = h.add_input();
        let y = h.and(x, Edge::TRUE);
        assert_eq!(y, x);
    }
}
