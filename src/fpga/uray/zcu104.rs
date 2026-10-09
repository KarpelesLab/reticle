//! Designs for the ZCU104: the processor's side of an XCZU7EV's fabric,
//! from a database on disk to a bitstream Linux can load.
//!
//! [`ZcuDatabase`] is everything the fabric is built from, read through a
//! [`FileProvider`]. [`Board`] is a design on the region beside the
//! processor: nets routed by hand or a design placed and routed by the
//! generic flow ([`Board::adopt`]), slice features and lookup table
//! contents, Vivado's clock network borrowed where Reticle has none of its
//! own ([`Board::replay`]), and [`Board::finish`], which ties the
//! processor's other inputs, gives every unused lookup table Vivado's
//! unused contents, holds the result to the project's checks and writes
//! it. [`implement`] is the whole flow for a synthesised design.
//!
//! What each step rests on, and what has run on the board, is in
//! `docs/fpga-uray.md`.

use std::collections::{HashMap, HashSet, VecDeque};

use super::processor;
use super::slice::{self, LUT_LETTERS, UNUSED_LUT_INIT};
use super::{
    FabricInputs, FrameLayout, GridRegion, SLICE_TYPES, SiteTypePins, TileConn, TileGrid,
    TileTypeBits, TileTypeWiring, UrayError, UrayFabric,
};
use crate::fpga::arch::{ConfigBit, NodeId, PipId, RoutingGraph};
use crate::fpga::xc7::BitHeader;
use crate::ir::memfile::FileProvider;

/// The die directory of the 2026 database for an XCZU7EV.
pub const DIE: &str = "xazu7ev";

/// The default eastern edge of the region, as an interconnect column.
pub const DEFAULT_EAST: u32 = 31;

/// The furthest east the region may reach. Interconnect column 37 is east
/// of an UltraRAM column, which the ZU3EG whose rules join the wires does
/// not have, so no data wire of the model crosses it and slices beyond it
/// cannot send a signal back (`docs/fpga-uray.md`, "Where it still
/// fails"). Track 14 carries `PL_CLK0` in every clock row beside the
/// processor up to here.
pub const MAX_EAST: u32 = 36;

/// The horizontal distribution track that carries `PL_CLK0` in Vivado's
/// clock network for the PYNQ base overlay. Measured on the ZCU104 at
/// 100.001 MHz (`docs/fpga-uray.md`).
pub const PL_CLK0_TRACK: u32 = 14;

fn err(message: impl Into<String>) -> UrayError {
    UrayError::Malformed {
        path: "zcu104".to_owned(),
        message: message.into(),
    }
}

/// Everything the fabric is built from: the 2026 database's tile grid and
/// each tile type's bits and defaults, and the 2020 snapshot's tile types,
/// site types and ZU3EG wiring rules.
pub struct ZcuDatabase {
    /// The die's tile grid.
    pub grid: TileGrid,
    /// Each database tile type's features and defaults.
    pub bits: HashMap<String, TileTypeBits>,
    /// Each wiring tile type's wires, sites and buffered pips.
    pub wiring: HashMap<String, TileTypeWiring>,
    /// Each site type's pin directions.
    pub sites: HashMap<String, SiteTypePins>,
    /// The ZU3EG's `tileconn.json`.
    pub rules: Vec<TileConn>,
}

