//! Routing: which pips carry which signal, by negotiated congestion.
//!
//! This is PathFinder (McMurchie and Ebeling, 1995), which is what every
//! open FPGA router since has been a variation of. The idea is that a
//! router which refuses to overuse a wire has to make routing decisions
//! in an arbitrary order and lives with them; a router which *allows*
//! overuse and then makes it progressively more expensive lets every
//! signal bid for the wire it wants, and the ones with an alternative
//! give way.
//!
//! One iteration rips up every signal and routes it again through a maze
//! expansion (A\* over the routing graph, with the distance to the sink
//! as the heuristic). The cost of entering a node is
//!
//! ```text
//! cost(n) = base(n) * (1 + present_factor * overuse(n)) * (1 + history(n))
//! ```
//!
//! where `overuse(n)` counts how many signals beyond its capacity would
//! be on the node, `history(n)` accumulates every iteration in which the
//! node was oversubscribed, and `present_factor` grows each iteration.
//! History is what makes the algorithm converge rather than oscillate: a
//! node that has been fought over stays expensive even once it is free,
//! so the loser looks elsewhere instead of taking it straight back.
//!
//! Iteration stops when no node is oversubscribed. [`RoutingReport`]
//! keeps the overuse of every iteration, so the convergence is visible
//! rather than asserted, and a design that will not converge is reported
//! ([`RouteError::Congested`]) rather than looped on.
//!
//! # What is routed
//!
//! One *signal* is one bit of one net with a driver and at least one
//! sink, as [`Netlist`] presents them. Its source is the graph node the
//! driving pin reaches on the site it was placed on, its sinks are the
//! nodes the reading pins reach. A constant is not a signal: the netlist
//! check ([`super::check_nextpnr_json`]) has already established that
//! every constant is a 0 or a 1, and tying those is the packer's job.
//!
//! A multi-sink signal is routed sink by sink onto a growing tree, with
//! the nodes already in the tree free, which is the usual way of getting
//! a shared trunk out of a shortest-path search.
//!
//! # Determinism
//!
//! Nothing here is random. Signals are routed in netlist order, sinks in
//! pin order, and the priority queue breaks ties by node id, so the same
//! placement always produces the same routing.

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap};
use std::error::Error;
use std::fmt;

use super::arch::{NodeId, PipId, RoutingGraph};
use super::place::{Netlist, Placement};

/// Why a design could not be routed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// A pin was not placed, so the router has no node to start from.
    Unplaced {
        /// The instance's name.
        instance: String,
    },
    /// A pin of a placed instance reaches no wire, which means the
    /// architecture's bel does not offer that pin role.
    NoNode {
        /// The instance's name.
        instance: String,
        /// The pin role the architecture lacks.
        role: String,
    },
    /// No path exists from a signal's source to one of its sinks, at any
    /// price. This is a fact about the architecture, not about
    /// congestion: rerouting cannot fix it.
    Unroutable {
        /// The signal's name.
        signal: String,
        /// The sink that could not be reached, as `cell.port[bit]`.
        sink: String,
    },
    /// The iteration cap was reached with nodes still oversubscribed.
    Congested {
        /// How many iterations ran.
        iterations: u32,
        /// How many nodes were still oversubscribed.
        overused: usize,
        /// The worst of them by name, most contended first, at most
        /// eight, so a report says *where* the design did not fit.
        worst: Vec<String>,
    },
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteError::Unplaced { instance } => {
                write!(f, "`{instance}` is not placed, so its pins have no wire")
            }
            RouteError::NoNode { instance, role } => write!(
                f,
                "the site `{instance}` is on offers no wire for its `{role}` pin"
            ),
            RouteError::Unroutable { signal, sink } => write!(
                f,
                "no path exists from the driver of `{signal}` to `{sink}`: \
                 the architecture has no wire joining them"
            ),
            RouteError::Congested {
                iterations,
                overused,
                worst,
            } => write!(
                f,
                "routing did not converge: {overused} node(s) are still oversubscribed \
                 after {iterations} iteration(s), worst at {}",
                worst.join(", ")
            ),
        }
    }
}

impl Error for RouteError {}

