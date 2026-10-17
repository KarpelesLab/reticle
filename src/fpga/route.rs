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

use std::collections::{BTreeMap, BinaryHeap, HashMap};
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
        /// For those of them that are a wire the cells of one tile have
        /// to agree about — a clock, an enable, a set/reset — what
        /// contends for it and what can be done about it. Empty when the
        /// contention is ordinary interconnect, which is a design that is
        /// simply too dense and not a legality problem.
        ///
        /// This exists because `oversubscribed node X24Y3/LSR1` names a
        /// tile and not a cause, and nobody meeting it for the first time
        /// can tell that a distributed RAM's write enable and a
        /// flip-flop's reset are the same piece of metal.
        contention: Vec<String>,
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
                contention,
            } => {
                write!(
                    f,
                    "routing did not converge: {overused} node(s) are still oversubscribed \
                     after {iterations} iteration(s), worst at {}",
                    worst.join(", ")
                )?;
                for line in contention {
                    write!(f, "\n{line}")?;
                }
                Ok(())
            }
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
    ///
    /// The charge is scaled per node by [`RouteOptions::node_base`], for
    /// exactly the reason in the paragraph above: a node whose class costs a
    /// twentieth of an ordinary wire cannot be charged a whole tile of
    /// distance for standing on it.
    pub astar_weight: f64,
    /// The base cost of entering each node, by node id, or empty for "one
    /// for every node".
    ///
    /// This is the `base(n)` of the cost formula in this module's header,
    /// which was 1 everywhere until a family turned up whose clock network
    /// a signal has to be *steered* onto rather than merely allowed to
    /// reach. On a Lattice ECP5 a flip-flop's clock mux takes sixteen
    /// global branch wires **and** seven ordinary interconnect wires, so the
    /// shortest path from a pad to a clock pin goes through general routing
    /// — seven hops against about eighteen — and a router with no
    /// preference builds a clock tree out of data wires. It works and it is
    /// the wrong answer: the skew across a dozen sinks is nobody's model.
    ///
    /// Making the network cheap is **a preference and not a permission**.
    /// Capacity is still one signal per node, so nothing here can put two
    /// signals on one wire, and a cheap wire a signal has no reason to
    /// enter is not entered: `super::trellis`' network is a one-way funnel
    /// whose only exits are flip-flop control pins, so the only signal that
    /// can traverse it is one that clocks or resets something.
    ///
    /// A base below one makes the distance heuristic inadmissible, so the
    /// search may return a path that is not the cheapest. That costs
    /// optimality and not correctness, and it errs towards the network,
    /// which is the direction the caller asked for.
    pub node_base: Vec<f32>,
    /// Nodes only some signals may enter, by node id, or empty for "none":
    /// a node marked `true` is closed to every signal
    /// [`RouteOptions::network_signals`] does not allow.
    ///
    /// This exists because a cheap class of wire is only sound when nothing
    /// but the signals it is meant for can reach it. On an ECP5 the clock
    /// network is a one-way funnel, so [`RouteOptions::node_base`] alone is
    /// enough. A 7-series clock network is **not**: the clock row takes
    /// interconnect inputs and a break tile joins general routing onto the
    /// vertical tracks, so making it cheap drew data signals onto it — a
    /// flip-flop's output rode 1322 pips of global clock wire, and took the
    /// ground fans a block RAM's write enable needed.
    pub network: Vec<bool>,
    /// The signals allowed onto [`RouteOptions::network`], by signal index,
    /// or empty for "every signal".
    pub network_signals: Vec<bool>,
    /// After the first iteration, reroute only the signals whose route
    /// shares a node, and leave the rest where they are.
    ///
    /// Off by default, which is PathFinder as published: every signal is
    /// ripped up and rerouted every iteration. That is what every family
    /// here was measured with, so turning it on changes their routes. On
    /// an UltraScale+ region of 1.3 million nodes a pass over 9 416
    /// signals takes about a minute, and most of them were never in
    /// contention; this is the usual remedy, and the congested signals
    /// still see the history and present costs every other node has
    /// accumulated.
    pub reroute_congested_only: bool,
    /// What makes the router timing-driven, or `None` for a router that
    /// minimises wire alone; see [`Timing`].
    pub timing: Option<Timing>,
    /// Called with each iteration as it finishes, or `None`. The router
    /// writes nothing itself; this is how a caller that does — the
    /// command line's `--timing` — shows a route of many minutes moving
    /// rather than only its summary at the end.
    pub on_iteration: Option<fn(&Iteration)>,
}

/// What the router needs to weigh delay against congestion.
///
/// This is the criticality of VPR's timing-driven router (Betz, Rose and
/// Marquardt, 1999) on top of PathFinder. After each pass a static timing
/// analysis over the routed netlist gives every connection a slack, and
/// its criticality is `1 - slack / longest path`: one on the longest path,
/// falling towards zero the more slack it has. On the next pass a
/// connection pays for each node it enters
///
/// ```text
/// criticality * delay(n) + (1 - criticality) * cost(n)
/// ```
///
/// with `cost(n)` the congestion cost of this module's header, so a
/// critical connection takes the fast path and a slack one the
/// uncongested one. Its start counts too: a sink joining the tree at a
/// node far from the source inherits the delay to that node.
///
/// **The delays are a model, not a measurement.** They are numbers the
/// caller chose for each node and for a lookup table, in units only the
/// ratio of which matters; the analysis is as good as they are, and what
/// it reports ([`Iteration::critical`]) is in the same units.
#[derive(Clone, Debug)]
pub struct Timing {
    /// Each node's delay, by node id, in the caller's unit. A node not
    /// listed costs nothing.
    pub node_delay: Vec<u32>,
    /// The delay through an instance [`Timing::combinational`] marks, input
    /// to output, in the same unit.
    pub cell_delay: u32,
    /// The instances whose outputs follow their inputs within a cycle, by
    /// instance index; every other instance starts and ends paths. Every
    /// input of a combinational instance is taken to reach every output,
    /// which is what a lookup table does.
    pub combinational: Vec<bool>,
    /// Pin roles that are not data, and that the analysis leaves out: a
    /// flip-flop's clock pin ends no path, and the clock network's delay
    /// is skew, which this does not model.
    pub clock_roles: Vec<String>,
    /// The highest criticality any connection is given, below one so that
    /// congestion still costs a critical connection something.
    pub max_criticality: f64,
    /// How many passes to make at least, so that the analysis of one pass
    /// steers another: a design that routes cleanly at the first pass
    /// would otherwise never see its timing.
    pub passes: u32,
    /// After the first pass, reroute also every signal one of whose
    /// connections is at least this critical, even when
    /// [`RouteOptions::reroute_congested_only`] would leave it.
    pub reroute_above: f64,
}

impl Timing {
    /// Timing for `netlist` with every node delaying by `node_delay`,
    /// `lut` and `carry` instances combinational and `clk` pins left out.
    pub fn for_netlist(netlist: &Netlist, node_delay: Vec<u32>, cell_delay: u32) -> Timing {
        Timing {
            node_delay,
            cell_delay,
            combinational: netlist
                .instances
                .iter()
                .map(|i| matches!(i.kind.as_str(), "lut" | "carry"))
                .collect(),
            clock_roles: vec!["clk".to_owned()],
            max_criticality: 0.99,
            passes: 4,
            reroute_above: 0.9,
        }
    }
}

