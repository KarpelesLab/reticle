//! The UltraScale+ routing fabric against Vivado's routing of a ZCU104
//! design.
//!
//! # What this checks
//!
//! [`uray::build_arch`] joins the ZU7EV's wires with rules learned on a
//! different die, the ZU3EG, adjusted in two measured ways
//! (`docs/fpga-uray.md`). This file asks the one question that tells
//! whether the result is the ZU7EV's metal: does Vivado's own routing of
//! a large design make sense on it? Every pip whose bit is set in
//! `base.bit` is taken as on, and then, over the nodes the joins make:
//!
//! - **no node is driven twice**, which a join between two different
//!   pieces of metal causes;
//! - **every node a pip reads is driven** by another pip or by a site
//!   output, which a missing join breaks;
//! - **every node a pip drives is read** by another pip or by a site
//!   input, likewise.
//!
//! A node the region's edge cuts is left out, exactly: a wire is cut
//! when one of its joins reaches a tile the die has and the region does
//! not.
//!
//! The three counts must be zero over **core** nodes — those made only
//! of interconnect, slice and clock-region break tiles, which is where a
//! design of lookup tables and flip-flops lives. Nodes that reach a block
//! RAM, a DSP or an interface tile are counted apart and pinned: those
//! tiles reach the interconnect through features the database writes as
//! two pips sharing one bit (`A<->B&C->D`), which [`uray::routing_of`]
//! cannot split, and a block RAM's output pins are driven from several of
//! its internal cores. Neither is a wrong join; both are block RAM support
//! that does not exist yet.
//!
//! `URAY_DEBUG=1` prints the wires of the first few failing nodes.
//!
//! # What it would not catch
//!
//! A join to the right wire of the wrong tile that happens to leave every
//! node singly driven and connected. A design Reticle routes on this
//! fabric and runs on the board is the check for that, and there is none
//! yet.
//!
//! # Inputs
//!
//! `reticle fetch prjuray-db prjuray-db-2020`, or `RETICLE_URAYDB` and
//! `RETICLE_URAYWIRING`; and `RETICLE_ZCU104_REF` naming a directory with
//! the PYNQ 3.1 base overlay's `base.bit`. Each test skips without them.

#![cfg(feature = "fpga")]

mod uray_board;

use uray_board::{Inputs, inputs, position, reference, uraydb, wiringdb};

use std::collections::{HashMap, HashSet};

use reticle::fpga::arch::NodeId;
use reticle::fpga::uray::{self, FabricInputs, FrameLayout, GridRegion};

/// A union-find over node ids.
struct Nodes(Vec<u32>);

impl Nodes {
    fn find(&mut self, mut a: u32) -> u32 {
        while self.0[a as usize] != a {
            let up = self.0[self.0[a as usize] as usize];
            self.0[a as usize] = up;
            a = up;
        }
        a
    }
    fn join(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b) as usize] = a.min(b);
        }
    }
}

/// What Vivado's routing looks like on the fabric of `region`.
#[derive(Debug, Default, PartialEq, Eq)]
struct Verdict {
    active_pips: usize,
    double_driven: usize,
    undriven: usize,
    unread: usize,
    /// The same three counts, over nodes that are not core.
    other: (usize, usize, usize),
}

