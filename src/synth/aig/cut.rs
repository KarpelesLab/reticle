//! Cuts: sets of nodes that separate a root from the inputs.
//!
//! A *cut* of a node `r` is a set of nodes `L` such that every path from an
//! input to `r` passes through `L`; the *cone* of the cut is what lies
//! between. Cuts are how every local optimisation picks the piece of logic
//! it looks at: rewriting enumerates all cuts of up to four leaves,
//! refactoring grows one reconvergence-driven cut of up to a dozen leaves,
//! and technology mapping covers the graph with cuts of up to `k` leaves.
//!
//! The enumerators here work by *expansion* from the root: a cut is
//! replaced by the cuts obtained by substituting a leaf with its two
//! fanins. Fanins are read through a caller-supplied closure so the passes
//! that keep a [`super::Forward`] map see the substituted graph.
//!
//! [`cone_truth`] computes the function of the root over the leaves as a
//! [`TruthTable`], by simulation of the cone — and refuses, rather than
//! inventing an answer, when the leaves are not a cut of the root.

use super::Edge;
use super::truth::TruthTable;

/// The fanins of an AND node, or `None` for an input or the constant.
pub type Fanins<'a> = dyn FnMut(u32) -> Option<(Edge, Edge)> + 'a;

/// Every cut of `root` with at most `k` leaves reachable by expansion,
/// excluding the trivial cut `{root}`, at most `limit` of them, sorted
/// leaves each. Deterministic: expansions are explored breadth first in
/// leaf order.
pub fn enumerate_cuts(fanins: &mut Fanins<'_>, root: u32, k: usize, limit: usize) -> Vec<Vec<u32>> {
    // The cuts already seen are the trivial cut plus `result`, because a cut
    // joins the one exactly when it joins the other — so `result` *is* the
    // membership test, there is no second copy of every cut to keep, and the
    // scan is over at most `limit` lists of at most `k` leaves, which is
    // cheaper than hashing each of them.
    let trivial = [root];
    let mut result: Vec<Vec<u32>> = Vec::new();
    let mut queue: Vec<Vec<u32>> = vec![vec![root]];
    let mut head = 0;
    while head < queue.len() && result.len() < limit {
        // Taken, not cloned: `head` only moves forward, so this entry is
        // never read again.
        let cut = std::mem::take(&mut queue[head]);
        head += 1;
        for (pos, &leaf) in cut.iter().enumerate() {
            let Some((a, b)) = fanins(leaf) else {
                continue;
            };
            let mut next: Vec<u32> = cut
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != pos)
                .map(|(_, &l)| l)
                .collect();
            for n in [a.node(), b.node()] {
                if n != 0 && !next.contains(&n) {
                    next.push(n);
                }
            }
            if next.len() > k {
                continue;
            }
            next.sort_unstable();
            if next != trivial && !result.contains(&next) {
                result.push(next.clone());
                if result.len() >= limit {
                    break;
                }
                queue.push(next);
            }
        }
    }
    result
}

/// One reconvergence-driven cut of `root` with at most `k` leaves: starting
/// from `{root}`, repeatedly expand the leaf whose expansion adds the
/// fewest new leaves, as in ABC's `refactor`. Returns the sorted leaves.
pub fn reconvergent_cut(fanins: &mut Fanins<'_>, root: u32, k: usize) -> Vec<u32> {
    let mut leaves: Vec<u32> = vec![root];
    loop {
        let mut best: Option<(usize, usize, u32, u32)> = None;
        for (pos, &leaf) in leaves.iter().enumerate() {
            let Some((a, b)) = fanins(leaf) else {
                continue;
            };
            let (a, b) = (a.node(), b.node());
            let mut added = 0usize;
            if a != 0 && !leaves.contains(&a) {
                added += 1;
            }
            if b != 0 && b != a && !leaves.contains(&b) {
                added += 1;
            }
            if leaves.len() - 1 + added > k {
                continue;
            }
            // Fewest new leaves first; ties broken by position for
            // determinism.
            if best.is_none_or(|(c, _, _, _)| added < c) {
                best = Some((added, pos, a, b));
            }
        }
        let Some((_, pos, a, b)) = best else {
            break;
        };
        leaves.remove(pos);
        for n in [a, b] {
            if n != 0 && !leaves.contains(&n) {
                leaves.push(n);
            }
        }
    }
    leaves.sort_unstable();
    leaves
}

