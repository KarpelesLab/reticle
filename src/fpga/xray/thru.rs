//! A flip-flop's data through a lookup table: what Vivado calls a
//! route-through, as Vivado writes it.
//!
//! # Why
//!
//! A slice has eight flip-flops and four bypass inputs, `AX` to `DX`, and the
//! bypass input of a lane is the only way into either of its flip-flops that
//! is not a cell of the lane: `<L>FFMUX` takes the lookup table's `O6` or
//! `O5`, `<L>X`, the carry or a wide multiplexer, and `<L>5FFMUX` takes `O5`
//! or `<L>X`. So four flip-flops fed from elsewhere fill a slice, and this
//! flow had slices of two and three where Vivado's held eight
//! (`docs/fpga-vivado-parity.md`, "Packing against Vivado's").
//!
//! Vivado's other four reach their flip-flops **through a lookup table no
//! cell needs**, which passes one of its inputs on. Nothing in the netlist
//! says so: the table is part of the implementation, written into its `INIT`
//! and a mux, and [`super::XrayDatabase::check_route_throughs`] accounts for
//! every table so written after decoding the bitstream.
//!
//! # What Vivado writes, decoded
//!
//! **Checked**, by decoding the bitstreams Vivado 2026.1 wrote for the designs
//! of `testdata/fpga/vivado/ffpack/` (eight flip-flops held to one slice, in a
//! `SLICEL` and a `SLICEM`, with and without a table in each lane;
//! `tools/vivado-parity/gen_ffpack.py`, `ffpack.sh`), as
//! `tests/fpga_xray_ffpack_vivado.rs` holds:
//!
//! - **A lane whose table holds nothing**: the flip-flop `<L>FF` reads the
//!   table's `O6` (`<L>FFMUX.O6`), the table passes its **`A6`** on, and its
//!   `INIT` is `FFFFFFFF00000000`, the upper half. The route of the signal
//!   ends at the slice's `<L>6` pin.
//! - **A lane whose table holds a function of four inputs or two**: `<L>FF`
//!   reads `O5` (`<L>FFMUX.O5`), and the table passes on **one of `A1` to
//!   `A5` the function does not read**, its lower half that input alone
//!   (`F0F0F0F0` for `A3`) and its upper half the function. `A6` is not
//!   routed, and reads one, so `O6` is still the function.
//! - In both, **the lane's other flip-flop, `<L>5FF`, takes the bypass input**
//!   (`<L>5FFMUX.IN_B`), and a flip-flop alone in its lane takes it too
//!   (`<L>FFMUX.<L>X`): Vivado goes through a table only for a flip-flop the
//!   bypass cannot serve.
//! - All eight share the slice's clock, enable and reset: given one of eight
//!   with an enable the others lack, or on another clock, Vivado placed the
//!   others elsewhere, against their `LOC`s. Each keeps its own initial and
//!   reset value (`ZINI`, `ZRST`), a five-input flip-flop as the other.
//!
//! Vivado's timer puts the table in the connection: the net to the flip-flop
//! through `B6` was 0.211 ns where the same hops to the bypass were 0.238, and
//! through `B4` and `O5` 0.413, about what the slower pin costs. The ways here
//! cost nothing of their own, then, beyond the pin's wire.
//!
//! # How it is built
//!
//! Each lane has two wires of its own ([`through_wire`]): from `<L>6` with the
//! upper half's bits to `<L>FF`'s `D` with `<L>FFMUX.O6`, and from each of
//! `<L>1` to `<L>5` with that input's lower-half bits, to `D` with
//! `<L>FFMUX.O5`. They are pips like any other, so the bitstream gets their
//! bits from the route ([`pass_throughs`]). The placer counts each as a wire
//! into the lane ([`crate::fpga::place::RouteThrough`]): the first while the
//! table holds nothing, the second beside a table that leaves it room
//! ([`route_through_room`]), and after placement the router is offered only
//! those the placement needs ([`XrayFabric::route_through_nodes`]). A table
//! that shares its lower half with a way gets only its upper half written
//! ([`XrayFabric::configure_route_throughs`]).
//!
//! # Which are on, and why not both
//!
//! [`RouteThroughMode::Empty`] gives a lane the upper half's way only;
//! [`RouteThroughMode::All`], the command line's default, the lower half's as
//! well. Measured on the timing corpus by Vivado's timer, the first built it 0.6%
//! faster than without ways over ten seeds (geometric mean over Vivado's own,
//! 1.248 against 1.256), and over five the two together were slower (1.252)
//! and the lower half's alone slower still (1.255), for about the same slices:
//! a table beside a way keeps off `A6`, its fastest input, and a placement that
//! leans on the way keeps such tables there (`docs/fpga-vivado-parity.md`,
//! "Eight flip-flops a slice"). **Measured again** once the placer had median
//! moves, a truthful estimate and the clock's arrival ("Filling slices"), the
//! two together were the faster, over ten seeds 1.231 against 1.239 with the
//! upper half's alone, for the same slices; why the two readings differ is not
//! known. Under `Empty` the lower half's wire and the pips into it are still
//! declared, with no way on to the flip-flop, so that the fabric is the one that
//! was measured; the router never enters it.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::sites::{PassThrough, ff_data_wire, slice_feature_prefix, slice_wire_prefix};
use super::{Decoded, XrayFabric, XrayTile};
use crate::fpga::arch::{NodeId, RoutingGraph};
use crate::fpga::place::{Netlist, Placement, RouteThrough, RouteThroughs};
use crate::fpga::route::Routing;
use crate::ir::{AttrValue, Design, ModuleId};

