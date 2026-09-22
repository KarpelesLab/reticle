//! And-Inverter Graphs: the logic optimisation core.
//!
//! An AIG represents combinational logic as a directed acyclic graph whose
//! only gate is the two-input AND, with inversions carried on the edges.
//! The representation is small, uniform, and every optimisation on it is a
//! local graph transformation, which is why it is the internal form of ABC
//! and of most modern logic synthesis tools. Reticle follows the same
//! design (Mishchenko, Chatterjee, Brayton, "DAG-aware AIG rewriting: a
//! fresh look at combinational logic synthesis", DAC 2006, and the ABC
//! system it describes).
//!
//! # Representation
//!
//! - A [`Node`] is an AND with two fanin [`Edge`]s, a primary input, or the
//!   constant node. Node 0 is always the constant *false*; primary inputs
//!   are registered in creation order.
//! - An [`Edge`] packs a node index and a complement bit in one `u32`, so
//!   `!edge` is a bit flip and inversions cost nothing.
//! - Nodes are created in topological order: a node's fanins always have
//!   smaller indices. Every builder keeps this invariant, so a plain index
//!   loop is a topological traversal.
//! - **Structural hashing** ([`Aig::and`]) normalises operand order, folds
//!   constants and the trivial cases `a & a`, `a & !a`, and looks the pair
//!   up in a hash table, so two structurally identical sub-graphs are
//!   always the same node.
//! - Reference counts ([`Aig::refs`]) record how many fanins and outputs
//!   point at a node, which the rewriting passes use to size maximum
//!   fanout-free cones.
//!
//! # Passes
//!
//! | Pass                  | Effect                                                      |
//! |-----------------------|-------------------------------------------------------------|
//! | [`Aig::rebuild`]      | `strash` + `sweep`: re-hash everything, drop dead nodes      |
//! | [`balance::balance`]  | Rebuild AND trees balanced by level to reduce depth          |
//! | [`rewrite::rewrite`]  | 4-input cut rewriting against a precomputed function table   |
//! | [`refactor::refactor`]| Collapse a large fanout-free cone and re-factor it            |
//! | [`fraig::fraig`]      | Merge functionally equivalent nodes (simulation, then proof)  |
//! | [`optimize`]          | The default script running the above to a fixed point         |
//!
//! [`blast::from_module`] bit-blasts the combinational part of a cell-form
//! [`Module`](crate::ir::Module) into an AIG and [`emit::to_module`] writes
//! an AIG back as `And` / `Not` cells; the technology mappers in
//! [`super::techmap`] write LUT or gate networks through the same path.

use std::collections::HashMap;
use std::fmt;
use std::ops::Not;

pub mod arith;
pub mod balance;
pub mod blast;
pub mod cut;
pub mod emit;
pub mod fraig;
pub mod mffc;
pub mod refactor;
pub mod rewrite;
pub mod truth;

pub use blast::{BitRef, Mapping, from_module};
pub use emit::to_module;

/// A reference to a node, with a complement bit.
///
/// The node index lives in the upper 31 bits and the complement in bit 0,
/// so `!edge` flips one bit and edges order first by node then by polarity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge(u32);

impl Edge {
    /// The constant *false*: the plain edge to node 0.
    pub const FALSE: Edge = Edge(0);
    /// The constant *true*: the complemented edge to node 0.
    pub const TRUE: Edge = Edge(1);

    /// An edge to `node`, complemented when `complement` is set.
    pub fn new(node: u32, complement: bool) -> Edge {
        Edge((node << 1) | u32::from(complement))
    }

    /// The plain edge to `node`.
    pub fn plain(node: u32) -> Edge {
        Edge(node << 1)
    }

    /// The constant edge for `value`.
    pub fn constant(value: bool) -> Edge {
        if value { Edge::TRUE } else { Edge::FALSE }
    }

    /// The node this edge points at.
    pub fn node(self) -> u32 {
        self.0 >> 1
    }