impl Default for RouteOptions {
    fn default() -> Self {
        RouteOptions {
            max_iterations: 40,
            present_factor: 0.5,
            present_growth: 1.8,
            history_factor: 1.0,
            astar_weight: 0.3,
            node_base: Vec::new(),
            network: Vec::new(),
            network_signals: Vec::new(),
            reroute_congested_only: false,
            timing: None,
            on_iteration: None,
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

    /// Physical nodes that carry more than one signal, one line each.
    ///
    /// The router keeps capacity per physical node (see
    /// [`physical_nodes`]), so this is empty for any routing it returns;
    /// it is here so that a writer can refuse what it is about to write
    /// rather than trust that, and because the defect it catches -- two
    /// signals on the two ends of one long line -- passes every per-route
    /// check there is.
    #[must_use]
    pub fn shorts(&self, netlist: &Netlist, graph: &RoutingGraph) -> Vec<String> {
        let group = physical_nodes(graph);
        let mut owner: HashMap<NodeId, usize> = HashMap::new();
        let mut out = Vec::new();
        for route in self.routes() {
            let mut roots: Vec<NodeId> = route.nodes.iter().map(|n| group[*n as usize]).collect();
            roots.sort_unstable();
            roots.dedup();
            for root in roots {
                match owner.get(&root) {
                    Some(other) if *other != route.signal => out.push(format!(
                        "`{}` and `{}` share {}",
                        netlist.signals[*other].name,
                        netlist.signals[route.signal].name,
                        graph.wire(root).full_name()
                    )),
                    Some(_) => {}
                    None => {
                        owner.insert(root, route.signal);
                    }
                }
            }
        }
        out
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
                // A sink role may name several wires — a distributed RAM's
                // read address is one address read by four lookup tables —
                // and every one of them has to be reached.
                let wires: Vec<NodeId> = graph.sites[site].pin_nodes(&pin.role).collect();
                for mut node in wires {
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
    /// The longest path the timing analysis found after it, in
    /// [`Timing`]'s unit, or `None` when the router is not timing-driven.
    pub critical: Option<u64>,
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
    /// How many nodes the maze expansions took off the queue, over every
    /// iteration. This is the router's work, and unlike a number of
    /// seconds it means the same thing on every machine: a sharper
    /// distance estimate shows up here and nowhere else.
    pub visited: u64,
    /// How many nodes those expansions put on the queue.
    pub queued: u64,
}

impl RoutingReport {
    /// The report as plain text, one line per iteration so that the
    /// convergence can be read off.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("routing:\n");
        for it in &self.iterations {
            let _ = write!(
                out,
                "  iteration {}: {} pips, {} overused node(s), {} total overuse",
                it.index, it.pips, it.overused_nodes, it.total_overuse
            );
            match it.critical {
                Some(delay) => {
                    let _ = writeln!(out, ", longest path {delay}");
                }
                None => out.push('\n'),
            }
        }
        let _ = writeln!(
            out,
            "  routed {} signal(s) with {} pips over {} node(s)",
            self.signals, self.pips, self.nodes
        );
        let _ = writeln!(
            out,
            "  work: {} node(s) visited, {} queued",
            self.visited, self.queued
        );
        out
    }
}

/// A cost as a sort key the queue can compare with one integer
/// instruction.
///
/// For a non-negative `f64` — and every cost here is a sum and a product of
/// non-negative numbers — `to_bits` is monotonically non-decreasing, so
/// this orders exactly as `total_cmp` does, ties and all, and the router
/// takes the same decisions. What it saves is the comparison itself:
/// the expansion pushes 1.3 billion entries for one design on this die and
/// each push sifts up through about twenty of them.
fn score(cost: f64) -> u64 {
    debug_assert!(cost >= 0.0, "a routing cost is never negative: {cost}");
    cost.to_bits()
}

/// What the maze expansion needs to know about one node, in one cache
/// line's worth of plain numbers.
///
/// A [`Wire`](super::arch::Wire) owns its name, so it is fifty-odd bytes
/// with a pointer in it, and the expansion reads one per node *and* one
/// per edge it relaxes: five hundred million reads for one design, every
/// one of them a cache miss into a vector of strings. This holds the four
/// numbers the distance estimate uses and the node's base cost beside
/// them, which is one sequential-ish read instead of two scattered ones.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    /// The tile the wire starts in.
    tile: (u32, u32),
    /// How far it reaches from there, east and north.
    span: (i32, i32),
    /// Its base cost; see [`RouteOptions::node_base`].
    base: f32,
    /// True when it reaches every tile.
    global: bool,
    /// True when it is one of [`RouteOptions::network`]'s nodes.
    reserved: bool,
    /// Its [`Timing::node_delay`] in units of cost: see [`delay_scale`].
    delay: f32,
}

impl Geometry {
    /// The table for a whole graph.
    fn of(
        graph: &RoutingGraph,
        base: &[f32],
        reserved: &[bool],
        timing: Option<&Timing>,
    ) -> Vec<Geometry> {
        let delays = timing.map_or(&[][..], |t| &t.node_delay[..]);
        let scale = delay_scale(graph.nodes.len(), base, delays);
        graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, wire)| Geometry {
                tile: wire.tile,
                span: wire.span,
                base: base.get(index).copied().unwrap_or(1.0),
                global: wire.global,
                reserved: reserved.get(index).copied().unwrap_or(false),
                #[allow(clippy::cast_possible_truncation, reason = "a delay in cost units")]
                delay: (f64::from(delays.get(index).copied().unwrap_or(0)) * scale) as f32,
            })
            .collect()
    }

    /// The tile of this wire closest to `(x, y)`; see
    /// [`Wire::nearest_tile`](super::arch::Wire::nearest_tile), which this
    /// has to agree with.
    fn nearest_tile(&self, x: u32, y: u32) -> (u32, u32) {
        if self.global {
            return (x, y);
        }
        (
            super::arch::clamp_span(self.tile.0, self.span.0, x),
            super::arch::clamp_span(self.tile.1, self.span.1, y),
        )
    }
}

/// The router's per-node state.
struct State {
    /// How many signals are on each **physical** node, indexed by its
    /// root in `group`; only roots are ever non-zero.
    occupancy: Vec<u32>,
    /// The accumulated congestion history, by root likewise.
    history: Vec<f64>,
    /// Each graph node's physical node: see [`physical_nodes`].
    group: Vec<NodeId>,
}

impl State {
    fn new(group: Vec<NodeId>) -> Self {
        let nodes = group.len();
        State {
            occupancy: vec![0; nodes],
            history: vec![0.0; nodes],
            group,
        }
    }

    /// The cost of putting one more signal on a node whose base cost is
    /// `base`.
    fn node_cost(&self, node: NodeId, base: f32, present: f64) -> f64 {
        let index = self.group[node as usize] as usize;
        // Capacity is one signal per wire; the excess is what the present
        // factor multiplies.
        let overuse = f64::from(self.occupancy[index]);
        f64::from(base) * (1.0 + present * overuse) * (1.0 + self.history[index])
    }

