//! The cells of a tile given their bels again, for time, after a route.
//!
//! The anneal places a cell on a site, and a site is a bel: which of a tile's
//! eight lookup tables, which of its sixteen flip-flops. Its cost is a box in
//! tiles, and every bel of a tile is the same tile to it, so which of them two
//! connected cells sit on is whatever legalisation and the last accepted swap
//! left. On a 7-series tile it is not nothing. Each slice output reaches six
//! `IMUX` wires of its interconnect tile by one pip (prjxray's
//! `tile_type_INT_L.json`), and the rest by a bounce; the fastest way from an
//! output to the best input of a table in its own tile is 98 ps for some pairs
//! of bels and 449 for others under the fitted model
//! (docs/fpga-vivado-parity.md, "The placement's share, measured again").
//!
//! So once a placement is routed, [`detail`] times it with each connection
//! between two tiles at its routed delay and each inside one at what the
//! routed estimate would charge its best way ([`BelDelays::node_delay`],
//! searched over the tile's own wires), each table's inputs given its pins as
//! the router's matching would give them, the latest first; and then, tile by
//! tile, moves each cell on a late path to another bel of its kind in its
//! tile, or swaps it with the cell there, when that makes the tile's
//! connections cheaper by their criticality. Only moves the site rules allow
//! are made ([`SiteRules::allows`]), and only of cells no constraint or
//! dedicated wiring holds. A round of moves is followed by a new analysis, and
//! if the last leaves the latest paths later, as this times them, the
//! placement is put back.
//!
//! **What it is worth is measured, not this module's estimate.** The pass
//! promises about three times what the route made of it, and a second route
//! of a placement given random bels instead is faster than the first about as
//! often: docs/fpga-vivado-parity.md, "A tile's bels", has both.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::fmt;
use std::sync::Arc;

use super::{
    Netlist, PinTiming, PlaceTiming, Placeable, Placement, SiteRules, TileSites, path_slack_against,
};
use crate::fpga::arch::{FastMap, NodeId, RoutingGraph};

/// What [`refine_bels`](super::refine_bels) needs beyond the placement's own
/// timing: the routed estimate's delay of each node, and which of a lookup
/// table's inputs the router may move.
#[derive(Clone, Default, PartialEq)]
pub struct BelDelays {
    /// Each node's delay in picoseconds, as the router's estimate sums it
    /// (`route::Timing::node_delay`).
    pub node_delay: Arc<[u32]>,
    /// Per instance, the inputs of a lookup table the router may end on
    /// another of the table's pins, bit `k` for `i<k>`
    /// (`xray::lut_movable_inputs`); zero, or past its end, for none.
    pub movable: Vec<u8>,
}

impl fmt::Debug for BelDelays {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BelDelays")
            .field("nodes", &self.node_delay.len())
            .field("movable", &self.movable.len())
            .finish()
    }
}

/// What [`refine_bels`](super::refine_bels) did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BelReport {
    /// Moves made, a swap counting once; zero when they were put back.
    pub moves: usize,
    /// Per domain, the latest path before and after, in picoseconds, with
    /// each connection inside a tile at its best way.
    pub before_ps: Vec<u64>,
    /// See [`BelReport::before_ps`].
    pub after_ps: Vec<u64>,
    /// Whether the moves were put back, the latest paths later after them.
    pub undone: bool,
}

/// How many rounds of moves and analysis [`detail`] makes at most.
const ROUNDS: u32 = 3;

/// The power of a connection's criticality its delay is weighed by: VPR's
/// timing-driven placer's eight, so that the latest paths' connections
/// outweigh the merely late ones'.
const EXPONENT: i32 = 8;

/// The criticality below which a connection moves neither of its cells.
const FLOOR: f64 = 0.5;

/// How far, in picoseconds, the search for a way inside a tile goes.
const REACH_PS: u32 = 1500;

/// How many nodes the search for a way inside a tile settles at most.
const REACH_NODES: usize = 2000;

/// The ways inside a tile from a bel's output, searched once each.
struct Reach<'a> {
    graph: &'a RoutingGraph,
    delay: &'a [u32],
    from: FastMap<NodeId, FastMap<NodeId, u32>>,
}