    /// The node index as a `usize`, for table lookups.
    pub fn index(self) -> usize {
        // A `u32` always fits a `usize` on supported targets.
        (self.0 >> 1) as usize
    }

    /// True when the edge carries an inversion.
    pub fn is_complement(self) -> bool {
        self.0 & 1 == 1
    }

    /// The same edge without its complement bit.
    pub fn regular(self) -> Edge {
        Edge(self.0 & !1)
    }

    /// This edge complemented when `c` is set.
    pub fn xor(self, c: bool) -> Edge {
        Edge(self.0 ^ u32::from(c))
    }

    /// True for either constant edge.
    pub fn is_const(self) -> bool {
        self.0 >> 1 == 0
    }

    /// The constant value when the edge is constant.
    pub fn const_value(self) -> Option<bool> {
        if self.is_const() {
            Some(self.is_complement())
        } else {
            None
        }
    }

    /// The packed `u32`, node index shifted left with the complement in
    /// bit 0.
    pub fn raw(self) -> u32 {
        self.0
    }

    /// The edge with the given packed value.
    pub fn from_raw(raw: u32) -> Edge {
        Edge(raw)
    }
}

impl Not for Edge {
    type Output = Edge;

    fn not(self) -> Edge {
        Edge(self.0 ^ 1)
    }
}

impl fmt::Debug for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_complement() {
            write!(f, "!n{}", self.node())
        } else {
            write!(f, "n{}", self.node())
        }
    }
}

impl fmt::Display for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// Node 0, the constant *false*.
    Const,
    /// A primary input; the payload is its position in [`Aig::inputs`].
    Input(u32),
    /// A two-input AND.
    And,
}

/// One node of the graph.
///
/// Inputs and the constant carry [`Edge::FALSE`] in both fanins; only
/// [`NodeKind::And`] nodes have meaningful fanins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    /// First operand (the smaller edge, by the ordering of [`Edge`]).
    pub fanin0: Edge,
    /// Second operand.
    pub fanin1: Edge,
    /// The node's role.
    pub kind: NodeKind,
}

/// Size figures of an AIG.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AigStats {
    /// Number of AND nodes.
    pub nodes: usize,
    /// Number of primary inputs.
    pub inputs: usize,
    /// Number of primary outputs.
    pub outputs: usize,
    /// Depth: the largest number of AND nodes on any input-to-output path.
    pub levels: u32,
}

impl fmt::Display for AigStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "i/o = {}/{}  and = {}  lev = {}",
            self.inputs, self.outputs, self.nodes, self.levels
        )
    }
}

/// An And-Inverter Graph; see the module docs.
#[derive(Clone)]
pub struct Aig {
    nodes: Vec<Node>,
    inputs: Vec<u32>,
    outputs: Vec<Edge>,
    table: HashMap<(Edge, Edge), u32>,
    refs: Vec<u32>,
    levels: Vec<u32>,
}

impl Default for Aig {
    fn default() -> Self {
        Aig::new()
    }
}

impl fmt::Debug for Aig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "aig {{")?;
        for (i, node) in self.nodes.iter().enumerate() {
            match node.kind {
                NodeKind::Const => writeln!(f, "  n{i} = const0")?,
                NodeKind::Input(p) => writeln!(f, "  n{i} = input {p}")?,
                NodeKind::And => {
                    writeln!(f, "  n{i} = and({}, {})", node.fanin0, node.fanin1)?;
                }
            }
        }
        for (i, out) in self.outputs.iter().enumerate() {
            writeln!(f, "  output {i} = {out}")?;
        }
        write!(f, "}}")
    }
}

impl Aig {
    /// An empty graph holding only the constant node.
    pub fn new() -> Aig {
        let mut aig = Aig {
            nodes: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            table: HashMap::new(),
            refs: Vec::new(),
            levels: Vec::new(),
        };
        aig.push_node(Node {
            fanin0: Edge::FALSE,
            fanin1: Edge::FALSE,
            kind: NodeKind::Const,
        });
        aig
    }