    /// The physical nodes a route occupies, each once.
    fn roots(&self, nodes: &[NodeId]) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = nodes.iter().map(|n| self.group[*n as usize]).collect();
        out.sort_unstable();
        out.dedup();
        out
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
            // One pin, and possibly several wires: see
            // [`ArchSite::pin_nodes`](super::arch::ArchSite::pin_nodes).
            // They are sinks of the same net, which is what the fabric
            // says they are, so the router sees them as it sees any other
            // fan-out.
            let nodes = nodes_of(netlist, graph, placement, *pin)?;
            let index = *pin;
            let pin = &netlist.pins[index];
            let name = format!(
                "{}.{}[{}]",
                netlist.instances[pin.instance].name, pin.port, pin.bit
            );
            for node in nodes {
                if node != source && !sinks.iter().any(|s: &Sink| s.node == node) {
                    sinks.push(Sink {
                        node,
                        name: name.clone(),
                        pin: index,
                    });
                }
            }
        }
        terminals.push(Terminals {
            signal: *signal,
            source,
            sinks,
        });
    }

    let mut state = State::new(physical_nodes(graph));
    let geometry = Geometry::of(
        graph,
        &options.node_base,
        &options.network,
        options.timing.as_ref(),
    );
    // The node every outgoing pip reaches, in the order
    // [`RoutingGraph::outgoing`] hands the pips over, so the inner loop of
    // the expansion walks two sequential arrays instead of chasing a
    // twenty-byte `Pip` per edge. Four bytes an edge, against a cache miss
    // per edge relaxed — 1.3 billion of them for one design on this die.
    let mut reach_start: Vec<u32> = Vec::with_capacity(graph.nodes.len() + 1);
    let mut reach: Vec<NodeId> = Vec::with_capacity(graph.pips.len());
    for node in 0..graph.nodes.len() {
        reach_start.push(u32::try_from(reach.len()).unwrap_or(u32::MAX));
        for pip in graph.outgoing(NodeId::try_from(node).unwrap_or(0)) {
            reach.push(graph.pip(*pip).to);
        }
    }
    reach_start.push(u32::try_from(reach.len()).unwrap_or(u32::MAX));
    let mut scratch = Scratch::new(graph.nodes.len());
    let mut routing = Routing::new(netlist.signals.len());
    let mut report = RoutingReport::default();
    let mut present = options.present_factor;

    // Each signal's sinks' criticality, in `terminals` order, all zero
    // until a pass has been analysed.
    let mut criticality: Vec<Vec<f64>> =
        terminals.iter().map(|t| vec![0.0; t.sinks.len()]).collect();
    for index in 1..=options.max_iterations {
        for (which, terminals) in terminals.iter().enumerate() {
            let critical = options.timing.as_ref().is_some_and(|timing| {
                criticality[which]
                    .iter()
                    .any(|c| *c >= timing.reroute_above)
            });
            if index > 1
                && options.reroute_congested_only
                && !critical
                && routing.routes[terminals.signal].as_ref().is_some_and(|r| {
                    state
                        .roots(&r.nodes)
                        .iter()
                        .all(|n| state.occupancy[*n as usize] <= 1)
                })
            {
                continue;
            }
            rip_up(&mut state, &mut routing, terminals.signal);
            let route = route_one(
                graph,
                &geometry,
                (&reach_start, &reach),
                &state,
                &mut scratch,
                terminals,
                &criticality[which],
                present,
                options,
                netlist,
            )?;
            for root in state.roots(&route.nodes) {
                state.occupancy[root as usize] += 1;
            }
            routing.routes[terminals.signal] = Some(route);
        }
        let (overused_nodes, total_overuse) = state.overuse();
        let critical = options.timing.as_ref().map(|timing| {
            let (sinks, longest) = analyse(netlist, graph, &terminals, &routing, timing);
            criticality = sinks;
            longest
        });
        report.iterations.push(Iteration {
            index,
            overused_nodes,
            total_overuse,
            pips: routing.pip_count(),
            critical,
        });
        if let (Some(hook), Some(last)) = (options.on_iteration, report.iterations.last()) {
            hook(last);
        }
        let passes = options.timing.as_ref().map_or(0, |t| t.passes);
        if overused_nodes == 0 && index >= passes {
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
            contention: worst
                .iter()
                .take(8)
                .filter_map(|(_, node)| contention(netlist, graph, placement, &routing, *node))
                .collect(),
        });
    }
    report.signals = routing.routed();
    report.pips = routing.pip_count();
    report.visited = scratch.visited;
    report.queued = scratch.queued;
    report.nodes = state.occupancy.iter().filter(|c| **c > 0).count();
    Ok((routing, report))
}

/// What one unit of [`Timing::node_delay`] costs, so that a critical
/// connection weighs delay and an uncritical one congestion on the same
/// scale: the mean base cost of the nodes that have a delay, over their
/// mean delay. A wire of average delay then costs what a wire of average
/// base cost does.
fn delay_scale(nodes: usize, base: &[f32], delays: &[u32]) -> f64 {
    let (mut cost, mut delay) = (0.0, 0.0);
    for node in 0..nodes {
        let d = delays.get(node).copied().unwrap_or(0);
        if d > 0 {
            cost += f64::from(base.get(node).copied().unwrap_or(1.0));
            delay += f64::from(d);
        }
    }
    if delay > 0.0 { cost / delay } else { 0.0 }
}