/// Knobs for [`route`].
#[derive(Clone, Debug)]
pub struct RouteOptions {
    /// How many rip-up and reroute iterations to allow.
    pub max_iterations: u32,
    /// The present-congestion factor of the first iteration.
    pub present_factor: f64,
    /// What the present-congestion factor is multiplied by each
    /// iteration. Growing it is what forces a decision.
    pub present_growth: f64,
    /// How much one iteration of overuse adds to a node's history cost.
    pub history_factor: f64,
    /// The cost the distance heuristic charges per tile of Manhattan
    /// distance still to cover.
    ///
    /// It must not exceed what a tile of travel really costs, or the
    /// search stops at the first path it finds instead of the cheapest
    /// one, which is exactly what a congestion-negotiating router must
    /// not do. On a fabric whose span-4 lines cross four tiles for one
    /// node, a tile costs about a quarter of a node, so the default is
    /// a little below that. Raising it makes the search faster and the
    /// routes worse; zero turns it into Dijkstra.
    pub astar_weight: f64,
}

impl Default for RouteOptions {
    fn default() -> Self {
        RouteOptions {
            max_iterations: 40,
            present_factor: 0.5,
            present_growth: 1.8,
            history_factor: 1.0,
            astar_weight: 0.3,
        }
    }
}

/// One signal's route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// The signal, as an index into [`Netlist::signals`].
    pub signal: usize,
    /// The node its driver reaches.
    pub source: NodeId,
    /// The pips it uses, in the order they were taken.
    pub pips: Vec<PipId>,
    /// Every node it occupies, including the source, sorted.
    pub nodes: Vec<NodeId>,
}

/// Every signal's route.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Routing {
    /// One entry per signal of the netlist; `None` for a signal with no
    /// driver or no sink.
    routes: Vec<Option<Route>>,
}

impl Routing {
    /// An empty routing for a netlist with `signals` signals.
    pub fn new(signals: usize) -> Self {
        Routing {
            routes: vec![None; signals],
        }
    }

    /// The route of one signal.
    pub fn route(&self, signal: usize) -> Option<&Route> {
        self.routes.get(signal).and_then(Option::as_ref)
    }

    /// The pips one signal uses; empty when it was not routed.
    pub fn pips(&self, signal: usize) -> &[PipId] {
        self.route(signal).map_or(&[][..], |r| r.pips.as_slice())
    }

    /// Every route, in signal order.
    pub fn routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter().flatten()
    }

    /// How many signals were routed.
    pub fn routed(&self) -> usize {
        self.routes.iter().flatten().count()
    }

    /// How many pips the whole routing uses.
    pub fn pip_count(&self) -> usize {
        self.routes.iter().flatten().map(|r| r.pips.len()).sum()
    }

    /// The routing as `signal: wire -> wire` lines, sorted by signal
    /// name, for reports and golden files.
    pub fn to_text(&self, netlist: &Netlist, graph: &RoutingGraph) -> String {
        let mut lines: Vec<String> = Vec::new();
        for route in self.routes() {
            let mut out = format!(
                "{} ({} pips)\n",
                netlist.signals[route.signal].name,
                route.pips.len()
            );
            for pip in &route.pips {
                let pip = graph.pip(*pip);
                out.push_str(&format!(
                    "  {} -> {}\n",
                    graph.wire(pip.from).full_name(),
                    graph.wire(pip.to).full_name()
                ));
            }
            lines.push(out);
        }
        lines.sort();
        lines.concat()
    }

    /// Checks that the routing really connects every sink to its driver.
    ///
    /// This is the property that matters, and it is checked the hard way:
    /// each sink node is walked *backwards* through the pips the signal
    /// was given until the source is reached, so a route that happens to
    /// occupy the right nodes without joining them is caught. Every
    /// failure is returned as a sentence; an empty result means the
    /// routing implements the netlist.
    pub fn verify(
        &self,
        netlist: &Netlist,
        graph: &RoutingGraph,
        placement: &Placement,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        for signal in netlist.routable() {
            let name = &netlist.signals[signal].name;
            let Some(route) = self.route(signal) else {
                problems.push(format!("`{name}` has a driver and sinks but no route"));
                continue;
            };
            // Where each node came from, for this signal only.
            let mut driven: Vec<(NodeId, NodeId)> = Vec::new();
            for pip in &route.pips {
                let pip = graph.pip(*pip);
                if driven.iter().any(|(to, _)| *to == pip.to) {
                    problems.push(format!(
                        "`{name}` drives {} from two pips",
                        graph.wire(pip.to).full_name()
                    ));
                }
                driven.push((pip.to, pip.from));
            }
            for pin in &netlist.signals[signal].sinks {
                let pin = &netlist.pins[*pin];
                let Some(site) = placement.site_of(pin.instance) else {
                    continue;
                };
                let Some(mut node) = graph.sites[site].pin(&pin.role) else {
                    continue;
                };
                let mut steps = 0usize;
                while node != route.source {
                    let Some((_, from)) = driven.iter().find(|(to, _)| *to == node) else {
                        problems.push(format!(
                            "`{name}` does not reach {}.{}[{}]: {} is driven by nothing",
                            netlist.instances[pin.instance].name,
                            pin.port,
                            pin.bit,
                            graph.wire(node).full_name()
                        ));
                        break;
                    };
                    node = *from;
                    steps += 1;
                    if steps > route.pips.len() {
                        problems.push(format!("`{name}` has a loop in its route"));
                        break;
                    }
                }
            }
        }
        problems
    }
}