    fn push_node(&mut self, node: Node) -> u32 {
        let id = u32::try_from(self.nodes.len()).expect("AIG exceeds 2^31 nodes");
        assert!(id < u32::MAX >> 1, "AIG exceeds 2^31 nodes");
        self.nodes.push(node);
        self.refs.push(0);
        self.levels.push(0);
        id
    }

    // ---- construction ------------------------------------------------------

    /// Adds a primary input and returns the plain edge to it.
    pub fn add_input(&mut self) -> Edge {
        let pos = u32::try_from(self.inputs.len()).expect("too many inputs");
        let id = self.push_node(Node {
            fanin0: Edge::FALSE,
            fanin1: Edge::FALSE,
            kind: NodeKind::Input(pos),
        });
        self.inputs.push(id);
        Edge::plain(id)
    }

    /// Registers `edge` as a primary output and returns its position.
    pub fn add_output(&mut self, edge: Edge) -> usize {
        self.refs[edge.index()] += 1;
        self.outputs.push(edge);
        self.outputs.len() - 1
    }

    /// Redirects output `index` to `edge`.
    pub fn set_output(&mut self, index: usize, edge: Edge) {
        let old = self.outputs[index];
        self.refs[old.index()] -= 1;
        self.refs[edge.index()] += 1;
        self.outputs[index] = edge;
    }

    /// Normalises an AND's operands and folds the trivial cases.
    ///
    /// Returns `Ok(edge)` when the result needs no node, `Err((a, b))` with
    /// `a < b` otherwise.
    fn normalise(a: Edge, b: Edge) -> Result<Edge, (Edge, Edge)> {
        if a == Edge::FALSE || b == Edge::FALSE {
            return Ok(Edge::FALSE);
        }
        if a == Edge::TRUE {
            return Ok(b);
        }
        if b == Edge::TRUE {
            return Ok(a);
        }
        if a == b {
            return Ok(a);
        }
        if a == !b {
            return Ok(Edge::FALSE);
        }
        if a < b { Err((a, b)) } else { Err((b, a)) }
    }

    /// The AND of two edges, folded and structurally hashed.
    pub fn and(&mut self, a: Edge, b: Edge) -> Edge {
        let (a, b) = match Self::normalise(a, b) {
            Ok(edge) => return edge,
            Err(pair) => pair,
        };
        if let Some(&id) = self.table.get(&(a, b)) {
            return Edge::plain(id);
        }
        let level = self.levels[a.index()].max(self.levels[b.index()]) + 1;
        let id = self.push_node(Node {
            fanin0: a,
            fanin1: b,
            kind: NodeKind::And,
        });
        self.levels[id as usize] = level;
        self.refs[a.index()] += 1;
        self.refs[b.index()] += 1;
        self.table.insert((a, b), id);
        Edge::plain(id)
    }

    /// The edge `and(a, b)` would return without creating anything: a
    /// folded constant or operand, an existing node, or `None`.
    pub fn lookup(&self, a: Edge, b: Edge) -> Option<Edge> {
        match Self::normalise(a, b) {
            Ok(edge) => Some(edge),
            Err(pair) => self.table.get(&pair).map(|&id| Edge::plain(id)),
        }
    }

    /// The OR of two edges (`!(!a & !b)`).
    pub fn or(&mut self, a: Edge, b: Edge) -> Edge {
        !self.and(!a, !b)
    }

    /// The XOR of two edges, three AND nodes.
    pub fn xor(&mut self, a: Edge, b: Edge) -> Edge {
        if let Some(c) = a.const_value() {
            return b.xor(c);
        }
        if let Some(c) = b.const_value() {
            return a.xor(c);
        }
        if a.node() == b.node() {
            return Edge::constant(a != b);
        }
        let t = self.and(a, !b);
        let u = self.and(!a, b);
        self.or(t, u)
    }

    /// The XNOR of two edges.
    pub fn xnor(&mut self, a: Edge, b: Edge) -> Edge {
        !self.xor(a, b)
    }