impl Reach<'_> {
    /// The least delay from `from` to `to` over the wires of `from`'s tile
    /// and the two columns either side on its row (a 7-series tile's
    /// interconnect is the next column), none of them reaching another tile:
    /// the sum of the nodes' delays after `from`, as the routed estimate
    /// sums them. `None` when no such way is found.
    fn get(&mut self, from: NodeId, to: NodeId) -> Option<u32> {
        if !self.from.contains_key(&from) {
            let found = self.search(from);
            self.from.insert(from, found);
        }
        self.from.get(&from).and_then(|m| m.get(&to).copied())
    }

    fn search(&self, from: NodeId) -> FastMap<NodeId, u32> {
        let origin = self.graph.wire(from).tile;
        let mut best: FastMap<NodeId, u32> = FastMap::default();
        let mut heap = BinaryHeap::new();
        best.insert(from, 0);
        heap.push(Reverse((0u32, from)));
        let mut settled = 0usize;
        while let Some(Reverse((at, node))) = heap.pop() {
            if best.get(&node).is_some_and(|b| *b < at) {
                continue;
            }
            settled += 1;
            if settled > REACH_NODES {
                break;
            }
            for pip in self.graph.outgoing(node) {
                let to = self.graph.pip(*pip).to;
                let wire = self.graph.wire(to);
                if wire.global
                    || wire.span != (0, 0)
                    || wire.tile.1 != origin.1
                    || wire.tile.0.abs_diff(origin.0) > 2
                {
                    continue;
                }
                let next = at.saturating_add(self.delay.get(to as usize).copied().unwrap_or(0));
                if next > REACH_PS {
                    continue;
                }
                if best.get(&to).is_none_or(|b| next < *b) {
                    best.insert(to, next);
                    heap.push(Reverse((next, to)));
                }
            }
        }
        best
    }
}

/// The pass's view of a placement: what a connection inside a tile costs.
struct Detail<'a> {
    netlist: &'a Netlist,
    graph: &'a RoutingGraph,
    delays: &'a BelDelays,
    reach: Reach<'a>,
    /// Per site, what a register there launches after the earliest, in ns.
    launch_over: Vec<f64>,
    /// What a connection inside a tile no way was found for is charged: the
    /// placement's line at one tile.
    fallback: f64,
    /// Each sink pin's routed delay: what a connection between two tiles is
    /// taken to cost.
    routed: &'a [f64],
}