impl ZcuDatabase {
    /// Reads the database: the 2026 one at `bits_root` (the directory
    /// holding `xazu7ev/` and the `segbits_*.db` files) and the 2020
    /// snapshot at `wiring_root` (the directory holding `zynqusp/`).
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a file that is missing where it must
    /// not be, or does not parse.
    pub fn load(
        files: &dyn FileProvider,
        bits_root: &str,
        wiring_root: &str,
    ) -> Result<ZcuDatabase, UrayError> {
        let read = |path: &str| {
            files
                .read_file(path)
                .ok_or_else(|| err(format!("`{path}` is not there")))
        };
        let grid_path = format!("{bits_root}/{DIE}/tilegrid.json");
        let grid = TileGrid::parse(&read(&grid_path)?, &grid_path)?;
        let mut bits = HashMap::new();
        let mut wiring = HashMap::new();
        let mut kinds: Vec<&str> = grid.tiles().iter().map(|t| t.kind.as_str()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        for kind in kinds {
            let stem = TileTypeBits::file_stem(kind);
            let seg = format!("{bits_root}/{stem}.db");
            if let Some(text) = files.read_file(&seg) {
                let dflt = format!(
                    "{bits_root}/{}.db",
                    stem.replacen("segbits_", "defaults_", 1)
                );
                let defaults = files.read_file(&dflt);
                bits.insert(
                    kind.to_owned(),
                    TileTypeBits::parse(
                        &text,
                        &seg,
                        defaults.as_deref().map(|d| (d, dflt.as_str())),
                    )?,
                );
            }
            let wiring_kind = super::wiring_type(kind);
            let path = format!("{wiring_root}/zynqusp/tile_types/tile_type_{wiring_kind}.json");
            if let Some(text) = files.read_file(&path) {
                wiring.insert(wiring_kind.to_owned(), TileTypeWiring::parse(&text, &path)?);
            }
        }
        let mut sites = HashMap::new();
        let site_types: Vec<String> = wiring
            .values()
            .flat_map(|w| w.sites.iter().map(|s| s.site_type.clone()))
            .collect();
        for site_type in site_types {
            if sites.contains_key(&site_type) {
                continue;
            }
            let path = format!("{wiring_root}/zynqusp/site_types/site_type_{site_type}.json");
            if let Some(text) = files.read_file(&path) {
                sites.insert(site_type, SiteTypePins::parse(&text, &path)?);
            }
        }
        let conn = format!("{wiring_root}/zynqusp/xczu3eg-sfvc784-1-e/tileconn.json");
        let rules = super::parse_tileconn(&read(&conn)?, &conn)?;
        Ok(ZcuDatabase {
            grid,
            bits,
            wiring,
            sites,
            rules,
        })
    }

    /// The grid position of the tile called `name`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] when the grid has no such tile.
    pub fn position(&self, name: &str) -> Result<(u32, u32), UrayError> {
        self.grid
            .tiles()
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.grid)
            .ok_or_else(|| err(format!("the grid has no `{name}`")))
    }
}

/// The cheapest path of pips from any wire of `from`'s node to any wire of
/// `to`'s node, counting each pip with bits as one and each join as
/// nothing, never entering a node in `forbidden`: the pips with bits, in
/// order.
pub fn search(
    graph: &RoutingGraph,
    roots: &[NodeId],
    from: NodeId,
    to: NodeId,
    forbidden: &HashSet<NodeId>,
) -> Option<Vec<PipId>> {
    let target = roots[to as usize];
    let mut cost = vec![u32::MAX; graph.nodes.len()];
    let mut came: Vec<Option<PipId>> = vec![None; graph.nodes.len()];
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
            let c = cost[wire as usize].saturating_add(step);
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

/// The clock-distribution tile types to borrow from Vivado: the fabric's
/// clock rows and the processor's clock buffers.
///
/// Never the I/O banks' clock rows: they share frames with tiles that drive
/// the board's pins, and replaying them set bits inside `HDIO_BOT_RIGHT`
/// and `XIPHY_BYTE_L`. With `leaves` false, not the leaf tiles
/// (`RCLK_INT_*`) either: a design placed by the generic flow turns its
/// own leaves on.
pub fn clock_tile_types(grid: &TileGrid, leaves: bool) -> Vec<String> {
    let mut types: Vec<String> = grid
        .tiles()
        .iter()
        .map(|t| t.kind.clone())
        .filter(|k| {
            (k.starts_with("RCLK_") || k.contains("INTF_LEFT_TERM_DA6"))
                && !["HDIO", "HPIO", "XIPHY", "_IO", "AMS", "GT", "PCIE"]
                    .iter()
                    .any(|io| k.contains(io))
                && (leaves || !k.starts_with("RCLK_INT_"))
        })
        .collect();
    types.sort_unstable();
    types.dedup();
    types
}

/// A finished bitstream: the `.bit`, the `.bin` the FPGA manager loads,
/// and what the checks found.
#[derive(Clone, Debug)]
pub struct Written {
    /// The `.bit` file.
    pub bit: Vec<u8>,
    /// The `.bin` for `/lib/firmware`.
    pub bin: Vec<u8>,
    /// One line per fact worth reporting.
    pub report: Vec<String>,
}

/// A design on the processor's side of the fabric: the `PS8` site, the
/// interface column and interconnect columns X27 to X31, over the four
/// clock region rows the processor spans.
pub struct Board<'a> {
    /// The database.
    pub db: &'a ZcuDatabase,
    /// The fabric of the region.
    pub fabric: UrayFabric,
    /// Its routing graph.
    pub graph: RoutingGraph,
    /// Each wire's node, by its lowest wire.
    pub roots: Vec<NodeId>,
    /// The region.
    pub region: GridRegion,
    /// The `PSS_ALTO` tile's position.
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
    /// Bits copied verbatim from Vivado's bitstream, by grid tile index.
    replayed: HashMap<usize, Vec<ConfigBit>>,
    /// Nodes the replayed bits drive, which a net may only start from.
    replay_driven: HashSet<NodeId>,
    /// Bits of a design placed and routed by the generic flow.
    design_bits: Vec<(usize, ConfigBit)>,
    /// Lookup tables that design occupies.
    occupied: HashSet<(usize, char)>,
}