/// Which ways through a lookup table a fabric is loaded with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RouteThroughMode {
    /// None: a flip-flop takes its lane's bypass input or a cell's output,
    /// as before them.
    #[default]
    Off,
    /// The upper half's, through a table no cell holds (`<L>FFMUX.O6` from
    /// `A6`). Measured, it builds the timing corpus no slower than without
    /// (`docs/fpga-vivado-parity.md`, "Eight flip-flops a slice").
    Empty,
    /// Those and the lower half's beside a cell that leaves it room
    /// (`<L>FFMUX.O5` from one of `A1` to `A5`), as Vivado has both. The
    /// command line's default: measured, it builds the timing corpus faster
    /// than [`RouteThroughMode::Empty`] (`docs/fpga-vivado-parity.md`,
    /// "Filling slices").
    All,
}

impl RouteThroughMode {
    /// The mode `--route-throughs` names, or `None` for a name that is none.
    #[must_use]
    pub fn from_name(name: &str) -> Option<RouteThroughMode> {
        match name {
            "off" => Some(RouteThroughMode::Off),
            "empty" => Some(RouteThroughMode::Empty),
            "all" => Some(RouteThroughMode::All),
            _ => None,
        }
    }
}

/// The four lanes of a slice.
const LETTERS: [char; 4] = ['A', 'B', 'C', 'D'];

/// The wire of lane `letter`'s way through its table to its flip-flop: from
/// `A6` and `O6` (`five` false) or from `A1`..`A5` and `O5`. Invented, as the
/// slice's other inside wires are, with `_MUX_` in it, which no database wire
/// has and Vivado's export leaves out.
#[must_use]
pub(super) fn through_wire(wires: &str, letter: char, five: bool) -> String {
    format!("{wires}_MUX_{letter}THRU{}", if five { 5 } else { 6 })
}

/// The lane, slice wire prefix and half of a wire [`through_wire`] named:
/// `CLBLL_LL_MUX_ATHRU5` is `("CLBLL_LL", 'A', true)`.
fn parse_through_wire(name: &str) -> Option<(&str, char, bool)> {
    let (wires, rest) = name.split_once("_MUX_")?;
    let rest = rest
        .strip_suffix("5")
        .map(|r| (r, true))
        .or_else(|| rest.strip_suffix("6").map(|r| (r, false)))?;
    let letter = rest.0.strip_suffix("THRU")?;
    let mut chars = letter.chars();
    let first = chars.next()?;
    if chars.next().is_some() || !LETTERS.contains(&first) {
        return None;
    }
    Some((wires, first, rest.1))
}