    /// `if s then t else e`, three AND nodes.
    pub fn mux(&mut self, s: Edge, t: Edge, e: Edge) -> Edge {
        if let Some(c) = s.const_value() {
            return if c { t } else { e };
        }
        if t == e {
            return t;
        }
        if t == !e {
            return self.xor(s, e);
        }
        let a = self.and(s, t);
        let b = self.and(!s, e);
        self.or(a, b)
    }

    /// The AND of many edges as a balanced tree; `true` when empty.
    pub fn and_n(&mut self, edges: &[Edge]) -> Edge {
        self.reduce_tree(edges, Edge::TRUE, Aig::and)
    }

    /// The OR of many edges as a balanced tree; `false` when empty.
    pub fn or_n(&mut self, edges: &[Edge]) -> Edge {
        self.reduce_tree(edges, Edge::FALSE, Aig::or)
    }

    /// The XOR of many edges as a balanced tree; `false` when empty.
    pub fn xor_n(&mut self, edges: &[Edge]) -> Edge {
        self.reduce_tree(edges, Edge::FALSE, Aig::xor)
    }

    fn reduce_tree(
        &mut self,
        edges: &[Edge],
        unit: Edge,
        op: fn(&mut Aig, Edge, Edge) -> Edge,
    ) -> Edge {
        match edges {
            [] => unit,
            [e] => *e,
            _ => {
                let (l, r) = edges.split_at(edges.len() / 2);
                let a = self.reduce_tree(l, unit, op);
                let b = self.reduce_tree(r, unit, op);
                op(self, a, b)
            }
        }
    }

    // ---- queries ---------------------------------------------------------

