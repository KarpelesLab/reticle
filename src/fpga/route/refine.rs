//! The last step of a timing-driven route: each lookup table on a late path
//! given its inputs again, the latest on the fastest pin.
//!
//! PathFinder hands a lookup table's inputs out one signal at a time, and the
//! first signal routed to a table takes whichever of its six pins it reaches
//! first. `A6` costs nothing past its `IMUX` under Vivado's timer and `A1`
//! 0.39 ns, so which signal is on which pin is a third of a nanosecond on the
//! table's latest input. Vivado's router gives them as a matching: asked "what
//! does Vivado write, in full, for this table?" of `qor/lutram32.v`'s latest
//! table at seed 1, it put the latest input, 838 ps on `B2` in Reticle's
//! routes, on `B6` in 192, and moved the one that had `B6` in 95 ps to `B3` in
//! 453, where it still had slack. The negotiation here cannot find that: a
//! critical search may take a held input ([`Timing::claim_inputs`]), and the
//! holder, as critical, takes it back.
//!
//! So once the route is legal, each table one of whose inputs is near its
//! domain's latest path has every input it reads lifted off the routing — the
//! branch of each signal's tree that serves that input alone — and each
//! searched for again, for delay alone and over wires no signal holds, to every
//! pin of the table that is free. The pins are then given as the assignment
//! that leaves the table's least slack largest ([`best_assignment`]): each
//! input's slack is what the analysis found, less what its new way costs over
//! its old one, since the table's required time is one for all six. A table
//! whose assignment is no better keeps its routes. Any other late sink, a
//! flip-flop's data or a block's address, is the same question with one pin:
//! its branch is searched for again and kept if it is faster.
//!
//! Nothing is taken from another signal, so the routing stays legal, and every
//! round ends with an analysis of the whole design: a round that made any
//! domain's latest path later is put back and the refinement ends.

use std::collections::BTreeMap;

use super::{
    Geometry, Netlist, NodeId, PipId, Route, Routing, RoutingGraph, Scratch, State, Terminals,
    Timing, sink_delays,
};
use crate::fpga::arch::FastMap;

/// What a search for one input's new way may visit before it gives up. A
/// search for all six pins of a table settles in a few thousand nodes when
/// they can all be had; one with a pin out of reach searches up to its bound,
/// and at 8,000 enough of those stopped short of a pin they would have found
/// that `widemux`'s and `bram_fifo`'s routed estimates were 0.11 and 0.13 ns
/// later at seed 1 than at 30,000.
const SEARCH_BUDGET: usize = 30_000;

/// What a tile of distance is reckoned to delay in a refinement search's
/// estimate, in picoseconds: `xray::ROUTE_TILE_DELAY_PS`'s figure. It is more
/// than a long line's 225 ps over its eighteen rows, so the estimate is not a
/// bound on every way; at 10, which is, the searches for a table's slow pins
/// ran out of [`SEARCH_BUDGET`] three times as often and found no better
/// routes.
const TILE_PS: u64 = 25;

/// A way a search found to one of its targets: its delay from the source in
/// picoseconds and the pips from the tree, or `None`.
type Way = Option<(u64, Vec<PipId>)>;

/// One sink in a group being given its pins again.
struct Member {
    /// Index into the route's terminals.
    which: usize,
    /// Index into that terminals' sinks.
    sink: usize,
    /// Its slack in picoseconds, as the analysis found it.
    slack: f64,
    /// Its routed delay in picoseconds, as the analysis took it.
    wire: f64,
}

/// What the refinement did.
#[derive(Default)]
pub(super) struct Refined {
    /// Groups given new routes and kept.
    pub(super) groups: usize,
}

/// The routing context a refinement reads.
pub(super) struct Context<'a> {
    pub(super) netlist: &'a Netlist,
    pub(super) graph: &'a RoutingGraph,
    pub(super) geometry: &'a [Geometry],
    /// The route's table of each node's pips and the nodes they reach.
    pub(super) edges: super::Edges<'a>,
    pub(super) terminals: &'a [Terminals],
    pub(super) timing: &'a Timing,
    pub(super) equivalent: &'a [u32],
    /// Per signal, whether it may enter the clock network; such a signal is
    /// left as it is.
    pub(super) network_signals: &'a [bool],
}