impl Detail<'_> {
    fn site_of(&self, placement: &Placement, instance: usize) -> Option<usize> {
        placement.site_of(instance)
    }

    fn tile_of(&self, placement: &Placement, instance: usize) -> Option<(u32, u32)> {
        self.site_of(placement, instance)
            .map(|s| self.graph.sites[s].tile)
    }

    /// The node `pin` of the instance on `site` reaches as `role`.
    fn pin_node(&self, site: usize, role: &str) -> Option<NodeId> {
        self.graph.sites[site].pin(role)
    }

    /// The delay inside its tile of the connection from driver pin `from`
    /// to the node `to` of a sink in the same tile, in ns.
    fn way(&mut self, placement: &Placement, from: usize, to: NodeId) -> f64 {
        let driver = &self.netlist.pins[from];
        let Some(site) = self.site_of(placement, driver.instance) else {
            return self.fallback;
        };
        let Some(node) = self.pin_node(site, &driver.role) else {
            return self.fallback;
        };
        self.reach
            .get(node, to)
            .map_or(self.fallback, |ps| f64::from(ps) / 1000.0)
    }

    /// Each input of `instance` driven from inside its tile, with its delay
    /// in ns: a lookup table's movable inputs each given the pin of the
    /// table its matching would, the heaviest by `weight` first, and every
    /// other input its own pin. Connections dedicated wiring makes
    /// (`PlaceTiming::local`) are left out: the placement charges them
    /// already.
    fn inputs(
        &mut self,
        placement: &Placement,
        timing: &PlaceTiming,
        instance: usize,
        weight: &[f64],
    ) -> Vec<(usize, f64)> {
        let mut out = Vec::new();
        let Some(site) = self.site_of(placement, instance) else {
            return out;
        };
        let tile = self.graph.sites[site].tile;
        let movable = self.delays.movable.get(instance).copied().unwrap_or(0);
        // The movable inputs driven from inside the tile, matched below.
        let mut matched: Vec<(usize, usize)> = Vec::new();
        let netlist = self.netlist;
        for pin in &netlist.instances[instance].pins {
            let p = &netlist.pins[*pin];
            if p.output || timing.local.get(*pin).copied().unwrap_or(false) {
                continue;
            }
            let Some(driver) = p.signal.and_then(|s| netlist.signals[s].driver) else {
                continue;
            };
            if self.tile_of(placement, netlist.pins[driver].instance) != Some(tile) {
                continue;
            }
            let lane = input_index(&p.role).filter(|k| movable & (1 << k) != 0);
            if lane.is_some() {
                matched.push((*pin, driver));
                continue;
            }
            let delay = match self.pin_node(site, &p.role) {
                Some(node) => self.way(placement, driver, node),
                None => self.fallback,
            };
            out.push((*pin, delay));
        }
        if matched.is_empty() {
            return out;
        }
        matched.sort_by(|a, b| {
            let wa = weight.get(a.0).copied().unwrap_or(0.0);
            let wb = weight.get(b.0).copied().unwrap_or(0.0);
            wb.total_cmp(&wa).then(a.0.cmp(&b.0))
        });
        let mut free = movable;
        for (pin, driver) in matched {
            let mut best: Option<(f64, u8)> = None;
            for k in 0..6u8 {
                if free & (1 << k) == 0 {
                    continue;
                }
                let Some(node) = self.pin_node(site, INPUTS[usize::from(k)]) else {
                    continue;
                };
                let delay = self.way(placement, driver, node);
                if best.is_none_or(|(d, _)| delay < d) {
                    best = Some((delay, k));
                }
            }
            match best {
                Some((delay, k)) => {
                    free &= !(1 << k);
                    out.push((pin, delay));
                }
                None => out.push((pin, self.fallback)),
            }
        }
        out
    }

    /// Each sink pin's connection delay: its routed one, and inside a tile
    /// [`Detail::inputs`]'.
    fn wire(&mut self, placement: &Placement, timing: &PlaceTiming, weight: &[f64]) -> Vec<f64> {
        let mut wire = self.routed.to_vec();
        wire.resize(self.netlist.pins.len(), 0.0);
        for instance in 0..self.netlist.instances.len() {
            for (pin, delay) in self.inputs(placement, timing, instance, weight) {
                wire[pin] = delay;
            }
        }
        wire
    }

    /// What the connections into `instance` from inside its tile cost, each
    /// delay weighed by its sink's `weight`.
    fn cost_in(
        &mut self,
        placement: &Placement,
        timing: &PlaceTiming,
        instance: usize,
        weight: &[f64],
    ) -> f64 {
        self.inputs(placement, timing, instance, weight)
            .into_iter()
            .map(|(pin, delay)| weight.get(pin).copied().unwrap_or(0.0) * delay)
            .sum()
    }

    /// What `instance`'s bel adds to the paths it starts: how much later a
    /// register there launches than on the earliest bel, weighed by every
    /// sink of the outputs it starts paths at.
    fn cost_launch(
        &self,
        placement: &Placement,
        timing: &PlaceTiming,
        instance: usize,
        weight: &[f64],
    ) -> f64 {
        let Some(site) = self.site_of(placement, instance) else {
            return 0.0;
        };
        let over = self.launch_over.get(site).copied().unwrap_or(0.0);
        if over == 0.0 {
            return 0.0;
        }
        let netlist = self.netlist;
        let mut total = 0.0;
        for pin in &netlist.instances[instance].pins {
            if !matches!(timing.pins.get(*pin), Some(PinTiming::Start)) {
                continue;
            }
            let Some(signal) = netlist.pins[*pin].signal else {
                continue;
            };
            for sink in &netlist.signals[signal].sinks {
                total += weight.get(*sink).copied().unwrap_or(0.0) * over;
            }
        }
        total
    }
}

/// A lookup table's input roles, `i0` (`A1`) first.
const INPUTS: [&str; 6] = ["i0", "i1", "i2", "i3", "i4", "i5"];