fn verdict(inputs: &Inputs, bitstream: &[u8], region: GridRegion) -> Verdict {
    let fabric = uray::build_arch(
        &FabricInputs {
            grid: &inputs.grid,
            bits: &inputs.bits,
            wiring: &inputs.wiring,
            rules: &inputs.rules,
            processor_clock_track: None,
        },
        region,
    );
    eprintln!("{:?}", fabric.stats);
    let arch = &fabric.arch;
    let graph = arch.build_graph();

    // Which wires a join of theirs leaves the region from: the far tile
    // is on the die but not in the region.
    let on_die: HashSet<(u32, u32)> = inputs
        .grid
        .tiles()
        .iter()
        .filter(|t| t.kind != "NULL")
        .map(|t| t.grid)
        .collect();
    let mut cut: HashSet<(u32, u32, &str)> = HashSet::new();
    for y in region.y0..=region.y1 {
        for x in region.x0..=region.x1 {
            let Some(tile_type) = arch.tile_at(x, y) else {
                continue;
            };
            for pip in tile_type.pips.iter().filter(|p| p.bits.is_empty()) {
                let (Some(tx), Some(ty)) = (
                    x.checked_add_signed(pip.to.dx),
                    y.checked_add_signed(pip.to.dy),
                ) else {
                    continue;
                };
                if (pip.to.dx, pip.to.dy) != (0, 0)
                    && !region.contains(tx, ty)
                    && on_die.contains(&(tx, ty))
                {
                    cut.insert((x, y, pip.from.name.as_str()));
                }
            }
        }
    }

    let mut nodes = Nodes((0..u32::try_from(graph.nodes.len()).unwrap()).collect());
    for (id, pip) in graph.pips.iter().enumerate() {
        if graph.pip_bits(u32::try_from(id).unwrap()).is_empty() {
            nodes.join(pip.from, pip.to);
        }
    }
    let node_at: HashMap<(u32, u32, &str), NodeId> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(id, w)| {
            (
                (w.tile.0, w.tile.1, w.name.as_str()),
                u32::try_from(id).unwrap(),
            )
        })
        .collect();
    let count = graph.nodes.len();
    let mut open = vec![false; count];
    let mut sited_out = vec![false; count];
    let mut sited_in = vec![false; count];
    for (id, wire) in graph.nodes.iter().enumerate() {
        let root = nodes.find(u32::try_from(id).unwrap()) as usize;
        if cut.contains(&(wire.tile.0, wire.tile.1, wire.name.as_str())) {
            open[root] = true;
        }
        if wire.name.contains("VCC") || wire.name.contains("GND") {
            sited_out[root] = true;
        }
    }
    // Site pins, from the 2020 tile and site types.
    for y in region.y0..=region.y1 {
        for x in region.x0..=region.x1 {
            let Some(index) = arch.tile_index_at(x, y) else {
                continue;
            };
            let kind = uray::wiring_type(&fabric.database_type[index]);
            let Some(wiring) = inputs.wiring.get(kind) else {
                continue;
            };
            for site in &wiring.sites {
                let Some(pins) = inputs.sites.get(&site.site_type) else {
                    continue;
                };
                for (pin, wire) in &site.pins {
                    let Some(&id) = node_at.get(&(x, y, wire.as_str())) else {
                        continue;
                    };
                    let root = nodes.find(id) as usize;
                    if pins.outputs.contains(pin) {
                        sited_out[root] = true;
                    }
                    if pins.inputs.contains(pin) {
                        sited_in[root] = true;
                    }
                }
            }
        }
    }

    // A node is core when every wire of it is in an interconnect, slice
    // or break tile: what a first design lives on. Block RAM, DSP and
    // interface tiles reach the interconnect through features the
    // database writes with `&` (see `uray::routing_of`), and are counted
    // apart.
    let mut core = vec![true; count];
    for (id, wire) in graph.nodes.iter().enumerate() {
        let root = nodes.find(u32::try_from(id).unwrap()) as usize;
        let Some(index) = arch.tile_index_at(wire.tile.0, wire.tile.1) else {
            continue;
        };
        let kind = fabric.database_type[index].as_str();
        let is_core = matches!(kind, "INT" | "CLEL_R" | "CLEL_L" | "CLEM" | "CLEM_R")
            || uray::is_break_type(kind);
        if !is_core {
            core[root] = false;
        }
    }

    // And the fabric's own bels, whose pins include wires no 2020 site
    // type lists (a slice's control-group wires).
    for site in &graph.sites {
        for (role, node) in &site.pins {
            let root = nodes.find(*node) as usize;
            if matches!(role.as_str(), "o" | "q" | "din") {
                sited_out[root] = true;
            } else {
                sited_in[root] = true;
            }
        }
    }

    // The bits Vivado set, tile by tile.
    let layout = FrameLayout::from_grid(&inputs.grid);
    let bit = uray::read_bit(bitstream).unwrap();
    let decoded = uray::decode(&inputs.grid, &layout, &bit.frames).unwrap();
    let mut set: HashMap<(u32, u32), HashSet<_>> = HashMap::new();
    for (tile, bits) in &decoded.tiles {
        let grid = inputs.grid.tiles()[*tile].grid;
        if region.contains(grid.0, grid.1) {
            set.insert(grid, bits.iter().copied().collect());
        }
    }

    let mut drivers = vec![0u32; count];
    let mut reads = vec![false; count];
    let mut active: Vec<(NodeId, NodeId)> = Vec::new();
    for (id, pip) in graph.pips.iter().enumerate() {
        let bits = graph.pip_bits(u32::try_from(id).unwrap());
        if bits.is_empty() {
            continue;
        }
        let Some(on) = set.get(&pip.tile) else {
            continue;
        };
        if bits.iter().all(|b| on.contains(b)) {
            let (from, to) = (nodes.find(pip.from), nodes.find(pip.to));
            drivers[to as usize] += 1;
            reads[from as usize] = true;
            active.push((from, to));
        }
    }
    let mut out = Verdict {
        active_pips: active.len(),
        ..Verdict::default()
    };
    let mut seen_from = HashSet::new();
    let mut seen_to = HashSet::new();
    let debug = std::env::var("URAY_DEBUG").is_ok();
    let describe = |nodes: &mut Nodes, root: u32| -> Vec<String> {
        (0..count)
            .filter(|&i| nodes.find(u32::try_from(i).unwrap()) == root)
            .map(|i| {
                let w = &graph.nodes[i];
                let t = arch
                    .tile_at(w.tile.0, w.tile.1)
                    .map_or("?", |t| t.name.as_str());
                format!("{t}@{:?}:{}", w.tile, w.name)
            })
            .collect()
    };
    for (from, to) in active {
        let (f, t) = (from as usize, to as usize);
        if seen_to.insert(to) && !open[t] {
            let double = drivers[t] > 1;
            let unread = !reads[t] && !sited_in[t];
            if !core[t] {
                if debug && double && out.other.0 < 8 {
                    eprintln!("other double: {:?}", describe(&mut nodes, to));
                }
                out.other.0 += usize::from(double);
                out.other.2 += usize::from(unread);
            } else {
                out.double_driven += usize::from(double);
                out.unread += usize::from(unread);
                if debug && (double || unread) && out.double_driven + out.unread <= 10 {
                    eprintln!(
                        "double {double} unread {unread}: {:?}",
                        describe(&mut nodes, to)
                    );
                }
            }
        }
        if seen_from.insert(from) && !open[f] && drivers[f] == 0 && !sited_out[f] {
            if !core[f] {
                out.other.1 += 1;
            } else {
                out.undriven += 1;
                if debug && out.undriven <= 8 {
                    eprintln!("undriven: {:?}", describe(&mut nodes, from));
                }
            }
        }
    }
    out
}

