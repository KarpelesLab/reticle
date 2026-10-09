//! Shared support for the ZCU104 tests: finding the databases and
//! Vivado's reference bitstream, loading what the fabric is built from,
//! and [`Board`], which builds a small design on the processor's side of
//! the fabric and writes it as a bitstream the board can load.
//!
//! Each input skips cleanly when absent, saying why: `reticle fetch
//! prjuray-db prjuray-db-2020` for the databases (or `RETICLE_URAYDB` and
//! `RETICLE_URAYWIRING`), and `RETICLE_ZCU104_REF` for a directory holding
//! the PYNQ 3.1 base overlay's `base.bit`.

// Each test binary uses a different part of this module.
#![allow(dead_code)]
#![allow(unreachable_pub)]

use std::collections::{HashMap, HashSet};
use std::path::Path;

use reticle::fpga::arch::{ConfigBit, NodeId, PipId, RoutingGraph};
use reticle::fpga::uray::slice::{self, LUT_LETTERS, UNUSED_LUT_INIT};
use reticle::fpga::uray::{
    self, FabricInputs, FrameLayout, GridRegion, SiteTypePins, TileConn, TileGrid, TileTypeBits,
    TileTypeWiring, UrayFabric, processor,
};
use reticle::fpga::xc7::BitHeader;

/// Where `reticle fetch <name>` puts the pinned copy, if it is there.
pub fn fetched(name: &str, version: &str, probe: &str) -> Option<String> {
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
pub fn database(env: &str, name: &str, version: &str, probe: &str) -> Option<String> {
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

pub fn uraydb() -> Option<String> {
    database(
        "RETICLE_URAYDB",
        "prjuray-db",
        "9e7d3e7965240fd260d6549a1883a01a152ae490",
        "xazu7ev/tilegrid.json",
    )
}

pub fn wiringdb() -> Option<String> {
    database(
        "RETICLE_URAYWIRING",
        "prjuray-db-2020",
        "affbc5e555ebae16475f32e8fb2d6565d4204f3f",
        "zynqusp/xczu3eg-sfvc784-1-e/tileconn.json",
    )
}

pub fn reference() -> Option<Vec<u8>> {
    let Ok(dir) = std::env::var("RETICLE_ZCU104_REF") else {
        eprintln!(
            "skipped: needs Vivado's ZCU104 base overlay; set RETICLE_ZCU104_REF to a \
             directory holding base.bit. See docs/fpga-uray.md"
        );
        return None;
    };
    std::fs::read(Path::new(&dir).join("base.bit")).ok()
}

pub fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("`{path}` would not read: {e}"))
}

/// Everything the fabric is built from.
pub struct Inputs {
    pub grid: TileGrid,
    pub bits: HashMap<String, TileTypeBits>,
    pub wiring: HashMap<String, TileTypeWiring>,
    pub sites: HashMap<String, SiteTypePins>,
    pub rules: Vec<TileConn>,
}

pub fn inputs(bits_root: &str, wiring_root: &str) -> Inputs {
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
pub fn position(grid: &TileGrid, name: &str) -> (u32, u32) {
    grid.tiles()
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("the grid has no `{name}`"))
        .grid
}

/// The cheapest path of pips from any wire of `from`'s node to any wire
/// of `to`'s node, counting each pip with bits as one and each join as
/// nothing, never entering a node in `forbidden`. The pips with bits, in
/// order.
pub fn search(
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

/// The tile types whose lookup tables a bitstream must write: every
/// slice, used or not, gets either a design's contents or Vivado's unused
/// ones.
pub const SLICE_TYPES: [&str; 4] = ["CLEL_L", "CLEL_R", "CLEM", "CLEM_R"];

/// A small design on the processor's side of the fabric: the `PS8` site,
/// the interface column and interconnect columns X27 to X31, over the four
/// clock region rows the processor spans.
///
/// Nets are routed one at a time with [`search`], each kept off every node
/// an earlier net, a site output or a processor pin it does not name
/// already holds. Slice features are named as `segbits` spells them,
/// lookup tables are given whole contents, and [`Board::finish`] ties the
/// processor's other inputs (`uray::processor`), writes every other table
/// as Vivado leaves an unused one, checks the result and writes it.
pub struct Board<'a> {
    pub inputs: &'a Inputs,
    pub bits_root: String,
    pub fabric: UrayFabric,
    pub graph: RoutingGraph,
    pub roots: Vec<NodeId>,
    pub region: GridRegion,
    pub ps: (u32, u32),
    wire_at: HashMap<((u32, u32), String), NodeId>,
    tile_at: HashMap<(u32, u32), usize>,
    /// Nodes that already have a driver or must not get one.
    blocked: HashSet<NodeId>,
    /// Every node a net of the design occupies.
    used: HashSet<NodeId>,
    /// The pips with bits the design's nets turn on.
    pips: Vec<PipId>,
    /// Slice features, by grid tile index.
    features: Vec<(usize, String)>,
    /// Lookup table contents, by grid tile index and letter.
    luts: HashMap<(usize, char), u64>,
    /// `PS8` input nodes the design drives itself.
    driven_inputs: HashSet<NodeId>,
}