/// Refines `routing` in place: see the module's header. `state`'s
/// occupancy is kept in step with it.
pub(super) fn refine(
    cx: &Context<'_>,
    state: &mut State,
    scratch: &mut Scratch,
    routing: &mut Routing,
) -> Refined {
    let mut out = Refined::default();
    let Some(paths) = cx.timing.paths.as_ref() else {
        return out;
    };
    // Each class's nodes, for the pins a table may take.
    let mut class_nodes: FastMap<u32, Vec<NodeId>> = FastMap::default();
    for (node, class) in cx.equivalent.iter().enumerate() {
        if *class != u32::MAX {
            class_nodes
                .entry(*class)
                .or_default()
                .push(NodeId::try_from(node).unwrap_or(NodeId::MAX));
        }
    }
    for _ in 0..cx.timing.refine_rounds {
        let (wire, critical, slack, before) = analysis(cx, routing, paths);
        let groups = groups(cx, routing, &wire, &critical, &slack);
        let mut saved: BTreeMap<usize, Option<Route>> = BTreeMap::new();
        let mut kept = 0;
        for group in groups {
            if try_group(
                cx,
                state,
                scratch,
                routing,
                &class_nodes,
                &group,
                &mut saved,
            ) {
                kept += 1;
            }
        }
        if kept == 0 {
            break;
        }
        // The round is kept only if no domain's latest path is later for it.
        let (_, _, _, after) = analysis(cx, routing, paths);
        if after.iter().zip(&before).any(|(a, b)| a > b) {
            for (signal, route) in saved {
                replace(state, routing, signal, route);
            }
            break;
        }
        out.groups += kept;
    }
    out
}

/// The routing's analysis: each pin's routed delay in picoseconds, its
/// criticality before any exponent, its slack in picoseconds, and each
/// domain's latest path.
fn analysis(
    cx: &Context<'_>,
    routing: &Routing,
    paths: &crate::fpga::place::PlaceTiming,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<u64>) {
    let mut wire = vec![0.0f64; cx.netlist.pins.len()];
    for terminals in cx.terminals {
        let Some(route) = routing.routes[terminals.signal].as_ref() else {
            continue;
        };
        let delays = sink_delays(
            cx.graph,
            route,
            &terminals.sinks,
            &cx.timing.node_delay,
            cx.timing.branch_ps,
        );
        for (sink, ps) in terminals.sinks.iter().zip(delays) {
            #[allow(clippy::cast_precision_loss, reason = "a delay in picoseconds")]
            let ps = ps as f64;
            wire[sink.pin] = wire[sink.pin].max(ps);
        }
    }
    let ns: Vec<f64> = wire.iter().map(|ps| ps / 1000.0).collect();
    let (critical, slack, longest) = crate::fpga::place::path_slack(cx.netlist, paths, &ns);
    let slack = slack.iter().map(|s| s * 1000.0).collect();
    (wire, critical, slack, longest)
}

/// Where a sink's route ends: the node of its class it moved to, or its own.
fn end_of(route: &Route, sink: &super::Sink) -> NodeId {
    route
        .moved
        .iter()
        .find(|(pin, _)| *pin == sink.pin)
        .map_or(sink.node, |(_, node)| *node)
}

/// The groups to give new routes, the latest first: per lookup table (a class
/// of [`super::RouteOptions::equivalent`]) one of whose inputs is at least
/// [`Timing::refine_above`] critical, every sink ending on it; and every other
/// sink that critical, on its own.
fn groups(
    cx: &Context<'_>,
    routing: &Routing,
    wire: &[f64],
    critical: &[f64],
    slack: &[f64],
) -> Vec<Vec<Member>> {
    // By key: a class, or a sink node past the classes' range.
    let mut by_key: BTreeMap<(bool, u32), Vec<Member>> = BTreeMap::new();
    for (which, terminals) in cx.terminals.iter().enumerate() {
        if cx.network_signals.get(terminals.signal) == Some(&true) {
            continue;
        }
        let Some(route) = routing.routes[terminals.signal].as_ref() else {
            continue;
        };
        for (k, sink) in terminals.sinks.iter().enumerate() {
            let end = end_of(route, sink);
            let class = cx.equivalent.get(end as usize).copied().unwrap_or(u32::MAX);
            let key = if class == u32::MAX {
                (false, end)
            } else {
                (true, class)
            };
            by_key.entry(key).or_default().push(Member {
                which,
                sink: k,
                slack: slack.get(sink.pin).copied().unwrap_or(f64::INFINITY),
                wire: wire.get(sink.pin).copied().unwrap_or(0.0),
            });
        }
    }
    let mut out: Vec<(f64, Vec<Member>)> = by_key
        .into_iter()
        .filter(|(_, members)| {
            members.iter().any(|m| {
                let pin = cx.terminals[m.which].sinks[m.sink].pin;
                critical.get(pin).copied().unwrap_or(0.0) >= cx.timing.refine_above
            })
        })
        .map(|(_, members)| {
            let least = members
                .iter()
                .map(|m| m.slack)
                .fold(f64::INFINITY, f64::min);
            (least, members)
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, members)| members).collect()
}

