//! Maximum fanout-free cones and the resynthesis plumbing shared by the
//! rewriting and refactoring passes.
//!
//! Both passes work the same way on a node: pick a cut, measure how many
//! nodes would disappear if the root were replaced (its *maximum
//! fanout-free cone*, the nodes used by nothing outside the cone), count
//! how many nodes a new implementation of the same function would add,
//! and apply the replacement when the difference is a gain. The count and
//! the construction go through one abstraction, [`Synth`], so a candidate
//! structure is expressed once and either dry-run against the hash table
//! ([`Counter`]) or built for real ([`Builder`]).
//!
//! Reference counting follows ABC (Mishchenko et al. 2006): dereferencing
//! the root decrements its fanins and recurses into any node whose count
//! drops to zero, which yields the MFFC size as a side effect; nodes left
//! at zero are dead unless the new structure revives them by hashing onto
//! them, in which case their cones are re-referenced.

use super::{Aig, Edge, Forward};

/// The pass-side view of the graph: the AIG plus the substitution map the
/// pass is building.
pub struct View<'a> {
    /// The graph being edited.
    pub aig: &'a mut Aig,
    /// Substitutions made so far.
    pub fwd: Forward,
    /// The node count when the pass started; nodes at or above it are the
    /// pass's own creations.
    pub old_len: usize,
}

impl<'a> View<'a> {
    /// Wraps `aig` for a pass.
    pub fn new(aig: &'a mut Aig) -> View<'a> {
        let old_len = aig.len();
        View {
            fwd: Forward::identity(old_len),
            aig,
            old_len,
        }
    }

    /// The substituted fanins of an AND node, or `None` otherwise.
    pub fn fanins(&mut self, id: u32) -> Option<(Edge, Edge)> {
        if !self.aig.is_and(id) {
            return None;
        }
        let (a, b) = self.aig.fanins(id);
        Some((self.fwd.resolve(a), self.fwd.resolve(b)))
    }

    /// Dereferences the cone of `root` bounded by `leaves`: every fanin
    /// count on the way down is decremented, recursing into nodes that hit
    /// zero, and the number of such nodes (plus the root) is returned. The
    /// root's own count is left alone. Leaves are decremented but never
    /// entered.
    pub fn deref_cone(&mut self, root: u32, leaves: &[u32]) -> usize {
        let mut count = 1;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some((a, b)) = self.fanins(id) else {
                continue;
            };
            for f in [a.node(), b.node()] {
                if f == 0 {
                    continue;
                }
                let r = self.aig.dec_ref(f);
                if r == 0 && !leaves.contains(&f) && self.aig.is_and(f) {
                    count += 1;
                    stack.push(f);
                }
            }
        }
        count
    }

    /// The inverse of [`View::deref_cone`], restoring the counts.
    pub fn ref_cone(&mut self, root: u32, leaves: &[u32]) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some((a, b)) = self.fanins(id) else {
                continue;
            };
            for f in [a.node(), b.node()] {
                if f == 0 {
                    continue;
                }
                let r = self.aig.inc_ref(f);
                if r == 1 && !leaves.contains(&f) && self.aig.is_and(f) {
                    stack.push(f);
                }
            }
        }
    }

    /// Fully dereferences a node whose count is already zero, so its dead
    /// cone stops being counted.
    pub fn release_dead(&mut self, node: u32) {
        if node == 0 || self.aig.refs(node) != 0 {
            return;
        }
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            let Some((a, b)) = self.fanins(id) else {
                continue;
            };
            for f in [a.node(), b.node()] {
                if f != 0 && self.aig.dec_ref(f) == 0 && self.aig.is_and(f) {
                    stack.push(f);
                }
            }
        }
    }

    /// Makes a dead pre-existing node usable again by re-referencing its
    /// cone, stopping at `leaves` (whose cones are still counted).
    fn revive(&mut self, node: u32, leaves: &[Edge]) {
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            let Some((a, b)) = self.fanins(id) else {
                continue;
            };
            for f in [a.node(), b.node()] {
                if f == 0 {
                    continue;
                }
                let r = self.aig.inc_ref(f);
                if r == 1
                    && self.aig.is_and(f)
                    && (f as usize) < self.old_len
                    && !leaves.iter().any(|l| l.node() == f)
                {
                    stack.push(f);
                }
            }
        }
    }

    /// Replaces `root` by `edge`, if that is legal: the root's fanouts are
    /// transferred and its own cone (already dereferenced) is released.
    ///
    /// `edge` is resolved through the substitution map first, because a
    /// structure built through the hash table may land on a node that has
    /// itself been replaced. The replacement is refused (and `false`
    /// returned) when it would map the root to itself, directly or through
    /// the map, which would make the map cyclic.
    pub fn replace(&mut self, root: u32, edge: Edge) -> bool {
        self.fwd.grow(self.aig.len());
        let target = self.fwd.resolve(edge);
        if target.node() == root {
            return false;
        }
        let n = self.aig.refs(root);
        self.aig.add_refs(target.node(), n);
        self.aig.set_refs(root, 0);
        self.fwd.set(root, target);
        true
    }
}