/// The truth table a way through input `k` (1 to 6) writes: for `A6`, the
/// upper half, which `O6` reads with `A6` high; for `A1` to `A5`, the lower
/// half's entries whose bit `k - 1` is set, which is `O5` passing input `k`
/// on whatever the upper half holds.
#[must_use]
pub(super) fn pass_bits(k: u32) -> u64 {
    if k == 6 {
        return 0xFFFF_FFFF_0000_0000;
    }
    (0..32u32)
        .filter(|i| i & (1 << (k - 1)) != 0)
        .fold(0u64, |acc, i| acc | 1u64 << i)
}

/// Every way through a table of a CLB tile type: per slice and lane, `<L>6`
/// to the upper-half wire and on to `<L>FF`'s `D`, and `<L>1` to `<L>5` to
/// the lower-half wire and on. See the module's documentation.
pub(super) fn pass_throughs(tile_type: &str, mode: RouteThroughMode) -> Vec<PassThrough> {
    if mode == RouteThroughMode::Off {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0..2u32 {
        let (Some(wires), Some(prefix)) = (
            slice_wire_prefix(tile_type, index),
            slice_feature_prefix(tile_type, index),
        ) else {
            continue;
        };
        for letter in LETTERS {
            let init = |bits: u64| -> Vec<String> {
                (0..64u32)
                    .filter(|i| bits >> i & 1 == 1)
                    .map(|i| format!("{prefix}.{letter}LUT.INIT[{i:02}]"))
                    .collect()
            };
            let six = through_wire(wires, letter, false);
            let five = through_wire(wires, letter, true);
            out.push(PassThrough {
                to: six.clone(),
                from: format!("{wires}_{letter}6"),
                features: init(pass_bits(6)),
            });
            for k in 1..=5u32 {
                out.push(PassThrough {
                    to: five.clone(),
                    from: format!("{wires}_{letter}{k}"),
                    features: init(pass_bits(k)),
                });
            }
            let data = ff_data_wire(wires, letter);
            out.push(PassThrough {
                to: data.clone(),
                from: six,
                features: vec![format!("{prefix}.{letter}FFMUX.O6")],
            });
            if mode == RouteThroughMode::All {
                out.push(PassThrough {
                    to: data,
                    from: five,
                    features: vec![format!("{prefix}.{letter}FFMUX.O5")],
                });
            }
        }
    }
    out
}

/// The 64-bit truth table of a lookup table cell, when it has one.
fn truth_table(design: &Design, top: ModuleId, netlist: &Netlist, instance: usize) -> Option<u64> {
    let cell = design.modules[top]
        .cells
        .get(netlist.instances[instance].cell)?;
    match cell.params.get("INIT") {
        Some(AttrValue::Const(init)) if init.width() == 64 => {
            Some((0..64u32).fold(0u64, |acc, i| {
                acc | u64::from(init.bit(i) == crate::logic::Bit::One) << i
            }))
        }
        _ => None,
    }
}

/// Whether `init` reads input `k` (0 to 5).
fn reads(init: u64, k: u32) -> bool {
    (0..64u32)
        .filter(|i| i & (1 << k) == 0)
        .any(|i| (init >> i & 1) != (init >> (i | 1 << k) & 1))
}

/// Per instance, whether a lookup table leaves its lower half to a way
/// through it ([`RouteThroughs::room`]): one function (no `O5` of its own), of
/// four inputs or fewer, not `I5`, each of them a signal. Its `O6` then reads
/// the upper half with `A6` unrouted, the lower is free for one input it does
/// not read, and Vivado puts a way there (`lut4x4`, `lut2x4`).
#[must_use]
pub fn route_through_room(design: &Design, top: ModuleId, netlist: &Netlist) -> Vec<bool> {
    let mut out = vec![false; netlist.instances.len()];
    for (index, inst) in netlist.instances.iter().enumerate() {
        if inst.kind != "lut" || inst.primitive == super::lutpack::DUAL {
            continue;
        }
        let Some(init) = truth_table(design, top, netlist, index) else {
            continue;
        };
        let pin = |role: &str| {
            inst.pins
                .iter()
                .map(|p| &netlist.pins[*p])
                .find(|p| p.role == role)
        };
        if pin("o5").is_some_and(|p| p.signal.is_some()) || reads(init, 5) {
            continue;
        }
        let read: Vec<u32> = (0..5u32).filter(|k| reads(init, *k)).collect();
        let signals = read
            .iter()
            .all(|k| pin(&format!("i{k}")).is_some_and(|p| p.signal.is_some()));
        out[index] = read.len() <= 4 && signals;
    }
    out
}

/// One way of [`XrayFabric::route_throughs`], with the lane it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Lane {
    /// The lookup table's site.
    site: usize,
    /// Whether it is the lower half's way.
    five: bool,
}