impl<'a> Board<'a> {
    /// The processor's side of the fabric, ready for nets.
    pub fn new(inputs: &'a Inputs, bits_root: &str) -> Board<'a> {
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
            },
            region,
        );
        let graph = fabric.arch.build_graph();
        let roots = processor::node_roots(&graph);
        let mut wire_at = HashMap::new();
        for (i, w) in graph.nodes.iter().enumerate() {
            wire_at.insert((w.tile, w.name.clone()), u32::try_from(i).unwrap());
        }
        let tile_at = grid
            .tiles()
            .iter()
            .enumerate()
            .map(|(i, t)| (t.grid, i))
            .collect();
        let mut board = Board {
            inputs,
            bits_root: bits_root.to_owned(),
            fabric,
            graph,
            roots,
            region,
            ps,
            wire_at,
            tile_at,
            blocked: HashSet::new(),
            used: HashSet::new(),
            pips: Vec::new(),
            features: Vec::new(),
            luts: HashMap::new(),
            driven_inputs: HashSet::new(),
        };
        // Every site output already has a driver, and every processor pin
        // is off limits until a net names it.
        for (pin_node, output, is_ps8) in board.site_pins() {
            if output || is_ps8 {
                board.blocked.insert(board.roots[pin_node as usize]);
            }
        }
        board
    }

    /// Every site pin of the region: its wire, whether the site drives it,
    /// and whether the site is the `PS8`.
    fn site_pins(&self) -> Vec<(NodeId, bool, bool)> {
        let mut out = Vec::new();
        for y in self.region.y0..=self.region.y1 {
            for x in self.region.x0..=self.region.x1 {
                let Some(index) = self.fabric.arch.tile_index_at(x, y) else {
                    continue;
                };
                let kind = uray::wiring_type(&self.fabric.database_type[index]);
                let Some(wiring) = self.inputs.wiring.get(kind) else {
                    continue;
                };
                for site in &wiring.sites {
                    let Some(pins) = self.inputs.sites.get(&site.site_type) else {
                        continue;
                    };
                    for (pin, wire) in &site.pins {
                        if let Some(&id) = self.wire_at.get(&((x, y), wire.clone())) {
                            out.push((id, pins.outputs.contains(pin), site.site_type == "PS8"));
                        }
                    }
                }
            }
        }
        out
    }

    /// The `PS8` pin's wire.
    pub fn ps8(&self, pin: &str) -> NodeId {
        let wiring = &self.inputs.wiring["PSS_ALTO"];
        let wire = wiring.sites[0]
            .pins
            .iter()
            .find(|(p, _)| p == pin)
            .map(|(_, w)| w.clone())
            .unwrap_or_else(|| panic!("the PS8 has no pin `{pin}`"));
        self.wire_at[&(self.ps, wire)]
    }

    /// EMIO GPIO output `k`, which Linux drives as `gpio{594 + k}`.
    pub fn emio_out(&self, k: u32) -> NodeId {
        self.ps8(&format!("FMIO_GPIO_OUT{k}"))
    }

    /// EMIO GPIO input `k`, which Linux reads as `gpio{594 + k}`.
    pub fn emio_in(&self, k: u32) -> NodeId {
        self.ps8(&format!("FMIO_GPIO_IN{k}"))
    }

    /// The grid tile index of the tile called `name`.
    pub fn tile(&self, name: &str) -> usize {
        self.inputs
            .grid
            .tiles()
            .iter()
            .position(|t| t.name == name)
            .unwrap_or_else(|| panic!("the grid has no `{name}`"))
    }

    /// The wire of slice pin `pin` in the slice tile called `tile`.
    pub fn slice_pin(&self, tile: &str, pin: &str) -> NodeId {
        let t = &self.inputs.grid.tiles()[self.tile(tile)];
        let wiring = &self.inputs.wiring[uray::wiring_type(&t.kind)];
        let wire = wiring.sites[0]
            .pins
            .iter()
            .find(|(p, _)| p == pin)
            .map(|(_, w)| w.clone())
            .unwrap_or_else(|| panic!("`{tile}` has no pin `{pin}`"));
        self.wire_at[&(t.grid, wire)]
    }

    /// Routes a net from `from` to `to`, keeping it off every node another
    /// net or a site holds. Returns the pips with bits, for a report.
    pub fn connect(&mut self, from: NodeId, to: NodeId) -> Vec<String> {
        let (source, sink) = (self.roots[from as usize], self.roots[to as usize]);
        let mut forbidden: HashSet<u32> = self.blocked.union(&self.used).copied().collect();
        forbidden.remove(&source);
        forbidden.remove(&sink);
        let route = search(&self.graph, &self.roots, from, to, &forbidden)
            .unwrap_or_else(|| panic!("no route from {} to {}", self.name(from), self.name(to)));
        self.used.insert(source);
        self.used.insert(sink);
        let mut report = Vec::new();
        for pip in &route {
            let p = &self.graph.pips[*pip as usize];
            self.used.insert(self.roots[p.from as usize]);
            self.used.insert(self.roots[p.to as usize]);
            report.push(format!(
                "{:?} {} -> {}",
                p.tile,
                self.graph.nodes[p.from as usize].name,
                self.graph.nodes[p.to as usize].name
            ));
        }
        self.pips.extend(route);
        let ps8_input = self.graph.nodes[to as usize].tile == self.ps;
        if ps8_input {
            self.driven_inputs.insert(sink);
        }
        report
    }

    fn name(&self, node: NodeId) -> String {
        let w = &self.graph.nodes[node as usize];
        format!("{:?}:{}", w.tile, w.name)
    }

    /// Turns on the slice feature `feature` (as `segbits` spells it, with
    /// no tile type: `SLICE_X0Y0.AFF.FFINIT=INIT0`) in the tile `tile`.
    pub fn feature(&mut self, tile: &str, feature: &str) {
        let index = self.tile(tile);
        let kind = &self.inputs.grid.tiles()[index].kind;
        assert!(
            self.inputs.bits[kind]
                .features
                .iter()
                .any(|f| f.name == feature),
            "`{kind}` has no feature `{feature}`"
        );
        self.features.push((index, feature.to_owned()));
    }

    /// Gives lookup table `letter` of the slice `tile` the contents `init`.
    pub fn lut(&mut self, tile: &str, letter: char, init: u64) {
        let index = self.tile(tile);
        self.luts.insert((index, letter), init);
    }

    /// Ties the processor's other inputs, writes every lookup table,
    /// checks the bitstream and writes `<name>.bit` and `<name>.bin` into
    /// `out_dir`.
    ///
    /// The checks: every set bit decodes to a feature; every lookup table
    /// reads back what it was given; and every pip the bits turn on is one
    /// the design chose, or shares all of a chosen one's bits (an interface
    /// tile's enable is shared by all of its outputs).
    pub fn finish(mut self, out_dir: &Path, name: &str) {
        let grid = &self.inputs.grid;
        // The tie-offs: every PS8 input Vivado would drive and this design
        // does not, from the lookup tables beside the interface column.
        let undriven = processor::left_undriven();
        let wiring = &self.inputs.wiring["PSS_ALTO"];
        let pins = &self.inputs.sites["PS8"];
        let mut sinks = Vec::new();
        let mut keep_off: HashSet<NodeId> = self.blocked.union(&self.used).copied().collect();
        for (pin, wire) in &wiring.sites[0].pins {
            if !pins.inputs.contains(pin) {
                continue;
            }
            let id = self.wire_at[&(self.ps, wire.clone())];
            let root = self.roots[id as usize];
            if undriven.contains(pin) || self.driven_inputs.contains(&root) {
                keep_off.insert(root);
            } else {
                keep_off.remove(&root);
                sinks.push(id);
            }
        }
        let int27 = position(grid, "INT_X27Y0").0;
        let mut sources = Vec::new();
        for (index, tile) in grid.tiles().iter().enumerate() {
            if tile.kind != "CLEL_R"
                || tile.grid.0 != int27 + 1
                || !self.region.contains(tile.grid.0, tile.grid.1)
            {
                continue;
            }
            for (l, letter) in LUT_LETTERS.iter().enumerate() {
                if self.luts.contains_key(&(index, *letter)) {
                    continue;
                }
                let wire = format!("CLE_CLE_L_SITE_0_{letter}_O");
                if let Some(&id) = self.wire_at.get(&(tile.grid, wire)) {
                    // A table this design uses elsewhere in the slice is
                    // still free: only its own output node is the source.
                    if self.used.contains(&self.roots[id as usize]) {
                        continue;
                    }
                    sources.push((id, index * 8 + l));
                    keep_off.remove(&self.roots[id as usize]);
                }
            }
        }
        let tie = processor::route_constant(&self.graph, &self.roots, &sources, &sinks, &keep_off);
        assert!(
            tie.unreached.is_empty(),
            "{} PS8 inputs could not be tied",
            tie.unreached.len()
        );
        for tag in &tie.sources {
            self.luts.insert((tag / 8, LUT_LETTERS[tag % 8]), 0);
        }
        eprintln!(
            "tied {} PS8 inputs with {} pips from {} lookup tables",
            sinks.len(),
            tie.pips.len(),
            tie.sources.len()
        );

        // Tile bits.
        let mut bits: Vec<(usize, ConfigBit)> = Vec::new();
        let chosen: Vec<PipId> = self.pips.iter().chain(&tie.pips).copied().collect();
        for pip in &chosen {
            let tile = self.tile_at[&self.graph.pips[*pip as usize].tile];
            bits.extend(self.graph.pip_bits(*pip).iter().map(|b| (tile, *b)));
        }
        for (index, feature) in &self.features {
            let kind = &grid.tiles()[*index].kind;
            let f = self.inputs.bits[kind]
                .features
                .iter()
                .find(|f| f.name == *feature)
                .unwrap();
            bits.extend(f.ones.iter().map(|b| (*index, *b)));
        }
        let mut layouts: HashMap<(String, char), [ConfigBit; 64]> = HashMap::new();
        for kind in SLICE_TYPES {
            for letter in LUT_LETTERS {
                layouts.insert(
                    (kind.to_owned(), letter),
                    slice::lut_init_bits(&self.inputs.bits[kind], letter).unwrap(),
                );
            }
        }
        let mut contents: HashMap<(usize, char), u64> = HashMap::new();
        for (index, tile) in grid.tiles().iter().enumerate() {
            for letter in LUT_LETTERS {
                let Some(layout) = layouts.get(&(tile.kind.clone(), letter)) else {
                    continue;
                };
                let value = self
                    .luts
                    .get(&(index, letter))
                    .copied()
                    .unwrap_or(UNUSED_LUT_INIT);
                contents.insert((index, letter), value);
                for (i, b) in layout.iter().enumerate() {
                    if value >> i & 1 == 1 {
                        bits.push((index, *b));
                    }
                }
            }
        }
        let layout = FrameLayout::from_grid(grid);
        let frames = uray::frames_from_tile_bits(grid, &layout, bits).unwrap();

        // The checks.
        let decoded = uray::decode(grid, &layout, &frames).unwrap();
        assert!(decoded.unowned.is_empty());
        let mut types = self.inputs.bits.clone();
        for kind in SLICE_TYPES {
            let stem = TileTypeBits::file_stem(kind).replacen("segbits_", "defaults_", 1);
            let path = format!("{}/{stem}.db", self.bits_root);
            let parsed = TileTypeBits::parse("", "none", Some((&read(&path), &path))).unwrap();
            types.entry(kind.to_owned()).or_default().defaults = parsed.defaults;
        }
        let explained = uray::explain(grid, &decoded, &types);
        assert_eq!(explained.unexplained_total(), 0, "{explained:?}");
        for ((index, letter), value) in &contents {
            let set: HashSet<_> = decoded
                .tiles
                .get(index)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            let layout = &layouts[&(grid.tiles()[*index].kind.clone(), *letter)];
            assert_eq!(slice::read_lut(layout, &|b| set.contains(&b)), *value);
        }
        let chosen_set: HashSet<PipId> = chosen.iter().copied().collect();
        let mut siblings = 0usize;
        for (index, pip) in self.graph.pips.iter().enumerate() {
            let id = PipId::try_from(index).unwrap();
            let pattern = self.graph.pip_bits(id);
            if pattern.is_empty() || chosen_set.contains(&id) {
                continue;
            }
            let Some(set) = decoded.tiles.get(&self.tile_at[&pip.tile]) else {
                continue;
            };
            if pattern.iter().all(|b| set.contains(b)) {
                let shares = chosen.iter().any(|c| {
                    self.graph.pips[*c as usize].tile == pip.tile
                        && self.graph.pip_bits(*c) == pattern
                });
                assert!(
                    shares,
                    "an unchosen pip is on: {}",
                    self.graph.nodes[pip.to as usize].name
                );
                siblings += 1;
            }
        }
        eprintln!(
            "{} set bits, every one explained; {siblings} pips on beside the chosen ones, each sharing a chosen one's bits",
            decoded.set_bits
        );

        let header = BitHeader::new(name, "xczu7ev-ffvc1156-2-e");
        let bit = uray::write_bit(&header, uray::IDCODE_XCZU7EV, &layout, &frames).unwrap();
        std::fs::write(out_dir.join(format!("{name}.bit")), &bit).unwrap();
        std::fs::write(
            out_dir.join(format!("{name}.bin")),
            uray::bin_from_bit(&bit).unwrap(),
        )
        .unwrap();
    }
}
