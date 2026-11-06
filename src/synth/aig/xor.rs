//! Rebuilding XOR logic for depth, sharing what its parities share.
//!
//! An XOR in an AIG is three AND nodes, `!(x & y) & !(!x & !y)`, and
//! [`super::balance`] cannot see through it: its super-gates are ANDs of
//! plain edges, and an XOR's own two ANDs hang off complemented ones. So a
//! parity written as a chain — a CRC's `c = c[0] ? (c >> 1) ^ POLY : c >> 1`
//! iterated, or any `^` reduction in source order — reaches the mapper as a
//! chain, and the mapper's cuts are cuts of that chain. `crc32` of the timing
//! corpus came out seven tables deep where Vivado's synthesis has three.
//!
//! Balancing each XOR tree on its own (the first version of this pass) did
//! not move it: a CRC's XORs are shared, each round's `c[0]` read by every
//! bit its polynomial taps, so no tree is anyone's alone and every shared
//! XOR stays where it is, a leaf of the trees above. So the pass collapses
//! instead. Every XOR whose two AND nodes are its own is linear, and the
//! value of one is the parity of a set of *leaves*, the nodes under it that
//! are not such XORs: its two operands' sets, a leaf in both cancelling, a
//! complemented edge flipping the parity. The XORs that something other
//! than another XOR reads are the region's *outputs*, and only those are
//! rebuilt; the rest are gone.
//!
//! They are rebuilt together, for the `k`-input tables they will be mapped
//! onto. First, as Paar's algorithm for the XOR count of a linear map, the
//! pair of signals found in the most outputs' sets becomes one XOR, which
//! replaces the pair in each of them: that is what keeps the sharing. A pair
//! is taken only into the sets it costs nothing, their shallowest tree of
//! tables no deeper (`table_depth`), and only when two sets or more can take
//! it. Then each set is that tree: Golumbic's rule, the shallowest `k` signals
//! into one table at a time, the first table taking only as many as make the
//! rest come out in whole tables, each table's worth a tree of two-input XORs
//! the mapper covers whole. A signal's depth is its leaf's estimated depth in
//! tables (`techmap::PriorityCuts::depths`), so a late leaf goes into a late
//! table.
//!
//! The first rebuilding here was a tree of pairs, the shallowest two first,
//! which is the shallowest tree of two-input XORs: `crc32` came out three
//! tables deep, 145 of them. A tree of pairs is not the shallowest tree of
//! tables, though: a 32-input parity is five levels of pairs, which six-input
//! tables cover three deep, where six tables of six and one over them are
//! two. Built for tables, `crc32` is two deep in 191 (letting a pair cost a
//! set one table more for its sharing was 151 three deep, and slower by
//! Vivado's timer: docs/fpga-vivado-parity.md).
//!
//! The outputs go in *waves*, since a leaf may be read through an output of
//! its own (a CRC's next state through a flip-flop is not, but an XOR read
//! by an AND that another XOR reads is): an output's wave is one more than
//! the latest of its leaves', and a wave is built when every one before it
//! is. A leaf set larger than [`MAX_LEAVES`] is not collapsed: that XOR is
//! an output and a leaf to the XORs above it, as it was. Within a wave,
//! pairs are counted only in sets of at most [`MAX_COUNTED`] signals; a larger
//! one is built as its tree with nothing shared.
//!
//! It changes no function, and the flow proves that it does not: the mapped
//! network is checked against the graph from before this pass
//! (`techmap::map_module_checked`).

use std::collections::HashMap;

use super::{Aig, Edge};

/// The most leaves an XOR's set may have and still be collapsed into the
/// XORs above it; past it the XOR is kept as a node of its own.
pub const MAX_LEAVES: usize = 256;

/// The most signals a set may have and its pairs still be counted; past it
/// the set is built as its tree with no regard to the others.
pub const MAX_COUNTED: usize = 64;

/// How many tables deeper than its shallowest tree a parity may come out for
/// the pairs it shares with others. None: at one, `crc32` was 151 tables
/// three deep and 2.91 ns by Vivado's timer, at none 191 two deep and 2.68.
pub const DEPTH_SLACK: u32 = 0;

