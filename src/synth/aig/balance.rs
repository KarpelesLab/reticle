//! Tree balancing for depth.
//!
//! An AND node whose fanins are plain (non-complemented) edges to AND nodes
//! with a single fanout is the root of a *super-gate*: a multi-input AND
//! that happens to be written as a chain or an arbitrary tree. Balancing
//! collects the leaves of every super-gate and rebuilds it as a tree that
//! pairs the shallowest leaves first (Huffman-style on levels), which
//! minimises the depth of that gate given its leaf arrival levels. It is
//! the `balance` command of ABC and is run before and after rewriting in
//! the standard scripts, both because the depth matters for technology
//! mapping and because the restructuring gives the cut-based passes new
//! structures to look at.
//!
//! The pass rebuilds the whole graph (so it also strashes and sweeps) and
//! removes duplicate leaves and contradictory pairs (`a` with `!a`) from a
//! super-gate on the way.

use super::{Aig, Edge};

/// Balances every AND tree of `aig` by level; the result is rebuilt and
/// swept. Returns the new depth.
pub fn balance(aig: &mut Aig) -> u32 {
    aig.recount_refs();
    let n = aig.len();
    let live = aig.live_nodes();
    let mut plain_and_fanouts = vec![0u32; n];
    for id in 0..n {
        let id = u32::try_from(id).expect("node index");
        if aig.is_and(id) && live[id as usize] {
            let (a, b) = aig.fanins(id);
            for e in [a, b] {
                if !e.is_complement() {
                    plain_and_fanouts[e.index()] += 1;
                }
            }
        }
    }
    let internal = |id: u32| -> bool {
        aig.is_and(id) && aig.refs(id) == 1 && plain_and_fanouts[id as usize] == 1
    };

    let mut out = Aig::new();
    let mut map: Vec<Edge> = vec![Edge::FALSE; n];
    for &pi in aig.inputs() {
        map[pi as usize] = out.add_input();
    }
    for id in 1..n {
        let id = u32::try_from(id).expect("node index");
        if !live[id as usize] || !aig.is_and(id) || internal(id) {
            continue;
        }
        // Collect the super-gate's leaves, descending through internal
        // nodes reached by plain edges.
        let (a, b) = aig.fanins(id);
        let mut stack = vec![a, b];
        let mut leaves: Vec<Edge> = Vec::new();
        while let Some(e) = stack.pop() {
            if !e.is_complement() && internal(e.node()) {
                let (x, y) = aig.fanins(e.node());
                stack.push(x);
                stack.push(y);
            } else {
                leaves.push(map[e.index()].xor(e.is_complement()));
            }
        }
        map[id as usize] = balance_leaves(&mut out, leaves);
    }
    for &o in aig.outputs() {
        let e = map[o.index()].xor(o.is_complement());
        out.add_output(e);
    }
    *aig = out;
    aig.max_level()
}

/// Builds a level-balanced AND of `leaves` in `out`.
fn balance_leaves(out: &mut Aig, mut leaves: Vec<Edge>) -> Edge {
    leaves.sort_unstable();
    leaves.dedup();
    if leaves.contains(&Edge::FALSE) {
        return Edge::FALSE;
    }
    leaves.retain(|&e| e != Edge::TRUE);
    for w in leaves.windows(2) {
        if w[0].node() == w[1].node() {
            // a & !a
            return Edge::FALSE;
        }
    }
    if leaves.is_empty() {
        return Edge::TRUE;
    }
    // Sorted by (level, edge); combine the two shallowest repeatedly.
    let key = |out: &Aig, e: Edge| (out.level(e.node()), e.raw());
    leaves.sort_by_key(|&e| key(out, e));
    while leaves.len() > 1 {
        let a = leaves.remove(0);
        let b = leaves.remove(0);
        let c = out.and(a, b);
        let k = key(out, c);
        let pos = leaves.partition_point(|&e| key(out, e) < k);
        leaves.insert(pos, c);
    }
    leaves[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_become_trees() {
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..8).map(|_| g.add_input()).collect();
        let mut acc = ins[0];
        for &i in &ins[1..] {
            acc = g.and(acc, i);
        }
        g.add_output(acc);
        assert_eq!(g.max_level(), 7);
        let reference = g.clone();
        let depth = balance(&mut g);
        assert_eq!(depth, 3);
        assert_eq!(g.num_ands(), 7);
        for pat in 0..256u32 {
            let bits: Vec<bool> = (0..8).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(g.eval(&bits), reference.eval(&bits));
        }
    }

    #[test]
    fn contradictions_and_duplicates_fold() {
        // `(a & b) & !a` is a single-fanout tree, so balancing sees all
        // three leaves at once and folds the contradiction to zero.
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let t = g.and(a, b);
        let u = g.and(t, !a);
        g.add_output(u);
        // A second output over its own node, so `t` keeps one fanout and
        // stays internal to the first tree.
        let v = g.or(a, b);
        g.add_output(v);
        balance(&mut g);
        assert_eq!(g.outputs()[0], Edge::FALSE);
        assert_eq!(g.num_ands(), 1);
        for pat in 0..4u32 {
            let bits = [pat & 1 == 1, pat & 2 == 2];
            assert_eq!(g.eval(&bits)[1], bits[0] || bits[1]);
        }
        // Unbalanced arrival levels: the deep leaf is paired last.
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..4).map(|_| g.add_input()).collect();
        let deep = g.and(ins[0], ins[1]);
        let deep = g.or(deep, ins[2]);
        let deep = g.and(deep, !ins[3]);
        g.add_output(deep);
        let chain = g.and(deep, ins[0]);
        let chain = g.and(chain, ins[1]);
        let chain = g.and(chain, ins[2]);
        g.add_output(chain);
        let before = g.max_level();
        let after = balance(&mut g);
        assert!(after < before, "{before} -> {after}");
    }
}