/// A block of fabric in clock region row 4, interconnect columns X5 to
/// X30: away from the processor system, the configuration column and the
/// transceivers, where `docs/fpga-uray.md` found the ZU3EG's rules hold.
#[test]
fn vivados_routing_is_consistent_on_the_fabric() {
    let (Some(bits_root), Some(wiring_root)) = (uraydb(), wiringdb()) else {
        return;
    };
    let Some(bitstream) = reference() else { return };
    let inputs = inputs(&bits_root, &wiring_root);
    let a = position(&inputs.grid, "INT_X5Y290");
    let b = position(&inputs.grid, "INT_X30Y250");
    let region = GridRegion::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
    let verdict = verdict(&inputs, &bitstream, region);
    eprintln!("{verdict:?}");
    // Measured: Vivado set 262 267 of the region's pips — 255 241 in the
    // interconnect and 7 026 slice control groups (a group's clock enable
    // or set/reset in use; `uray::slice`).
    assert_eq!(verdict.active_pips, 262_267, "{verdict:?}");
    assert_eq!(
        (verdict.double_driven, verdict.undriven, verdict.unread),
        (0, 0, 0),
        "a core node of Vivado's routing does not fit the fabric: {verdict:?}"
    );
    // Block RAM and interface nodes, pinned so a change shows: see the
    // module documentation for why they are not zero.
    assert_eq!(verdict.other, (167, 652, 2560), "{verdict:?}");
}