/// Puts `route` in place of `signal`'s, keeping the occupancy in step.
fn replace(state: &mut State, routing: &mut Routing, signal: usize, route: Option<Route>) {
    super::rip_up(state, routing, signal);
    if let Some(route) = &route {
        for root in state.roots(&route.nodes) {
            state.usage[root as usize].occupancy += 1;
        }
    }
    routing.routes[signal] = route;
}

/// `route` without the branch that serves `end` alone, or `None` when `end`
/// leads on to anything.
fn strip(
    graph: &RoutingGraph,
    route: &Route,
    ends: &[NodeId],
    end: NodeId,
    pin: usize,
) -> Option<Route> {
    let mut children: FastMap<NodeId, u32> = FastMap::default();
    let mut parent: FastMap<NodeId, PipId> = FastMap::default();
    for pip in &route.pips {
        let at = graph.pip(*pip);
        *children.entry(at.from).or_default() += 1;
        parent.insert(at.to, *pip);
    }
    if children.get(&end).copied().unwrap_or(0) > 0 || end == route.source {
        return None;
    }
    let mut removed: Vec<NodeId> = Vec::new();
    let mut node = end;
    loop {
        if node == route.source {
            break;
        }
        if node != end && (children.get(&node).copied().unwrap_or(0) > 1 || ends.contains(&node)) {
            break;
        }
        removed.push(node);
        node = graph.pip(*parent.get(&node)?).from;
    }
    removed.sort_unstable();
    let pips: Vec<PipId> = route
        .pips
        .iter()
        .copied()
        .filter(|pip| removed.binary_search(&graph.pip(*pip).to).is_err())
        .collect();
    let mut nodes: Vec<NodeId> = std::iter::once(route.source)
        .chain(pips.iter().map(|pip| graph.pip(*pip).to))
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    Some(Route {
        signal: route.signal,
        source: route.source,
        pips,
        nodes,
        moved: route
            .moved
            .iter()
            .copied()
            .filter(|(p, _)| *p != pin)
            .collect(),
    })
}

/// Each node of `route`'s tree with its delay from the source in
/// picoseconds, the source's own and any branch's load left out.
fn tree_delays(cx: &Context<'_>, route: &Route) -> BTreeMap<NodeId, u64> {
    let mut out = BTreeMap::from([(route.source, 0u64)]);
    for pip in &route.pips {
        let at = cx.graph.pip(*pip);
        let d = out.get(&at.from).copied().unwrap_or(0)
            + u64::from(
                cx.timing
                    .node_delay
                    .get(at.to as usize)
                    .copied()
                    .unwrap_or(0),
            );
        out.insert(at.to, d);
    }
    out
}