/// The lookup table sites of `graph` by each of their input nodes.
fn tables_by_input(graph: &RoutingGraph) -> HashMap<NodeId, usize> {
    let mut out = HashMap::new();
    for (index, site) in graph.sites.iter().enumerate() {
        if site.kind != "lut" {
            continue;
        }
        for (role, node) in &site.pins {
            if role.starts_with('i') {
                out.insert(*node, index);
            }
        }
    }
    out
}

/// Every way of `graph`, by node, with its lane.
fn lanes(graph: &RoutingGraph) -> BTreeMap<NodeId, Lane> {
    let tables = tables_by_input(graph);
    let mut out = BTreeMap::new();
    for (index, wire) in graph.nodes.iter().enumerate() {
        let Some((_, _, five)) = parse_through_wire(&wire.name) else {
            continue;
        };
        let node = NodeId::try_from(index).unwrap_or(NodeId::MAX);
        let site = graph
            .incoming(node)
            .iter()
            .find_map(|pip| tables.get(&graph.pip(*pip).from).copied());
        if let Some(site) = site {
            out.insert(node, Lane { site, five });
        }
    }
    out
}

impl XrayFabric {
    /// The ways through a lookup table this fabric was loaded with
    /// ([`super::XrayOptions::route_throughs`]), for the placer: the upper
    /// half's while its table holds nothing, the lower half's beside a table
    /// that leaves it room too. Empty for a fabric loaded without them.
    #[must_use]
    pub fn route_throughs(
        &self,
        design: &Design,
        top: ModuleId,
        netlist: &Netlist,
        graph: &RoutingGraph,
    ) -> RouteThroughs {
        let nodes: Vec<RouteThrough> = lanes(graph)
            .into_iter()
            .map(|(node, lane)| RouteThrough {
                node,
                site: lane.site,
                beside: lane.five,
            })
            .collect();
        if nodes.is_empty() {
            return RouteThroughs::default();
        }
        RouteThroughs {
            nodes,
            room: route_through_room(design, top, netlist),
        }
    }

    /// Closes every way through a table `graph` has but `needed` to the
    /// router, and keeps a table whose lower half is a way off its `A6`: its
    /// function stays in the upper half only with `A6` unrouted, so `A6` leaves
    /// its class of interchangeable inputs
    /// ([`crate::fpga::route::RouteOptions::equivalent`]). Every way the graph
    /// has, and not only those the placer was told of: a way left open that the
    /// placer did not count on is a route through a table a cell may hold.
    /// Returns how many ways are left open.
    pub fn route_through_nodes(
        &self,
        graph: &RoutingGraph,
        needed: &[NodeId],
        closed: &mut Vec<bool>,
        equivalent: &mut [u32],
    ) -> usize {
        let lanes = lanes(graph);
        if lanes.is_empty() {
            return 0;
        }
        if closed.len() < graph.nodes.len() {
            closed.resize(graph.nodes.len(), false);
        }
        let mut open = 0usize;
        for (node, lane) in &lanes {
            if needed.binary_search(node).is_ok() {
                open += 1;
                if lane.five
                    && let Some(a6) = graph.sites[lane.site].pin("i5")
                    && let Some(class) = equivalent.get_mut(a6 as usize)
                {
                    *class = u32::MAX;
                }
            } else {
                closed[*node as usize] = true;
            }
        }
        open
    }