impl<'a> Board<'a> {
    /// The processor's side of the fabric, ready for nets. With `track`,
    /// the processor's `PL_CLK0` is a global wire reaching every leaf clock
    /// of the region on that distribution track ([`super::PL_CLK0`]).
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] when the grid lacks a tile the region is
    /// defined by.
    pub fn new(db: &'a ZcuDatabase, track: Option<u32>) -> Result<Board<'a>, UrayError> {
        Board::reaching(db, track, DEFAULT_EAST)
    }

    /// As [`Board::new`], with the region reaching east to interconnect
    /// column `east` (at most [`MAX_EAST`]).
    ///
    /// # Errors
    ///
    /// As [`Board::new`], and for an `east` outside 27 to [`MAX_EAST`].
    pub fn reaching(
        db: &'a ZcuDatabase,
        track: Option<u32>,
        east: u32,
    ) -> Result<Board<'a>, UrayError> {
        if !(27..=MAX_EAST).contains(&east) {
            return Err(err(format!(
                "the region can reach east to interconnect column 27 to {MAX_EAST}, not {east}"
            )));
        }
        let ps = db.position("PSS_ALTO_X0Y60")?;
        let low = db.position(&format!("INT_X{east}Y0"))?;
        let high = db.position("INT_X27Y239")?;
        let region = GridRegion::new(
            ps.0,
            high.1.min(low.1),
            low.0.max(high.0),
            high.1.max(low.1),
        );
        let fabric = super::build_arch(
            &FabricInputs {
                grid: &db.grid,
                bits: &db.bits,
                wiring: &db.wiring,
                rules: &db.rules,
                processor_clock_track: track,
            },
            region,
        );
        let graph = fabric.arch.build_graph();
        let roots = processor::node_roots(&graph);
        let mut wire_at = HashMap::new();
        for (i, w) in graph.nodes.iter().enumerate() {
            wire_at.insert(
                (w.tile, w.name.clone()),
                NodeId::try_from(i).unwrap_or(NodeId::MAX),
            );
        }
        let tile_at = db
            .grid
            .tiles()
            .iter()
            .enumerate()
            .map(|(i, t)| (t.grid, i))
            .collect();
        let mut board = Board {
            db,
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
            replayed: HashMap::new(),
            replay_driven: HashSet::new(),
            design_bits: Vec::new(),
            occupied: HashSet::new(),
        };
        // Every site output already has a driver, and every processor pin
        // is off limits until a net names it.
        for (pin_node, output, is_ps8) in board.site_pins() {
            if output || is_ps8 {
                board.blocked.insert(board.roots[pin_node as usize]);
            }
        }
        Ok(board)
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
                let kind = super::wiring_type(&self.fabric.database_type[index]);
                let Some(wiring) = self.db.wiring.get(kind) else {
                    continue;
                };
                for site in &wiring.sites {
                    let Some(pins) = self.db.sites.get(&site.site_type) else {
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

    fn ps8_pins(&self) -> Result<&'a [(String, String)], UrayError> {
        self.db
            .wiring
            .get("PSS_ALTO")
            .and_then(|w| w.sites.first())
            .map(|s| s.pins.as_slice())
            .ok_or_else(|| err("the 2020 snapshot has no PSS_ALTO site"))
    }

    /// The `PS8` pin's wire.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a pin the `PS8` does not have.
    pub fn ps8(&self, pin: &str) -> Result<NodeId, UrayError> {
        let wire = self
            .ps8_pins()?
            .iter()
            .find(|(p, _)| p == pin)
            .map(|(_, w)| w.clone())
            .ok_or_else(|| err(format!("the PS8 has no pin `{pin}`")))?;
        self.wire_at
            .get(&(self.ps, wire))
            .copied()
            .ok_or_else(|| err(format!("the fabric has no wire for PS8 pin `{pin}`")))
    }

    /// EMIO GPIO output `k`, which Linux drives as `gpio{594 + k}`.
    ///
    /// # Errors
    ///
    /// As [`Board::ps8`].
    pub fn emio_out(&self, k: u32) -> Result<NodeId, UrayError> {
        self.ps8(&format!("FMIO_GPIO_OUT{k}"))
    }

    /// EMIO GPIO input `k`, which Linux reads as `gpio{594 + k}`.
    ///
    /// # Errors
    ///
    /// As [`Board::ps8`].
    pub fn emio_in(&self, k: u32) -> Result<NodeId, UrayError> {
        self.ps8(&format!("FMIO_GPIO_IN{k}"))
    }

    /// The grid tile index of the tile called `name`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] when the grid has no such tile.
    pub fn tile(&self, name: &str) -> Result<usize, UrayError> {
        self.db
            .grid
            .tiles()
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| err(format!("the grid has no `{name}`")))
    }

    /// The root of `node`'s node.
    pub fn root(&self, node: NodeId) -> NodeId {
        self.roots[node as usize]
    }

    /// Nodes the generic router must not enter, by wire id: every node the
    /// replayed bits drive, and every processor input but those whose
    /// roots are in `allowed` (the design's own EMIO inputs). Give it to
    /// `route` as `RouteOptions::network`, with no signal allowed onto it.
    ///
    /// # Errors
    ///
    /// As [`Board::ps8`].
    pub fn closed_nodes(&self, allowed: &HashSet<NodeId>) -> Result<Vec<bool>, UrayError> {
        let mut closed: HashSet<NodeId> = self.replay_driven.clone();
        let inputs = self.db.sites.get("PS8").map(|p| &p.inputs);
        for (pin, wire) in self.ps8_pins()? {
            if !inputs.is_some_and(|i| i.contains(pin)) {
                continue;
            }
            if let Some(&id) = self.wire_at.get(&(self.ps, wire.clone())) {
                let root = self.roots[id as usize];
                if !allowed.contains(&root) {
                    closed.insert(root);
                }
            }
        }
        Ok(self.roots.iter().map(|r| closed.contains(r)).collect())
    }

    /// Takes over a design the generic flow placed and routed on this
    /// board's fabric: its routes' nodes become used, its pips with bits
    /// become the design's, every bit `bitstream` set is the design's, and
    /// the lookup tables it occupies keep the contents it gave them.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a bit in a tile outside the grid.
    pub fn adopt(
        &mut self,
        netlist: &crate::fpga::Netlist,
        placement: &crate::fpga::Placement,
        routing: &crate::fpga::Routing,
        bitstream: &crate::fpga::Bitstream,
    ) -> Result<(), UrayError> {
        for route in routing.routes() {
            for node in &route.nodes {
                self.used.insert(self.roots[*node as usize]);
            }
            self.pips.extend(route.pips.iter().copied());
            if let Some(sink) = route
                .nodes
                .iter()
                .find(|n| self.graph.nodes[**n as usize].tile == self.ps)
            {
                self.driven_inputs.insert(self.roots[*sink as usize]);
            }
        }
        for (format, bits) in bitstream.used_tiles() {
            let index = *self
                .tile_at
                .get(&format.tile)
                .ok_or_else(|| err(format!("no tile at {:?}", format.tile)))?;
            self.design_bits
                .extend(bits.into_iter().map(|b| (index, b)));
        }
        for (instance, _) in netlist.instances.iter().enumerate() {
            let Some(site) = placement.site_of(instance) else {
                continue;
            };
            let site = &self.graph.sites[site];
            if let Some(letter) = site.bel.strip_suffix("6LUT").and_then(|l| l.chars().next())
                && let Some(&index) = self.tile_at.get(&site.tile)
            {
                self.occupied.insert((index, letter));
            }
            // A placed bel's output already has a driver.
            for (role, node) in &site.pins {
                if matches!(role.as_str(), "o" | "q" | "din") {
                    self.blocked.insert(self.roots[*node as usize]);
                }
            }
        }
        Ok(())
    }

    /// The wire of slice pin `pin` in the slice tile called `tile`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a tile or pin that does not exist.
    pub fn slice_pin(&self, tile: &str, pin: &str) -> Result<NodeId, UrayError> {
        let t = &self.db.grid.tiles()[self.tile(tile)?];
        let wire = self
            .db
            .wiring
            .get(super::wiring_type(&t.kind))
            .and_then(|w| w.sites.first())
            .and_then(|s| s.pins.iter().find(|(p, _)| p == pin))
            .map(|(_, w)| w.clone())
            .ok_or_else(|| err(format!("`{tile}` has no pin `{pin}`")))?;
        self.wire_at
            .get(&(t.grid, wire))
            .copied()
            .ok_or_else(|| err(format!("`{tile}` is not in the region")))
    }

    /// The node of wire `wire` in the tile called `tile`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a tile or wire not in the fabric.
    pub fn wire(&self, tile: &str, wire: &str) -> Result<NodeId, UrayError> {
        let t = &self.db.grid.tiles()[self.tile(tile)?];
        self.wire_at
            .get(&(t.grid, wire.to_owned()))
            .copied()
            .ok_or_else(|| err(format!("`{tile}` has no wire `{wire}` in the fabric")))
    }

    /// The tiles holding a wire of `node`'s node.
    pub fn node_tiles(&self, node: NodeId) -> Vec<(u32, u32)> {
        let root = self.roots[node as usize];
        let mut out: Vec<(u32, u32)> = (0..self.graph.nodes.len())
            .filter(|&i| self.roots[i] == root)
            .map(|i| self.graph.nodes[i].tile)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    fn name(&self, node: NodeId) -> String {
        let w = &self.graph.nodes[node as usize];
        format!("{:?}:{}", w.tile, w.name)
    }

    /// Routes a net from `from` to `to`, keeping it off every node another
    /// net or a site holds. Returns the pips with bits, for a report.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] when no route exists, or `to` is driven by
    /// replayed bits.
    pub fn connect(&mut self, from: NodeId, to: NodeId) -> Result<Vec<String>, UrayError> {
        let (source, sink) = (self.roots[from as usize], self.roots[to as usize]);
        if self.replay_driven.contains(&sink) {
            return Err(err(format!(
                "{} is driven by the replayed bits",
                self.name(to)
            )));
        }
        let mut forbidden: HashSet<NodeId> = self.blocked.union(&self.used).copied().collect();
        forbidden.remove(&source);
        forbidden.remove(&sink);
        let route = search(&self.graph, &self.roots, from, to, &forbidden).ok_or_else(|| {
            err(format!(
                "no route from {} to {}",
                self.name(from),
                self.name(to)
            ))
        })?;
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
        if self.graph.nodes[to as usize].tile == self.ps {
            self.driven_inputs.insert(sink);
        }
        Ok(report)
    }

    /// Copies, verbatim, every bit Vivado set in every tile whose type is
    /// one of `types`, from `bitstream`; returns how many. Every node a pip
    /// of those bits drives is then off limits, except as a net's source.
    ///
    /// This is how a design borrows what Reticle cannot yet build: the
    /// clock network beside the processor, whose buffers no database
    /// describes on this die. The copied tiles are exempt from
    /// [`Board::finish`]'s checks, which say how many there were.
    ///
    /// # Errors
    ///
    /// [`UrayError`] for a bitstream that does not read or decode.
    pub fn replay(&mut self, bitstream: &[u8], types: &[String]) -> Result<usize, UrayError> {
        let grid = &self.db.grid;
        let layout = FrameLayout::from_grid(grid);
        let bit = super::read_bit(bitstream)?;
        let decoded = super::decode(grid, &layout, &bit.frames)?;
        let mut count = 0;
        for (index, bits) in &decoded.tiles {
            if types.contains(&grid.tiles()[*index].kind) {
                count += bits.len();
                self.replayed.insert(*index, bits.clone());
            }
        }
        for (id, pip) in self.graph.pips.iter().enumerate() {
            let pattern = self
                .graph
                .pip_bits(PipId::try_from(id).unwrap_or(PipId::MAX));
            let Some(&index) = self.tile_at.get(&pip.tile) else {
                continue;
            };
            let Some(bits) = self.replayed.get(&index) else {
                continue;
            };
            if !pattern.is_empty() && pattern.iter().all(|b| bits.contains(b)) {
                let root = self.roots[pip.to as usize];
                self.replay_driven.insert(root);
                self.blocked.insert(root);
            }
        }
        Ok(count)
    }

    /// Turns on the slice feature `feature`, as `segbits` spells it with no
    /// tile type (`SLICE_X0Y0.AFF.FFINIT=INIT0`), in the tile `tile`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a tile or feature that does not exist.
    pub fn feature(&mut self, tile: &str, feature: &str) -> Result<(), UrayError> {
        let index = self.tile(tile)?;
        let kind = &self.db.grid.tiles()[index].kind;
        let known = self
            .db
            .bits
            .get(kind)
            .is_some_and(|b| b.features.iter().any(|f| f.name == feature));
        if !known {
            return Err(err(format!("`{kind}` has no feature `{feature}`")));
        }
        self.features.push((index, feature.to_owned()));
        Ok(())
    }

    /// Gives lookup table `letter` of the slice `tile` the contents `init`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a tile that does not exist.
    pub fn lut(&mut self, tile: &str, letter: char, init: u64) -> Result<(), UrayError> {
        let index = self.tile(tile)?;
        self.luts.insert((index, letter), init);
        Ok(())
    }

    /// Ties the processor's other inputs, writes every lookup table, checks
    /// the bitstream and gives it back with the `.bin` and a report.
    ///
    /// The checks: every set bit outside the replayed tiles decodes to a
    /// feature; every lookup table this pass wrote reads back what it was
    /// given; and every pip the bits turn on is one the design chose or is
    /// made of a chosen one's bits (an interface tile's shared enable, a
    /// leaf clock pip's two features).
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] naming the check that failed.
    pub fn finish(mut self, design_name: &str) -> Result<Written, UrayError> {
        let grid = &self.db.grid;
        let mut report = Vec::new();

        // The tie-offs: every PS8 input Vivado would drive and this design
        // does not, from the lookup tables beside the interface column.
        let undriven = processor::left_undriven();
        let ps8_inputs = self
            .db
            .sites
            .get("PS8")
            .map(|p| p.inputs.clone())
            .unwrap_or_default();
        let mut sinks = Vec::new();
        let mut keep_off: HashSet<NodeId> = self.blocked.union(&self.used).copied().collect();
        for (pin, wire) in self.ps8_pins()? {
            if !ps8_inputs.contains(pin) {
                continue;
            }
            let Some(&id) = self.wire_at.get(&(self.ps, wire.clone())) else {
                continue;
            };
            let root = self.roots[id as usize];
            if undriven.contains(pin) || self.driven_inputs.contains(&root) {
                keep_off.insert(root);
            } else {
                keep_off.remove(&root);
                sinks.push(id);
            }
        }
        let int27 = self.db.position("INT_X27Y0")?.0;
        let mut sources = Vec::new();
        for (index, tile) in grid.tiles().iter().enumerate() {
            if tile.kind != "CLEL_R"
                || tile.grid.0 != int27 + 1
                || !self.region.contains(tile.grid.0, tile.grid.1)
            {
                continue;
            }
            for (l, letter) in LUT_LETTERS.iter().enumerate() {
                if self.luts.contains_key(&(index, *letter))
                    || self.occupied.contains(&(index, *letter))
                {
                    continue;
                }
                let wire = format!("CLE_CLE_L_SITE_0_{letter}_O");
                if let Some(&id) = self.wire_at.get(&(tile.grid, wire)) {
                    if self.used.contains(&self.roots[id as usize]) {
                        continue;
                    }
                    sources.push((id, index * 8 + l));
                    keep_off.remove(&self.roots[id as usize]);
                }
            }
        }
        let tie = processor::route_constant(&self.graph, &self.roots, &sources, &sinks, &keep_off);
        if !tie.unreached.is_empty() {
            return Err(err(format!(
                "{} PS8 inputs could not be tied to zero",
                tie.unreached.len()
            )));
        }
        for tag in &tie.sources {
            self.luts.insert((tag / 8, LUT_LETTERS[tag % 8]), 0);
        }
        report.push(format!(
            "tied {} PS8 inputs to zero with {} pips from {} lookup tables",
            sinks.len(),
            tie.pips.len(),
            tie.sources.len()
        ));

        // Tile bits.
        let mut bits: Vec<(usize, ConfigBit)> = Vec::new();
        let chosen: Vec<PipId> = self.pips.iter().chain(&tie.pips).copied().collect();
        for pip in &chosen {
            if let Some(&tile) = self.tile_at.get(&self.graph.pips[*pip as usize].tile) {
                bits.extend(self.graph.pip_bits(*pip).iter().map(|b| (tile, *b)));
            }
        }
        for (index, replayed) in &self.replayed {
            bits.extend(replayed.iter().map(|b| (*index, *b)));
        }
        bits.extend(self.design_bits.iter().copied());
        for (index, feature) in &self.features {
            let kind = &grid.tiles()[*index].kind;
            if let Some(f) = self
                .db
                .bits
                .get(kind)
                .and_then(|b| b.features.iter().find(|f| f.name == *feature))
            {
                bits.extend(f.ones.iter().map(|b| (*index, *b)));
            }
        }
        let mut layouts: HashMap<(String, char), [ConfigBit; 64]> = HashMap::new();
        for kind in SLICE_TYPES {
            let Some(type_bits) = self.db.bits.get(kind) else {
                continue;
            };
            for letter in LUT_LETTERS {
                layouts.insert(
                    (kind.to_owned(), letter),
                    slice::lut_init_bits(type_bits, letter)?,
                );
            }
        }
        let mut contents: HashMap<(usize, char), u64> = HashMap::new();
        for (index, tile) in grid.tiles().iter().enumerate() {
            for letter in LUT_LETTERS {
                let Some(layout) = layouts.get(&(tile.kind.clone(), letter)) else {
                    continue;
                };
                // A table the adopted design placed has its contents in
                // the design's bits already.
                if self.occupied.contains(&(index, letter)) {
                    continue;
                }
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
        let frames = super::frames_from_tile_bits(grid, &layout, bits)?;

        // The checks.
        let decoded = super::decode(grid, &layout, &frames)?;
        if !decoded.unowned.is_empty() {
            return Err(err(format!(
                "{} set bits belong to no tile",
                decoded.unowned.len()
            )));
        }
        let mut own = decoded.clone();
        own.tiles
            .retain(|index, _| !self.replayed.contains_key(index));
        let explained = super::explain(grid, &own, &self.db.bits);
        if explained.unexplained_total() != 0 {
            return Err(err(format!(
                "{} set bits are not a feature of the database: {:?}",
                explained.unexplained_total(),
                explained.unexplained
            )));
        }
        for ((index, letter), value) in &contents {
            let set: HashSet<_> = decoded
                .tiles
                .get(index)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            let Some(layout) = layouts.get(&(grid.tiles()[*index].kind.clone(), *letter)) else {
                continue;
            };
            let read = slice::read_lut(layout, &|b| set.contains(&b));
            if read != *value {
                return Err(err(format!(
                    "{}.{letter}6LUT reads {read:#018x}, given {value:#018x}",
                    grid.tiles()[*index].name
                )));
            }
        }
        let chosen_set: HashSet<PipId> = chosen.iter().copied().collect();
        let mut siblings = 0usize;
        for (index, pip) in self.graph.pips.iter().enumerate() {
            let id = PipId::try_from(index).unwrap_or(PipId::MAX);
            let pattern = self.graph.pip_bits(id);
            if pattern.is_empty() || chosen_set.contains(&id) {
                continue;
            }
            let Some(set) = self.tile_at.get(&pip.tile).and_then(|t| own.tiles.get(t)) else {
                continue;
            };
            if pattern.iter().all(|b| set.contains(b)) {
                let shares = chosen.iter().any(|c| {
                    self.graph.pips[*c as usize].tile == pip.tile
                        && pattern.iter().all(|b| self.graph.pip_bits(*c).contains(b))
                });
                if !shares {
                    return Err(err(format!(
                        "a pip the design did not choose is on: {}",
                        self.name(pip.to)
                    )));
                }
                siblings += 1;
            }
        }
        report.push(format!(
            "{} bits replayed verbatim from Vivado's clock network in {} tiles",
            self.replayed.values().map(Vec::len).sum::<usize>(),
            self.replayed.len()
        ));
        report.push(format!(
            "{} set bits, every one outside the replayed tiles a feature; {siblings} pips on \
             beside the chosen ones, each made of a chosen one's bits",
            decoded.set_bits
        ));

        let header = BitHeader::new(design_name, "xczu7ev-ffvc1156-2-e");
        let bit = super::write_bit(&header, super::IDCODE_XCZU7EV, &layout, &frames)?;
        let bin = super::bin_from_bit(&bit)?;
        Ok(Written { bit, bin, report })
    }
}

/// How [`implement`] runs.
pub struct ImplementOptions<'a> {
    /// The PYNQ base overlay's `base.bit`, whose clock network is borrowed.
    pub oracle: &'a [u8],
    /// The placer's options.
    pub place: &'a crate::fpga::PlaceOptions,
    /// How far east the region reaches, as an interconnect column
    /// ([`DEFAULT_EAST`] to [`MAX_EAST`]).
    pub east: u32,
}

/// The whole flow for a design `synthesize_for` has mapped onto
/// `xczu7ev-ffvc1156`: place and route it on the processor's side of the
/// fabric, with `PL_CLK0` reaching its flip-flops through Vivado's clock
/// network replayed from the PYNQ base overlay's `base.bit`, tie the
/// processor's other inputs, and write it; see [`ImplementOptions`].
///
/// # Errors
///
/// A message naming the step that failed.
#[cfg(feature = "synth")]
pub fn implement(
    db: &ZcuDatabase,
    design: &crate::ir::Design,
    top: crate::ir::ModuleId,
    device: &crate::fpga::Device,
    constraints: &crate::fpga::Constraints,
    options: &ImplementOptions<'_>,
) -> Result<Written, String> {
    let (oracle, place_options, east) = (options.oracle, options.place, options.east);
    use crate::fpga::route::{RouteOptions, route};
    use crate::fpga::{Netlist, bitstream, place};

    let mut board = Board::reaching(db, Some(PL_CLK0_TRACK), east).map_err(|e| e.to_string())?;
    let replayed = board
        .replay(oracle, &clock_tile_types(&db.grid, false))
        .map_err(|e| format!("the base overlay's clock network: {e}"))?;
    let netlist = Netlist::build(design, top, device, &board.graph).map_err(|e| e.to_string())?;
    let (placement, _) = place(
        &netlist,
        &board.fabric.arch,
        &board.graph,
        constraints,
        place_options,
    )
    .map_err(|e| e.to_string())?;
    // The design's own EMIO inputs may be driven; every other processor
    // input, and every node Vivado's clock network drives, is closed.
    let mut allowed = HashSet::new();
    for k in 0..super::EMIO_LINES {
        allowed.insert(board.root(board.emio_in(k).map_err(|e| e.to_string())?));
    }
    let options = RouteOptions {
        network: board.closed_nodes(&allowed).map_err(|e| e.to_string())?,
        network_signals: vec![false; netlist.signals.len()],
        ..RouteOptions::default()
    };
    let (routing, routed) =
        route(&netlist, &board.graph, &placement, &options).map_err(|e| e.to_string())?;
    let problems = routing.verify(&netlist, &board.graph, &placement);
    if !problems.is_empty() {
        return Err(format!(
            "the routing does not implement the netlist, so nothing was written: {}",
            problems.join("; ")
        ));
    }
    let bits = bitstream::generate(
        design,
        top,
        &board.fabric.arch,
        &board.graph,
        &netlist,
        &placement,
        &routing,
    )
    .map_err(|e| e.to_string())?;
    board
        .adopt(&netlist, &placement, &routing, &bits)
        .map_err(|e| e.to_string())?;
    let name = design
        .modules
        .get(top)
        .map_or_else(|| "top".to_owned(), |m| m.name.as_str().to_owned());
    let mut written = board.finish(&name).map_err(|e| e.to_string())?;
    written.report.insert(
        0,
        format!(
            "placed {} instances; routed {} signals with {} pips; borrowed {replayed} bits of \
             Vivado's clock network",
            netlist.instances.len(),
            routed.signals,
            routed.pips
        ),
    );
    Ok(written)
}