impl Aig {
    pub(crate) fn inc_ref(&mut self, id: u32) -> u32 {
        self.refs[id as usize] += 1;
        self.refs[id as usize]
    }

    pub(crate) fn dec_ref(&mut self, id: u32) -> u32 {
        let r = &mut self.refs[id as usize];
        *r = r.saturating_sub(1);
        *r
    }

    pub(crate) fn add_refs(&mut self, id: u32, n: u32) {
        self.refs[id as usize] += n;
    }

    pub(crate) fn set_refs(&mut self, id: u32, n: u32) {
        self.refs[id as usize] = n;
    }
}

/// A way to build a small AND/inverter structure over cut leaves, either
/// for counting or for real.
pub trait Synth {
    /// The handle for a signal.
    type Sig: Copy;
    /// The `i`-th leaf.
    fn leaf(&self, i: usize) -> Self::Sig;
    /// A constant.
    fn constant(&self, value: bool) -> Self::Sig;
    /// The complement.
    fn not(&self, s: Self::Sig) -> Self::Sig;
    /// The AND.
    fn and(&mut self, a: Self::Sig, b: Self::Sig) -> Self::Sig;
    /// The OR, as `!(!a & !b)`.
    fn or(&mut self, a: Self::Sig, b: Self::Sig) -> Self::Sig {
        let t = self.and(self.not(a), self.not(b));
        self.not(t)
    }
    /// The XOR, as three ANDs.
    fn xor(&mut self, a: Self::Sig, b: Self::Sig) -> Self::Sig {
        let t = self.and(a, self.not(b));
        let u = self.and(self.not(a), b);
        self.or(t, u)
    }
}

/// A signal in a dry run: an existing edge, or a node that would be new.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trial {
    /// An edge that exists (possibly a leaf or constant).
    Real(Edge),
    /// The `n`-th node the structure would add, possibly complemented.
    New(u32, bool),
}

/// Counts how many nodes a structure would add to the graph: existing
/// nodes found through the hash table cost nothing, unless they are dead
/// (reference count zero, which is where the dereferenced MFFC sits).
pub struct Counter<'v, 'a> {
    view: &'v View<'a>,
    leaves: &'v [Edge],
    root: u32,
    /// Nodes the structure would add.
    pub added: u32,
    /// True when the structure hashed onto the root being replaced, which
    /// would make the replacement circular; such a candidate is rejected.
    pub hit_root: bool,
    memo: std::collections::HashMap<(Trial, Trial), Trial>,
}

impl<'v, 'a> Counter<'v, 'a> {
    /// A counter over `leaves` in `view`, for a structure meant to replace
    /// `root`.
    pub fn new(view: &'v View<'a>, leaves: &'v [Edge], root: u32) -> Counter<'v, 'a> {
        Counter {
            view,
            leaves,
            root,
            added: 0,
            hit_root: false,
            memo: std::collections::HashMap::new(),
        }
    }
}

impl Synth for Counter<'_, '_> {
    type Sig = Trial;

    fn leaf(&self, i: usize) -> Trial {
        Trial::Real(self.leaves[i])
    }

    fn constant(&self, value: bool) -> Trial {
        Trial::Real(Edge::constant(value))
    }

    fn not(&self, s: Trial) -> Trial {
        match s {
            Trial::Real(e) => Trial::Real(!e),
            Trial::New(n, c) => Trial::New(n, !c),
        }
    }

    fn and(&mut self, a: Trial, b: Trial) -> Trial {
        let key = if a <= b { (a, b) } else { (b, a) };
        if let Some(&r) = self.memo.get(&key) {
            return r;
        }
        let result = match (a, b) {
            (Trial::Real(x), Trial::Real(y)) => match self.view.aig.lookup(x, y) {
                Some(e) if e.node() == self.root => {
                    self.hit_root = true;
                    self.added += 1;
                    Trial::New(self.added, false)
                }
                Some(e) if e.is_const() || !self.view.aig.is_and(e.node()) => Trial::Real(e),
                Some(e) if self.view.aig.refs(e.node()) > 0 => Trial::Real(e),
                Some(e) => {
                    // A dead node: it would have to be revived, which costs
                    // as much as a new node.
                    self.added += 1;
                    Trial::Real(e)
                }
                None => {
                    self.added += 1;
                    Trial::New(self.added, false)
                }
            },
            _ => {
                self.added += 1;
                Trial::New(self.added, false)
            }
        };
        self.memo.insert(key, result);
        result
    }
}

impl PartialOrd for Trial {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Trial {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let key = |t: &Trial| match t {
            Trial::Real(e) => (0u8, e.raw(), false),
            Trial::New(n, c) => (1u8, *n, *c),
        };
        key(self).cmp(&key(other))
    }
}