/// The `k` of an input role `i<k>`.
fn input_index(role: &str) -> Option<u8> {
    INPUTS
        .iter()
        .position(|r| *r == role)
        .and_then(|k| u8::try_from(k).ok())
}

/// Gives the cells on late paths other bels of their tiles: see the module's
/// documentation. `routed` is each sink pin's routed delay in nanoseconds, and
/// `timing` the placement's, its skew and launches brought up to date here as
/// the cells move.
#[allow(
    clippy::too_many_arguments,
    reason = "the placement's rules, its timing and the route's delays"
)]
pub(super) fn detail(
    netlist: &Netlist,
    graph: &RoutingGraph,
    info: &[Placeable],
    shared: &SiteRules,
    sites_by_kind: &BTreeMap<String, Vec<usize>>,
    timing: &mut PlaceTiming,
    delays: &BelDelays,
    routed: &[f64],
    placement: &mut Placement,
) -> BelReport {
    let earliest = timing
        .bel_launch
        .iter()
        .copied()
        .filter(|t| t.is_finite())
        .fold(f32::INFINITY, f32::min);
    let launch_over = timing
        .bel_launch
        .iter()
        .map(|t| {
            if t.is_finite() && earliest.is_finite() {
                f64::from(*t - earliest)
            } else {
                0.0
            }
        })
        .collect();
    let mut pass = Detail {
        netlist,
        graph,
        delays,
        reach: Reach {
            graph,
            delay: &delays.node_delay,
            from: FastMap::default(),
        },
        launch_over,
        fallback: timing.wire.0 + timing.wire.1,
        routed,
    };
    let saved = placement.clone();
    let analyse =
        |pass: &mut Detail<'_>, timing: &mut PlaceTiming, placement: &Placement, weight: &[f64]| {
            timing.skew_from(placement);
            timing.launch_from(netlist, placement);
            let wire = pass.wire(placement, timing, weight);
            let (critical, _, longest) = path_slack_against(netlist, timing, &wire, true);
            (critical, longest)
        };
    let weigh =
        |critical: &[f64]| -> Vec<f64> { critical.iter().map(|c| c.powi(EXPONENT)).collect() };
    // The first analysis matches each table's inputs in pin order, the
    // second by the first's criticality, as every one after.
    let (critical, _) = analyse(&mut pass, timing, placement, &[]);
    let (mut critical, before) = analyse(&mut pass, timing, placement, &weigh(&critical));
    let mut report = BelReport {
        before_ps: before.clone(),
        ..BelReport::default()
    };
    let mut tile_sites: BTreeMap<&str, TileSites> = BTreeMap::new();
    let movable =
        |instance: usize| info[instance].fixed.is_none() && info[instance].macro_index.is_none();
    let mut after = before.clone();
    for _ in 0..ROUNDS {
        let weight = weigh(&critical);
        // The cells on late paths, by tile: a sink with a late input, and
        // the driver of one.
        let mut hot: BTreeSet<((u32, u32), usize)> = BTreeSet::new();
        for (pin, c) in critical.iter().enumerate() {
            if *c < FLOOR {
                continue;
            }
            let p = &netlist.pins[pin];
            let driver = p
                .signal
                .and_then(|s| netlist.signals[s].driver)
                .map(|d| netlist.pins[d].instance);
            for instance in std::iter::once(p.instance).chain(driver) {
                if let Some(tile) = pass.tile_of(placement, instance)
                    && movable(instance)
                {
                    hot.insert((tile, instance));
                }
            }
        }
        let mut moved = 0usize;
        for (tile, cell) in hot {
            let Some(here) = placement.site_of(cell) else {
                continue;
            };
            let kind = netlist.instances[cell].kind.as_str();
            let Some(of_kind) = sites_by_kind.get(kind) else {
                continue;
            };
            let sites = tile_sites
                .entry(kind)
                .or_insert_with(|| TileSites::new(graph, of_kind));
            let Some(candidates) = sites.at(tile.0, tile.1).map(<[usize]>::to_vec) else {
                continue;
            };
            let mut best: Option<(f64, usize, Option<usize>)> = None;
            for site in candidates {
                if site == here || !info[cell].may_take(site) {
                    continue;
                }
                let occupant = placement.instance_at(site);
                if let Some(other) = occupant
                    && (!movable(other)
                        || netlist.instances[other].kind != netlist.instances[cell].kind
                        || !info[other].may_take(here))
                {
                    continue;
                }
                let mut moving = vec![(cell, site)];
                if let Some(other) = occupant {
                    moving.push((other, here));
                }
                if !shared.allows(placement, &moving) {
                    continue;
                }
                let touched = touched(netlist, placement, &pass, tile, &[Some(cell), occupant]);
                let moved_cells = [Some(cell), occupant];
                let cost = |pass: &mut Detail<'_>, placement: &Placement| -> f64 {
                    touched
                        .iter()
                        .map(|i| pass.cost_in(placement, timing, *i, &weight))
                        .sum::<f64>()
                        + moved_cells
                            .iter()
                            .flatten()
                            .map(|i| pass.cost_launch(placement, timing, *i, &weight))
                            .sum::<f64>()
                };
                let cost_before = cost(&mut pass, placement);
                swap(placement, cell, site, occupant, here);
                let cost_after = cost(&mut pass, placement);
                swap(placement, cell, here, occupant, site);
                let delta = cost_after - cost_before;
                if delta < -1e-9 && best.is_none_or(|(d, ..)| delta < d) {
                    best = Some((delta, site, occupant));
                }
            }
            if let Some((_, site, occupant)) = best {
                swap(placement, cell, site, occupant, here);
                moved += 1;
            }
        }
        report.moves += moved;
        let (next, longest) = analyse(&mut pass, timing, placement, &weight);
        critical = next;
        after = longest;
        if moved == 0 {
            break;
        }
    }
    report.after_ps = after;
    let worse: f64 = report
        .before_ps
        .iter()
        .zip(&report.after_ps)
        .filter(|(b, a)| **b > 0 && **a > 0)
        .map(|(b, a)| (*a as f64 / *b as f64).ln())
        .sum();
    if worse > 1e-9 {
        *placement = saved;
        report.undone = true;
        report.moves = 0;
    }
    timing.skew_from(placement);
    timing.launch_from(netlist, placement);
    report
}