/// Reusable working space for the cone walks.
///
/// A pass that walks a cone per cut of every node — rewriting takes 32 cuts
/// a node, refactoring one — used to allocate a visited set, a table map, a
/// stack and an order vector on each of those walks, and to hash a node
/// index on every step of them. One of these instead is allocated once per
/// pass and grown to the graph; starting a walk costs one counter increment.
///
/// Nothing about the walk changes: the same traversal order, the same cone,
/// the same refusals. That is why the allocating [`cone_nodes`] and
/// [`cone_truth`] are still here, as wrappers over this code with a scratch
/// of their own.
#[derive(Clone, Debug, Default)]
pub struct ConeScratch {
    /// The walk a node was last seen in; equal to `epoch` means "seen".
    stamp: Vec<u32>,
    /// This walk's number. Zero is never a walk, so a stamp that has just
    /// been grown reads as unseen.
    epoch: u32,
    /// For a node seen in this walk, where its table is in `tables`, or
    /// [`NO_SLOT`] when it has none.
    slot: Vec<u32>,
    /// The tables of this walk, in the order they were computed.
    tables: Vec<TruthTable>,
    /// The traversal stack: `(node, its fanins have been pushed)`.
    stack: Vec<(u32, bool)>,
    /// The cone of the last walk, fanins before fanouts.
    order: Vec<u32>,
}

/// The `slot` of a node that has been seen but has no table.
const NO_SLOT: u32 = u32::MAX;

impl ConeScratch {
    /// Empty working space.
    pub fn new() -> ConeScratch {
        ConeScratch::default()
    }

    /// Starts a walk: every node reads as unseen again, in constant time.
    fn begin(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            // Wrapped around: a stale stamp would alias the new epoch.
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.epoch = 1;
        }
        self.tables.clear();
        self.stack.clear();
        self.order.clear();
    }

    /// Makes room for `node`.
    fn room(&mut self, node: u32) {
        let need = node as usize + 1;
        if self.stamp.len() < need {
            self.stamp.resize(need, 0);
            self.slot.resize(need, NO_SLOT);
        }
    }

    /// True when `node` has been seen in this walk.
    fn seen(&self, node: u32) -> bool {
        self.stamp.get(node as usize) == Some(&self.epoch)
    }

    /// Marks `node` seen; false when it already was.
    fn mark(&mut self, node: u32) -> bool {
        self.room(node);
        if self.stamp[node as usize] == self.epoch {
            return false;
        }
        self.stamp[node as usize] = self.epoch;
        self.slot[node as usize] = NO_SLOT;
        true
    }

    /// Records `table` as the function of `node`.
    fn set_table(&mut self, node: u32, table: TruthTable) {
        self.room(node);
        self.stamp[node as usize] = self.epoch;
        self.slot[node as usize] = u32::try_from(self.tables.len()).expect("table count");
        self.tables.push(table);
    }

    /// Where `node`'s table is, when it has one in this walk.
    fn table_at(&self, node: u32) -> Option<usize> {
        if !self.seen(node) {
            return None;
        }
        let at = *self.slot.get(node as usize)?;
        (at != NO_SLOT).then_some(at as usize)
    }

    /// Walks the cone between `leaves` and `root` into `self.order`.
    fn walk(&mut self, fanins: &mut Fanins<'_>, root: u32, leaves: &[u32]) {
        self.begin();
        for &leaf in leaves {
            self.mark(leaf);
        }
        self.mark(0);
        self.stack.push((root, false));
        while let Some((id, expanded)) = self.stack.pop() {
            if expanded {
                self.order.push(id);
                continue;
            }
            if !self.mark(id) {
                continue;
            }
            let Some((a, b)) = fanins(id) else {
                // An input reached without passing a leaf: the cut does not
                // separate it, treat it as an implicit leaf.
                continue;
            };
            self.stack.push((id, true));
            if !self.seen(b.node()) {
                self.stack.push((b.node(), false));
            }
            if !self.seen(a.node()) {
                self.stack.push((a.node(), false));
            }
        }
    }
}