/// The two operands of node `id` when it is an XOR (or an XNOR) written the
/// AIG's way: `id = !p & !q` with `p = x & y` and `q = !x & !y`, so that
/// `id` is `x ^ y`. The operands come back as edges, complements and all.
fn xor_operands(aig: &Aig, id: u32) -> Option<(Edge, Edge)> {
    if !aig.is_and(id) {
        return None;
    }
    let (e0, e1) = aig.fanins(id);
    if !e0.is_complement() || !e1.is_complement() {
        return None;
    }
    let (p, q) = (e0.node(), e1.node());
    if !aig.is_and(p) || !aig.is_and(q) {
        return None;
    }
    let (x1, y1) = aig.fanins(p);
    let (x2, y2) = aig.fanins(q);
    let opposite = (x2 == !x1 && y2 == !y1) || (x2 == !y1 && y2 == !x1);
    // `!(x & y) & !(!x & !y)` is one exactly when the two differ.
    opposite.then_some((x1, y1))
}

/// The symmetric difference of two ascending lists.
fn cancel(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

/// Collapses and rebuilds the XOR logic of `aig` (see the module docs); the
/// result is rebuilt and swept. Returns the new depth.
///
/// `k` is the size of the tables the graph will be mapped onto, and `depth`
/// each node's estimated depth in them (`techmap::PriorityCuts::depths`).
pub fn balance_xor(aig: &mut Aig, k: usize, depth: &[u32]) -> u32 {
    let k = k.max(2);
    aig.recount_refs();
    let n = aig.len();
    let live = aig.live_nodes();
    let ids = || (0..n).map(|i| u32::try_from(i).expect("node index"));
    // A *linear* XOR: its two AND nodes read by it alone.
    let mut xors: Vec<Option<(Edge, Edge)>> = vec![None; n];
    let mut owned = vec![false; n];
    for id in ids() {
        if !live[id as usize] {
            continue;
        }
        if let Some(operands) = xor_operands(aig, id) {
            let (e0, e1) = aig.fanins(id);
            if aig.refs(e0.node()) == 1 && aig.refs(e1.node()) == 1 {
                xors[id as usize] = Some(operands);
                owned[e0.index()] = true;
                owned[e1.index()] = true;
            }
        }
    }
    // Each linear XOR's leaves and parity, in topological order. One whose
    // set is too large is *opaque*: a leaf to those above it.
    let mut sets: Vec<Option<(Vec<u32>, bool)>> = vec![None; n];
    let mut opaque = vec![false; n];
    for id in ids() {
        let Some((x, y)) = xors[id as usize] else {
            continue;
        };
        let term = |e: Edge| -> (Vec<u32>, bool) {
            match &sets[e.index()] {
                Some((leaves, parity)) if !opaque[e.index()] => {
                    (leaves.clone(), parity ^ e.is_complement())
                }
                _ => (vec![e.node()], e.is_complement()),
            }
        };
        let ((a, pa), (b, pb)) = (term(x), term(y));
        let leaves = cancel(&a, &b);
        opaque[id as usize] = leaves.len() > MAX_LEAVES;
        sets[id as usize] = Some((leaves, pa ^ pb));
    }
    // The XORs something other than a linear XOR's own ANDs reads: the
    // outputs, which are rebuilt; and the opaque ones, read by those too.
    let mut output = vec![false; n];
    for &o in aig.outputs() {
        output[o.index()] = true;
    }
    for id in ids() {
        if !live[id as usize] || !aig.is_and(id) || xors[id as usize].is_some() {
            continue;
        }
        let (a, b) = aig.fanins(id);
        if owned[id as usize] {
            // A linear XOR's AND reads its operands, which are leaves or
            // collapsed XORs; an opaque one among them must be built.
            for e in [a, b] {
                if opaque[e.index()] {
                    output[e.index()] = true;
                }
            }
            continue;
        }
        output[a.index()] = true;
        output[b.index()] = true;
    }
    // Waves: a node's is the latest wave its value needs.
    let mut wave = vec![0u32; n];
    for id in ids() {
        if !live[id as usize] || !aig.is_and(id) {
            continue;
        }
        wave[id as usize] = match &sets[id as usize] {
            Some((leaves, _)) => 1 + leaves.iter().map(|&l| wave[l as usize]).max().unwrap_or(0),
            None => {
                let (a, b) = aig.fanins(id);
                wave[a.index()].max(wave[b.index()])
            }
        };
    }
    let mut by_wave: Vec<Vec<u32>> = Vec::new();
    for id in ids() {
        if xors[id as usize].is_some() && output[id as usize] && live[id as usize] {
            let w = wave[id as usize] as usize;
            if by_wave.len() <= w {
                by_wave.resize(w + 1, Vec::new());
            }
            by_wave[w].push(id);
        }
    }

    let mut out = Aig::new();
    let mut map: Vec<Option<Edge>> = vec![None; n];
    // The estimated depth of each output rebuilt, by its node in `out`.
    let mut rebuilt: HashMap<u32, u32> = HashMap::new();
    map[0] = Some(Edge::FALSE);
    for &pi in aig.inputs() {
        map[pi as usize] = Some(out.add_input());
    }
    for outputs in &by_wave {
        if outputs.is_empty() {
            continue;
        }
        let mut signals: Vec<Vec<Signal>> = Vec::with_capacity(outputs.len());
        let mut parities: Vec<bool> = Vec::with_capacity(outputs.len());
        for &id in outputs {
            let (leaves, mut parity) = sets[id as usize].clone().expect("an output is an XOR");
            let mut here: Vec<Signal> = Vec::with_capacity(leaves.len());
            for leaf in leaves {
                let e = build(aig, &xors, Edge::plain(leaf), &mut map, &mut out);
                parity ^= e.is_complement();
                let d = rebuilt
                    .get(&e.node())
                    .copied()
                    .unwrap_or_else(|| depth.get(leaf as usize).copied().unwrap_or(0));
                here.push((e.regular(), d));
            }
            signals.push(here);
            parities.push(parity);
        }
        let roots = pair_up(&mut out, signals, k);
        for ((&id, (root, d)), parity) in outputs.iter().zip(roots).zip(parities) {
            rebuilt.insert(root.node(), d);
            map[id as usize] = Some(root.xor(parity));
        }
    }
    let outs: Vec<Edge> = aig.outputs().to_vec();
    for o in outs {
        let e = build(aig, &xors, o, &mut map, &mut out);
        out.add_output(e);
    }
    *aig = out.rebuild();
    aig.max_level()
}

/// The value of `edge` in `out`, building the plain AND nodes under it on
/// the way. Every linear XOR it meets is an output of an earlier wave and
/// mapped already.
fn build(
    aig: &Aig,
    xors: &[Option<(Edge, Edge)>],
    edge: Edge,
    map: &mut [Option<Edge>],
    out: &mut Aig,
) -> Edge {
    let mut stack: Vec<(u32, bool)> = vec![(edge.node(), false)];
    while let Some(&(id, expanded)) = stack.last() {
        if map[id as usize].is_some() {
            stack.pop();
            continue;
        }
        assert!(
            xors[id as usize].is_none(),
            "a collapsed XOR is reached only through the outputs built before it"
        );
        let (a, b) = aig.fanins(id);
        if expanded {
            let x = map[a.index()].expect("fanin built").xor(a.is_complement());
            let y = map[b.index()].expect("fanin built").xor(b.is_complement());
            map[id as usize] = Some(out.and(x, y));
            stack.pop();
        } else {
            stack.last_mut().expect("stack").1 = true;
            for e in [b, a] {
                if map[e.index()].is_none() {
                    stack.push((e.node(), false));
                }
            }
        }
    }
    map[edge.index()].expect("built").xor(edge.is_complement())
}

/// One signal of a parity being built: a plain edge of the new graph, and
/// the depth in tables it is estimated to arrive at.
type Signal = (Edge, u32);

/// How deep in `k`-input tables the XOR of signals arriving at `depths` can
/// be: Golumbic's rule, the shallowest `k` merged into one table at a time,
/// the first merge taking only as many as make the rest come out in whole
/// tables. That is the shallowest a tree of `k`-input tables can be.
fn table_depth(depths: &[u32], k: usize) -> u32 {
    let mut d: Vec<u32> = depths.to_vec();
    d.sort_unstable();
    let mut first = true;
    while d.len() > 1 {
        let take = if first {
            (d.len() - 2) % (k - 1) + 2
        } else {
            k.min(d.len())
        };
        first = false;
        let merged = d[..take].iter().copied().max().unwrap_or(0) + 1;
        d.drain(..take);
        let at = d.partition_point(|&x| x < merged);
        d.insert(at, merged);
    }
    d.first().copied().unwrap_or(0)
}

/// The XOR of `members` as a tree of two-input XORs pairing the shallowest
/// first, which a `k`-input table covers whole when there are at most `k`.
///
/// By node level and not by estimated depth: ordering a table's members by
/// their estimated depth, so that a late one is an XOR from the root, was
/// measured too, and `bram_fifo` and `regfile_alu` were slower for it by
/// Vivado's timer (3.95 to 4.17 ns, 6.40 to 6.57) with every other design
/// the same; the mapper finds the table over the members either way.
fn xor_tree(out: &mut Aig, mut members: Vec<Edge>) -> Edge {
    let key = |out: &Aig, e: Edge| (out.level(e.node()), e.raw());
    members.sort_by_key(|&e| key(out, e));
    let mut flip = false;
    while members.len() > 1 {
        let a = members.remove(0);
        let b = members.remove(0);
        let c = out.xor(a, b);
        flip ^= c.is_complement();
        let c = c.regular();
        let k = key(out, c);
        let pos = members.partition_point(|&e| key(out, e) < k);
        members.insert(pos, c);
    }
    members.first().copied().unwrap_or(Edge::FALSE).xor(flip)
}

/// The XOR of each set of signals, each result an edge of `out` and its
/// estimated depth: first the pairs two sets or more share, as long as a
/// pair costs no set its shallowest tree of `k`-input tables, and then each
/// set as that tree (see the module docs).
fn pair_up(out: &mut Aig, mut sets: Vec<Vec<Signal>>, k: usize) -> Vec<Signal> {
    let mut flips = vec![false; sets.len()];
    // A leaf twice cancels; the constant drops out.
    for set in &mut sets {
        set.sort_unstable();
        let mut kept: Vec<Signal> = Vec::with_capacity(set.len());
        for &s in set.iter() {
            if s.0 == Edge::FALSE {
                continue;
            }
            if kept.last().is_some_and(|l| l.0 == s.0) {
                kept.pop();
            } else {
                kept.push(s);
            }
        }
        *set = kept;
    }
    let depth_of = |set: &[Signal]| table_depth(&set.iter().map(|s| s.1).collect::<Vec<_>>(), k);
    // How deep each set may come out: its shallowest tree, and the slack.
    let targets: Vec<u32> = sets.iter().map(|set| depth_of(set) + DEPTH_SLACK).collect();
    let mut refused: std::collections::HashSet<(Edge, Edge)> = std::collections::HashSet::new();
    loop {
        let mut counts: HashMap<(Edge, Edge), (u32, u32)> = HashMap::new();
        for set in &sets {
            if set.len() < 3 || set.len() > MAX_COUNTED {
                continue;
            }
            for i in 0..set.len() {
                for j in i + 1..set.len() {
                    let (a, b) = (set[i], set[j]);
                    let entry = counts.entry((a.0, b.0)).or_insert((0, a.1.max(b.1)));
                    entry.0 += 1;
                }
            }
        }
        // The pairs most sets share, the shallowest of those, then the first;
        // the first of them that two sets or more can take is taken.
        let mut candidates: Vec<((Edge, Edge), u32, u32)> = counts
            .into_iter()
            .filter(|(pair, (c, _))| *c >= 2 && !refused.contains(pair))
            .map(|(pair, (c, d))| (pair, c, d))
            .collect();
        candidates
            .sort_unstable_by_key(|&((a, b), c, d)| (std::cmp::Reverse(c), d, a.raw(), b.raw()));
        let mut taken = false;
        for ((a, b), _, deeper) in candidates {
            // The sets the pair costs nothing, which must still be two.
            let pair_depth = deeper + 1;
            let fits: Vec<usize> = sets
                .iter()
                .enumerate()
                .filter(|(i, set)| {
                    set.len() > 2
                        && set.len() <= MAX_COUNTED
                        && set.iter().any(|s| s.0 == a)
                        && set.iter().any(|s| s.0 == b)
                        && {
                            let mut after: Vec<Signal> = set
                                .iter()
                                .copied()
                                .filter(|s| s.0 != a && s.0 != b)
                                .collect();
                            after.push((Edge::FALSE, pair_depth));
                            depth_of(&after) <= targets[*i]
                        }
                })
                .map(|(i, _)| i)
                .collect();
            if fits.len() < 2 {
                refused.insert((a, b));
                continue;
            }
            let t = out.xor(a, b);
            for i in fits {
                let set = &mut sets[i];
                set.retain(|s| s.0 != a && s.0 != b);
                set.push((t.regular(), pair_depth));
                set.sort_unstable();
                flips[i] ^= t.is_complement();
            }
            taken = true;
            break;
        }
        if !taken {
            break;
        }
    }
    // Each set as its shallowest tree of tables: Golumbic's merges, each one
    // table's worth of signals built as a tree of XORs.
    sets.into_iter()
        .zip(flips)
        .map(|(mut set, flip)| {
            let mut parity = false;
            set.sort_by_key(|s| (s.1, s.0.raw()));
            let mut first = true;
            while set.len() > 1 {
                let take = if first {
                    (set.len() - 2) % (k - 1) + 2
                } else {
                    k.min(set.len())
                };
                first = false;
                let group: Vec<Signal> = set.drain(..take).collect();
                let depth = group.iter().map(|s| s.1).max().unwrap_or(0) + 1;
                let root = xor_tree(out, group.iter().map(|s| s.0).collect());
                // The group's root, kept plain with its complement in the
                // set's parity, as every signal is.
                let at = set.partition_point(|s| (s.1, s.0.raw()) < (depth, root.regular().raw()));
                set.insert(at, (root.regular(), depth));
                parity ^= root.is_complement();
            }
            let (edge, depth) = set.first().copied().unwrap_or((Edge::FALSE, 0));
            (edge.xor(flip ^ parity), depth)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every input pattern of `inputs` inputs, both graphs.
    fn same(a: &Aig, b: &Aig, inputs: usize) {
        for pat in 0..(1u32 << inputs) {
            let bits: Vec<bool> = (0..inputs).map(|i| (pat >> i) & 1 == 1).collect();
            assert_eq!(a.eval(&bits), b.eval(&bits), "pattern {pat:b}");
        }
    }

    /// `g` rebuilt for six-input tables, every input at depth zero.
    fn rebuild(g: &mut Aig) {
        let depths = vec![0; g.len()];
        balance_xor(g, 6, &depths);
    }

    /// `g` mapped onto six-input tables as the 7 series' default synthesis
    /// maps, its cuts kept shallowest first.
    fn mapped(g: &Aig) -> crate::synth::techmap::LutNetwork {
        let options = crate::synth::techmap::CutOptions {
            depth_priority: true,
            ..crate::synth::techmap::CutOptions::default()
        };
        crate::synth::techmap::lut_map_with(g, 6, &options, 2)
    }

    /// The depth of `g` mapped onto six-input tables.
    fn tables(g: &Aig) -> u32 {
        mapped(g).depth
    }

    /// **A chain of XORs becomes the shallowest tree of tables.** The 32
    /// inputs of `srl_delay`'s `^d[63]` XORed in a row: mapped as written,
    /// a ripple of tables seven deep; rebuilt, two, as 32 inputs fit 36 (six
    /// tables of six and one over them), in seven tables. Golumbic's first
    /// merge takes two inputs so the rest come out in whole tables; with
    /// the binary tree of the first version of this pass it was three deep
    /// and nine tables. Would catch the chain left as it was, a tree of
    /// pairs, a leaf lost or doubled, and a complement dropped on the way
    /// (the chain has an XNOR in it).
    #[test]
    fn a_chain_of_xors_becomes_the_shallowest_tree_of_tables() {
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..32).map(|_| g.add_input()).collect();
        let mut acc = ins[0];
        for (i, &e) in ins[1..].iter().enumerate() {
            acc = if i == 4 {
                g.xnor(acc, e)
            } else {
                g.xor(acc, e)
            };
        }
        g.add_output(acc);
        assert_eq!(tables(&g), 7);
        let reference = g.clone();
        rebuild(&mut g);
        let net = mapped(&g);
        assert_eq!(net.depth, 2);
        assert_eq!(net.tables(), 7);
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..4096 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let bits: Vec<bool> = (0..32).map(|i| (rng >> i) & 1 == 1).collect();
            assert_eq!(g.eval(&bits), reference.eval(&bits));
        }
        // And every pattern of a smaller one.
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..12).map(|_| g.add_input()).collect();
        let mut acc = ins[0];
        for &e in &ins[1..] {
            acc = g.xnor(acc, e);
        }
        g.add_output(acc);
        let reference = g.clone();
        rebuild(&mut g);
        same(&g, &reference, 12);
        assert_eq!(tables(&g), 2);
    }

    /// **A parity of more than [`MAX_LEAVES`] is a leaf of its own, its
    /// complement and all.** A 300-input parity, its last XOR an XNOR, read
    /// by one more XOR: the first is too large to collapse into the second,
    /// so it is rebuilt as an output and the second reads it as a leaf, in
    /// whichever polarity its rebuilding left it. Would catch that polarity
    /// dropped (every pattern wrong, `a_pair_cancels` and the rest have no
    /// leaf that is a rebuilt parity), and the large one collapsed past the
    /// bound; checked on random patterns, there being 301 inputs.
    #[test]
    fn a_parity_too_large_to_collapse_is_a_leaf_with_its_polarity() {
        for xnor in [false, true] {
            let mut g = Aig::new();
            let ins: Vec<Edge> = (0..301).map(|_| g.add_input()).collect();
            let mut acc = ins[0];
            for (i, &e) in ins[1..300].iter().enumerate() {
                acc = if xnor && i == 298 {
                    g.xnor(acc, e)
                } else {
                    g.xor(acc, e)
                };
            }
            let top = g.xor(acc, ins[300]);
            g.add_output(top);
            g.add_output(acc);
            let reference = g.clone();
            rebuild(&mut g);
            let mut rng = 0x1234_5678_9abc_def1u64;
            for _ in 0..512 {
                let bits: Vec<bool> = (0..301)
                    .map(|_| {
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        rng & 1 == 1
                    })
                    .collect();
                assert_eq!(g.eval(&bits), reference.eval(&bits), "xnor {xnor}");
            }
        }
    }

    /// **Golumbic's rule is the shallowest tree.** Would catch the first
    /// merge taking a whole table (33 inputs at depth zero would be three
    /// deep: 6 + 6 + 6 + 6 + 6 + 3 leaves the last table seven signals) and
    /// a merge not counting its deepest member.
    #[test]
    fn table_depth_is_golumbics() {
        assert_eq!(table_depth(&[0; 36], 6), 2);
        assert_eq!(table_depth(&[0; 37], 6), 3);
        assert_eq!(table_depth(&[0; 33], 6), 2);
        assert_eq!(table_depth(&[0; 6], 6), 1);
        assert_eq!(table_depth(&[0], 6), 0);
        // A late signal: the five others and it in one table.
        assert_eq!(table_depth(&[3, 0, 0, 0, 0, 0], 6), 4);
        assert_eq!(table_depth(&[3, 0, 0, 0, 0, 0, 0], 6), 4);
    }

    /// **Shared chains are collapsed and their sharing found again.** Two
    /// parities over eight inputs written as one chain each, the second
    /// reading the first's partial XORs (`u = s3 ^ e ^ f ^ g ^ h` where `s3 =
    /// a ^ b ^ c ^ d`, read by the first as well): every partial is read
    /// twice, so balancing tree by tree could not touch them. Rebuilt, each
    /// is two tables deep, the shallowest eight inputs can be, and smaller
    /// as a graph than it was. Would catch the collapse stopping at a
    /// shared XOR (the chain's ripple), and an output's parity lost; would
    /// not catch a shared pair missed, which only the count says.
    #[test]
    fn shared_chains_collapse_and_share_again() {
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..8).map(|_| g.add_input()).collect();
        let mut s = ins[0];
        let mut partials = Vec::new();
        for &e in &ins[1..4] {
            s = g.xor(s, e);
            partials.push(s);
        }
        let mut t = s;
        for &e in &ins[4..8] {
            t = g.xor(t, e);
        }
        // `!(s3 ^ h) ^ e ^ f ^ g`, through a complement.
        let mut u = g.xnor(s, ins[7]);
        for &e in &ins[4..7] {
            u = g.xor(u, e);
        }
        g.add_output(t);
        g.add_output(u);
        // A partial read by an AND, so it is an output of its own.
        let v = g.and(partials[1], ins[7]);
        g.add_output(v);
        let reference = g.clone();
        let ands = g.num_ands();
        rebuild(&mut g);
        same(&g, &reference, 8);
        assert_eq!(tables(&g), 2);
        assert!(g.num_ands() <= ands, "{} nodes from {ands}", g.num_ands());
    }

    /// **A repeated leaf cancels and what is not an XOR is left alone.**
    /// `t = (a ^ b) ^ c ^ a` is `b ^ c`. Would catch the pair of `a` kept,
    /// or the AND beside it rebuilt wrong.
    #[test]
    fn a_pair_cancels() {
        let mut g = Aig::new();
        let ins: Vec<Edge> = (0..4).map(|_| g.add_input()).collect();
        let (a, b, c, d) = (ins[0], ins[1], ins[2], ins[3]);
        let s = g.xor(a, b);
        let t = g.xor(s, c);
        let t = g.xor(t, a);
        let v = g.and(c, d);
        g.add_output(t);
        g.add_output(v);
        let reference = g.clone();
        rebuild(&mut g);
        same(&g, &reference, 4);
        assert_eq!(g.num_ands(), 4, "b ^ c is three nodes, c & d one");
    }
}