    /// Number of nodes including the constant and the inputs.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the graph holds only the constant node.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 1
    }

    /// Number of AND nodes.
    pub fn num_ands(&self) -> usize {
        self.nodes.len() - 1 - self.inputs.len()
    }

    /// The node with index `id`.
    pub fn node(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    /// True when node `id` is an AND.
    pub fn is_and(&self, id: u32) -> bool {
        self.nodes[id as usize].kind == NodeKind::And
    }

    /// The fanins of AND node `id`.
    pub fn fanins(&self, id: u32) -> (Edge, Edge) {
        let n = &self.nodes[id as usize];
        (n.fanin0, n.fanin1)
    }

    /// The primary input nodes, in creation order.
    pub fn inputs(&self) -> &[u32] {
        &self.inputs
    }

    /// The primary outputs, in registration order.
    pub fn outputs(&self) -> &[Edge] {
        &self.outputs
    }

    /// Number of edges and outputs pointing at node `id`.
    pub fn refs(&self, id: u32) -> u32 {
        self.refs[id as usize]
    }

    /// The level of node `id`: 0 for inputs and the constant, one more than
    /// the deepest fanin for an AND.
    pub fn level(&self, id: u32) -> u32 {
        self.levels[id as usize]
    }

    /// The depth of the graph: the largest level of any output.
    pub fn max_level(&self) -> u32 {
        self.outputs
            .iter()
            .map(|e| self.levels[e.index()])
            .max()
            .unwrap_or(0)
    }

    /// Size figures.
    pub fn stats(&self) -> AigStats {
        AigStats {
            nodes: self.num_ands(),
            inputs: self.inputs.len(),
            outputs: self.outputs.len(),
            levels: self.max_level(),
        }
    }

    /// Recomputes every reference count from the fanins and outputs.
    pub fn recount_refs(&mut self) {
        self.refs.iter_mut().for_each(|r| *r = 0);
        for node in &self.nodes {
            if node.kind == NodeKind::And {
                self.refs[node.fanin0.index()] += 1;
                self.refs[node.fanin1.index()] += 1;
            }
        }
        for out in &self.outputs {
            self.refs[out.index()] += 1;
        }
    }

    /// Recomputes every level from the fanins.
    pub fn recompute_levels(&mut self) {
        for i in 0..self.nodes.len() {
            let node = self.nodes[i];
            self.levels[i] = match node.kind {
                NodeKind::And => {
                    self.levels[node.fanin0.index()].max(self.levels[node.fanin1.index()]) + 1
                }
                _ => 0,
            };
        }
    }

    /// True for each node reachable from an output.
    pub fn live_nodes(&self) -> Vec<bool> {
        let mut live = vec![false; self.nodes.len()];
        live[0] = true;
        for &id in &self.inputs {
            live[id as usize] = true;
        }
        for out in &self.outputs {
            live[out.index()] = true;
        }
        for i in (0..self.nodes.len()).rev() {
            if live[i] && self.nodes[i].kind == NodeKind::And {
                live[self.nodes[i].fanin0.index()] = true;
                live[self.nodes[i].fanin1.index()] = true;
            }
        }
        live
    }

    // ---- rebuilding ------------------------------------------------------

    /// Rebuilds the graph from its outputs through a fresh hash table:
    /// nodes that nothing reaches are dropped, every AND is re-hashed (so
    /// duplicates that in-place passes could not merge become one), levels
    /// and reference counts are exact, and node order is topological.
    ///
    /// Inputs keep their positions, outputs their order. This is the
    /// combined `strash` and `sweep` of other tools.
    pub fn rebuild(&self) -> Aig {
        self.rebuild_with(&mut Forward::identity(self.len()))
    }

    /// [`Aig::rebuild`] with every edge first redirected through `fwd`.
    pub fn rebuild_with(&self, fwd: &mut Forward) -> Aig {
        let mut out = Aig::new();
        for _ in &self.inputs {
            out.add_input();
        }
        let mut map: Vec<Option<Edge>> = vec![None; self.nodes.len()];
        map[0] = Some(Edge::FALSE);
        for (pos, &id) in self.inputs.iter().enumerate() {
            map[id as usize] = Some(Edge::plain(out.inputs[pos]));
        }
        let outputs = self.outputs.clone();
        for edge in outputs {
            let e = self.copy_cone(edge, fwd, &mut map, &mut out);
            out.add_output(e);
        }
        out
    }

    /// Copies the cone of `edge` into `out`, memoised through `map`.
    fn copy_cone(
        &self,
        edge: Edge,
        fwd: &mut Forward,
        map: &mut [Option<Edge>],
        out: &mut Aig,
    ) -> Edge {
        let edge = fwd.resolve(edge);
        // Iterative post-order so deep chains cannot overflow the stack.
        let mut stack: Vec<(u32, bool)> = vec![(edge.node(), false)];
        while let Some(&(id, expanded)) = stack.last() {
            if map[id as usize].is_some() {
                stack.pop();
                continue;
            }
            let (f0, f1) = self.fanins(id);
            let f0 = fwd.resolve(f0);
            let f1 = fwd.resolve(f1);
            if expanded {
                let a = map[f0.index()]
                    .expect("fanin mapped")
                    .xor(f0.is_complement());
                let b = map[f1.index()]
                    .expect("fanin mapped")
                    .xor(f1.is_complement());
                map[id as usize] = Some(out.and(a, b));
                stack.pop();
            } else {
                stack.last_mut().expect("stack").1 = true;
                if map[f1.index()].is_none() {
                    stack.push((f1.node(), false));
                }
                if map[f0.index()].is_none() {
                    stack.push((f0.node(), false));
                }
            }
        }
        map[edge.index()]
            .expect("root mapped")
            .xor(edge.is_complement())
    }

    // ---- simulation ------------------------------------------------------

    /// Bit-parallel simulation: `words` 64-bit words of patterns per input,
    /// laid out as `inputs[pi * words + w]`, giving `words` words per node
    /// laid out the same way (`result[node * words + w]`). The constant node
    /// simulates to zero.
    pub fn simulate(&self, inputs: &[u64], words: usize) -> Vec<u64> {
        assert_eq!(
            inputs.len(),
            self.inputs.len() * words,
            "input pattern size"
        );
        let mut vals = vec![0u64; self.nodes.len() * words];
        for (i, node) in self.nodes.iter().enumerate() {
            match node.kind {
                NodeKind::Const => {}
                NodeKind::Input(p) => {
                    let p = p as usize;
                    vals[i * words..(i + 1) * words]
                        .copy_from_slice(&inputs[p * words..(p + 1) * words]);
                }
                NodeKind::And => {
                    let a = node.fanin0.index() * words;
                    let b = node.fanin1.index() * words;
                    let ma = if node.fanin0.is_complement() { !0 } else { 0 };
                    let mb = if node.fanin1.is_complement() { !0 } else { 0 };
                    for w in 0..words {
                        let v = (vals[a + w] ^ ma) & (vals[b + w] ^ mb);
                        vals[i * words + w] = v;
                    }
                }
            }
        }
        vals
    }

    /// The value of `edge` in a simulation result from [`Aig::simulate`].
    pub fn sim_value(values: &[u64], words: usize, edge: Edge, word: usize) -> u64 {
        let v = values[edge.index() * words + word];
        if edge.is_complement() { !v } else { v }
    }

    /// Evaluates every output for one input assignment.
    pub fn eval(&self, inputs: &[bool]) -> Vec<bool> {
        let words: Vec<u64> = inputs.iter().map(|&b| u64::from(b)).collect();
        let vals = self.simulate(&words, 1);
        self.outputs
            .iter()
            .map(|&e| Aig::sim_value(&vals, 1, e, 0) & 1 == 1)
            .collect()
    }
}

