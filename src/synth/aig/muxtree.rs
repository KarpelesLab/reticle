//! Keeping wide multiplexers in the shape they were written in.
//!
//! [`optimize`](super::optimize) rewrites for fewer AND nodes, and a
//! multiplexer tree is exactly what that takes apart: on an 8-to-1 of
//! `testdata/fpga/vivado/muxf/`, what reached the mapper from it was four
//! six-input tables three levels deep, where the tree itself is two tables and
//! a `MUXF7`, or three tables two levels deep without one. No cut enumerated
//! over the rewritten graph finds what the rewriting removed.
//!
//! So [`optimize_around_trees`] takes the trees out first. A **tree** is a
//! multiplexer whose data are multiplexers read by nothing else, at least
//! [`MIN_LEVELS`] full levels of them: the rest of the graph is optimised with
//! each tree's root as an input and its data and selects as outputs, and each
//! tree is then put back as it was, over whatever those became.
//!
//! The levels are a bar so that a chain of `if`/`else` stays with the optimiser,
//! which does well by one, and a four-to-one, which fits one table anyway.
//!
//! **Nothing here is trusted.** `techmap` proves a network mapped from this
//! graph against the graph from before all of it, not against this one.

use super::{Aig, AigOptions, Edge, optimize};

/// How many full levels make a tree worth keeping: an 8-to-1, the narrowest
/// that one table cannot hold and a `MUXF7` can.
const MIN_LEVELS: u32 = 3;

/// A node that is `select ? when1 : when0`, as an AIG writes one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MuxShape {
    /// The select, in plain polarity: the other polarity swaps the data.
    pub select: u32,
    /// What the node is with the select low.
    pub when0: Edge,
    /// What the node is with the select high.
    pub when1: Edge,
    /// The two AND nodes under it, which hold the select.
    gates: (u32, u32),
}

impl MuxShape {
    /// The shape of `id`, if it is one: `!(s & a) & !(!s & b)`, which is
    /// `s ? !a : !b`. Data of one node are an exclusive OR, not a choice.
    pub(crate) fn of(aig: &Aig, id: u32) -> Option<MuxShape> {
        let (left, right) = aig.fanins(id);
        if !left.is_complement()
            || !right.is_complement()
            || !aig.is_and(left.node())
            || !aig.is_and(right.node())
        {
            return None;
        }
        let (a0, a1) = aig.fanins(left.node());
        let (b0, b1) = aig.fanins(right.node());
        for (s, data) in [(a0, a1), (a1, a0)] {
            for (other, data_other) in [(b0, b1), (b1, b0)] {
                if s != other.xor(true) {
                    continue;
                }
                let (select, when1, when0) = if s.is_complement() {
                    (other.node(), data_other.xor(true), data.xor(true))
                } else {
                    (s.node(), data.xor(true), data_other.xor(true))
                };
                if when0.node() == when1.node() || select == when0.node() || select == when1.node()
                {
                    continue;
                }
                return Some(MuxShape {
                    select,
                    when0,
                    when1,
                    gates: (left.node(), right.node()),
                });
            }
        }
        None
    }
}

/// One multiplexer of a tree, with its data as what it reads.
struct Member {
    node: u32,
    /// Which of [`Tree::reads`] the select is.
    select: usize,
    when0: Datum,
    when1: Datum,
}

/// A data input: another member of the tree, or one of [`Tree::reads`].
#[derive(Clone, Copy)]
enum Datum {
    Member { node: u32, complement: bool },
    Read(usize),
}

/// One kept tree. Its members are in topological order, so the root is last.
struct Tree {
    members: Vec<Member>,
    /// What it reads from the rest of the graph, as edges of the original.
    reads: Vec<Edge>,
    /// Where those are among the outputs of the graph that is optimised.
    first_output: usize,
}