/// What one rip-up and reroute iteration achieved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iteration {
    /// Which iteration, from 1.
    pub index: u32,
    /// How many nodes carry more signals than they can.
    pub overused_nodes: usize,
    /// The sum of the excess over every such node.
    pub total_overuse: usize,
    /// How many pips the routing used at the end of the iteration.
    pub pips: usize,
}

/// What [`route`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoutingReport {
    /// One entry per iteration, in order.
    pub iterations: Vec<Iteration>,
    /// How many signals were routed.
    pub signals: usize,
    /// How many pips the finished routing uses.
    pub pips: usize,
    /// How many distinct nodes it occupies.
    pub nodes: usize,
}

impl RoutingReport {
    /// The report as plain text, one line per iteration so that the
    /// convergence can be read off.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("routing:\n");
        for it in &self.iterations {
            let _ = writeln!(
                out,
                "  iteration {}: {} pips, {} overused node(s), {} total overuse",
                it.index, it.pips, it.overused_nodes, it.total_overuse
            );
        }
        let _ = writeln!(
            out,
            "  routed {} signal(s) with {} pips over {} node(s)",
            self.signals, self.pips, self.nodes
        );
        out
    }
}

/// A total order over costs, so they can go in a heap.
#[derive(Clone, Copy, PartialEq)]
struct Score(f64);

impl Eq for Score {}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The router's per-node state.
struct State {
    /// How many signals are on each node.
    occupancy: Vec<u32>,
    /// The accumulated congestion history of each node.
    history: Vec<f64>,
}

impl State {
    fn new(nodes: usize) -> Self {
        State {
            occupancy: vec![0; nodes],
            history: vec![0.0; nodes],
        }
    }

    /// The cost of putting one more signal on a node.
    fn node_cost(&self, node: NodeId, present: f64) -> f64 {
        let index = node as usize;
        // Capacity is one signal per wire; the excess is what the present
        // factor multiplies.
        let overuse = f64::from(self.occupancy[index]);
        (1.0 + present * overuse) * (1.0 + self.history[index])
    }

    /// The nodes carrying more than one signal.
    fn overuse(&self) -> (usize, usize) {
        let mut nodes = 0;
        let mut total = 0;
        for count in &self.occupancy {
            if *count > 1 {
                nodes += 1;
                total += usize::try_from(*count - 1).unwrap_or(0);
            }
        }
        (nodes, total)
    }
}