/// Static timing analysis of a routing: every connection's criticality,
/// by signal in `terminals` order and sink in its order, and the longest
/// path, in [`Timing`]'s unit.
///
/// A path starts at an output of an instance [`Timing::combinational`]
/// does not mark — a flip-flop, a pad, the processor — at time zero, and
/// ends at an input of one. A connection's delay is the sum of the
/// [`Timing::node_delay`] of the nodes from its source to its sink along
/// the routed tree, the source's own excluded. A combinational loop is
/// broken where it is met, which under-reports it rather than looping.
fn analyse(
    netlist: &Netlist,
    graph: &RoutingGraph,
    terminals: &[Terminals],
    routing: &Routing,
    timing: &Timing,
) -> (Vec<Vec<f64>>, u64) {
    let pins = netlist.pins.len();
    let combinational =
        |instance: usize| timing.combinational.get(instance).copied().unwrap_or(false);
    let timed = |pin: usize| {
        !timing
            .clock_roles
            .iter()
            .any(|r| *r == netlist.pins[pin].role)
    };
    // Each input pin's connection delay.
    let mut wire = vec![0u64; pins];
    for terminals in terminals {
        let Some(route) = routing.routes[terminals.signal].as_ref() else {
            continue;
        };
        let mut delay: HashMap<NodeId, u64> = HashMap::new();
        delay.insert(route.source, 0);
        for pip in &route.pips {
            let pip = graph.pip(*pip);
            let to = delay.get(&pip.from).copied().unwrap_or(0)
                + u64::from(timing.node_delay.get(pip.to as usize).copied().unwrap_or(0));
            delay.insert(pip.to, to);
        }
        for sink in &terminals.sinks {
            let d = delay.get(&sink.node).copied().unwrap_or(0);
            wire[sink.pin] = wire[sink.pin].max(d);
        }
    }
    let driver = |pin: usize| -> Option<usize> {
        netlist.pins[pin]
            .signal
            .and_then(|s| netlist.signals[s].driver)
    };
    // The instances in an order in which every combinational instance
    // comes after the combinational instances driving it.
    let mut waiting = vec![0usize; netlist.instances.len()];
    for (index, instance) in netlist.instances.iter().enumerate() {
        if !combinational(index) {
            continue;
        }
        for pin in &instance.pins {
            if !netlist.pins[*pin].output
                && timed(*pin)
                && driver(*pin).is_some_and(|d| combinational(netlist.pins[d].instance))
            {
                waiting[index] += 1;
            }
        }
    }
    let mut order: Vec<usize> = (0..netlist.instances.len())
        .filter(|i| waiting[*i] == 0)
        .collect();
    let mut placed = vec![false; netlist.instances.len()];
    for i in &order {
        placed[*i] = true;
    }
    let mut next = 0;
    loop {
        while next < order.len() {
            let instance = order[next];
            next += 1;
            if !combinational(instance) {
                continue;
            }
            for pin in &netlist.instances[instance].pins {
                let pin = &netlist.pins[*pin];
                let Some(signal) = pin.signal.filter(|_| pin.output) else {
                    continue;
                };
                for sink in &netlist.signals[signal].sinks {
                    let reader = netlist.pins[*sink].instance;
                    if combinational(reader) && timed(*sink) && !placed[reader] {
                        waiting[reader] = waiting[reader].saturating_sub(1);
                        if waiting[reader] == 0 {
                            placed[reader] = true;
                            order.push(reader);
                        }
                    }
                }
            }
        }
        // A loop: take the first instance it holds up, and go on.
        match (0..netlist.instances.len()).find(|i| !placed[*i]) {
            Some(i) => {
                placed[i] = true;
                order.push(i);
            }
            None => break,
        }
    }
    // Arrival at every output pin.
    let mut arrival = vec![0u64; pins];
    let arrives =
        |arrival: &[u64], pin: usize| -> u64 { driver(pin).map_or(0, |d| arrival[d]) + wire[pin] };
    for instance in &order {
        if !combinational(*instance) {
            continue;
        }
        let pins = &netlist.instances[*instance].pins;
        let latest = pins
            .iter()
            .filter(|p| !netlist.pins[**p].output && timed(**p))
            .map(|p| arrives(&arrival, *p))
            .max()
            .unwrap_or(0)
            + u64::from(timing.cell_delay);
        for pin in pins {
            if netlist.pins[*pin].output {
                arrival[*pin] = latest;
            }
        }
    }
    let mut longest = 0;
    for (index, pin) in netlist.pins.iter().enumerate() {
        if !pin.output && timed(index) && !combinational(pin.instance) {
            longest = longest.max(arrives(&arrival, index));
        }
    }
    // Required time at every input pin, backwards.
    let mut required = vec![longest; pins];
    for instance in order.iter().rev() {
        if !combinational(*instance) {
            continue;
        }
        let pins = &netlist.instances[*instance].pins;
        let mut latest = longest;
        for pin in pins.iter().filter(|p| netlist.pins[**p].output) {
            let Some(signal) = netlist.pins[*pin].signal else {
                continue;
            };
            for sink in &netlist.signals[signal].sinks {
                if timed(*sink) {
                    latest = latest.min(required[*sink].saturating_sub(wire[*sink]));
                }
            }
        }
        let latest = latest.saturating_sub(u64::from(timing.cell_delay));
        for pin in pins.iter().filter(|p| !netlist.pins[**p].output) {
            required[*pin] = latest;
        }
    }
    let criticality = terminals
        .iter()
        .map(|terminals| {
            terminals
                .sinks
                .iter()
                .map(|sink| {
                    if longest == 0 || !timed(sink.pin) {
                        return 0.0;
                    }
                    let slack = required[sink.pin].saturating_sub(arrives(&arrival, sink.pin));
                    #[allow(clippy::cast_precision_loss, reason = "a ratio of delays")]
                    let ratio = slack as f64 / longest as f64;
                    (1.0 - ratio).clamp(0.0, timing.max_criticality)
                })
                .collect()
        })
        .collect();
    (criticality, longest)
}

/// How many pips forward [`contention`] looks for the cell pins a wire
/// serves. A control wire reaches its slice's pin in one or two.
const CONTROL_DEPTH: usize = 3;

/// Why an oversubscribed node could not be shared, when it is a wire the
/// cells of one tile have to agree about.
///
/// `oversubscribed node X24Y3/LSR1` is where this backend's distributed
/// RAM defect surfaced, and the message named a tile and not a cause.
/// `LSR1` is one of a logic tile's two set/reset wires; a
/// `TRELLIS_DPR16X4` joins its write enable to it with a `.fixed_conn`,
/// and a flip-flop of the same tile whose reset is a different net then
/// has nowhere to go. Nobody could have read that off the name.
///
/// Nothing here knows what an `LSR` is. What it asks is the question that
/// makes a wire a control wire on any family: **do several of this tile's
/// bels have a pin only reachable through it?** If two or more do, the
/// node is shared silicon rather than interconnect, and the answer says
/// which cells wanted it and which signals they wanted on it. Returns
/// `None` for an ordinary congested wire, where the honest answer is that
/// the design is too dense there and the placer is not at fault.
fn contention(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    routing: &Routing,
    node: NodeId,
) -> Option<String> {
    let tile = graph.wire(node).tile;
    // Everything this wire reaches without leaving the tile.
    let mut reached = vec![node];
    let mut from = 0usize;
    for _ in 0..CONTROL_DEPTH {
        let until = reached.len();
        for at in from..until {
            for pip in graph.outgoing(reached[at]) {
                let to = graph.pip(*pip).to;
                if graph.wire(to).tile == tile && !reached.contains(&to) {
                    reached.push(to);
                }
            }
        }
        from = until;
    }
    // The bel pins among them, and the cells sitting on those bels.
    let mut pins: Vec<(usize, &str)> = Vec::new();
    for (index, site) in graph.sites.iter().enumerate() {
        if site.tile != tile {
            continue;
        }
        for (role, wire) in &site.pins {
            if reached.contains(wire) && !pins.iter().any(|(s, r)| *s == index && *r == role) {
                pins.push((index, role.as_str()));
            }
        }
    }
    let bels = pins
        .iter()
        .map(|(site, _)| *site)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    if bels < 2 {
        return None;
    }
    // And which signal each of the cells on them asked for.
    let mut wanted: Vec<String> = Vec::new();
    for (signal, route) in routing.routes.iter().enumerate() {
        if route.as_ref().is_none_or(|r| !r.nodes.contains(&node)) {
            continue;
        }
        let name = &netlist.signals[signal].name;
        let by = pins.iter().find_map(|(site, role)| {
            let instance = placement.instance_at(*site)?;
            netlist
                .pins
                .iter()
                .find(|pin| {
                    pin.instance == instance && pin.role == *role && pin.signal == Some(signal)
                })
                .map(|pin| {
                    format!(
                        "`{}`'s `{}` pin",
                        netlist.instances[instance].name, pin.role
                    )
                })
        });
        wanted.push(match by {
            Some(by) => format!("`{name}` for {by}"),
            None => format!("`{name}`"),
        });
    }
    if wanted.len() < 2 {
        return None;
    }
    Some(format!(
        "  {} is a control wire of X{}Y{}: {bels} of that tile's bels have a pin that can only be \
         reached through it, and {} signals were routed onto it — {}. A wire carries one signal, \
         so there are two ways out and a placement has to take one of them: give the cells that \
         share the tile the **same** signal on that pin, or put the ones that disagree in \
         **different tiles**. The placer rejects this arrangement before it is made \
         (`SiteRules` in `src/fpga/place.rs`), so a design that gets here has found a control \
         wire the architecture does not describe yet — say which one, it is a fabric fact and \
         not a budget.",
        graph.wire(node).full_name(),
        tile.0,
        tile.1,
        wanted.len(),
        wanted.join(" and "),
    ))
}

/// The graph node one pin reaches, given where its instance was placed.
///
/// A role naming several wires gives the first of them, which is what a
/// *driver* is: a cell output drives one wire, and a bel that offered two
/// would be two nets. Sinks go through [`nodes_of`].
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