/// Puts `cell` on `site`, and `occupant`, if any, on `back`.
fn swap(placement: &mut Placement, cell: usize, site: usize, occupant: Option<usize>, back: usize) {
    placement.unplace(cell);
    if let Some(other) = occupant {
        placement.unplace(other);
    }
    placement.place(cell, site);
    if let Some(other) = occupant {
        placement.place(other, back);
    }
}

/// The instances whose inputs a move of `cells` inside `tile` can change:
/// the cells, and every sink in the tile of a signal they drive.
fn touched(
    netlist: &Netlist,
    placement: &Placement,
    pass: &Detail<'_>,
    tile: (u32, u32),
    cells: &[Option<usize>],
) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for cell in cells.iter().flatten() {
        out.push(*cell);
        for pin in &netlist.instances[*cell].pins {
            let p = &netlist.pins[*pin];
            if !p.output {
                continue;
            }
            let Some(signal) = p.signal else {
                continue;
            };
            for sink in &netlist.signals[signal].sinks {
                let instance = netlist.pins[*sink].instance;
                if pass.tile_of(placement, instance) == Some(tile) {
                    out.push(instance);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Instance, NetPin, PathDomain, PinDelays, RouteThroughs, Signal};
    use super::*;
    use crate::fpga::arch::{Arch, BelDecl, ConfigBit, PipDecl, TileType, WireDecl, WireRef};
    use crate::ir::{CellId, Id};

    /// A row of `width` tiles of one type: lookup tables `d` and `e`, whose
    /// outputs drive, and `a` and `b`, whose inputs `i0` and `i1` read. From
    /// `d` a wire `fast` reaches `a.i0`, a wire `slow` reaches `b.i0`, `a.i0`
    /// and `a.i1`, and from `e` the wire `near` reaches `a.i0` and `a.i1`. With
    /// each node's delay in picoseconds: `fast` 100, `slow` 400, `near` 150,
    /// every other wire nothing.
    fn tiles(width: u32) -> (RoutingGraph, Vec<u32>) {
        let mut arch = Arch::new("t", "test", width, 1);
        let mut tile = TileType::new("clb", "clb", 8, 8);
        let wires = [
            "d_o", "e_o", "a_i0", "a_i1", "b_i0", "b_i1", "a_o", "b_o", "fast", "slow", "near",
        ];
        for name in wires {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        for bel in ["d", "e", "a", "b"] {
            let mut decl = BelDecl::new(bel, "lut");
            if bel == "a" || bel == "b" {
                for role in ["i0", "i1"] {
                    decl.pins
                        .push((role.to_owned(), WireRef::local(format!("{bel}_{role}"))));
                }
            }
            decl.pins
                .push(("o".to_owned(), WireRef::local(format!("{bel}_o"))));
            tile.bels.push(decl);
        }
        for (bit, (from, to)) in [
            ("d_o", "fast"),
            ("fast", "a_i0"),
            ("d_o", "slow"),
            ("slow", "b_i0"),
            ("slow", "a_i0"),
            ("slow", "a_i1"),
            ("e_o", "near"),
            ("near", "a_i0"),
            ("near", "a_i1"),
        ]
        .into_iter()
        .enumerate()
        {
            let bit = u32::try_from(bit).unwrap();
            tile.pips.push(PipDecl {
                from: WireRef::local(from),
                to: WireRef::local(to),
                bits: vec![ConfigBit::new(bit / 8, bit % 8)],
            });
        }
        arch.tile_types.push(tile);
        for x in 0..width {
            arch.set_tile(x, 0, 0);
        }
        let graph = arch.build_graph();
        let delay = graph
            .nodes
            .iter()
            .map(|wire| match wire.name.as_str() {
                "fast" => 100,
                "slow" => 400,
                "near" => 150,
                _ => 0,
            })
            .collect();
        (graph, delay)
    }

    /// The site of bel `bel` in tile `x`.
    fn site(graph: &RoutingGraph, x: u32, bel: &str) -> usize {
        graph
            .sites
            .iter()
            .position(|s| s.tile == (x, 0) && s.bel == bel)
            .unwrap()
    }

    /// A netlist of lookup tables, each `(name, the signals it reads on
    /// i0, i1, …, the signal it drives)`.
    fn tables(cells: &[(&str, &[usize], Option<usize>)], signals: usize) -> Netlist {
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: (0..signals)
                .map(|s| Signal {
                    name: format!("n{s}"),
                    driver: None,
                    sinks: Vec::new(),
                })
                .collect(),
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        for (index, (name, reads, drives)) in cells.iter().enumerate() {
            let mut pins = Vec::new();
            for (k, signal) in reads.iter().enumerate() {
                let pin = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance: index,
                    port: format!("I{k}"),
                    bit: 0,
                    role: format!("i{k}"),
                    output: false,
                    signal: Some(*signal),
                    constant: None,
                });
                netlist.signals[*signal].sinks.push(pin);
                pins.push(pin);
            }
            if let Some(signal) = drives {
                let pin = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance: index,
                    port: "O".to_owned(),
                    bit: 0,
                    role: "o".to_owned(),
                    output: true,
                    signal: Some(*signal),
                    constant: None,
                });
                netlist.signals[*signal].driver = Some(pin);
                pins.push(pin);
            }
            netlist.instances.push(Instance {
                cell: CellId::from_index(index),
                name: (*name).to_owned(),
                primitive: "L".to_owned(),
                kind: "lut".to_owned(),
                pins,
                pin: None,
            });
        }
        netlist
    }

    /// Timing for `netlist`: the instances `starts` launch at 0.3 ns, `ends`
    /// capture 0.05 ns before a 5 ns clock, and every other passes a path on
    /// in 0.1 ns.
    fn timing(netlist: &Netlist, starts: &[usize], ends: &[usize]) -> PlaceTiming {
        let n = netlist.instances.len();
        let pins = netlist
            .pins
            .iter()
            .map(|pin| {
                if starts.contains(&pin.instance) {
                    PinTiming::Start
                } else if ends.contains(&pin.instance) {
                    PinTiming::End
                } else {
                    PinTiming::Through
                }
            })
            .collect();
        let edge = |i: usize| starts.contains(&i) || ends.contains(&i);
        PlaceTiming {
            pins,
            cell: (0..n).map(|i| if edge(i) { 0.0 } else { 0.1 }).collect(),
            launch: (0..n)
                .map(|i| if starts.contains(&i) { 0.3 } else { 0.0 })
                .collect(),
            capture: (0..n)
                .map(|i| if ends.contains(&i) { 0.05 } else { 0.0 })
                .collect(),
            domain: (0..n)
                .map(|i| {
                    if edge(i) {
                        PathDomain::Clock(0)
                    } else {
                        PathDomain::Untimed
                    }
                })
                .collect(),
            periods: vec![5.0],
            wire: (0.5, 0.1, 0.0),
            rounds: 1,
            weight: 8.0,
            exponent: 3,
            reaches: Vec::new(),
            cycles: Vec::new(),
            untimed_from: Vec::new(),
            untimed_to: Vec::new(),
            frozen: vec![false; n],
            pin_domain: Vec::new(),
            pin_delays: PinDelays::default(),
            local_wire: None,
            local: Vec::new(),
            clock: Vec::new(),
            skew: Vec::new(),
            bel_launch: Vec::new(),
            clock_region: Vec::new(),
            region_margin: (0.0, 0.0),
            region: Vec::new(),
        }
    }

    /// Every instance free to move.
    fn free(netlist: &Netlist) -> Vec<Placeable> {
        netlist
            .instances
            .iter()
            .map(|_| Placeable {
                fixed: None,
                region: None,
                macro_index: None,
                allowed: None,
                soft: false,
            })
            .collect()
    }

    fn by_kind(graph: &RoutingGraph) -> BTreeMap<String, Vec<usize>> {
        let mut out: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            out.entry(site.kind.clone()).or_default().push(index);
        }
        out
    }

    /// A launch in tile 0 on `d`, a table beside it on `b`, and the capture
    /// in tile 5, its connection routed at 1 ns: the path is 0.3 + 0.4 +
    /// 0.1 + 1 + 0.05 ns with the table on `b`, whose input `d` reaches by
    /// `slow`.
    fn late_table() -> (
        RoutingGraph,
        Vec<u32>,
        Netlist,
        Placement,
        PlaceTiming,
        Vec<f64>,
    ) {
        let (graph, delay) = tiles(6);
        let netlist = tables(
            &[
                ("launch", &[], Some(0)),
                ("table", &[0], Some(1)),
                ("capture", &[1], None),
            ],
            2,
        );
        let mut placement = Placement::new(3, graph.sites.len());
        placement.place(0, site(&graph, 0, "d"));
        placement.place(1, site(&graph, 0, "b"));
        placement.place(2, site(&graph, 5, "a"));
        let timing = timing(&netlist, &[0], &[2]);
        let mut routed = vec![0.0; netlist.pins.len()];
        routed[netlist.signals[1].sinks[0]] = 1.0;
        (graph, delay, netlist, placement, timing, routed)
    }

    /// **A table on a late path is moved to the bel of its tile its input
    /// reaches fastest.** The table of [`late_table`] on `b` reads `d` by
    /// `slow` (400 ps); `a`, empty, by `fast` (100 ps). The pass moves it, and
    /// the latest path is 300 ps earlier as it times it. With the table held
    /// by a constraint, nothing moves.
    ///
    /// Would catch the pass not moving a cell, moving it to the slower bel,
    /// timing a connection inside a tile other than by its fastest way over
    /// the nodes' delays, or moving a held cell. Would not catch the delays
    /// being the fabric's: they are this test's; nor whether a route of the
    /// moved placement realises them, which is what
    /// `docs/fpga-vivado-parity.md`, "A tile's bels", measured.
    #[test]
    fn a_late_table_moves_to_the_bel_its_input_reaches_fastest() {
        let (graph, delay, netlist, mut placement, mut timing, routed) = late_table();
        let delays = BelDelays {
            node_delay: Arc::from(delay.as_slice()),
            movable: Vec::new(),
        };
        let shared = SiteRules::find_with(&netlist, &graph, &RouteThroughs::default());
        let kinds = by_kind(&graph);
        let report = detail(
            &netlist,
            &graph,
            &free(&netlist),
            &shared,
            &kinds,
            &mut timing,
            &delays,
            &routed,
            &mut placement,
        );
        assert_eq!(report.moves, 1, "{report:?}");
        assert!(!report.undone);
        assert_eq!(placement.site_of(1), Some(site(&graph, 0, "a")));
        assert_eq!(placement.site_of(0), Some(site(&graph, 0, "d")));
        assert_eq!(report.before_ps, vec![1850]);
        assert_eq!(report.after_ps, vec![1550]);

        let (graph, _, netlist, mut placement, mut timing, routed) = late_table();
        let mut info = free(&netlist);
        info[1].fixed = Some(site(&graph, 0, "b"));
        let report = detail(
            &netlist,
            &graph,
            &info,
            &shared,
            &kinds,
            &mut timing,
            &delays,
            &routed,
            &mut placement,
        );
        assert_eq!(report.moves, 0);
        assert_eq!(placement.site_of(1), Some(site(&graph, 0, "b")));
    }

    /// **A table's movable inputs are given its pins latest first.** A table
    /// on `a` reads `d` and `e`, both in its tile: `d` reaches `i0` in 100 ps
    /// and `i1` in 400, `e` either in 150. Weighed for `d`, `d` takes `i0`
    /// and `e` `i1`, 100 and 150 ps; weighed for `e`, `e` takes `i0`, the
    /// first of its two alike, and `d` is left `i1`, 400 ps. With no input
    /// movable, each is timed on its own pin, `d` on `i0` and `e` on `i1`;
    /// with only `i1` movable, the same, however weighed. `d` reaches `i0` by
    /// `slow` too, in 400 ps: its 100 is the least of two ways.
    ///
    /// Would catch the matching ignoring the weights, ignoring which inputs
    /// may move, giving two inputs one pin, or a way's delay not the least
    /// of its tile's. Would not catch the router matching otherwise, which
    /// `route::refine` does, by slack.
    #[test]
    fn a_tables_inputs_take_its_pins_latest_first() {
        let (graph, delay) = tiles(1);
        let netlist = tables(
            &[
                ("d", &[], Some(0)),
                ("e", &[], Some(1)),
                ("table", &[0, 1], None),
            ],
            2,
        );
        let mut placement = Placement::new(3, graph.sites.len());
        placement.place(0, site(&graph, 0, "d"));
        placement.place(1, site(&graph, 0, "e"));
        placement.place(2, site(&graph, 0, "a"));
        let timing = timing(&netlist, &[0, 1], &[2]);
        let (from_d, from_e) = (netlist.signals[0].sinks[0], netlist.signals[1].sinks[0]);
        let delays_with = |movable: Vec<u8>| BelDelays {
            node_delay: Arc::from(delay.as_slice()),
            movable,
        };
        let routed = vec![0.0; netlist.pins.len()];
        let inputs = |delays: &BelDelays, weight: &[f64]| {
            let mut pass = Detail {
                netlist: &netlist,
                graph: &graph,
                delays,
                reach: Reach {
                    graph: &graph,
                    delay: &delays.node_delay,
                    from: FastMap::default(),
                },
                launch_over: Vec::new(),
                fallback: 0.6,
                routed: &routed,
            };
            let mut out = pass.inputs(&placement, &timing, 2, weight);
            out.sort_by_key(|(pin, _)| *pin);
            out.into_iter()
                .map(|(pin, d)| (pin, (d * 1000.0).round()))
                .collect::<Vec<_>>()
        };
        let mut weight = vec![0.0; netlist.pins.len()];
        let both = delays_with(vec![0, 0, 0b11]);
        weight[from_d] = 1.0;
        weight[from_e] = 0.5;
        assert_eq!(
            inputs(&both, &weight),
            vec![(from_d, 100.0), (from_e, 150.0)]
        );
        weight[from_d] = 0.5;
        weight[from_e] = 1.0;
        assert_eq!(
            inputs(&both, &weight),
            vec![(from_d, 400.0), (from_e, 150.0)]
        );
        let fixed = delays_with(Vec::new());
        assert_eq!(
            inputs(&fixed, &weight),
            vec![(from_d, 100.0), (from_e, 150.0)]
        );
        let second = delays_with(vec![0, 0, 0b10]);
        assert_eq!(
            inputs(&second, &weight),
            vec![(from_d, 100.0), (from_e, 150.0)]
        );
    }
}