/// Routes a placed netlist.
///
/// # Errors
///
/// [`RouteError::Unplaced`] and [`RouteError::NoNode`] when the placement
/// and the architecture disagree, [`RouteError::Unroutable`] when the
/// fabric has no path at all, and [`RouteError::Congested`] when the
/// iteration cap is reached with overuse left.
pub fn route(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    options: &RouteOptions,
) -> Result<(Routing, RoutingReport), RouteError> {
    let signals = netlist.routable();
    let mut terminals: Vec<Terminals> = Vec::new();
    for signal in &signals {
        let s = &netlist.signals[*signal];
        let driver = s.driver.expect("routable signals have a driver");
        let source = node_of(netlist, graph, placement, driver)?;
        let mut sinks = Vec::new();
        for pin in &s.sinks {
            let node = node_of(netlist, graph, placement, *pin)?;
            let pin = &netlist.pins[*pin];
            let name = format!(
                "{}.{}[{}]",
                netlist.instances[pin.instance].name, pin.port, pin.bit
            );
            if node != source && !sinks.iter().any(|(n, _)| *n == node) {
                sinks.push((node, name));
            }
        }
        terminals.push(Terminals {
            signal: *signal,
            source,
            sinks,
        });
    }

    let mut state = State::new(graph.nodes.len());
    let mut scratch = Scratch::new(graph.nodes.len());
    let mut routing = Routing::new(netlist.signals.len());
    let mut report = RoutingReport::default();
    let mut present = options.present_factor;

    for index in 1..=options.max_iterations {
        for terminals in &terminals {
            rip_up(&mut state, &mut routing, terminals.signal);
            let route = route_one(
                graph,
                &state,
                &mut scratch,
                terminals,
                present,
                options,
                netlist,
            )?;
            for node in &route.nodes {
                state.occupancy[*node as usize] += 1;
            }
            routing.routes[terminals.signal] = Some(route);
        }
        let (overused_nodes, total_overuse) = state.overuse();
        report.iterations.push(Iteration {
            index,
            overused_nodes,
            total_overuse,
            pips: routing.pip_count(),
        });
        if overused_nodes == 0 {
            break;
        }
        for node in 0..graph.nodes.len() {
            if state.occupancy[node] > 1 {
                state.history[node] += options.history_factor;
            }
        }
        present *= options.present_growth;
    }

    let (overused, _) = state.overuse();
    if overused > 0 {
        let mut worst: Vec<(u32, NodeId)> = state
            .occupancy
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 1)
            .map(|(node, count)| (*count, NodeId::try_from(node).unwrap_or(0)))
            .collect();
        worst.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        return Err(RouteError::Congested {
            iterations: options.max_iterations,
            overused,
            worst: worst
                .iter()
                .take(8)
                .map(|(count, node)| format!("{} ({count} signals)", graph.wire(*node).full_name()))
                .collect(),
        });
    }
    report.signals = routing.routed();
    report.pips = routing.pip_count();
    report.nodes = state.occupancy.iter().filter(|c| **c > 0).count();
    Ok((routing, report))
}

/// The graph node one pin reaches, given where its instance was placed.
fn node_of(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    pin: usize,
) -> Result<NodeId, RouteError> {
    let pin = &netlist.pins[pin];
    let instance = &netlist.instances[pin.instance];
    let Some(site) = placement.site_of(pin.instance) else {
        return Err(RouteError::Unplaced {
            instance: instance.name.clone(),
        });
    };
    graph.sites[site].pin(&pin.role).ok_or(RouteError::NoNode {
        instance: instance.name.clone(),
        role: pin.role.clone(),
    })
}

/// Takes a signal's route out of the occupancy counts.
fn rip_up(state: &mut State, routing: &mut Routing, signal: usize) {
    if let Some(route) = routing.routes[signal].take() {
        for node in route.nodes {
            let slot = &mut state.occupancy[node as usize];
            *slot = slot.saturating_sub(1);
        }
    }
}

/// Where one signal starts and where it has to get to, resolved onto the
/// graph once and reused by every iteration.
struct Terminals {
    /// The signal, as an index into [`Netlist::signals`].
    signal: usize,
    /// The node its driver reaches.
    source: NodeId,
    /// The nodes its sinks reach, each with a name for diagnostics.
    sinks: Vec<(NodeId, String)>,
}

/// Routes one signal onto a tree that starts at its source.
fn route_one(
    graph: &RoutingGraph,
    state: &State,
    scratch: &mut Scratch,
    terminals: &Terminals,
    present: f64,
    options: &RouteOptions,
    netlist: &Netlist,
) -> Result<Route, RouteError> {
    let source = terminals.source;
    let signal = terminals.signal;
    let mut tree: BTreeSet<NodeId> = BTreeSet::new();
    tree.insert(source);
    let mut pips: Vec<PipId> = Vec::new();
    for (sink, name) in &terminals.sinks {
        let Some(path) = maze(graph, state, scratch, &tree, *sink, present, options) else {
            return Err(RouteError::Unroutable {
                signal: netlist.signals[signal].name.clone(),
                sink: name.clone(),
            });
        };
        for pip in path {
            tree.insert(graph.pip(pip).to);
            pips.push(pip);
        }
    }
    Ok(Route {
        signal,
        source,
        pips,
        nodes: tree.into_iter().collect(),
    })
}

/// The per-expansion arrays, reused across the sinks of one signal.
struct Scratch {
    cost: Vec<f64>,
    from: Vec<PipId>,
    stamp: Vec<u32>,
    generation: u32,
}