/// A search from `tree` for delay alone, over nodes no signal holds, to each
/// of `targets`: per target its delay from the source in picoseconds and the
/// pips from the tree to it, or `None` where it was not reached at or under
/// `bound`.
///
/// The estimate is the distance to the targets' tile at [`TILE_PS`] a tile and
/// the least of the targets not yet reached's own delay, which any way to one
/// of them pays on entering it: a table's `A1` is 0.39 ns of wire past its
/// `IMUX`, and without that term a search for it settled every node of the
/// neighbourhood nearer than that first, forty thousand and more.
fn search(
    cx: &Context<'_>,
    state: &State,
    scratch: &mut Scratch,
    tree: &BTreeMap<NodeId, u64>,
    targets: &[NodeId],
    bound: u64,
) -> Vec<Way> {
    let mut out: Vec<Way> = vec![None; targets.len()];
    let Some(first) = targets.first() else {
        return out;
    };
    let goal = cx.geometry[*first as usize].tile;
    let own = |node: NodeId| {
        u64::from(
            cx.timing
                .node_delay
                .get(node as usize)
                .copied()
                .unwrap_or(0),
        )
    };
    let mut least_left = targets.iter().map(|t| own(*t)).min().unwrap_or(0);
    let estimate = |node: NodeId, least_left: u64| -> u64 {
        if targets.contains(&node) {
            return 0;
        }
        let (x, y) = cx.geometry[node as usize].nearest_tile(goal.0, goal.1);
        TILE_PS * (u64::from(x.abs_diff(goal.0)) + u64::from(y.abs_diff(goal.1))) + least_left
    };
    // The router's own arrays, a node's entry stamped by the search: a tree
    // node is one whose pip is none.
    scratch.start();
    let mut heap = std::mem::take(&mut scratch.queue);
    heap.clear();
    #[allow(clippy::cast_precision_loss, reason = "a delay in picoseconds")]
    for (node, d) in tree {
        let f = d + estimate(*node, least_left);
        scratch.set(*node, *d as f64, f as f64, PipId::MAX);
        heap.push((f, *node));
    }
    let in_tree = |scratch: &Scratch, node: NodeId| {
        scratch.cost_of(node).is_finite() && scratch.from(node) == PipId::MAX
    };
    let mut left = targets.len();
    let mut visited = 0usize;
    while let Some((f, node)) = heap.pop() {
        #[allow(clippy::cast_precision_loss, reason = "a delay in picoseconds")]
        if (f as f64) > scratch.estimate_of(node) {
            continue;
        }
        if f > bound {
            break;
        }
        visited += 1;
        if visited > SEARCH_BUDGET {
            break;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a delay in picoseconds, a whole number"
        )]
        let cost = scratch.cost_of(node) as u64;
        if let Some(at) = targets.iter().position(|t| *t == node)
            && out[at].is_none()
        {
            let mut path = Vec::new();
            let mut n = node;
            while !in_tree(scratch, n) {
                let pip = scratch.from(n);
                path.push(pip);
                n = cx.graph.pip(pip).from;
            }
            path.reverse();
            out[at] = Some((cost, path));
            left -= 1;
            if left == 0 {
                break;
            }
            least_left = targets
                .iter()
                .zip(&out)
                .filter(|(_, found)| found.is_none())
                .map(|(t, _)| own(*t))
                .min()
                .unwrap_or(0);
            continue;
        }
        let from_root = state.group[node as usize];
        let (reach_start, reach) = cx.edges;
        let run = reach_start[node as usize] as usize..reach_start[node as usize + 1] as usize;
        for (pip, next) in &reach[run] {
            let next = *next;
            let at = &cx.geometry[next as usize];
            if at.closed || at.reserved || in_tree(scratch, next) {
                continue;
            }
            let root = state.group[next as usize];
            if root != from_root && state.usage[root as usize].occupancy > 0 {
                continue;
            }
            let c = cost + own(next);
            let f = c + estimate(next, least_left);
            #[allow(clippy::cast_precision_loss, reason = "a delay in picoseconds")]
            if f <= bound && (c as f64) < scratch.cost_of(next) {
                scratch.set(next, c as f64, f as f64, *pip);
                heap.push((f, next));
            }
        }
    }
    scratch.queue = heap;
    out
}

/// The assignment of `delays[member][pin]` that leaves the least slack,
/// `slack[member] - delay`, largest, and of those the least delay in all:
/// per member its pin. `None` when the members cannot all have one.
fn best_assignment(base: &[f64], delays: &[Vec<Option<u64>>]) -> Option<(f64, Vec<usize>)> {
    #[allow(clippy::too_many_arguments, reason = "a recursion's whole state")]
    fn walk(
        at: usize,
        base: &[f64],
        delays: &[Vec<Option<u64>>],
        taken: &mut Vec<bool>,
        chosen: &mut Vec<usize>,
        least: f64,
        total: u64,
        best: &mut Option<(f64, u64, Vec<usize>)>,
    ) {
        if let Some((b, t, _)) = best
            && (least < *b || (least == *b && total >= *t))
        {
            return;
        }
        if at == base.len() {
            *best = Some((least, total, chosen.clone()));
            return;
        }
        for (pin, d) in delays[at].iter().enumerate() {
            let Some(d) = d else { continue };
            if taken[pin] {
                continue;
            }
            taken[pin] = true;
            chosen.push(pin);
            #[allow(clippy::cast_precision_loss, reason = "a delay in picoseconds")]
            let s = base[at] - *d as f64;
            walk(
                at + 1,
                base,
                delays,
                taken,
                chosen,
                least.min(s),
                total + d,
                best,
            );
            chosen.pop();
            taken[pin] = false;
        }
    }
    let pins = delays.first().map_or(0, Vec::len);
    let mut best = None;
    walk(
        0,
        base,
        delays,
        &mut vec![false; pins],
        &mut Vec::new(),
        f64::INFINITY,
        0,
        &mut best,
    );
    best.map(|(least, _, chosen)| (least, chosen))
}