/// Every graph node one pin reaches, given where its instance was placed.
fn nodes_of(
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    pin: usize,
) -> Result<Vec<NodeId>, RouteError> {
    let pin = &netlist.pins[pin];
    let instance = &netlist.instances[pin.instance];
    let Some(site) = placement.site_of(pin.instance) else {
        return Err(RouteError::Unplaced {
            instance: instance.name.clone(),
        });
    };
    let nodes: Vec<NodeId> = graph.sites[site].pin_nodes(&pin.role).collect();
    if nodes.is_empty() {
        return Err(RouteError::NoNode {
            instance: instance.name.clone(),
            role: pin.role.clone(),
        });
    }
    Ok(nodes)
}

/// Takes a signal's route out of the occupancy counts.
fn rip_up(state: &mut State, routing: &mut Routing, signal: usize) {
    if let Some(route) = routing.routes[signal].take() {
        for root in state.roots(&route.nodes) {
            let slot = &mut state.occupancy[root as usize];
            *slot = slot.saturating_sub(1);
        }
    }
}

/// Each graph node's **physical** node, as the smallest graph node id of
/// the metal it is part of.
///
/// A graph node is a wire *name in one tile*. On a 7-series part one piece
/// of metal has a different name in every tile it crosses, and the loader
/// joins those names with a free pip **each way** (`xray`'s module header,
/// mismatch 1), because the architecture has no other way to say it. To
/// the router those were separate nodes, each with a capacity of one
/// signal. For a wire driven only from its start that cost nothing, since
/// a second signal had no way on. **A long line is driven from either
/// end**, and the router put one signal on `LV0` and another on `LV18` of
/// the same `LV` -- 38 such pairs in one build of
/// `examples/basys3/iso_display.v`, every route individually valid, and on
/// the part the two signals shorted. One of them was the enable that
/// loads the terminal's output byte, so `8` printed as `0`.
///
/// So two nodes are one physical node when a free pip runs between them
/// **in both directions**: that is exactly what a join is. A fixed buffer
/// (`always` in a `ppips` file) or a site's own wiring is one-way and is
/// not merged -- its two ends really are two wires. On a family whose
/// architecture spells wires by span, and so declares no joins, every
/// node is its own physical node and this changes nothing.
pub fn physical_nodes(graph: &RoutingGraph) -> Vec<NodeId> {
    let n = graph.nodes.len();
    let mut parent: Vec<NodeId> = (0..n).map(|i| NodeId::try_from(i).unwrap_or(0)).collect();
    fn find(parent: &mut [NodeId], mut x: NodeId) -> NodeId {
        while parent[x as usize] != x {
            let up = parent[parent[x as usize] as usize];
            parent[x as usize] = up;
            x = up;
        }
        x
    }
    for a in 0..n {
        let a = NodeId::try_from(a).unwrap_or(0);
        for pip in graph.outgoing(a) {
            if !graph.pip_bits(*pip).is_empty() {
                continue;
            }
            let b = graph.pip(*pip).to;
            if b == a {
                continue;
            }
            let back = graph
                .outgoing(b)
                .iter()
                .any(|p| graph.pip(*p).to == a && graph.pip_bits(*p).is_empty());
            if back {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                    parent[hi as usize] = lo;
                }
            }
        }
    }
    (0..n)
        .map(|i| find(&mut parent, NodeId::try_from(i).unwrap_or(0)))
        .collect()
}

/// Where one signal starts and where it has to get to, resolved onto the
/// graph once and reused by every iteration.
struct Terminals {
    /// The signal, as an index into [`Netlist::signals`].
    signal: usize,
    /// The node its driver reaches.
    source: NodeId,
    /// The nodes its sinks reach.
    sinks: Vec<Sink>,
}

/// One node a signal has to reach.
struct Sink {
    node: NodeId,
    /// `cell.port[bit]`, for diagnostics.
    name: String,
    /// The pin, as an index into [`Netlist::pins`]; one pin may reach
    /// several nodes.
    pin: usize,
}

/// Routes one signal onto a tree that starts at its source.
#[allow(clippy::too_many_arguments, reason = "the expansion's whole state")]
fn route_one(
    graph: &RoutingGraph,
    geometry: &[Geometry],
    edges: (&[u32], &[NodeId]),
    state: &State,
    scratch: &mut Scratch,
    terminals: &Terminals,
    criticality: &[f64],
    present: f64,
    options: &RouteOptions,
    netlist: &Netlist,
) -> Result<Route, RouteError> {
    let source = terminals.source;
    let signal = terminals.signal;
    // Each tree node's delay from the source, in cost units, which is
    // where a critical sink joining the tree there starts.
    let mut tree: BTreeMap<NodeId, f64> = BTreeMap::new();
    tree.insert(source, 0.0);
    let mut pips: Vec<PipId> = Vec::new();
    let barred = !options
        .network_signals
        .get(signal)
        .copied()
        .unwrap_or(options.network_signals.is_empty());
    // The most critical sinks first, so that they have the tree's choice
    // of trunk; with no timing every criticality is zero and the order is
    // the netlist's.
    let mut order: Vec<usize> = (0..terminals.sinks.len()).collect();
    order.sort_by(|a, b| {
        let (a, b) = (
            criticality.get(*a).copied().unwrap_or(0.0),
            criticality.get(*b).copied().unwrap_or(0.0),
        );
        b.total_cmp(&a)
    });
    for which in order {
        let sink = &terminals.sinks[which];
        let critical = criticality.get(which).copied().unwrap_or(0.0);
        let Some(path) = maze(
            graph, geometry, edges, state, scratch, &tree, sink.node, critical, present, options,
            barred,
        ) else {
            return Err(RouteError::Unroutable {
                signal: netlist.signals[signal].name.clone(),
                sink: sink.name.clone(),
            });
        };
        for pip in path {
            let pip_ = graph.pip(pip);
            let delay = tree.get(&pip_.from).copied().unwrap_or(0.0)
                + f64::from(geometry[pip_.to as usize].delay);
            tree.insert(pip_.to, delay);
            pips.push(pip);
        }
    }
    Ok(Route {
        signal,
        source,
        pips,
        nodes: tree.into_keys().collect(),
    })
}

/// The per-expansion arrays, reused across the sinks of one signal.
struct Scratch {
    cost: Vec<f64>,
    /// The cost plus the distance estimate, so that a stale queue entry is
    /// recognised by one comparison rather than by estimating the distance
    /// again. The expansion pops half a billion nodes for one design here.
    estimate: Vec<f64>,
    from: Vec<PipId>,
    stamp: Vec<u32>,
    generation: u32,
    /// Nodes taken off the queue, for [`RoutingReport::visited`].
    visited: u64,
    /// Nodes put on it, for [`RoutingReport::queued`].
    queued: u64,
}