    /// A table whose lower half a route passes through keeps only its upper
    /// half: the cell's truth table is its function on every entry, and the
    /// way's own bits are the lower half's, so the lower half of the cell's is
    /// cleared and the way's set again. Returns how many tables it rewrote.
    ///
    /// # Errors
    ///
    /// When a bit is outside its tile.
    pub fn configure_route_throughs(
        &self,
        graph: &RoutingGraph,
        placement: &Placement,
        routing: &Routing,
        tiles: &mut crate::fpga::bitstream::Bitstream,
    ) -> Result<usize, String> {
        let lanes = lanes(graph);
        if lanes.is_empty() {
            return Ok(0);
        }
        let mut rewritten = 0usize;
        for route in routing.routes() {
            for pip in &route.pips {
                let way = graph.pip(*pip);
                let Some(lane) = lanes.get(&way.to) else {
                    continue;
                };
                if !lane.five || placement.instance_at(lane.site).is_none() {
                    continue;
                }
                let site = &graph.sites[lane.site];
                let Some(bel) = self.arch.tile_types[site.tile_type].bel(&site.bel) else {
                    continue;
                };
                for entry in &bel.config {
                    if let crate::fpga::arch::ConfigEntry::Param { name, index, at } = entry
                        && name == "INIT"
                        && *index < 32
                    {
                        tiles.clear(site.tile, *at).map_err(|e| e.to_string())?;
                    }
                }
                for bit in graph.pip_bits(*pip) {
                    tiles.set(way.tile, *bit).map_err(|e| e.to_string())?;
                }
                rewritten += 1;
            }
        }
        Ok(rewritten)
    }
}

/// What [`super::XrayDatabase::check_route_throughs`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteThroughReport {
    /// Tables a route passes through to a flip-flop with nothing else on them.
    pub empty: usize,
    /// Tables a route passes through beside a cell's function.
    pub beside: usize,
    /// Lookup table cells whose truth table decoded as their own.
    pub cells: usize,
    /// Tables nothing is on that decoded as nothing.
    pub idle: usize,
}

impl RouteThroughReport {
    /// One line, for the build's notes; empty when no table was passed
    /// through.
    #[must_use]
    pub fn to_text(&self) -> String {
        if self.empty + self.beside == 0 {
            return String::new();
        }
        format!(
            "note: {} flip-flop(s) reach their data through a lookup table, {} of them beside \
             the table's own function; every such table, every table cell ({}) and every idle \
             table ({}) decodes to exactly what was placed and routed\n",
            self.empty + self.beside,
            self.beside,
            self.cells,
            self.idle
        )
    }
}

/// Each table's truth table by tile name, slice prefix and lane, and every
/// feature by tile name, of a decoding.
type DecodedTables<'a> = (
    HashMap<(&'a str, &'a str, char), u64>,
    HashSet<(&'a str, &'a str)>,
);

/// Every lookup table's truth table as `decoded` has it, by tile name, slice
/// prefix and lane, and every feature it names.
fn decoded_tables(decoded: &Decoded) -> DecodedTables<'_> {
    let mut tables: HashMap<(&str, &str, char), u64> = HashMap::new();
    let mut features: HashSet<(&str, &str)> = HashSet::new();
    for (tile, feature) in &decoded.features {
        features.insert((tile.as_str(), feature.as_str()));
        let Some((prefix, rest)) = feature.split_once('.') else {
            continue;
        };
        let Some(rest) = rest.strip_suffix(']') else {
            continue;
        };
        let Some((lut, index)) = rest.split_once("LUT.INIT[") else {
            continue;
        };
        let (Some(letter), Ok(index)) = (lut.chars().next(), index.parse::<u32>()) else {
            continue;
        };
        if lut.len() != 1 || index >= 64 {
            continue;
        }
        *tables.entry((tile.as_str(), prefix, letter)).or_default() |= 1u64 << index;
    }
    (tables, features)
}