/// The processor's side of the fabric: the `PS8` site, the interface
/// column beside it and four interconnect columns, over the four clock
/// region rows the processor system spans. Vivado routes the base
/// overlay's EMIO GPIO outputs and its AXI port through here, and this is
/// the path a design has to use to talk to Linux.
#[test]
fn vivados_routing_is_consistent_beside_the_processor() {
    let (Some(bits_root), Some(wiring_root)) = (uraydb(), wiringdb()) else {
        return;
    };
    let Some(bitstream) = reference() else { return };
    let inputs = inputs(&bits_root, &wiring_root);
    let ps = position(&inputs.grid, "PSS_ALTO_X0Y60");
    let low = position(&inputs.grid, "INT_X31Y0");
    let high = position(&inputs.grid, "INT_X27Y239");
    let region = GridRegion::new(
        ps.0,
        high.1.min(low.1),
        low.0.max(high.0),
        high.1.max(low.1),
    );
    let verdict = verdict(&inputs, &bitstream, region);
    eprintln!("{verdict:?}");
    // 163 313 in the interconnect and the interface column, and 3 597
    // slice control groups.
    assert_eq!(verdict.active_pips, 166_910, "{verdict:?}");
    assert_eq!(
        (verdict.double_driven, verdict.undriven, verdict.unread),
        (0, 0, 0),
        "a core node of Vivado's routing does not fit the fabric: {verdict:?}"
    );
    // The interface column's nodes are not core. Every `LOGIC_OUTS` pass
    // of an interface tile shares that tile's three enable bits, so
    // decoding counts all thirty of a used tile as on: most of the
    // unread nodes are those.
    assert_eq!(verdict.other, (439, 839, 6380), "{verdict:?}");
}

/// The table of `PS8` inputs Vivado leaves undriven, re-derived from
/// Vivado's routing on Reticle's own fabric: an input is undriven when no
/// pip whose bits Vivado set drives its node. It must be exactly
/// `uray::processor::left_undriven`, all 585.
#[test]
fn the_undriven_processor_inputs_are_the_ones_vivado_leaves() {
    use reticle::fpga::uray::processor;
    let (Some(bits_root), Some(wiring_root)) = (uraydb(), wiringdb()) else {
        return;
    };
    let Some(bitstream) = reference() else { return };
    let inputs = inputs(&bits_root, &wiring_root);
    let grid = &inputs.grid;
    let ps = position(grid, "PSS_ALTO_X0Y60");
    let low = position(grid, "INT_X31Y0");
    let high = position(grid, "INT_X27Y239");
    let region = GridRegion::new(
        ps.0,
        high.1.min(low.1),
        low.0.max(high.0),
        high.1.max(low.1),
    );
    let fabric = uray::build_arch(
        &FabricInputs {
            grid,
            bits: &inputs.bits,
            wiring: &inputs.wiring,
            rules: &inputs.rules,
            processor_clock_track: None,
        },
        region,
    );
    let graph = fabric.arch.build_graph();
    let roots = processor::node_roots(&graph);

    let layout = FrameLayout::from_grid(grid);
    let bit = uray::read_bit(&bitstream).unwrap();
    let decoded = uray::decode(grid, &layout, &bit.frames).unwrap();
    let mut set: HashMap<(u32, u32), HashSet<_>> = HashMap::new();
    for (tile, bits) in &decoded.tiles {
        set.insert(grid.tiles()[*tile].grid, bits.iter().copied().collect());
    }
    let mut driven: HashSet<u32> = HashSet::new();
    for (id, pip) in graph.pips.iter().enumerate() {
        let bits = graph.pip_bits(u32::try_from(id).unwrap());
        if !bits.is_empty()
            && set
                .get(&pip.tile)
                .is_some_and(|s| bits.iter().all(|b| s.contains(b)))
        {
            driven.insert(roots[pip.to as usize]);
        }
    }
    let wiring = &inputs.wiring["PSS_ALTO"];
    let pins = &inputs.sites["PS8"];
    let mut undriven = std::collections::BTreeSet::new();
    for (pin, wire) in &wiring.sites[0].pins {
        if !pins.inputs.contains(pin) {
            continue;
        }
        let id = graph
            .nodes
            .iter()
            .position(|w| w.tile == ps && w.name == *wire)
            .unwrap();
        if !driven.contains(&roots[id]) {
            undriven.insert(pin.clone());
        }
    }
    let table = processor::left_undriven();
    let missing: Vec<_> = table.difference(&undriven).collect();
    let extra: Vec<_> = undriven.difference(&table).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "in the table but driven: {missing:?}; undriven but not in the table: {extra:?}"
    );
}

