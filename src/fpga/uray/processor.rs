//! The processor's side of a Zynq UltraScale+ design: which of the `PS8`
//! site's inputs a bitstream must drive, and a router for the constant
//! that drives them.
//!
//! # Why every input is driven
//!
//! An input of the processor that nothing in the fabric drives reads as
//! asserted. On the ZCU104 on 2026-10-09, a design that drove only one
//! `PS8` input raised the fabric's first interrupt line and both USB
//! controllers' over-current inputs the moment it was loaded
//! (`docs/fpga-uray.md`). The same reading would put `AWVALID` and
//! `ARVALID` up on every AXI port the processor's configuration enables.
//!
//! Vivado drives them. Of the base overlay's `PS8` inputs, 2 958 are tied
//! to a lookup table of contents zero in a slice beside the interface
//! column. Most of the rest are driven by the overlay's own logic, and
//! **585 are left undriven**. Those 585 are test, scan, analogue-PHY and
//! debug pins, and a few active-low functional ones, such as
//! `NIRQ0_LPD_RPU` and `FMIO_SPI0_SS_IN_B`, for which reading 1 *is* idle.
//! They are listed in `ps8_undriven.txt`. A design should drive every
//! other input it does not use to zero, which is what Vivado would do with
//! the inputs the overlay drives itself, had it not used them.
//!
//! **Measured, from Vivado's bitstream**: each input's node was traced
//! back through the routing Vivado turned on, and the lookup table at its
//! end read through [`super::slice`].

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::fpga::arch::{NodeId, PipId, RoutingGraph};

/// The `PS8` inputs Vivado leaves undriven, expanded from
/// `ps8_undriven.txt`.
pub fn left_undriven() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in include_str!("ps8_undriven.txt").lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once(':') {
            Some((family, indices)) => {
                for index in indices.split(',') {
                    out.insert(format!("{family}{index}"));
                }
            }
            None => {
                out.insert(line.to_owned());
            }
        }
    }
    out
}

/// For each wire of `graph`, the node it belongs to, named by its lowest
/// wire: wires joined by a bitless pip are one node.
pub fn node_roots(graph: &RoutingGraph) -> Vec<NodeId> {
    let count = graph.nodes.len();
    let mut up: Vec<NodeId> = (0..u32::try_from(count).unwrap_or(u32::MAX)).collect();
    fn find(up: &mut [NodeId], mut a: NodeId) -> NodeId {
        while up[a as usize] != a {
            let next = up[up[a as usize] as usize];
            up[a as usize] = next;
            a = next;
        }
        a
    }
    for (id, pip) in graph.pips.iter().enumerate() {
        let id = PipId::try_from(id).unwrap_or(PipId::MAX);
        if graph.pip_bits(id).is_empty() {
            let (a, b) = (find(&mut up, pip.from), find(&mut up, pip.to));
            if a != b {
                up[a.max(b) as usize] = a.min(b);
            }
        }
    }
    (0..u32::try_from(count).unwrap_or(u32::MAX))
        .map(|i| find(&mut up, i))
        .collect()
}

/// A routed constant: the pips with bits it turns on, and which of the
/// candidate sources it drew on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConstantTree {
    /// The pips with bits, each once.
    pub pips: Vec<PipId>,
    /// The tags of the sources the tree starts from.
    pub sources: BTreeSet<usize>,
    /// Sinks no source could reach, which the caller has to answer for.
    pub unreached: Vec<NodeId>,
}

/// Routes one constant from any of `sources` to every one of `sinks`.
///
/// Each source is a wire and a tag the caller knows it by (a lookup
/// table, say). The tree grows sink by sink, each from whatever is
/// cheaper: a fresh source, or the tree so far, which costs nothing to
/// reuse. A pip with bits costs one and a join nothing. The search never
/// enters a node in `blocked`, which must hold every node that already
/// has a driver — a site output, another net — and every input that must
/// not be driven.
///
/// A sink no source reaches is listed in [`ConstantTree::unreached`]
/// rather than stopping the rest.
pub fn route_constant(
    graph: &RoutingGraph,
    roots: &[NodeId],
    sources: &[(NodeId, usize)],
    sinks: &[NodeId],
    blocked: &HashSet<NodeId>,
) -> ConstantTree {
    let count = graph.nodes.len();
    let mut tree: HashSet<NodeId> = HashSet::new();
    let mut tag_of: HashMap<NodeId, usize> = HashMap::new();
    for (wire, tag) in sources {
        tag_of.insert(roots[*wire as usize], *tag);
    }
    let mut out = ConstantTree::default();
    let mut on: HashSet<PipId> = HashSet::new();
    let mut cost = vec![u32::MAX; count];
    let mut came: Vec<Option<PipId>> = vec![None; count];
    let mut touched: Vec<usize> = Vec::new();
    let mut tree_wires: Vec<NodeId> = Vec::new();
    for &sink in sinks {
        let target = roots[sink as usize];
        if tree.contains(&target) {
            continue;
        }
        for &i in &touched {
            cost[i] = u32::MAX;
            came[i] = None;
        }
        touched.clear();
        let mut queue = VecDeque::new();
        for &start in sources.iter().map(|(w, _)| w).chain(tree_wires.iter()) {
            if cost[start as usize] != 0 {
                cost[start as usize] = 0;
                touched.push(start as usize);
                queue.push_back(start);
            }
        }
        let mut reached = None;
        while let Some(wire) = queue.pop_front() {
            if roots[wire as usize] == target {
                reached = Some(wire);
                break;
            }
            for &pip in graph.outgoing(wire) {
                let next = graph.pips[pip as usize].to;
                let root = roots[next as usize];
                if root != target && blocked.contains(&root) {
                    continue;
                }
                let step = u32::from(!graph.pip_bits(pip).is_empty());
                let c = cost[wire as usize].saturating_add(step);
                if c < cost[next as usize] {
                    if cost[next as usize] == u32::MAX {
                        touched.push(next as usize);
                    }
                    cost[next as usize] = c;
                    came[next as usize] = Some(pip);
                    if step == 0 {
                        queue.push_front(next);
                    } else {
                        queue.push_back(next);
                    }
                }
            }
        }
        let Some(mut at) = reached else {
            out.unreached.push(sink);
            continue;
        };
        loop {
            tree.insert(roots[at as usize]);
            tree_wires.push(at);
            match came[at as usize] {
                Some(pip) => {
                    if !graph.pip_bits(pip).is_empty() && on.insert(pip) {
                        out.pips.push(pip);
                    }
                    at = graph.pips[pip as usize].from;
                }
                None => {
                    if let Some(tag) = tag_of.get(&roots[at as usize]) {
                        out.sources.insert(*tag);
                    }
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_undriven_list_expands_its_families() {
        let list = left_undriven();
        assert_eq!(list.len(), 585);
        assert!(list.contains("NIRQ0_LPD_RPU"));
        assert!(list.contains("ACE_PL_INTFPD_ARUSER9"));
        assert!(!list.contains("ACE_PL_INTFPD_ARUSER5"));
        assert!(!list.contains("EMIO_HUB_PORT_OVERCRNT_USB2_0"));
    }
}