impl Scratch {
    fn new(nodes: usize) -> Self {
        Scratch {
            cost: vec![0.0; nodes],
            from: vec![0; nodes],
            stamp: vec![0; nodes],
            generation: 0,
        }
    }

    fn start(&mut self) {
        self.generation += 1;
    }

    fn seen(&self, node: NodeId) -> bool {
        self.stamp[node as usize] == self.generation
    }

    fn cost_of(&self, node: NodeId) -> f64 {
        if self.seen(node) {
            self.cost[node as usize]
        } else {
            f64::INFINITY
        }
    }

    fn set(&mut self, node: NodeId, cost: f64, from: PipId) {
        let index = node as usize;
        self.cost[index] = cost;
        self.from[index] = from;
        self.stamp[index] = self.generation;
    }
}

/// A\* from every node of `tree` to `sink`, returning the pips of the
/// path from the tree to the sink, nearest the tree first.
fn maze(
    graph: &RoutingGraph,
    state: &State,
    scratch: &mut Scratch,
    tree: &BTreeSet<NodeId>,
    sink: NodeId,
    present: f64,
    options: &RouteOptions,
) -> Option<Vec<PipId>> {
    let target = graph.wire(sink).tile;
    let heuristic = |node: NodeId| -> f64 {
        let wire = graph.wire(node);
        let (x, y) = wire.nearest_tile(target.0, target.1);
        let distance = u64::from(x.abs_diff(target.0)) + u64::from(y.abs_diff(target.1));
        options.astar_weight * distance as f64
    };

    scratch.start();
    let mut heap: BinaryHeap<std::cmp::Reverse<(Score, NodeId)>> = BinaryHeap::new();
    for node in tree {
        scratch.set(*node, 0.0, PipId::MAX);
        heap.push(std::cmp::Reverse((Score(heuristic(*node)), *node)));
    }
    let mut reached = false;
    while let Some(std::cmp::Reverse((Score(estimate), node))) = heap.pop() {
        let here = scratch.cost_of(node);
        if estimate > here + heuristic(node) {
            continue;
        }
        if node == sink {
            reached = true;
            break;
        }
        for pip in graph.outgoing(node) {
            let next = graph.pip(*pip).to;
            let step = state.node_cost(next, present);
            let candidate = here + step;
            if candidate < scratch.cost_of(next) {
                scratch.set(next, candidate, *pip);
                heap.push(std::cmp::Reverse((
                    Score(candidate + heuristic(next)),
                    next,
                )));
            }
        }
    }
    if !reached {
        return None;
    }
    let mut path = Vec::new();
    let mut node = sink;
    while !tree.contains(&node) {
        let pip = scratch.from[node as usize];
        if pip == PipId::MAX {
            return None;
        }
        path.push(pip);
        node = graph.pip(pip).from;
    }
    path.reverse();
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::arch::{Arch, BelDecl, ConfigBit, PipDecl, TileType, WireDecl, WireRef};
    use crate::fpga::place::{Instance, NetPin, Signal};
    use crate::ir::{CellId, Id};

    /// A line of tiles, each with two source bels and two sink bels
    /// joined by `tracks` parallel tracks running the length of the row.
    /// How many tracks there are is what decides whether two signals fit.
    fn line(length: u32, tracks: usize) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("line", "test", length, 1);
        let mut tile = TileType::new("t", "t_tile", 8, 8);
        for k in 0..tracks {
            tile.wires.push(WireDecl {
                name: format!("track{k}"),
                dx: 0,
                dy: 0,
            });
        }
        for n in 0..2 {
            tile.wires.push(WireDecl {
                name: format!("out{n}"),
                dx: 0,
                dy: 0,
            });
            tile.wires.push(WireDecl {
                name: format!("in{n}"),
                dx: 0,
                dy: 0,
            });
            let mut src = BelDecl::new(format!("src{n}"), "lut");
            src.pins
                .push(("o".to_owned(), WireRef::local(format!("out{n}"))));
            tile.bels.push(src);
            let mut dst = BelDecl::new(format!("dst{n}"), "ff");
            dst.pins
                .push(("d".to_owned(), WireRef::local(format!("in{n}"))));
            tile.bels.push(dst);
        }
        let mut bit = 0u32;
        let mut next_bit = || {
            let at = ConfigBit::new(bit / 8, bit % 8);
            bit += 1;
            at
        };
        for k in 0..tracks {
            let track = format!("track{k}");
            for n in 0..2 {
                tile.pips.push(PipDecl {
                    from: WireRef::local(format!("out{n}")),
                    to: WireRef::local(&track),
                    bits: vec![next_bit()],
                });
                tile.pips.push(PipDecl {
                    from: WireRef::local(&track),
                    to: WireRef::local(format!("in{n}")),
                    bits: vec![next_bit()],
                });
            }
            // The track continues east and west.
            tile.pips.push(PipDecl {
                from: WireRef::at(&track, -1, 0),
                to: WireRef::local(&track),
                bits: vec![next_bit()],
            });
            tile.pips.push(PipDecl {
                from: WireRef::at(&track, 1, 0),
                to: WireRef::local(&track),
                bits: vec![next_bit()],
            });
        }
        arch.tile_types.push(tile);
        for x in 0..length {
            arch.set_tile(x, 0, 0);
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// `n` independent source-to-sink signals.
    fn pairs(n: usize) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: Vec::new(),
            off_fabric: Vec::new(),
        };
        for i in 0..n {
            let source = netlist.instances.len();
            let pin = netlist.pins.len();
            netlist.pins.push(NetPin {
                instance: source,
                port: "O".to_owned(),
                bit: 0,
                role: "o".to_owned(),
                output: true,
                signal: Some(i),
                constant: None,
            });
            netlist.instances.push(Instance {
                cell: CellId::from_index(netlist.instances.len()),
                name: format!("src{i}"),
                primitive: "L".to_owned(),
                kind: "lut".to_owned(),
                pins: vec![pin],
                pin: None,
            });
            let sink = netlist.instances.len();
            let sink_pin = netlist.pins.len();
            netlist.pins.push(NetPin {
                instance: sink,
                port: "D".to_owned(),
                bit: 0,
                role: "d".to_owned(),
                output: false,
                signal: Some(i),
                constant: None,
            });
            netlist.instances.push(Instance {
                cell: CellId::from_index(netlist.instances.len()),
                name: format!("dst{i}"),
                primitive: "F".to_owned(),
                kind: "ff".to_owned(),
                pins: vec![sink_pin],
                pin: None,
            });
            netlist.signals.push(Signal {
                name: format!("n{i}"),
                driver: Some(pin),
                sinks: vec![sink_pin],
            });
        }
        netlist
    }

    /// Places instance `i` on the first free site of the kind it needs
    /// in tile `tiles[i]`.
    fn place_in_order(netlist: &Netlist, graph: &RoutingGraph, tiles: &[u32]) -> Placement {
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, instance) in netlist.instances.iter().enumerate() {
            let tile = tiles[index];
            let site = graph
                .sites
                .iter()
                .position(|s| {
                    s.kind == instance.kind
                        && s.tile == (tile, 0)
                        && placement
                            .instance_at(graph.site_index(&s.name).unwrap())
                            .is_none()
                })
                .expect("a free site of that kind in that tile");
            placement.place(index, site);
        }
        placement
    }

    #[test]
    fn a_routable_design_converges_in_one_iteration() {
        let (_, graph) = line(6, 2);
        let netlist = pairs(1);
        let placement = place_in_order(&netlist, &graph, &[0, 5]);
        let (routing, report) =
            route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
        assert_eq!(report.iterations.len(), 1);
        assert_eq!(report.iterations[0].overused_nodes, 0);
        assert_eq!(report.signals, 1);
        // out -> track -> four hops east -> in.
        assert_eq!(routing.pips(0).len(), 7, "{:?}", routing.pips(0));
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        assert!(report.to_text().contains("iteration 1"));
        assert!(
            routing
                .to_text(&netlist, &graph)
                .starts_with("n0 (7 pips)\n")
        );
        assert_eq!(routing.routes().count(), 1);
        assert!(routing.route(1).is_none());
        assert!(routing.pips(1).is_empty());
    }

    #[test]
    fn two_signals_take_a_track_each() {
        // Two signals crossing the same stretch with two tracks: the
        // router gives them one each, and the present-congestion cost is
        // enough to do it in the first pass.
        let (_, graph) = line(6, 2);
        let netlist = pairs(2);
        let placement = place_in_order(&netlist, &graph, &[0, 5, 1, 4]);
        let (routing, report) =
            route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
        assert_eq!(report.iterations.last().unwrap().overused_nodes, 0);
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        let a: BTreeSet<NodeId> = routing.route(0).unwrap().nodes.iter().copied().collect();
        let b: BTreeSet<NodeId> = routing.route(1).unwrap().nodes.iter().copied().collect();
        assert!(a.is_disjoint(&b), "{a:?} {b:?}");
    }

    /// A row of tiles with one track each, plus a second row of tracks
    /// above it: going the long way round costs two extra hops.
    fn detour(length: u32) -> (Arch, RoutingGraph) {
        let mut arch = Arch::new("detour", "test", length, 2);
        let track = |t: &mut TileType| {
            t.wires.push(WireDecl {
                name: "track".to_owned(),
                dx: 0,
                dy: 0,
            });
            for (dx, row) in [(-1, 0), (1, 0)] {
                t.pips.push(PipDecl {
                    from: WireRef::at("track", dx, row),
                    to: WireRef::local("track"),
                    bits: vec![ConfigBit::new(0, 0)],
                });
            }
        };
        let mut main = TileType::new("main", "main", 8, 8);
        track(&mut main);
        for n in 0..2 {
            main.wires.push(WireDecl {
                name: format!("out{n}"),
                dx: 0,
                dy: 0,
            });
            main.wires.push(WireDecl {
                name: format!("in{n}"),
                dx: 0,
                dy: 0,
            });
            let mut src = BelDecl::new(format!("src{n}"), "lut");
            src.pins
                .push(("o".to_owned(), WireRef::local(format!("out{n}"))));
            main.bels.push(src);
            let mut dst = BelDecl::new(format!("dst{n}"), "ff");
            dst.pins
                .push(("d".to_owned(), WireRef::local(format!("in{n}"))));
            main.bels.push(dst);
            main.pips.push(PipDecl {
                from: WireRef::local(format!("out{n}")),
                to: WireRef::local("track"),
                bits: vec![ConfigBit::new(1, 0)],
            });
            main.pips.push(PipDecl {
                from: WireRef::local("track"),
                to: WireRef::local(format!("in{n}")),
                bits: vec![ConfigBit::new(1, 1)],
            });
        }
        main.pips.push(PipDecl {
            from: WireRef::local("track"),
            to: WireRef::at("track", 0, 1),
            bits: vec![ConfigBit::new(2, 0)],
        });
        main.pips.push(PipDecl {
            from: WireRef::at("track", 0, 1),
            to: WireRef::local("track"),
            bits: vec![ConfigBit::new(2, 1)],
        });
        let mut aux = TileType::new("aux", "aux", 8, 8);
        track(&mut aux);
        arch.tile_types.push(main);
        arch.tile_types.push(aux);
        for x in 0..length {
            arch.set_tile(x, 0, 0);
            arch.set_tile(x, 1, 1);
        }
        let graph = arch.build_graph();
        (arch, graph)
    }

    /// The property PathFinder exists for: two signals want one track,
    /// the detour is too expensive to take at first, and the growing
    /// present-congestion cost makes one of them take it anyway.
    #[test]
    fn congestion_is_negotiated_away() {
        let (_, graph) = detour(8);
        let netlist = pairs(2);
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, (bel, tile)) in [("src0", 0), ("dst0", 7), ("src1", 1), ("dst1", 6)]
            .into_iter()
            .enumerate()
        {
            let site = graph
                .site_index(&format!("X{tile}Y0/{bel}"))
                .expect("the bel exists");
            placement.place(index, site);
        }
        let options = RouteOptions {
            // Low enough that one shared track is cheaper than the
            // detour, so the first pass collides.
            present_factor: 0.05,
            present_growth: 3.0,
            ..RouteOptions::default()
        };
        let (routing, report) = route(&netlist, &graph, &placement, &options).unwrap();
        assert!(
            report.iterations.len() > 1,
            "the first pass should have collided:\n{}",
            report.to_text()
        );
        assert!(report.iterations[0].total_overuse > 0);
        assert_eq!(report.iterations.last().unwrap().overused_nodes, 0);
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        // One of them ended up on the second row.
        let used_aux = routing
            .routes()
            .any(|r| r.nodes.iter().any(|n| graph.wire(*n).tile.1 == 1));
        assert!(used_aux, "nobody took the detour");
    }

    #[test]
    fn an_over_subscribed_fabric_is_reported_not_looped_on() {
        // Three signals, one track: no rerouting can fix it.
        let (_, graph) = line(4, 1);
        let netlist = pairs(3);
        let placement = place_in_order(&netlist, &graph, &[0, 3, 0, 3, 1, 2]);
        let options = RouteOptions {
            max_iterations: 5,
            ..RouteOptions::default()
        };
        let err = route(&netlist, &graph, &placement, &options).unwrap_err();
        let RouteError::Congested {
            iterations,
            overused,
            ..
        } = err
        else {
            panic!("expected congestion, got {err}");
        };
        assert_eq!(iterations, 5);
        assert!(overused > 0);
        assert!(err.to_string().contains("did not converge"));
    }

    #[test]
    fn a_disconnected_fabric_is_reported() {
        // Two tiles with no track between them at all.
        let mut arch = Arch::new("split", "test", 2, 1);
        let mut tile = TileType::new("t", "t", 2, 2);
        tile.wires.push(WireDecl {
            name: "out".to_owned(),
            dx: 0,
            dy: 0,
        });
        tile.wires.push(WireDecl {
            name: "in".to_owned(),
            dx: 0,
            dy: 0,
        });
        let mut src = BelDecl::new("src", "lut");
        src.pins.push(("o".to_owned(), WireRef::local("out")));
        tile.bels.push(src);
        let mut dst = BelDecl::new("dst", "ff");
        dst.pins.push(("d".to_owned(), WireRef::local("in")));
        tile.bels.push(dst);
        arch.tile_types.push(tile);
        arch.set_tile(0, 0, 0);
        arch.set_tile(1, 0, 0);
        let graph = arch.build_graph();
        let netlist = pairs(1);
        let placement = place_in_order(&netlist, &graph, &[0, 1]);
        let err = route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap_err();
        assert_eq!(
            err,
            RouteError::Unroutable {
                signal: "n0".to_owned(),
                sink: "dst0.D[0]".to_owned(),
            }
        );
        assert!(err.to_string().contains("no wire joining them"));
    }

    #[test]
    fn an_unplaced_or_pinless_instance_is_reported() {
        let (_, graph) = line(3, 1);
        let netlist = pairs(1);
        let placement = Placement::new(netlist.instances.len(), graph.sites.len());
        let err = route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap_err();
        assert_eq!(
            err,
            RouteError::Unplaced {
                instance: "src0".to_owned()
            }
        );
        assert!(err.to_string().contains("is not placed"));

        let mut netlist = pairs(1);
        netlist.pins[1].role = "nope".to_owned();
        let placement = place_in_order(&netlist, &graph, &[0, 2]);
        let err = route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap_err();
        assert_eq!(
            err,
            RouteError::NoNode {
                instance: "dst0".to_owned(),
                role: "nope".to_owned(),
            }
        );
        assert!(err.to_string().contains("offers no wire"));
    }

    #[test]
    fn verification_catches_a_route_that_does_not_connect() {
        let (_, graph) = line(4, 1);
        let netlist = pairs(1);
        let placement = place_in_order(&netlist, &graph, &[0, 3]);
        let (mut routing, _) =
            route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        // Drop the last pip: the sink is no longer driven.
        let route = routing.routes[0].as_mut().unwrap();
        route.pips.pop();
        let problems = routing.verify(&netlist, &graph, &placement);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("is driven by nothing"), "{problems:?}");

        // No route at all for a signal that needs one.
        routing.routes[0] = None;
        let problems = routing.verify(&netlist, &graph, &placement);
        assert!(problems[0].contains("no route"), "{problems:?}");
    }

    #[test]
    fn a_multi_sink_signal_shares_its_trunk() {
        let (_, graph) = line(8, 2);
        let mut netlist = pairs(1);
        // A second sink at the far end, on the same signal.
        let instance = netlist.instances.len();
        let pin = netlist.pins.len();
        netlist.pins.push(NetPin {
            instance,
            port: "D".to_owned(),
            bit: 0,
            role: "d".to_owned(),
            output: false,
            signal: Some(0),
            constant: None,
        });
        netlist.instances.push(Instance {
            cell: CellId::from_index(instance),
            name: "dst1".to_owned(),
            primitive: "F".to_owned(),
            kind: "ff".to_owned(),
            pins: vec![pin],
            pin: None,
        });
        netlist.signals[0].sinks.push(pin);
        let placement = place_in_order(&netlist, &graph, &[0, 4, 7]);
        let (routing, report) =
            route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        // One trunk to the far sink plus a tap, not two full paths.
        assert!(report.pips < 16, "{}", report.to_text());
        assert_eq!(report.signals, 1);
    }
}
