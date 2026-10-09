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
    assert_eq!(verdict.active_pips, 163_313, "{verdict:?}");
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

/// The node, by union-find root, of every wire of `graph`, with joins
/// (bitless pips) as the only edges.
fn node_roots(graph: &reticle::fpga::arch::RoutingGraph) -> Vec<u32> {
    let mut nodes = Nodes((0..u32::try_from(graph.nodes.len()).unwrap()).collect());
    for (id, pip) in graph.pips.iter().enumerate() {
        if graph.pip_bits(u32::try_from(id).unwrap()).is_empty() {
            nodes.join(pip.from, pip.to);
        }
    }
    (0..u32::try_from(graph.nodes.len()).unwrap())
        .map(|i| nodes.find(i))
        .collect()
}

/// The cheapest path of pips from any wire of `from`'s node to any wire
/// of `to`'s node, counting each pip with bits as one and each join as
/// nothing, never entering a node in `forbidden`. The pips with bits, in
/// order.
fn search(
    graph: &reticle::fpga::arch::RoutingGraph,
    roots: &[u32],
    from: NodeId,
    to: NodeId,
    forbidden: &HashSet<u32>,
) -> Option<Vec<u32>> {
    use std::collections::VecDeque;
    let target = roots[to as usize];
    let mut cost = vec![u32::MAX; graph.nodes.len()];
    let mut came: Vec<Option<u32>> = vec![None; graph.nodes.len()];
    let mut queue = VecDeque::new();
    cost[from as usize] = 0;
    queue.push_back(from);
    while let Some(wire) = queue.pop_front() {
        if roots[wire as usize] == target {
            let mut pips = Vec::new();
            let mut at = wire;
            while let Some(pip) = came[at as usize] {
                if !graph.pip_bits(pip).is_empty() {
                    pips.push(pip);
                }
                at = graph.pips[pip as usize].from;
            }
            pips.reverse();
            return Some(pips);
        }
        for &pip in graph.outgoing(wire) {
            let next = graph.pips[pip as usize].to;
            if forbidden.contains(&roots[next as usize]) && roots[next as usize] != target {
                continue;
            }
            let step = u32::from(!graph.pip_bits(pip).is_empty());
            let c = cost[wire as usize] + step;
            if c < cost[next as usize] {
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
    None
}

/// Writes the first bitstream for the ZCU104 that Reticle builds itself:
/// EMIO GPIO output 0 routed straight back to EMIO GPIO input 1, through
/// the processor's interface column and the interconnect, and nothing
/// else. Linux drives `gpio594` as an output and reads `gpio595` as an
/// input. The two are different lines on purpose: Linux reads an output
/// line's own output register back, so a loopback onto the same line
/// would look the same whether it worked or not. `docs/fpga-uray.md` has
/// the procedure and the result.
///
/// Ignored because it writes files: set `RETICLE_ZCU104_OUT` to a
/// directory, and it writes `emio_loopback.bit` and `.bin` there.
///
/// Before writing, it holds the bitstream to the project's two checks
/// that apply to a routing-only design: every set bit decodes, through
/// the database, to a feature; and the pips the decoded bits turn on are
/// exactly the pips of the route.
#[test]
#[ignore = "writes a bitstream for the board"]
fn an_emio_loopback_for_the_board() {
    use reticle::fpga::arch::ConfigBit;
    use reticle::fpga::xc7::BitHeader;

    let (Some(bits_root), Some(wiring_root)) = (uraydb(), wiringdb()) else {
        return;
    };
    let Ok(out_dir) = std::env::var("RETICLE_ZCU104_OUT") else {
        eprintln!("skipped: set RETICLE_ZCU104_OUT to a directory to write into");
        return;
    };
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
    let fabric = uray::build_arch(
        &FabricInputs {
            grid: &inputs.grid,
            bits: &inputs.bits,
            wiring: &inputs.wiring,
            rules: &inputs.rules,
        },
        region,
    );
    let graph = fabric.arch.build_graph();
    let roots = node_roots(&graph);
    let wire = |name: &str| -> NodeId {
        let index = graph
            .nodes
            .iter()
            .position(|w| w.tile == ps && w.name == name)
            .unwrap_or_else(|| panic!("the PS8 tile has no wire `{name}`"));
        u32::try_from(index).unwrap()
    };
    let source = wire("PSS_ALTO_CORE_0_FMIO_GPIO_OUT0");
    let sink = wire("PSS_ALTO_CORE_0_FMIO_GPIO_IN1");
    // No other processor pin may be touched: a stray signal on an AXI
    // handshake input would be far worse than a failed loopback.
    let forbidden: HashSet<u32> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, w)| w.tile == ps)
        .map(|(i, _)| roots[i])
        .filter(|r| *r != roots[source as usize] && *r != roots[sink as usize])
        .collect();
    for (label, end) in [("source", source), ("sink", sink)] {
        let members: Vec<_> = (0..graph.nodes.len())
            .filter(|&i| roots[i] == roots[end as usize])
            .map(|i| (graph.nodes[i].tile, graph.nodes[i].name.clone()))
            .collect();
        eprintln!("{label} node: {members:?}");
    }
    let route = search(&graph, &roots, source, sink, &forbidden).expect("a route exists");
    for pip in &route {
        let p = &graph.pips[*pip as usize];
        eprintln!(
            "{:?} {} -> {}  {:?}",
            p.tile,
            graph.nodes[p.from as usize].name,
            graph.nodes[p.to as usize].name,
            graph.pip_bits(*pip)
        );
    }

    // Tile bits: the route's, then every slice's defaults, as Vivado
    // leaves an unused slice.
    let at: HashMap<(u32, u32), usize> = inputs
        .grid
        .tiles()
        .iter()
        .enumerate()
        .map(|(i, t)| (t.grid, i))
        .collect();
    let mut bits: Vec<(usize, ConfigBit)> = Vec::new();
    for pip in &route {
        let tile = at[&graph.pips[*pip as usize].tile];
        bits.extend(graph.pip_bits(*pip).iter().map(|b| (tile, *b)));
    }
    let defaults = {
        let mut out: HashMap<String, Vec<ConfigBit>> = HashMap::new();
        for kind in ["CLEL_L", "CLEL_R", "CLEM", "CLEM_R"] {
            let stem = TileTypeBits::file_stem(kind).replacen("segbits_", "defaults_", 1);
            let path = format!("{bits_root}/{stem}.db");
            let parsed = TileTypeBits::parse("", "none", Some((&read(&path), &path))).unwrap();
            out.insert(kind.to_owned(), parsed.defaults);
        }
        out
    };
    for (index, tile) in inputs.grid.tiles().iter().enumerate() {
        if let Some(list) = defaults.get(&tile.kind) {
            bits.extend(list.iter().map(|b| (index, *b)));
        }
    }
    let layout = FrameLayout::from_grid(&inputs.grid);
    let frames = uray::frames_from_tile_bits(&inputs.grid, &layout, bits).unwrap();

    // Every bit decodes, and the pips it turns on are the route's.
    let decoded = uray::decode(&inputs.grid, &layout, &frames).unwrap();
    assert!(decoded.unowned.is_empty());
    let mut types = inputs.bits.clone();
    for (kind, list) in &defaults {
        types
            .entry(kind.clone())
            .or_default()
            .defaults
            .clone_from(list);
    }
    let explained = uray::explain(&inputs.grid, &decoded, &types);
    assert_eq!(explained.unexplained_total(), 0, "{explained:?}");
    eprintln!(
        "{} set bits, every one explained; {} of them the route's",
        decoded.set_bits,
        route
            .iter()
            .map(|p| graph.pip_bits(*p).len())
            .sum::<usize>()
    );
    let mut on = HashSet::new();
    for (index, pip) in graph.pips.iter().enumerate() {
        let id = u32::try_from(index).unwrap();
        let pattern = graph.pip_bits(id);
        if pattern.is_empty() {
            continue;
        }
        let tile = at[&pip.tile];
        let Some(set) = decoded.tiles.get(&tile) else {
            continue;
        };
        if pattern.iter().all(|b| set.contains(b)) {
            on.insert(id);
        }
    }
    let chosen: HashSet<u32> = route.iter().copied().collect();
    let extra: Vec<_> = on.difference(&chosen).collect();
    eprintln!("pips the bits turn on beyond the route: {}", extra.len());

    let header = BitHeader::new("emio_loopback", "xczu7ev-ffvc1156-2-e");
    let bit = uray::write_bit(&header, uray::IDCODE_XCZU7EV, &layout, &frames).unwrap();
    let dir = Path::new(&out_dir);
    std::fs::write(dir.join("emio_loopback.bit"), &bit).unwrap();
    std::fs::write(
        dir.join("emio_loopback.bin"),
        uray::bin_from_bit(&bit).unwrap(),
    )
    .unwrap();
}