impl Scratch {
    fn new(nodes: usize) -> Self {
        Scratch {
            cost: vec![0.0; nodes],
            estimate: vec![0.0; nodes],
            from: vec![0; nodes],
            stamp: vec![0; nodes],
            generation: 0,
            visited: 0,
            queued: 0,
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

    /// The estimate this node was last queued under, or infinity.
    fn estimate_of(&self, node: NodeId) -> f64 {
        if self.seen(node) {
            self.estimate[node as usize]
        } else {
            f64::INFINITY
        }
    }

    fn set(&mut self, node: NodeId, cost: f64, estimate: f64, from: PipId) {
        let index = node as usize;
        self.cost[index] = cost;
        self.estimate[index] = estimate;
        self.from[index] = from;
        self.stamp[index] = self.generation;
    }
}

/// A\* from every node of `tree` to `sink`, returning the pips of the
/// path from the tree to the sink, nearest the tree first.
#[allow(clippy::too_many_arguments, reason = "the expansion's whole state")]
fn maze(
    graph: &RoutingGraph,
    geometry: &[Geometry],
    edges: (&[u32], &[NodeId]),
    state: &State,
    scratch: &mut Scratch,
    tree: &BTreeMap<NodeId, f64>,
    sink: NodeId,
    critical: f64,
    present: f64,
    options: &RouteOptions,
    barred: bool,
) -> Option<Vec<PipId>> {
    let slack = 1.0 - critical;
    let target = geometry[sink as usize].tile;
    // The per-tile charge is scaled by the node's own base cost, because
    // the estimate has to stay below what the rest of the journey really
    // costs and on a fabric with a cheap wire class it would not. A clock
    // network node costs a twentieth of an ordinary wire
    // (`RouteOptions::node_base`), so charging a full tile of distance for
    // standing on one is twenty times too much — enough to make the search
    // walk past the network and build a clock tree out of data wires, which
    // is exactly what it did before this line. With a uniform base this is
    // what it always was.
    let heuristic = |node: NodeId| -> f64 {
        let geometry = &geometry[node as usize];
        let (x, y) = geometry.nearest_tile(target.0, target.1);
        let distance = u64::from(x.abs_diff(target.0)) + u64::from(y.abs_diff(target.1));
        options.astar_weight
            * distance as f64
            * (slack * f64::from(geometry.base) + critical * f64::from(geometry.delay))
    };

    scratch.start();
    let mut heap: BinaryHeap<std::cmp::Reverse<(u64, NodeId)>> = BinaryHeap::new();
    for (node, delay) in tree {
        let start = critical * delay;
        let estimate = start + heuristic(*node);
        scratch.set(*node, start, estimate, PipId::MAX);
        heap.push(std::cmp::Reverse((score(estimate), *node)));
    }
    let (reach_start, reach) = edges;
    let mut reached = false;
    while let Some(std::cmp::Reverse((estimate, node))) = heap.pop() {
        scratch.visited += 1;
        if estimate > score(scratch.estimate_of(node)) {
            continue;
        }
        if node == sink {
            reached = true;
            break;
        }
        let here = scratch.cost_of(node);
        let run = reach_start[node as usize] as usize..reach_start[node as usize + 1] as usize;
        for (pip, next) in graph.outgoing(node).iter().zip(&reach[run]) {
            if barred && geometry[*next as usize].reserved {
                continue;
            }
            let at = &geometry[*next as usize];
            let step = if critical > 0.0 {
                slack * state.node_cost(*next, at.base, present) + critical * f64::from(at.delay)
            } else {
                state.node_cost(*next, at.base, present)
            };
            let candidate = here + step;
            if candidate < scratch.cost_of(*next) {
                let estimate = candidate + heuristic(*next);
                scratch.set(*next, candidate, estimate, *pip);
                scratch.queued += 1;
                heap.push(std::cmp::Reverse((score(estimate), *next)));
            }
        }
    }
    if !reached {
        return None;
    }
    let mut path = Vec::new();
    let mut node = sink;
    while !tree.contains_key(&node) {
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
    use std::collections::BTreeSet;

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

    /// Two tiles whose `long` wires are **one piece of metal**, joined the
    /// way the 7-series loader joins a long line -- a free pip each way --
    /// and each with a `short` wire of its own. Each tile has a source and
    /// a sink that can use either.
    fn joined_long(base_long: f32) -> (RoutingGraph, Vec<f32>) {
        joined_long_across(2, base_long)
    }

    /// [`joined_long`] over `tiles` tiles in a row, all of whose `long`
    /// wires are one piece of metal.
    fn joined_long_across(tiles: u32, base_long: f32) -> (RoutingGraph, Vec<f32>) {
        let mut arch = Arch::new("joined", "test", tiles, 1);
        let mut tile = TileType::new("t", "t_tile", 8, 8);
        for name in ["long", "short", "out0", "in0"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        let mut src = BelDecl::new("src0", "lut");
        src.pins.push(("o".to_owned(), WireRef::local("out0")));
        tile.bels.push(src);
        let mut dst = BelDecl::new("dst0", "ff");
        dst.pins.push(("d".to_owned(), WireRef::local("in0")));
        tile.bels.push(dst);
        let mut bit = 0u32;
        let mut next = || {
            let at = ConfigBit::new(bit / 8, bit % 8);
            bit += 1;
            vec![at]
        };
        for track in ["long", "short"] {
            tile.pips.push(PipDecl {
                from: WireRef::local("out0"),
                to: WireRef::local(track),
                bits: next(),
            });
            tile.pips.push(PipDecl {
                from: WireRef::local(track),
                to: WireRef::local("in0"),
                bits: next(),
            });
        }
        for dx in [-1, 1] {
            tile.pips.push(PipDecl {
                from: WireRef::at("long", dx, 0),
                to: WireRef::local("long"),
                bits: Vec::new(),
            });
        }
        arch.tile_types.push(tile);
        for x in 0..tiles {
            arch.set_tile(x, 0, 0);
        }
        let graph = arch.build_graph();
        let base = graph
            .nodes
            .iter()
            .map(|w| if w.name == "long" { base_long } else { 1.0 })
            .collect();
        (graph, base)
    }

    /// Two signals may not share a physical wire just because it has a
    /// different name in each tile. Before [`physical_nodes`] the router
    /// counted capacity per name, and on a 7-series part it put two
    /// signals on the two ends of one long line, which the part shorted.
    ///
    /// What it would catch: capacity kept per graph node again, or joins
    /// not recognised as joins. What it would not catch: a family whose
    /// loader joins wires some other way than a free pip each way.
    /// [`RouteOptions::reroute_congested_only`] judges a signal by its
    /// physical nodes. Two signals in the second and third tiles of a
    /// three-tile long wire each take their own tile's piece of it, and
    /// neither piece is the one the wire is counted under (the first
    /// tile's). Read by graph node, as the option first did after physical
    /// nodes arrived, both looked uncontended and were never rerouted: a
    /// network of 6 989 tables on the ZCU104 kept 43 nodes oversubscribed
    /// through 100 iterations. With two tiles this would not have caught
    /// it, because one signal always holds the counted piece.
    #[test]
    fn rerouting_only_the_congested_reads_physical_nodes() {
        let (graph, base) = joined_long_across(3, 0.1);
        let netlist = pairs(2);
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, site) in ["X1Y0/src0", "X1Y0/dst0", "X2Y0/src0", "X2Y0/dst0"]
            .iter()
            .enumerate()
        {
            placement.place(index, graph.site_index(site).expect("site"));
        }
        let options = RouteOptions {
            node_base: base,
            reroute_congested_only: true,
            ..RouteOptions::default()
        };
        let (routing, report) = route(&netlist, &graph, &placement, &options).expect("it routes");
        assert!(report.iterations.len() > 1, "{}", report.to_text());
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        assert!(routing.shorts(&netlist, &graph).is_empty());
    }

    #[test]
    fn two_signals_never_share_one_piece_of_metal() {
        let (graph, base) = joined_long(0.1);
        let groups = physical_nodes(&graph);
        let node = |name: &str| -> NodeId {
            let index = (0..graph.nodes.len())
                .find(|i| graph.wire(NodeId::try_from(*i).unwrap_or(0)).full_name() == name)
                .unwrap_or_else(|| panic!("no node {name}"));
            NodeId::try_from(index).unwrap_or(0)
        };
        let (long0, long1, short0) = (node("X0Y0/long"), node("X1Y0/long"), node("X0Y0/short"));
        assert_eq!(
            groups[long0 as usize], groups[long1 as usize],
            "a join is one node"
        );
        assert_ne!(groups[long0 as usize], groups[short0 as usize]);

        let netlist = pairs(2);
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, site) in ["X0Y0/src0", "X0Y0/dst0", "X1Y0/src0", "X1Y0/dst0"]
            .iter()
            .enumerate()
        {
            placement.place(index, graph.site_index(site).expect("site"));
        }
        let options = RouteOptions {
            node_base: base,
            ..RouteOptions::default()
        };
        let (routing, _) = route(&netlist, &graph, &placement, &options).expect("it routes");
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        assert!(
            routing.shorts(&netlist, &graph).is_empty(),
            "{:?}",
            routing.shorts(&netlist, &graph)
        );
        let on_long = routing
            .routes()
            .filter(|r| {
                r.nodes
                    .iter()
                    .any(|n| groups[*n as usize] == groups[long0 as usize])
            })
            .count();
        assert_eq!(on_long, 1, "exactly one signal takes the long wire");
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

    /// [`RouteOptions::node_base`]: a class of wire made cheap is taken
    /// although the path through it is longer.
    ///
    /// The detour fabric is the smallest thing that can show it. One signal,
    /// nothing in its way, and two ways across: the main row, which is the
    /// shortest, and the row above it, which costs two extra hops. With a
    /// uniform base the router takes the short way; with the second row at a
    /// twentieth of the cost it takes the long one.
    ///
    /// **This pins the cost half of the knob and not the heuristic half.**
    /// It was checked: with the `state.base_of(node)` taken out of
    /// [`maze`]'s heuristic this test still passes, because eight tiles is
    /// too short a detour for the over-estimate to matter. The heuristic
    /// half was needed on a real fabric and its evidence is a measurement
    /// there rather than a synthetic case here — seven of a counter's
    /// twenty-six flip-flops on a Lattice ECP5 took a data wire to their
    /// clock pin with the base alone in place, because the search had
    /// already reached the sink by another route before the cheap class was
    /// expanded. See
    /// `super::trellis::TrellisFabric::clock_node_costs`.
    /// With [`RouteOptions::reroute_congested_only`], a signal nothing
    /// contends with is routed once and left alone, and the two that
    /// collide still negotiate. Two signals collide on one track as in
    /// `congestion_is_negotiated_away`, and a third, far from them, is
    /// alone. It would catch the option skipping the colliding signals
    /// (they would never converge) or doing nothing (the expansion would
    /// queue as much as without it); it would not catch a congested
    /// signal skipped in one iteration and taken up in the next.
    #[test]
    fn rerouting_only_congested_signals_leaves_the_others_and_still_converges() {
        let (_, graph) = detour(12);
        let netlist = pairs(3);
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, (bel, tile)) in [
            ("src0", 0),
            ("dst0", 7),
            ("src1", 1),
            ("dst1", 6),
            ("src0", 9),
            ("dst0", 11),
        ]
        .into_iter()
        .enumerate()
        {
            let site = graph
                .site_index(&format!("X{tile}Y0/{bel}"))
                .expect("the bel exists");
            placement.place(index, site);
        }
        let all = RouteOptions {
            present_factor: 0.05,
            present_growth: 3.0,
            ..RouteOptions::default()
        };
        let some = RouteOptions {
            reroute_congested_only: true,
            ..all.clone()
        };
        let (every, every_report) = route(&netlist, &graph, &placement, &all).unwrap();
        let (only, only_report) = route(&netlist, &graph, &placement, &some).unwrap();
        assert!(
            only_report.iterations.len() > 1,
            "{}",
            only_report.to_text()
        );
        assert_eq!(only_report.iterations.last().unwrap().overused_nodes, 0);
        assert!(only.verify(&netlist, &graph, &placement).is_empty());
        let alone = netlist.routable()[2];
        assert_eq!(every.route(alone), only.route(alone));
        assert!(
            only_report.queued < every_report.queued,
            "{} against {}",
            only_report.queued,
            every_report.queued
        );
    }

    #[test]
    fn a_cheap_class_of_wire_is_taken_although_the_path_is_longer() {
        let (_, graph) = detour(8);
        let netlist = pairs(1);
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, (bel, tile)) in [("src0", 0), ("dst0", 7)].into_iter().enumerate() {
            let site = graph
                .site_index(&format!("X{tile}Y0/{bel}"))
                .expect("the bel exists");
            placement.place(index, site);
        }
        let on_second_row = |routing: &Routing| {
            routing
                .routes()
                .any(|r| r.nodes.iter().any(|n| graph.wire(*n).tile.1 == 1))
        };

        let (plain, _) = route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
        assert!(
            !on_second_row(&plain),
            "with nothing in the way the short path is the one to take"
        );

        let mut base = vec![1.0f32; graph.nodes.len()];
        for (index, wire) in graph.nodes.iter().enumerate() {
            if wire.tile.1 == 1 {
                base[index] = 0.05;
            }
        }
        let options = RouteOptions {
            node_base: base,
            ..RouteOptions::default()
        };
        let (steered, report) = route(&netlist, &graph, &placement, &options).unwrap();
        assert!(on_second_row(&steered), "the preference was not followed");
        assert!(
            steered.pips(0).len() > plain.pips(0).len(),
            "the cheap path should be the longer one: {} against {}",
            steered.pips(0).len(),
            plain.pips(0).len()
        );
        // And it is still a route: a preference changes which wires are
        // chosen and nothing about whether they join up.
        assert!(steered.verify(&netlist, &graph, &placement).is_empty());
        assert_eq!(report.iterations.last().unwrap().overused_nodes, 0);

        // A vector of the wrong length is ignored rather than trusted, so a
        // caller that builds one against a stale graph gets the old
        // behaviour instead of a panic or a silent mis-scaling.
        let options = RouteOptions {
            node_base: vec![0.05; graph.nodes.len() - 1],
            ..RouteOptions::default()
        };
        let (ignored, _) = route(&netlist, &graph, &placement, &options).unwrap();
        assert!(!on_second_row(&ignored));
    }

    /// [`Timing`]: a critical connection leaves a cheap long path for the
    /// fast short one. This is `a_cheap_class_of_wire_is_taken_although_the_path_is_longer`
    /// with the router told about delay: the second row is cheap, so the
    /// first pass takes it, and the one connection there is is the longest
    /// path, so the analysis makes it as critical as anything can be and
    /// the next pass weighs its delay and takes the main row.
    ///
    /// It pins the reported longest path too, which is the analysis' sum:
    /// one unit for the lookup table and one for each node after the
    /// source. It would catch the criticality never reaching the cost (the
    /// route would stay on the second row), the delay never reaching the
    /// sum, or the router stopping before the analysis had been used. It
    /// would not catch a wrong criticality of an uncritical connection:
    /// there is only one connection, and `the_analysis_finds_the_longest_path_and_who_is_on_it`
    /// is the test of that.
    #[test]
    fn a_critical_connection_takes_the_fast_path_over_the_cheap_one() {
        let (_, graph) = detour(8);
        let netlist = pairs(1);
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, (bel, tile)) in [("src0", 0), ("dst0", 7)].into_iter().enumerate() {
            let site = graph
                .site_index(&format!("X{tile}Y0/{bel}"))
                .expect("the bel exists");
            placement.place(index, site);
        }
        let on_second_row = |routing: &Routing| {
            routing
                .routes()
                .any(|r| r.nodes.iter().any(|n| graph.wire(*n).tile.1 == 1))
        };
        let mut base = vec![1.0f32; graph.nodes.len()];
        for (index, wire) in graph.nodes.iter().enumerate() {
            if wire.tile.1 == 1 {
                base[index] = 0.05;
            }
        }
        let options = RouteOptions {
            node_base: base,
            timing: Some(Timing::for_netlist(&netlist, vec![1; graph.nodes.len()], 1)),
            ..RouteOptions::default()
        };
        let (routing, report) = route(&netlist, &graph, &placement, &options).unwrap();
        assert!(!on_second_row(&routing), "{}", report.to_text());
        assert!(routing.verify(&netlist, &graph, &placement).is_empty());
        let critical: Vec<u64> = report
            .iterations
            .iter()
            .map(|i| i.critical.unwrap())
            .collect();
        // The cheap way: up, along nine tracks, down and in, and the table.
        // The fast way: eight tracks and in, and the table.
        let fast = u64::try_from(routing.pips(0).len()).unwrap() + 1;
        assert_eq!(fast, 10);
        assert!(critical[0] > fast, "{critical:?}");
        assert_eq!(*critical.last().unwrap(), fast, "{critical:?}");
        assert_eq!(report.iterations.len(), 4, "{}", report.to_text());
        assert!(report.to_text().contains("longest path 10"));
    }