/// A replacement map used by in-place passes: `fwd[node]` is the edge that
/// now stands for `node`. Chains are resolved with path compression.
///
/// Passes that merge or replace nodes record the substitution here rather
/// than editing fanouts, and finish with [`Aig::rebuild_with`], which applies
/// every substitution while re-hashing the graph.
///
/// # Invariant
///
/// The map must stay acyclic. A pass keeps it so by only ever mapping a
/// node to an edge it has just [`resolved`](Forward::resolve) — a fixed
/// point of the map — and never to the node itself. A replacement built
/// through the structural hash table can land on an *existing* node that
/// is itself already replaced, which is exactly how a cycle would
/// otherwise appear, so resolving first is not optional.
#[derive(Clone, Debug)]
pub struct Forward {
    map: Vec<Edge>,
}

impl Forward {
    /// The identity map over `len` nodes.
    pub fn identity(len: usize) -> Forward {
        Forward {
            map: (0..len)
                .map(|i| Edge::plain(u32::try_from(i).expect("node index")))
                .collect(),
        }
    }

    /// Makes sure nodes up to `len` have an entry.
    pub fn grow(&mut self, len: usize) {
        while self.map.len() < len {
            let i = u32::try_from(self.map.len()).expect("node index");
            self.map.push(Edge::plain(i));
        }
    }

    /// Records that `node` is now represented by `edge`.
    pub fn set(&mut self, node: u32, edge: Edge) {
        self.map[node as usize] = edge;
    }

    /// True when `node` has been replaced.
    pub fn is_replaced(&self, node: u32) -> bool {
        self.map[node as usize] != Edge::plain(node)
    }

    /// The edge `edge` currently stands for: the end of its substitution
    /// chain, with the accumulated polarity.
    ///
    /// # Panics
    ///
    /// If the map contains a cycle, which is a bug in the pass that built
    /// it (see the type's invariant).
    pub fn resolve(&mut self, edge: Edge) -> Edge {
        let start = edge.node();
        // `cur` is the edge currently standing for the plain edge to
        // `start`; each hop composes the next substitution with the parity
        // accumulated so far.
        let mut cur = Edge::plain(start);
        let mut hops = 0usize;
        loop {
            let next = self.map[cur.index()];
            if next.node() == cur.node() {
                break;
            }
            cur = next.xor(cur.is_complement());
            hops += 1;
            assert!(
                hops <= self.map.len(),
                "cycle in the AIG forwarding map at node {start}"
            );
        }
        if hops > 1 {
            // Path compression: the head now points straight at the end.
            self.map[start as usize] = cur;
        }
        debug_assert_eq!(
            self.map[cur.index()],
            Edge::plain(cur.node()),
            "resolve must end at a fixed point"
        );
        cur.xor(edge.is_complement())
    }
}