/// The nodes of the cone between `leaves` and `root`, in topological
/// order (fanins first), `root` last; the leaves are not included.
pub fn cone_nodes(fanins: &mut Fanins<'_>, root: u32, leaves: &[u32]) -> Vec<u32> {
    let mut scratch = ConeScratch::new();
    scratch.walk(fanins, root, leaves);
    std::mem::take(&mut scratch.order)
}

/// The function of `root` over the cut `leaves` (leaf `i` is variable
/// `i`), as a table over `vars >= leaves.len()` variables.
///
/// `None` when `leaves` is **not a cut of `root`**: when some path from an
/// input to `root` misses every leaf, there is no such function, and the
/// only honest answer is to say so. This used to substitute the constant
/// *false* for the input the cut failed to separate and return a table
/// anyway, which is a wrong function presented as a right one — see
/// `src/synth/techmap/cuts.rs` for the miscompilation that caused.
pub fn cone_truth(
    fanins: &mut Fanins<'_>,
    root: u32,
    leaves: &[u32],
    vars: usize,
) -> Option<TruthTable> {
    cone_truth_with(&mut ConeScratch::new(), fanins, root, leaves, vars)
}

/// [`cone_truth`] over working space the caller keeps, for a pass that wants
/// one cone per cut of every node.
pub fn cone_truth_with(
    scratch: &mut ConeScratch,
    fanins: &mut Fanins<'_>,
    root: u32,
    leaves: &[u32],
    vars: usize,
) -> Option<TruthTable> {
    scratch.walk(fanins, root, leaves);
    scratch.set_table(0, TruthTable::constant(vars, false));
    for (i, &leaf) in leaves.iter().enumerate() {
        scratch.set_table(leaf, TruthTable::var(vars, i));
    }
    // `order` is finished and only `tables` grows below, so indexing it is
    // sound and saves copying the cone out of the scratch.
    for at in 0..scratch.order.len() {
        let id = scratch.order[at];
        let (a, b) = fanins(id).expect("cone node is an AND");
        // A fanin with no table is neither the constant, nor a leaf, nor a
        // node of the cone: it is an input this leaf set does not separate.
        let ia = scratch.table_at(a.node())?;
        let ib = scratch.table_at(b.node())?;
        // One table built, then conjoined in place: the second fanin's
        // complement folds into the AND rather than becoming a table of its
        // own, which is two fewer per node of every cone walked.
        let mut t = if a.is_complement() {
            scratch.tables[ia].not()
        } else {
            scratch.tables[ia].clone()
        };
        if b.is_complement() {
            t.and_not_with(&scratch.tables[ib]);
        } else {
            t.and_with(&scratch.tables[ib]);
        }
        scratch.set_table(id, t);
    }
    let at = scratch.table_at(root)?;
    // The caller owns the answer; the slot it came from is not read again.
    Some(std::mem::replace(
        &mut scratch.tables[at],
        TruthTable::constant(0, false),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::Aig;
    use super::*;

    #[test]
    fn enumerates_cuts_by_expansion() {
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let c = g.add_input();
        let ab = g.and(a, b);
        let bc = g.and(b, c);
        let root = g.and(ab, bc);
        let mut fan = |id: u32| g.is_and(id).then(|| g.fanins(id));
        let cuts = enumerate_cuts(&mut fan, root.node(), 4, 100);
        assert!(cuts.contains(&vec![ab.node(), bc.node()]));
        assert!(cuts.contains(&vec![a.node(), b.node(), bc.node()]));
        assert!(cuts.contains(&vec![a.node(), b.node(), c.node()]));
        assert!(!cuts.contains(&vec![root.node()]));
        let capped = enumerate_cuts(&mut fan, root.node(), 4, 2);
        assert_eq!(capped.len(), 2);
        let rc = reconvergent_cut(&mut fan, root.node(), 3);
        assert_eq!(rc, vec![a.node(), b.node(), c.node()]);
        let rc2 = reconvergent_cut(&mut fan, root.node(), 2);
        assert_eq!(rc2, vec![ab.node(), bc.node()]);
        let cone = cone_nodes(&mut fan, root.node(), &[a.node(), b.node(), c.node()]);
        assert_eq!(cone.len(), 3);
        assert_eq!(*cone.last().unwrap(), root.node());
        let tt = cone_truth(&mut fan, root.node(), &[a.node(), b.node(), c.node()], 3)
            .expect("a cut of the root");
        assert_eq!(tt.as_u64(), 0x80); // a & b & c
        let tt4 = cone_truth(&mut fan, root.node(), &[ab.node(), bc.node()], 4)
            .expect("a cut of the root");
        assert_eq!(tt4.as_u64(), 0x8888);
        // A leaf set that does not separate the root has no function over
        // it: `{a, b}` leaves `c` reachable through `bc`.
        assert!(cone_truth(&mut fan, root.node(), &[a.node(), b.node()], 2).is_none());
    }

    /// Working space carried from one walk to the next answers exactly as a
    /// fresh one does, including when the epoch counter wraps.
    ///
    /// This is the risk the scratch introduced. Its visited marks are an
    /// epoch stamp rather than a cleared set, so a stale stamp that aliased
    /// the current epoch would make a node read as already visited and
    /// silently shrink a cone — the function would be of a smaller cut than
    /// the one asked for, which is the shape of defect
    /// `every_cut_computes_its_node` exists to catch, found here instead at
    /// the level it is introduced.
    #[test]
    fn a_reused_scratch_answers_like_a_fresh_one() {
        // A graph with reconvergence and a constant, so the walks differ in
        // size and some leaf sets fail to separate their root.
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..5).map(|_| g.add_input()).collect();
        let mut pool = ins.clone();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..30 {
            let pick = |r: u64| {
                let at = usize::try_from(r % pool.len() as u64).expect("index");
                if r & 0x1_0000 != 0 {
                    !pool[at]
                } else {
                    pool[at]
                }
            };
            let (a, b) = (pick(next()), pick(next()));
            let node = g.and(a, b);
            if !pool.contains(&node) {
                pool.push(node);
            }
        }
        // Every (node, leaf set) pair either walk could be asked about.
        let mut questions: Vec<(u32, Vec<u32>)> = Vec::new();
        for id in 1..u32::try_from(g.len()).expect("node count") {
            if !g.is_and(id) {
                continue;
            }
            let mut fan = |x: u32| g.is_and(x).then(|| g.fanins(x));
            for cut in enumerate_cuts(&mut fan, id, 4, 16) {
                questions.push((id, cut));
            }
            // And a leaf set that is deliberately not a cut.
            questions.push((id, vec![ins[0].node()]));
        }
        assert!(questions.len() > 50, "only {} questions", questions.len());

        // One scratch for all of them, with the epoch set to wrap in the
        // middle so the reset path is exercised too.
        let mut shared = ConeScratch::new();
        shared.epoch = u32::MAX - u32::try_from(questions.len() / 2).expect("count");
        for (id, leaves) in &questions {
            let vars = leaves.len().max(1);
            let mut fan = |x: u32| g.is_and(x).then(|| g.fanins(x));
            let fresh = cone_truth(&mut fan, *id, leaves, vars);
            let mut fan = |x: u32| g.is_and(x).then(|| g.fanins(x));
            let reused = cone_truth_with(&mut shared, &mut fan, *id, leaves, vars);
            assert_eq!(fresh, reused, "node {id} over leaves {leaves:?}");
        }
        // The counter really did wrap, so the branch above ran.
        assert!(shared.epoch < u32::MAX / 2, "the epoch did not wrap");
    }
}