impl super::XrayDatabase {
    /// Accounts for every lookup table a way passes through, which no cell
    /// stands for: after decoding, each such table's truth table is exactly
    /// the way's (`FFFFFFFF00000000` through `A6`), or the way's lower half
    /// under the cell's upper; its lane's flip-flop mux selects the half the
    /// way took; every lookup table cell's truth table is its own; and a table
    /// nothing is on and nothing passes through decodes as nothing. The arcs
    /// [`super::XrayDatabase::arc_differences`] compares are the interconnect's;
    /// this is the same question asked of the tables, without which a way's
    /// bits would decode as a table no one placed and pass as explained.
    ///
    /// # Errors
    ///
    /// The first table that does not decode as it should, in words.
    #[allow(clippy::too_many_arguments)]
    pub fn check_route_throughs(
        &self,
        tiles: &[XrayTile],
        graph: &RoutingGraph,
        design: &Design,
        top: ModuleId,
        netlist: &Netlist,
        placement: &Placement,
        routing: &Routing,
        decoded: &Decoded,
    ) -> Result<RouteThroughReport, String> {
        let at: HashMap<(u32, u32), &XrayTile> =
            tiles.iter().map(|t| ((t.grid_x, t.grid_y), t)).collect();
        let lanes = lanes(graph);
        // The way each table's route took, by its site: which half, and from
        // which input.
        let mut through: HashMap<usize, (bool, u32)> = HashMap::new();
        for route in routing.routes() {
            for pip in &route.pips {
                let way = graph.pip(*pip);
                let Some(lane) = lanes.get(&way.to) else {
                    continue;
                };
                let site = &graph.sites[lane.site];
                let k = site
                    .pins
                    .iter()
                    .find(|(_, node)| *node == way.from)
                    .and_then(|(role, _)| role.strip_prefix('i'))
                    .and_then(|n| n.parse::<u32>().ok())
                    .map_or(0, |n| n + 1);
                if through.insert(lane.site, (lane.five, k)).is_some() {
                    return Err(format!(
                        "two routes pass through the lookup table at {}",
                        site.name
                    ));
                }
            }
        }
        let (tables, features) = decoded_tables(decoded);
        // Which sites exclude each, both ways round, as the placer reads them.
        let mut excluders: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, site) in graph.sites.iter().enumerate() {
            for other in &site.blocks {
                excluders.entry(index).or_default().push(*other);
                excluders.entry(*other).or_default().push(index);
            }
        }
        let mut report = RouteThroughReport::default();
        for (index, site) in graph.sites.iter().enumerate() {
            if site.kind != "lut" {
                continue;
            }
            let Some(tile) = at.get(&site.tile) else {
                continue;
            };
            let Some((prefix, letter)) = site
                .bel
                .strip_suffix("LUT")
                .and_then(|head| head.rsplit_once('_'))
                .and_then(|(prefix, letter)| {
                    let mut chars = letter.chars();
                    let first = chars.next()?;
                    chars.next().is_none().then_some((prefix, first))
                })
            else {
                continue;
            };
            // A shift register or a RAM on the same table writes it as its
            // own, which is not this check's business.
            let others = excluders.get(&index).map_or(&[][..], Vec::as_slice);
            if others.iter().any(|b| placement.instance_at(*b).is_some()) {
                if through.contains_key(&index) {
                    return Err(format!(
                        "a route passes through the lookup table at {}, which another cell holds",
                        site.name
                    ));
                }
                continue;
            }
            let cell = placement.instance_at(index);
            let own = match cell {
                Some(cell) if netlist.instances[cell].kind == "lut" => {
                    match truth_table(design, top, netlist, cell) {
                        Some(init) => Some(init),
                        None => continue,
                    }
                }
                Some(_) => continue,
                None => None,
            };
            let way = through.get(&index).copied();
            let wanted = match (own, way) {
                (None, None) => 0,
                (Some(init), None) => init,
                (None, Some((false, 6))) => pass_bits(6),
                (None, Some((true, k))) if (1..=5).contains(&k) => pass_bits(k),
                // The cell's function is left only its upper half, which `O6`
                // reads while `A6` is unrouted: a cell reading `A6` would read
                // the way's half too.
                (Some(init), Some((true, _))) if reads(init, 5) => {
                    return Err(format!(
                        "a route passes through the lower half of the lookup table at {}, \
                         whose cell reads `A6`",
                        site.name
                    ));
                }
                (Some(init), Some((true, k))) if (1..=5).contains(&k) => {
                    (init & 0xFFFF_FFFF_0000_0000) | pass_bits(k)
                }
                (_, Some((five, k))) => {
                    return Err(format!(
                        "a route passes through the lookup table at {} from input {k} to {}, \
                         which no way goes",
                        site.name,
                        if five { "O5" } else { "O6" }
                    ));
                }
            };
            let found = tables
                .get(&(tile.name.as_str(), prefix, letter))
                .copied()
                .unwrap_or(0);
            if found != wanted {
                return Err(format!(
                    "the lookup table at {} decodes as {found:016X} where {} wants {wanted:016X}",
                    site.name,
                    match (own, way) {
                        (None, None) => "nothing on it",
                        (Some(_), None) => "its cell",
                        _ => "the route through it",
                    }
                ));
            }
            if let Some((five, _)) = way {
                let mux = format!("{prefix}.{letter}FFMUX.{}", if five { "O5" } else { "O6" });
                if !features.contains(&(tile.name.as_str(), mux.as_str())) {
                    return Err(format!(
                        "a route passes through the lookup table at {} and `{mux}` is not set",
                        site.name
                    ));
                }
                if five {
                    report.beside += 1;
                } else {
                    report.empty += 1;
                }
            } else if own.is_some() {
                report.cells += 1;
            } else {
                report.idle += 1;
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each input's pattern is Vivado's: `FFFFFFFF00000000` through `A6`, and
    /// through `A3` the lower half `F0F0F0F0` that `lut4x4` and `lut2x4` decode
    /// to. Would catch an input off by one, or a lower-half way that writes the
    /// upper half too.
    #[test]
    fn a_ways_truth_table_is_vivados() {
        assert_eq!(pass_bits(6), 0xFFFF_FFFF_0000_0000);
        assert_eq!(pass_bits(1), 0x0000_0000_AAAA_AAAA);
        assert_eq!(pass_bits(2), 0x0000_0000_CCCC_CCCC);
        assert_eq!(pass_bits(3), 0x0000_0000_F0F0_F0F0);
        assert_eq!(pass_bits(4), 0x0000_0000_FF00_FF00);
        assert_eq!(pass_bits(5), 0x0000_0000_FFFF_0000);
    }

    /// The wire names come back apart; a name that is not one is refused.
    #[test]
    fn a_ways_wire_is_read_back() {
        assert_eq!(
            parse_through_wire(&through_wire("CLBLL_LL", 'C', true)),
            Some(("CLBLL_LL", 'C', true))
        );
        assert_eq!(
            parse_through_wire(&through_wire("CLBLM_M", 'A', false)),
            Some(("CLBLM_M", 'A', false))
        );
        assert_eq!(parse_through_wire("CLBLL_LL_MUX_AFF_D"), None);
        assert_eq!(parse_through_wire("CLBLL_LL_MUX_ETHRU6"), None);
    }

    /// `reads`, which room is counted by and which the check asks of a table
    /// beside a way, sees the inputs a truth table reads: a two-input XOR's two
    /// and not the rest, and the way through `A6` reads `A6`. Would catch the
    /// halves compared along the wrong bit. Whether a table leaves room in a
    /// build is `tests/fpga_xray_ffpack_vivado.rs`'s `lut2x16`, which does not
    /// fit its tile when none does.
    #[test]
    fn reads_sees_the_inputs_a_table_reads() {
        assert!(!reads(0, 0));
        let xor2 = (0..64u32).fold(0u64, |a, i| a | u64::from((i ^ (i >> 1)) & 1 == 1) << i);
        assert!(reads(xor2, 0) && reads(xor2, 1) && !reads(xor2, 2) && !reads(xor2, 5));
        assert!(reads(0xFFFF_FFFF_0000_0000, 5));
    }
}