/// Gives one group its pins again, or leaves it as it was: true when it kept
/// new routes. Every route it changes is filed in `saved` first, as it was
/// before the round.
fn try_group(
    cx: &Context<'_>,
    state: &mut State,
    scratch: &mut Scratch,
    routing: &mut Routing,
    class_nodes: &FastMap<u32, Vec<NodeId>>,
    group: &[Member],
    saved: &mut BTreeMap<usize, Option<Route>>,
) -> bool {
    // One sink a signal: two would have to share a search.
    let mut signals: Vec<usize> = group.iter().map(|m| cx.terminals[m.which].signal).collect();
    signals.sort_unstable();
    if signals.windows(2).any(|w| w[0] == w[1]) {
        return false;
    }
    // Each member's route without its branch.
    let mut originals: Vec<Route> = Vec::with_capacity(group.len());
    let mut stripped: Vec<Route> = Vec::with_capacity(group.len());
    let mut first_end = None;
    for m in group {
        let terminals = &cx.terminals[m.which];
        let Some(route) = routing.routes[terminals.signal].as_ref() else {
            return false;
        };
        let ends: Vec<NodeId> = terminals.sinks.iter().map(|s| end_of(route, s)).collect();
        let sink = &terminals.sinks[m.sink];
        let end = ends[m.sink];
        first_end.get_or_insert(end);
        let Some(rest) = strip(cx.graph, route, &ends, end, sink.pin) else {
            return false;
        };
        originals.push(route.clone());
        stripped.push(rest);
    }
    let Some(first_end) = first_end else {
        return false;
    };
    for (m, rest) in group.iter().zip(&stripped) {
        replace(
            state,
            routing,
            cx.terminals[m.which].signal,
            Some(rest.clone()),
        );
    }
    let class = cx
        .equivalent
        .get(first_end as usize)
        .copied()
        .unwrap_or(u32::MAX);
    let targets: Vec<NodeId> = if class == u32::MAX {
        vec![first_end]
    } else {
        class_nodes.get(&class).cloned().unwrap_or_default()
    }
    .into_iter()
    .filter(|n| state.usage[state.group[*n as usize] as usize].occupancy == 0)
    .collect();
    let put_back = |state: &mut State, routing: &mut Routing| {
        for (m, route) in group.iter().zip(&originals) {
            replace(
                state,
                routing,
                cx.terminals[m.which].signal,
                Some(route.clone()),
            );
        }
    };
    if targets.len() < group.len() {
        put_back(state, routing);
        return false;
    }
    // Each member's way to each free pin, searched for alone, the least slack
    // first. A way that leaves its member no more slack than the group's least
    // before is no part of a better assignment, and is not searched past; and
    // once a member's best way leaves it no more than that, no assignment can
    // be better, and the rest are not searched at all.
    let base: Vec<f64> = group.iter().map(|m| m.slack + m.wire).collect();
    let before = group.iter().map(|m| m.slack).fold(f64::INFINITY, f64::min);
    let mut by_slack: Vec<usize> = (0..group.len()).collect();
    by_slack.sort_by(|a, b| group[*a].slack.total_cmp(&group[*b].slack).then(a.cmp(b)));
    let mut found: Vec<Vec<Way>> = vec![Vec::new(); group.len()];
    let mut delays: Vec<Vec<Option<u64>>> = vec![Vec::new(); group.len()];
    for i in by_slack {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a bound in picoseconds, clamped at zero"
        )]
        let bound = if (base[i] - before).is_finite() {
            (base[i] - before - 1.0).max(0.0) as u64
        } else {
            u64::MAX
        };
        found[i] = search(
            cx,
            state,
            scratch,
            &tree_delays(cx, &stripped[i]),
            &targets,
            bound,
        );
        delays[i] = found[i]
            .iter()
            .map(|way| way.as_ref().map(|(d, _)| *d))
            .collect();
        if delays[i].iter().all(Option::is_none) {
            put_back(state, routing);
            return false;
        }
    }
    let Some((least, chosen)) = best_assignment(&base, &delays) else {
        put_back(state, routing);
        return false;
    };
    if least < before + 1.0 {
        put_back(state, routing);
        return false;
    }
    // Routed for real, the tightest first, each over what the ones before
    // took: its way as searched for alone while that is still clear, and
    // searched for again where one before it took part of it.
    let mut order: Vec<usize> = (0..group.len()).collect();
    #[allow(clippy::cast_precision_loss, reason = "a delay in picoseconds")]
    order.sort_by(|a, b| {
        let s = |i: usize| base[i] - delays[i][chosen[i]].unwrap_or(u64::MAX) as f64;
        s(*a).total_cmp(&s(*b)).then(a.cmp(b))
    });
    let mut least_done = f64::INFINITY;
    let mut done: Vec<Route> = stripped.clone();
    for i in order {
        let target = targets[chosen[i]];
        let clear = |path: &[PipId]| {
            path.iter().all(|pip| {
                let at = cx.graph.pip(*pip);
                let root = state.group[at.to as usize];
                root == state.group[at.from as usize] || state.usage[root as usize].occupancy == 0
            })
        };
        let way = match found[i][chosen[i]].take() {
            Some((d, path)) if clear(&path) => Some((d, path)),
            _ => {
                let tree = tree_delays(cx, &done[i]);
                search(cx, state, scratch, &tree, &[target], u64::MAX)
                    .into_iter()
                    .next()
                    .flatten()
            }
        };
        let Some((d, path)) = way else {
            put_back(state, routing);
            return false;
        };
        #[allow(clippy::cast_precision_loss, reason = "a delay in picoseconds")]
        let s = base[i] - d as f64;
        least_done = least_done.min(s);
        let route = &mut done[i];
        route.pips.extend(path.iter().copied());
        for pip in &path {
            route.nodes.push(cx.graph.pip(*pip).to);
        }
        route.nodes.sort_unstable();
        route.nodes.dedup();
        let sink = &cx.terminals[group[i].which].sinks[group[i].sink];
        if target != sink.node {
            route.moved.push((sink.pin, target));
        }
        replace(
            state,
            routing,
            cx.terminals[group[i].which].signal,
            Some(route.clone()),
        );
    }
    if least_done < before + 1.0 {
        put_back(state, routing);
        return false;
    }
    for (m, route) in group.iter().zip(originals) {
        saved
            .entry(cx.terminals[m.which].signal)
            .or_insert(Some(route));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::best_assignment;

    /// **The latest input takes the fastest pin, though another loses its own
    /// for it.** Two inputs: the first has 400 ps before its way and reaches the
    /// two pins in 10 and 300; the second, the latest, has 100 and reaches them
    /// in 50 and 100. The first pin goes to the second input, which leaves 50
    /// ps, and the first takes the slow pin and keeps 100. Would catch an
    /// assignment that gave the pins in the inputs' order, or by the least total
    /// delay (both give the first pin to the first input, which leaves the
    /// second nothing).
    #[test]
    fn the_latest_input_takes_the_fastest_pin() {
        let base = [400.0, 100.0];
        let delays = vec![vec![Some(10), Some(300)], vec![Some(50), Some(100)]];
        let (least, chosen) = best_assignment(&base, &delays).expect("an assignment");
        assert_eq!(chosen, vec![1, 0]);
        assert!((least - 50.0).abs() < 1e-9, "{least}");
    }

    /// **A pin no way reaches is never given.** Would catch an assignment that
    /// read a missing way as free.
    #[test]
    fn a_pin_no_way_reaches_is_never_given() {
        let base = [100.0, 100.0];
        let delays = vec![vec![Some(10), None], vec![Some(10), None]];
        assert!(best_assignment(&base, &delays).is_none());
        let delays = vec![vec![Some(10), Some(20)], vec![Some(10), None]];
        let (_, chosen) = best_assignment(&base, &delays).expect("an assignment");
        assert_eq!(chosen, vec![1, 0]);
    }
}