/// A small deterministic pseudo-random generator (xorshift64*), for
/// simulation vectors.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator seeded with `seed` (zero is remapped to a fixed value).
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Options for [`optimize`].
#[derive(Clone, Debug)]
pub struct AigOptions {
    /// Maximum number of `balance; rewrite; refactor; balance; rewrite -z`
    /// rounds; the loop stops early once a round no longer shrinks the
    /// graph.
    pub max_rounds: u32,
    /// Run functional reduction ([`fraig::fraig`]) at the end.
    pub fraig: bool,
    /// Seed for the simulation vectors used by `fraig`.
    pub seed: u64,
    /// Conflict limit per SAT call in `fraig` (with the `formal` feature).
    pub sat_conflicts: u64,
    /// Largest cut used by `refactor`, at most 12.
    pub refactor_cut: usize,
}

impl Default for AigOptions {
    fn default() -> Self {
        AigOptions {
            max_rounds: 3,
            fraig: true,
            seed: 1,
            sat_conflicts: 1000,
            refactor_cut: 10,
        }
    }
}

/// Runs the default optimisation script, in the spirit of ABC's `resyn2`:
///
/// ```text
/// strash;
/// repeat up to max_rounds while the node count drops:
///     balance; rewrite; refactor; balance; rewrite -z
/// fraig; strash
/// ```
///
/// Returns the statistics before and after.
pub fn optimize(aig: &mut Aig, options: &AigOptions) -> (AigStats, AigStats) {
    let before = aig.stats();
    *aig = aig.rebuild();
    let mut best = aig.num_ands();
    for _ in 0..options.max_rounds {
        balance::balance(aig);
        rewrite::rewrite(aig, false);
        refactor::refactor(aig, options.refactor_cut.clamp(4, 12));
        balance::balance(aig);
        rewrite::rewrite(aig, true);
        let now = aig.num_ands();
        if now >= best {
            break;
        }
        best = now;
    }
    if options.fraig {
        fraig::fraig(
            aig,
            &fraig::FraigOptions {
                seed: options.seed,
                sat_conflicts: options.sat_conflicts,
                ..fraig::FraigOptions::default()
            },
        );
    }
    balance::balance(aig);
    (before, aig.stats())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_pack_and_flip() {
        let e = Edge::new(5, true);
        assert_eq!(e.node(), 5);
        assert!(e.is_complement());
        assert_eq!(!e, Edge::plain(5));
        assert_eq!(e.regular(), Edge::plain(5));
        assert_eq!(Edge::TRUE.const_value(), Some(true));
        assert_eq!(Edge::FALSE.const_value(), Some(false));
        assert_eq!(e.const_value(), None);
        assert_eq!(format!("{e}"), "!n5");
        assert_eq!(Edge::from_raw(e.raw()), e);
        assert!(Edge::plain(3) < Edge::new(3, true));
        assert!(Edge::new(3, true) < Edge::plain(4));
    }

    #[test]
    fn strashing_folds_and_shares() {
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        assert_eq!(g.and(a, Edge::FALSE), Edge::FALSE);
        assert_eq!(g.and(Edge::TRUE, b), b);
        assert_eq!(g.and(a, a), a);
        assert_eq!(g.and(a, !a), Edge::FALSE);
        let ab = g.and(a, b);
        let ba = g.and(b, a);
        assert_eq!(ab, ba);
        assert_eq!(g.num_ands(), 1);
        assert_eq!(g.lookup(b, a), Some(ab));
        assert_eq!(g.lookup(!a, b), None);
        assert_eq!(g.refs(a.node()), 1);
        assert_eq!(g.level(ab.node()), 1);
        let x = g.xor(a, b);
        // `ab` plus the two half-products and the output AND of the xor.
        assert_eq!(g.num_ands(), 4);
        assert_eq!(g.xor(a, b), x);
        assert_eq!(g.xor(a, a), Edge::FALSE);
        assert_eq!(g.xor(a, !a), Edge::TRUE);
        assert_eq!(g.xor(a, Edge::TRUE), !a);
        assert_eq!(g.mux(Edge::TRUE, a, b), a);
        assert_eq!(g.mux(a, b, b), b);
        g.add_output(x);
        assert_eq!(g.eval(&[false, true]), [true]);
        assert_eq!(g.eval(&[true, true]), [false]);
        let s = g.stats();
        assert_eq!((s.inputs, s.outputs, s.nodes, s.levels), (2, 1, 4, 2));
        assert_eq!(s.to_string(), "i/o = 2/1  and = 4  lev = 2");
        // A mux of complementary arms is the xor it folds to.
        assert_eq!(g.mux(a, b, !b), g.xor(a, !b));
    }

    #[test]
    fn rebuild_drops_dead_nodes_and_merges() {
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let c = g.add_input();
        let ab = g.and(a, b);
        let _dead = g.and(b, c);
        let out = g.and(ab, c);
        g.add_output(out);
        g.add_output(!ab);
        assert_eq!(g.num_ands(), 3);
        let r = g.rebuild();
        assert_eq!(r.num_ands(), 2);
        assert_eq!(r.inputs().len(), 3);
        assert_eq!(r.outputs().len(), 2);
        for pat in 0..8u32 {
            let ins = [pat & 1 == 1, pat & 2 == 2, pat & 4 == 4];
            assert_eq!(g.eval(&ins), r.eval(&ins));
        }
        // Forwarding: replace `ab` by `c`, then rebuild.
        let mut fwd = Forward::identity(g.len());
        fwd.set(ab.node(), !c);
        let r2 = g.rebuild_with(&mut fwd);
        assert_eq!(r2.num_ands(), 0);
        assert_eq!(r2.outputs()[0], Edge::FALSE);
        assert_eq!(r2.outputs()[1], c);
        let live = g.live_nodes();
        assert!(live[out.node() as usize]);
        assert!(!live[_dead.node() as usize]);
    }

    #[test]
    fn forward_chains_resolve_with_parity() {
        let mut fwd = Forward::identity(5);
        fwd.set(1, Edge::new(2, true));
        fwd.set(2, Edge::new(3, true));
        fwd.set(3, Edge::plain(4));
        assert_eq!(fwd.resolve(Edge::plain(1)), Edge::plain(4));
        assert_eq!(fwd.resolve(Edge::new(1, true)), Edge::new(4, true));
        assert_eq!(fwd.resolve(Edge::plain(2)), Edge::new(4, true));
        assert!(fwd.is_replaced(1));
        assert!(!fwd.is_replaced(4));
        fwd.grow(7);
        assert_eq!(fwd.resolve(Edge::plain(6)), Edge::plain(6));
    }

    #[test]
    fn simulation_is_bit_parallel() {
        let mut g = Aig::new();
        let a = g.add_input();
        let b = g.add_input();
        let y = g.or(a, b);
        g.add_output(y);
        let vals = g.simulate(&[0b1100, 0b1010], 1);
        assert_eq!(Aig::sim_value(&vals, 1, y, 0) & 0xF, 0b1110);
        let mut rng = Rng::new(0);
        let x = rng.next_u64();
        assert_ne!(x, rng.next_u64());
        assert_eq!(Rng::new(7).next_u64(), Rng::new(7).next_u64());
        g.recount_refs();
        g.recompute_levels();
        assert_eq!(g.level(y.node()), 1);
        assert_eq!(g.refs(y.node()), 1);
        assert!(!g.is_empty());
        assert_eq!(g.and_n(&[]), Edge::TRUE);
        assert_eq!(g.or_n(&[]), Edge::FALSE);
        assert_eq!(g.xor_n(&[a]), a);
        let all = g.and_n(&[a, b, y]);
        g.set_output(0, all);
        assert_eq!(g.eval(&[true, true]), [true]);
        assert_eq!(g.eval(&[true, false]), [false]);
    }
}