    /// The analysis on its own: a register drives two tables in a row into
    /// another register, and straight into a third. With no wires routed,
    /// the longest path is the two tables, every connection on it is as
    /// critical as the cap allows, and the direct one has all the slack
    /// there is. A loop of two tables is broken rather than looped on, and
    /// a clock pin is left out: a clock's delay is skew, not a path. It
    /// would catch arrival or required times summed
    /// the wrong way, a missing cell delay, or a criticality that does not
    /// fall with slack; it would not catch a wrong wire delay, which the
    /// routed test above pins.
    #[test]
    fn the_analysis_finds_the_longest_path_and_who_is_on_it() {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: Vec::new(),
            off_fabric: Vec::new(),
        };
        let mut add = |name: &str, kind: &str, pins: &[(&str, bool, usize)]| {
            let instance = netlist.instances.len();
            let mut ids = Vec::new();
            for (role, output, signal) in pins {
                ids.push(netlist.pins.len());
                netlist.pins.push(NetPin {
                    instance,
                    port: role.to_uppercase(),
                    bit: 0,
                    role: (*role).to_owned(),
                    output: *output,
                    signal: Some(*signal),
                    constant: None,
                });
            }
            netlist.instances.push(Instance {
                cell: CellId::from_index(instance),
                name: name.to_owned(),
                primitive: kind.to_uppercase(),
                kind: kind.to_owned(),
                pins: ids,
                pin: None,
            });
        };
        // Signals: 0 a.q, 1 t1.o, 2 t2.o, 3 the clock, 4 and 5 the loop.
        add("a", "ff", &[("q", true, 0), ("clk", false, 3)]);
        add(
            "t1",
            "lut",
            &[("i0", false, 0), ("i1", false, 3), ("o", true, 1)],
        );
        add("t2", "lut", &[("i0", false, 1), ("o", true, 2)]);
        add("b", "ff", &[("d", false, 2), ("clk", false, 3)]);
        add("c", "ff", &[("d", false, 0), ("clk", false, 3)]);
        add("pad", "io", &[("o", true, 3)]);
        add("l1", "lut", &[("i0", false, 5), ("o", true, 4)]);
        add("l2", "lut", &[("i0", false, 4), ("o", true, 5)]);
        for signal in 0..6 {
            netlist.signals.push(Signal {
                name: format!("s{signal}"),
                driver: netlist
                    .pins
                    .iter()
                    .position(|p| p.output && p.signal == Some(signal)),
                sinks: (0..netlist.pins.len())
                    .filter(|p| !netlist.pins[*p].output && netlist.pins[*p].signal == Some(signal))
                    .collect(),
            });
        }
        let (_, graph) = line(1, 1);
        let terminals: Vec<Terminals> = (0..6)
            .map(|signal| Terminals {
                signal,
                source: 0,
                sinks: netlist.signals[signal]
                    .sinks
                    .iter()
                    .map(|pin| Sink {
                        node: 0,
                        name: String::new(),
                        pin: *pin,
                    })
                    .collect(),
            })
            .collect();
        let routing = Routing::new(netlist.signals.len());
        let mut timing = Timing::for_netlist(&netlist, Vec::new(), 10);
        timing.max_criticality = 0.9;
        let (criticality, longest) = analyse(&netlist, &graph, &terminals, &routing, &timing);
        assert_eq!(longest, 20);
        // a.q reaches t1 (on the path) and c (twenty units of slack).
        assert_eq!(criticality[0], vec![0.9, 0.0]);
        assert_eq!(criticality[1], vec![0.9]);
        assert_eq!(criticality[2], vec![0.9]);
        // The clock reaches a, t1, b and c. Its clock pins are left out;
        // t1 takes it on a data pin, where it is data, and starts as early
        // as a.q does.
        assert_eq!(criticality[3], vec![0.0, 0.9, 0.0, 0.0]);
        assert_eq!(criticality[4].len(), 1);
        assert_eq!(criticality[5].len(), 1);
    }

    /// [`RouteOptions::network`] closes the cheap class to a signal
    /// [`RouteOptions::network_signals`] does not allow, and leaves it open
    /// to one it does. This is the half a 7-series clock network needs and
    /// an ECP5's does not: there, data signals can reach the clock wires,
    /// and making them cheap drew a flip-flop's output across 1322 pips of
    /// them.
    ///
    /// What it would catch: the barrier ignored, applied to the wrong
    /// signals, or applied to an allowed one. What it would not catch: the
    /// wrong set of nodes being marked on a real fabric, which is
    /// `xray::is_clock_wire`'s test and a real build's business.
    #[test]
    fn a_barred_signal_keeps_off_the_network_and_an_allowed_one_takes_it() {
        let (_, graph) = detour(8);
        let netlist = pairs(1);
        let mut placement = Placement::new(netlist.instances.len(), graph.sites.len());
        for (index, (bel, tile)) in [("src0", 0), ("dst0", 7)].into_iter().enumerate() {
            let site = graph
                .site_index(&format!("X{tile}Y0/{bel}"))
                .expect("the bel exists");
            placement.place(index, site);
        }
        let on_second_row = |routing: &Routing| {
            routing
                .routes()
                .any(|r| r.nodes.iter().any(|n| graph.wire(*n).tile.1 == 1))
        };
        let network: Vec<bool> = graph.nodes.iter().map(|wire| wire.tile.1 == 1).collect();
        let base: Vec<f32> = network
            .iter()
            .map(|on| if *on { 0.05 } else { 1.0 })
            .collect();
        let signal = netlist
            .signals
            .iter()
            .position(Signal::is_routable)
            .expect("one signal to route");

        let mut allowed = vec![false; netlist.signals.len()];
        allowed[signal] = true;
        let options = RouteOptions {
            node_base: base.clone(),
            network: network.clone(),
            network_signals: allowed,
            ..RouteOptions::default()
        };
        let (taken, _) = route(&netlist, &graph, &placement, &options).unwrap();
        assert!(
            on_second_row(&taken),
            "an allowed signal should take the cheap network"
        );

        let options = RouteOptions {
            node_base: base,
            network,
            network_signals: vec![false; netlist.signals.len()],
            ..RouteOptions::default()
        };
        let (kept_off, _) = route(&netlist, &graph, &placement, &options).unwrap();
        assert!(
            !on_second_row(&kept_off),
            "a barred signal went onto the network however cheap it was"
        );
        assert!(kept_off.verify(&netlist, &graph, &placement).is_empty());
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
