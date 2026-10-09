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

use std::collections::{HashMap, HashSet};
use std::path::Path;

use reticle::fpga::arch::NodeId;
use reticle::fpga::uray::{
    self, FabricInputs, FrameLayout, GridRegion, SiteTypePins, TileConn, TileGrid, TileTypeBits,
    TileTypeWiring,
};

/// Where `reticle fetch <name>` puts the pinned copy, if it is there.
fn fetched(name: &str, version: &str, probe: &str) -> Option<String> {
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let root = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")))?;
    let dir = format!("{root}/{name}/{version}");
    Path::new(&format!("{dir}/{probe}"))
        .is_file()
        .then_some(dir)
}

/// A database root from its variable or the cache, or `None` having
/// said why.
fn database(env: &str, name: &str, version: &str, probe: &str) -> Option<String> {
    match std::env::var(env) {
        Ok(root) if Path::new(&format!("{root}/{probe}")).is_file() => Some(root),
        Ok(root) => {
            eprintln!("skipped: {env} is `{root}` but `{probe}` is not in it");
            None
        }
        Err(_) => {
            let found = fetched(name, version, probe);
            if found.is_none() {
                eprintln!("skipped: needs `reticle fetch {name}` or {env}");
            }
            found
        }
    }
}

fn uraydb() -> Option<String> {
    database(
        "RETICLE_URAYDB",
        "prjuray-db",
        "9e7d3e7965240fd260d6549a1883a01a152ae490",
        "xazu7ev/tilegrid.json",
    )
}

fn wiringdb() -> Option<String> {
    database(
        "RETICLE_URAYWIRING",
        "prjuray-db-2020",
        "affbc5e555ebae16475f32e8fb2d6565d4204f3f",
        "zynqusp/xczu3eg-sfvc784-1-e/tileconn.json",
    )
}

fn reference() -> Option<Vec<u8>> {
    let Ok(dir) = std::env::var("RETICLE_ZCU104_REF") else {
        eprintln!(
            "skipped: needs Vivado's ZCU104 base overlay; set RETICLE_ZCU104_REF to a \
             directory holding base.bit. See docs/fpga-uray.md"
        );
        return None;
    };
    std::fs::read(Path::new(&dir).join("base.bit")).ok()
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("`{path}` would not read: {e}"))
}

/// Everything the fabric is built from.
struct Inputs {
    grid: TileGrid,
    bits: HashMap<String, TileTypeBits>,
    wiring: HashMap<String, TileTypeWiring>,
    sites: HashMap<String, SiteTypePins>,
    rules: Vec<TileConn>,
}

fn inputs(bits_root: &str, wiring_root: &str) -> Inputs {
    let grid_path = format!("{bits_root}/xazu7ev/tilegrid.json");
    let grid = TileGrid::parse(&read(&grid_path), &grid_path).unwrap();
    let mut bits = HashMap::new();
    let mut wiring = HashMap::new();
    let kinds: HashSet<&str> = grid.tiles().iter().map(|t| t.kind.as_str()).collect();
    for kind in kinds {
        let stem = TileTypeBits::file_stem(kind);
        let seg = format!("{bits_root}/{stem}.db");
        if let Ok(text) = std::fs::read_to_string(&seg) {
            bits.insert(
                kind.to_owned(),
                TileTypeBits::parse(&text, &seg, None).unwrap(),
            );
        }
        let wiring_kind = uray::wiring_type(kind);
        let path = format!("{wiring_root}/zynqusp/tile_types/tile_type_{wiring_kind}.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            wiring.insert(
                wiring_kind.to_owned(),
                TileTypeWiring::parse(&text, &path).unwrap(),
            );
        }
    }
    let mut sites = HashMap::new();
    for decl in wiring.values().flat_map(|w| &w.sites) {
        if sites.contains_key(&decl.site_type) {
            continue;
        }
        let path = format!(
            "{wiring_root}/zynqusp/site_types/site_type_{}.json",
            decl.site_type
        );
        if let Ok(text) = std::fs::read_to_string(&path) {
            sites.insert(
                decl.site_type.clone(),
                SiteTypePins::parse(&text, &path).unwrap(),
            );
        }
    }
    let conn = format!("{wiring_root}/zynqusp/xczu3eg-sfvc784-1-e/tileconn.json");
    let rules = uray::parse_tileconn(&read(&conn), &conn).unwrap();
    Inputs {
        grid,
        bits,
        wiring,
        sites,
        rules,
    }
}

/// The grid position of the tile called `name`.
fn position(grid: &TileGrid, name: &str) -> (u32, u32) {
    grid.tiles()
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("the grid has no `{name}`"))
        .grid
}

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
    // Measured: Vivado set 255 241 of the region's pips.
    assert_eq!(verdict.active_pips, 255_241, "{verdict:?}");
    assert_eq!(
        (verdict.double_driven, verdict.undriven, verdict.unread),
        (0, 0, 0),
        "a core node of Vivado's routing does not fit the fabric: {verdict:?}"
    );
    // Block RAM and interface nodes, pinned so a change shows: see the
    // module documentation for why they are not zero.
    assert_eq!(verdict.other, (167, 652, 2560), "{verdict:?}");
}