/// The kept trees of `aig`, whose reference counts must be current, and which
/// nodes they take with them.
fn find_trees(aig: &Aig) -> (Vec<Tree>, Vec<bool>) {
    let n = aig.len();
    let shapes: Vec<Option<MuxShape>> = (0..u32::try_from(n).expect("node count"))
        .map(|id| {
            if !aig.is_and(id) {
                return None;
            }
            MuxShape::of(aig, id).filter(|s| aig.refs(s.gates.0) == 1 && aig.refs(s.gates.1) == 1)
        })
        .collect();
    // How many full levels are under each multiplexer, through data nothing
    // else reads.
    let mut levels = vec![0u32; n];
    let below = |levels: &[u32], e: Edge| {
        if aig.refs(e.node()) == 1 {
            levels[e.index()]
        } else {
            0
        }
    };
    for id in 0..n {
        if let Some(s) = &shapes[id] {
            levels[id] = 1 + below(&levels, s.when0).min(below(&levels, s.when1));
        }
    }
    let member = |e: Edge| aig.refs(e.node()) == 1 && shapes[e.index()].is_some();

    let mut taken = vec![false; n];
    let mut trees = Vec::new();
    let mut first_output = aig.outputs().len();
    for root in (0..n).rev() {
        if taken[root] || levels[root] < MIN_LEVELS {
            continue;
        }
        let mut nodes = Vec::new();
        let mut work = vec![u32::try_from(root).expect("node index")];
        while let Some(id) = work.pop() {
            nodes.push(id);
            let s = shapes[id as usize].expect("a member is a multiplexer");
            for data in [s.when0, s.when1] {
                if member(data) {
                    work.push(data.node());
                }
            }
        }
        nodes.sort_unstable();
        let mut reads: Vec<Edge> = Vec::new();
        let mut read = |e: Edge| {
            reads.iter().position(|r| *r == e).unwrap_or_else(|| {
                reads.push(e);
                reads.len() - 1
            })
        };
        let mut members = Vec::with_capacity(nodes.len());
        for id in nodes {
            let s = shapes[id as usize].expect("a member is a multiplexer");
            let mut datum = |e: Edge| {
                if member(e) {
                    Datum::Member {
                        node: e.node(),
                        complement: e.is_complement(),
                    }
                } else {
                    Datum::Read(read(e))
                }
            };
            let (when0, when1) = (datum(s.when0), datum(s.when1));
            members.push(Member {
                node: id,
                select: read(Edge::plain(s.select)),
                when0,
                when1,
            });
            taken[s.gates.0 as usize] = true;
            taken[s.gates.1 as usize] = true;
            taken[id as usize] = true;
        }
        let count = reads.len();
        trees.push(Tree {
            members,
            reads,
            first_output,
        });
        first_output += count;
    }
    (trees, taken)
}

