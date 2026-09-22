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
//! [`TruthTable`], by simulation of the cone.

use std::collections::{HashMap, HashSet};

use super::Edge;
use super::truth::TruthTable;

/// The fanins of an AND node, or `None` for an input or the constant.
pub type Fanins<'a> = dyn FnMut(u32) -> Option<(Edge, Edge)> + 'a;

/// Every cut of `root` with at most `k` leaves reachable by expansion,
/// excluding the trivial cut `{root}`, at most `limit` of them, sorted
/// leaves each. Deterministic: expansions are explored breadth first in
/// leaf order.
pub fn enumerate_cuts(fanins: &mut Fanins<'_>, root: u32, k: usize, limit: usize) -> Vec<Vec<u32>> {
    let mut seen: HashSet<Vec<u32>> = HashSet::new();
    let mut result = Vec::new();
    let mut queue: Vec<Vec<u32>> = vec![vec![root]];
    seen.insert(vec![root]);
    let mut head = 0;
    while head < queue.len() && result.len() < limit {
        let cut = queue[head].clone();
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
            if seen.insert(next.clone()) {
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

/// The nodes of the cone between `leaves` and `root`, in topological
/// order (fanins first), `root` last; the leaves are not included.
pub fn cone_nodes(fanins: &mut Fanins<'_>, root: u32, leaves: &[u32]) -> Vec<u32> {
    let mut order = Vec::new();
    let mut visited: HashSet<u32> = leaves.iter().copied().collect();
    visited.insert(0);
    let mut stack: Vec<(u32, bool)> = vec![(root, false)];
    while let Some((id, expanded)) = stack.pop() {
        if expanded {
            order.push(id);
            continue;
        }
        if !visited.insert(id) {
            continue;
        }
        let Some((a, b)) = fanins(id) else {
            // An input reached without passing a leaf: the cut does not
            // separate it, treat it as an implicit leaf.
            continue;
        };
        stack.push((id, true));
        if !visited.contains(&b.node()) {
            stack.push((b.node(), false));
        }
        if !visited.contains(&a.node()) {
            stack.push((a.node(), false));
        }
    }
    order
}

/// The function of `root` over the cut `leaves` (leaf `i` is variable
/// `i`), as a table over `vars >= leaves.len()` variables.
pub fn cone_truth(fanins: &mut Fanins<'_>, root: u32, leaves: &[u32], vars: usize) -> TruthTable {
    let mut tables: HashMap<u32, TruthTable> = HashMap::new();
    tables.insert(0, TruthTable::constant(vars, false));
    for (i, &leaf) in leaves.iter().enumerate() {
        tables.insert(leaf, TruthTable::var(vars, i));
    }
    for id in cone_nodes(fanins, root, leaves) {
        let (a, b) = fanins(id).expect("cone node is an AND");
        let ta = tables
            .get(&a.node())
            .cloned()
            .unwrap_or_else(|| TruthTable::constant(vars, false));
        let tb = tables
            .get(&b.node())
            .cloned()
            .unwrap_or_else(|| TruthTable::constant(vars, false));
        let ta = if a.is_complement() { ta.not() } else { ta };
        let tb = if b.is_complement() { tb.not() } else { tb };
        tables.insert(id, ta.and(&tb));
    }
    tables
        .remove(&root)
        .unwrap_or_else(|| TruthTable::constant(vars, false))
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
        let tt = cone_truth(&mut fan, root.node(), &[a.node(), b.node(), c.node()], 3);
        assert_eq!(tt.as_u64(), 0x80); // a & b & c
        let tt4 = cone_truth(&mut fan, root.node(), &[ab.node(), bc.node()], 4);
        assert_eq!(tt4.as_u64(), 0x8888);
    }
}