/// Builds a structure for real, reviving dead nodes it hashes onto.
pub struct Builder<'v, 'a> {
    view: &'v mut View<'a>,
    leaves: Vec<Edge>,
    root: u32,
    revived: std::collections::HashSet<u32>,
}

impl<'v, 'a> Builder<'v, 'a> {
    /// A builder over `leaves` in `view`, for a structure replacing `root`
    /// (0 when nothing is replaced).
    pub fn new(view: &'v mut View<'a>, leaves: Vec<Edge>, root: u32) -> Builder<'v, 'a> {
        Builder {
            view,
            leaves,
            root,
            revived: std::collections::HashSet::new(),
        }
    }
}

impl Synth for Builder<'_, '_> {
    type Sig = Edge;

    fn leaf(&self, i: usize) -> Edge {
        self.leaves[i]
    }

    fn constant(&self, value: bool) -> Edge {
        Edge::constant(value)
    }

    fn not(&self, s: Edge) -> Edge {
        !s
    }

    fn and(&mut self, a: Edge, b: Edge) -> Edge {
        if let Some(e) = self.view.aig.lookup(a, b) {
            let n = e.node();
            assert!(n != self.root, "structure hashed onto the node it replaces");
            // A dead pre-existing node is revived once; leaves keep their
            // counted cones and never need it.
            if !e.is_const()
                && self.view.aig.is_and(n)
                && self.view.aig.refs(n) == 0
                && (n as usize) < self.view.old_len
                && !self.leaves.iter().any(|l| l.node() == n)
                && self.revived.insert(n)
            {
                self.view.revive(n, &self.leaves);
            }
            return e;
        }
        // A new node; `Aig::and` counts its fanins, which are leaves,
        // constants or signals this builder already made alive.
        self.view.aig.and(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deref_counts_the_mffc() {
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let c = g.add_input();
        let ab = g.and(a, b);
        let abc = g.and(ab, c);
        let root = g.and(abc, !a);
        g.add_output(root);
        g.add_output(ab);
        let mut view = View::new(&mut g);
        // `ab` has another fanout, so the MFFC of root is {root, abc}.
        let leaves = [a.node(), b.node(), c.node()];
        assert_eq!(view.deref_cone(root.node(), &leaves), 2);
        assert_eq!(view.aig.refs(ab.node()), 1);
        assert_eq!(view.aig.refs(abc.node()), 0);
        view.ref_cone(root.node(), &leaves);
        assert_eq!(view.aig.refs(abc.node()), 1);
        assert_eq!(view.aig.refs(ab.node()), 2);
        // Counting: rebuilding the same structure costs nothing while
        // alive, everything once dereferenced.
        let leaf_edges = [a, b, c];
        let mut counter = Counter::new(&view, &leaf_edges, root.node());
        let t = counter.and(Trial::Real(a), Trial::Real(b));
        let u = counter.and(t, Trial::Real(c));
        let _ = counter.and(u, Trial::Real(!a));
        assert_eq!(counter.added, 1);
        assert!(counter.hit_root);
        view.deref_cone(root.node(), &leaves);
        let mut counter = Counter::new(&view, &leaf_edges, root.node());
        let t = counter.and(Trial::Real(a), Trial::Real(b));
        let u = counter.and(t, Trial::Real(c));
        let v = counter.and(u, Trial::Real(!a));
        assert_eq!(counter.added, 2);
        assert!(counter.hit_root);
        assert_eq!(counter.and(v, t), counter.and(t, v));
        assert_eq!(counter.added, 3);
        let n = counter.not(Trial::New(1, false));
        assert_eq!(n, Trial::New(1, true));
        assert_eq!(counter.leaf(1), Trial::Real(b));
        assert_eq!(counter.constant(true), Trial::Real(Edge::TRUE));
        // Building a different structure and replacing the root.
        let mut builder = Builder::new(&mut view, vec![a, b, c], root.node());
        let x = builder.and(c, !a);
        let y = builder.and(x, b);
        assert_eq!(builder.leaf(0), a);
        assert_eq!(builder.constant(false), Edge::FALSE);
        assert!(view.replace(root.node(), y));
        assert_eq!(view.aig.refs(root.node()), 0);
        // Replacing a node by itself is refused: `ab` still stands for
        // itself, so mapping it onto itself would make the map cyclic.
        assert!(!view.replace(ab.node(), Edge::plain(ab.node())));
        // `c` and `a` gained references, `abc` is dead and uncounted.
        assert_eq!(view.aig.refs(abc.node()), 0);
        assert_eq!(view.aig.refs(c.node()), 1);
        let mut fwd = view.fwd.clone();
        let r = g.rebuild_with(&mut fwd);
        assert_eq!(r.num_ands(), 3);
        for pat in 0..8u32 {
            let ins = [pat & 1 == 1, pat & 2 == 2, pat & 4 == 4];
            assert_eq!(r.eval(&ins)[0], ins[2] && !ins[0] && ins[1]);
        }
    }
}