/// Every lookup table of the widest region the flow places on can reach
/// one in the middle, and be reached from it, over the nodes the router
/// may use: the region's own fabric with Vivado's borrowed clock network
/// closed, as `zcu104::implement` routes.
///
/// It exists because the region once reached interconnect column 38, and
/// a network placed there failed after ten minutes with "no path exists".
/// Column 37 is east of an UltraRAM column, the ZU3EG whose rules join
/// the wires has none, and no wire of the model crosses it but 856 clock
/// wires. The slices beyond could send nothing back west. The *forward*
/// half of this test passed against that region, by a path nobody
/// traced; the backward half fails, with 3 840 outputs in the two columns
/// past the UltraRAM.
///
/// It would not catch a pair of lookup tables joined only through a third
/// table's pins, or a path that exists but is too congested to use, and it
/// asks about one table in the middle rather than every pair.
#[test]
fn every_lookup_table_of_the_widest_region_reaches_the_middle() {
    use reticle::fpga::uray::zcu104::{Board, MAX_EAST, PL_CLK0_TRACK, clock_tile_types};
    use std::collections::VecDeque;
    let (Some(bits_root), Some(wiring_root)) = (uraydb(), wiringdb()) else {
        return;
    };
    let Some(bitstream) = reference() else { return };
    let inputs = inputs(&bits_root, &wiring_root);
    let mut board = Board::reaching(&inputs, Some(PL_CLK0_TRACK), MAX_EAST).unwrap();
    board
        .replay(&bitstream, &clock_tile_types(&inputs.grid, false))
        .unwrap();
    let closed = board.closed_nodes(&HashSet::new()).unwrap();
    let graph = &board.graph;
    let tables: Vec<_> = graph.sites.iter().filter(|s| s.kind == "lut").collect();
    let search = |start: Vec<NodeId>, forward: bool| {
        let mut seen = vec![false; graph.nodes.len()];
        let mut queue = VecDeque::new();
        for node in start {
            seen[node as usize] = true;
            queue.push_back(node);
        }
        while let Some(node) = queue.pop_front() {
            let pips = if forward {
                graph.outgoing(node)
            } else {
                graph.incoming(node)
            };
            for &pip in pips {
                let pip = graph.pip(pip);
                let next = if forward { pip.to } else { pip.from };
                if !seen[next as usize] && !closed[next as usize] {
                    seen[next as usize] = true;
                    queue.push_back(next);
                }
            }
        }
        seen
    };
    let middle = tables[tables.len() / 2];
    let from = search(middle.pin("o").into_iter().collect(), true);
    let to = search(middle.pin_nodes("i0").collect(), false);
    let unreached: Vec<_> = tables
        .iter()
        .flat_map(|t| t.pins.iter().filter(|(r, _)| r.starts_with('i')))
        .filter(|(_, n)| !from[*n as usize])
        .collect();
    let unreaching: Vec<_> = tables
        .iter()
        .filter(|t| t.pin("o").is_some_and(|o| !to[o as usize]))
        .map(|t| t.name.as_str())
        .collect();
    eprintln!(
        "{} lookup tables to interconnect column {MAX_EAST}; {} inputs unreached from {}, \
         {} outputs that cannot reach it",
        tables.len(),
        unreached.len(),
        middle.name,
        unreaching.len()
    );
    assert_eq!(tables.len(), 26_880);
    assert!(
        unreached.is_empty(),
        "{:?}",
        &unreached[..unreached.len().min(8)]
    );
    assert!(
        unreaching.is_empty(),
        "{:?}",
        &unreaching[..unreaching.len().min(8)]
    );
}