/// [`optimize`], leaving every wide multiplexer tree of `aig` as it is: see the
/// module docs. Inputs and outputs keep their positions. `None` when `aig` has
/// no such tree, so that it would only be [`optimize`] over again.
pub(crate) fn optimize_around_trees(aig: &Aig, options: &AigOptions) -> Option<Aig> {
    let mut aig = aig.rebuild();
    aig.recount_refs();
    let (trees, taken) = find_trees(&aig);
    if trees.is_empty() {
        return None;
    }

    // The rest: a tree's root is an input of it, and what the tree reads is
    // an output.
    let mut rest = Aig::new();
    for _ in aig.inputs() {
        rest.add_input();
    }
    let mut copy: Vec<Edge> = vec![Edge::FALSE; aig.len()];
    for (pos, &pi) in aig.inputs().iter().enumerate() {
        copy[pi as usize] = Edge::plain(rest.inputs()[pos]);
    }
    let real_inputs = aig.inputs().len();
    for tree in &trees {
        let root = tree.members.last().expect("a tree has members").node;
        copy[root as usize] = rest.add_input();
    }
    for id in 0..u32::try_from(aig.len()).expect("node count") {
        if !aig.is_and(id) || taken[id as usize] {
            continue;
        }
        let (a, b) = aig.fanins(id);
        copy[id as usize] = rest.and(
            copy[a.index()].xor(a.is_complement()),
            copy[b.index()].xor(b.is_complement()),
        );
    }
    for &out in aig.outputs() {
        rest.add_output(copy[out.index()].xor(out.is_complement()));
    }
    for tree in &trees {
        for read in &tree.reads {
            rest.add_output(copy[read.index()].xor(read.is_complement()));
        }
    }
    optimize(&mut rest, options);

    // Back together: `rest`'s nodes become a new graph's, and an input that
    // stands for a tree becomes that tree once all it reads has been built.
    // Nothing here can go round in a circle, since `rest`'s outputs depend on
    // no input they did not depend on before it was optimised.
    let mut out = Aig::new();
    for _ in aig.inputs() {
        out.add_input();
    }
    let mut value: Vec<Option<Edge>> = vec![None; rest.len()];
    value[0] = Some(Edge::FALSE);
    let mut tree_of = vec![usize::MAX; rest.len()];
    for (pos, &pi) in rest.inputs().iter().enumerate() {
        if pos < real_inputs {
            value[pi as usize] = Some(Edge::plain(out.inputs()[pos]));
        } else {
            tree_of[pi as usize] = pos - real_inputs;
        }
    }
    let outputs = rest.outputs().to_vec();
    let mut stack: Vec<u32> = outputs.iter().map(|e| e.node()).collect();
    while let Some(&id) = stack.last() {
        if value[id as usize].is_some() {
            stack.pop();
            continue;
        }
        let tree = tree_of[id as usize];
        if tree == usize::MAX {
            let (a, b) = rest.fanins(id);
            match (value[a.index()], value[b.index()]) {
                (Some(x), Some(y)) => {
                    value[id as usize] =
                        Some(out.and(x.xor(a.is_complement()), y.xor(b.is_complement())));
                    stack.pop();
                }
                (x, y) => {
                    if x.is_none() {
                        stack.push(a.node());
                    }
                    if y.is_none() {
                        stack.push(b.node());
                    }
                }
            }
            continue;
        }
        let tree = &trees[tree];
        let reads: Vec<Edge> = (0..tree.reads.len())
            .map(|k| outputs[tree.first_output + k])
            .collect();
        let missing: Vec<u32> = reads
            .iter()
            .filter(|e| value[e.index()].is_none())
            .map(|e| e.node())
            .collect();
        if !missing.is_empty() {
            stack.extend(missing);
            continue;
        }
        let read = |k: usize| {
            let e = reads[k];
            value[e.index()]
                .expect("built above")
                .xor(e.is_complement())
        };
        let mut built: Vec<(u32, Edge)> = Vec::with_capacity(tree.members.len());
        for m in &tree.members {
            let datum = |d: Datum, built: &[(u32, Edge)]| match d {
                Datum::Read(k) => read(k),
                Datum::Member { node, complement } => built
                    .iter()
                    .find(|(n, _)| *n == node)
                    .expect("members are in topological order")
                    .1
                    .xor(complement),
            };
            let (when0, when1) = (datum(m.when0, &built), datum(m.when1, &built));
            built.push((m.node, out.mux(read(m.select), when1, when0)));
        }
        value[id as usize] = Some(built.last().expect("a tree has members").1);
        stack.pop();
    }
    for e in outputs.iter().take(aig.outputs().len()) {
        let edge = value[e.index()]
            .expect("built above")
            .xor(e.is_complement());
        out.add_output(edge);
    }
    Some(out.rebuild())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full tree over fresh inputs, `levels` selects deep, and what it
    /// should answer given an input pattern.
    fn tree(levels: usize) -> (Aig, impl Fn(&[bool]) -> bool) {
        let mut aig = Aig::new();
        let select: Vec<Edge> = (0..levels).map(|_| aig.add_input()).collect();
        let mut layer: Vec<Edge> = (0..1usize << levels).map(|_| aig.add_input()).collect();
        for s in &select {
            layer = layer
                .chunks(2)
                .map(|pair| aig.mux(*s, pair[1], pair[0]))
                .collect();
        }
        aig.add_output(layer[0]);
        (aig, move |pattern: &[bool]| {
            let mut index = 0;
            for (bit, value) in pattern[..levels].iter().enumerate() {
                if *value {
                    index |= 1 << bit;
                }
            }
            pattern[levels + index]
        })
    }

    /// **It would catch** a tree put back with its data swapped or a select
    /// misread, at any of its levels, and a polarity lost on the way through
    /// `MuxShape`: every pattern of the 4-level tree is tried. It says nothing
    /// about the mapper, which `techmap`'s own tests and proof answer for.
    #[test]
    fn a_tree_put_back_answers_as_it_did() {
        let (aig, answer) = tree(4);
        let kept = optimize_around_trees(&aig, &AigOptions::default()).expect("it has a tree");
        for raw in 0..1u32 << (4 + 16) {
            // Sparse, with both ends: every select against a walking data bit.
            if raw & 0xF_0000 != 0 && raw.count_ones() > 6 {
                continue;
            }
            let pattern: Vec<bool> = (0..20).map(|b| (raw >> b) & 1 == 1).collect();
            assert_eq!(kept.eval(&pattern)[0], answer(&pattern), "pattern {raw:#x}");
        }
    }

    /// The point of the pass: what comes out still reads as a tree.
    #[test]
    fn a_tree_keeps_its_multiplexers_where_optimizing_it_need_not() {
        let (aig, _) = tree(4);
        let kept = optimize_around_trees(&aig, &AigOptions::default()).expect("it has a tree");
        let shapes = (0..u32::try_from(kept.len()).unwrap())
            .filter(|id| kept.is_and(*id) && MuxShape::of(&kept, *id).is_some())
            .count();
        assert_eq!(shapes, 15, "a 16-to-1 is fifteen two-to-one choices");
    }

    /// Under the bar there is nothing to keep, which is what leaves a design
    /// with no wide multiplexer on [`optimize`] alone.
    #[test]
    fn a_four_to_one_is_no_tree() {
        let (aig, _) = tree(2);
        assert!(optimize_around_trees(&aig, &AigOptions::default()).is_none());
    }

    /// A shared table is a tree's edge: the node stays in the rest of the
    /// graph, read by both, and both still answer.
    #[test]
    fn a_datum_read_elsewhere_is_left_for_the_rest_of_the_graph() {
        let mut aig = Aig::new();
        let select: Vec<Edge> = (0..4).map(|_| aig.add_input()).collect();
        let data: Vec<Edge> = (0..16).map(|_| aig.add_input()).collect();
        let shared = aig.mux(select[0], data[1], data[0]);
        let mut layer = vec![shared];
        for pair in data[2..].chunks(2).take(7) {
            layer.push(aig.mux(select[0], pair[1], pair[0]));
        }
        for s in &select[1..] {
            layer = layer
                .chunks(2)
                .map(|pair| aig.mux(*s, pair[1], pair[0]))
                .collect();
        }
        aig.add_output(layer[0]);
        aig.add_output(shared);
        let kept = optimize_around_trees(&aig, &AigOptions::default()).expect("it has a tree");
        let mut rng = 0x1234_5678u32;
        for _ in 0..4096 {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let pattern: Vec<bool> = (0..20).map(|b| (rng >> b) & 1 == 1).collect();
            assert_eq!(kept.eval(&pattern), aig.eval(&pattern));
        }
    }
}
