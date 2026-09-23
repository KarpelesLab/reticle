//! The routing architecture: tiles, wires, programmable interconnect
//! points and the sites a design is placed onto.
//!
//! [`device`](super::device) says *what* a part offers — LUT size,
//! flip-flop variants, block RAM shapes, package pins. It says nothing
//! about how a signal gets from one of them to another. That is this
//! module: an [`Arch`] describes a family's fabric as data, and
//! [`Arch::build_graph`] expands it into a [`RoutingGraph`], the directed
//! graph the placer, the router and the bitstream writer all work on.
//!
//! # The model
//!
//! A part is a rectangular grid of **tiles**. Every tile has a **tile
//! type**, and a tile type is declared once and used by every tile of
//! that type, which is what keeps the description small: a 14 x 18 grid
//! with three tile types is three declarations and a few rectangles, not
//! 252 copies.
//!
//! A tile type declares:
//!
//! - **wires**, each with a *span*: `span 0 0` is a wire local to one
//!   tile, `span 4 0` a wire that starts in its tile and reaches four
//!   tiles east, `span 0 -12` one that reaches twelve tiles south. A
//!   wire declared in a tile type becomes one graph **node** per tile of
//!   that type, identified by the tile it starts in; the tiles it crosses
//!   can reach the same node.
//! - **pips**, the programmable interconnect points: a directed
//!   connection from one wire to another, belonging to one tile, with the
//!   configuration bits that switch it on.
//! - **bels**, the placeable elements: a name, a kind (`lut`, `ff`,
//!   `carry`, `io`, `gb`, `bram`), and which wire each of its pins
//!   reaches. A bel becomes one **site** per tile of that type, named
//!   `X<x>Y<y>/<bel>`.
//! - **configuration entries** for a bel: which bits to set when a cell
//!   of a given primitive sits on it, and which bit carries bit *n* of
//!   one of its parameters. This is how `LUT_INIT` reaches the
//!   bitstream without a line of Rust knowing what a LUT is.
//!
//! Wires that reach the whole die (the global clock network) are declared
//! once at the top of the file and are one node each for the whole part.
//!
//! A **wire reference** inside a pip or a bel pin says which node is
//! meant: `local0` is the wire of that name starting in this tile,
//! `sp4e@-4,0` the one starting four tiles west (so it arrives here), and
//! `*glb0` the global wire `glb0`.
//!
//! # The `.arch` text format
//!
//! One directive per line, whitespace separated, `#` or `//` comments,
//! sharing the tokenizer of the `.dev` and `.rcf` formats. The grammar is
//! deliberately flat: an `arch` block has exactly one `end`, and a tile
//! type's members name their tile type rather than nesting, so a file may
//! hold `device` blocks and `arch` blocks side by side and either parser
//! skips what is not its business.
//!
//! ```text
//! arch ice40-synthetic
//!   family ice40
//!   part ice40-hx1k-tq144
//!   grid 14 18
//!   asc_device 1k
//!   global glb0
//!   tiletype logic asc logic_tile bits 16 54
//!   wire logic local0 span 0 0
//!   wire logic sp4e span 4 0
//!   wire logic lc0_i0 span 0 0
//!   bel logic lc0_lut lut pin i0=lc0_i0 pin o=lc0_out
//!   config logic lc0_lut cell SB_LUT4 bits 3.20
//!   config logic lc0_lut param LUT_INIT 0 4.0
//!   pip logic local0 lc0_i0 bits 5.12
//!   pip logic sp4e@-4,0 local0 bits 5.13
//!   tile 0 0 io
//!   tiles logic rect 1 1 12 16
//!   pinmap 21 X0Y3/io0
//! end
//! ```
//!
//! [`Arch::to_text`] writes the directives in that order and
//! `parse(to_text(a)) == a` holds, so an architecture can be generated,
//! diffed and reviewed like source. The one normalisation is that
//! `tiles ... rect` is written back as one `tile` line per tile.
//!
//! # What ships, and what does not
//!
//! The architecture compiled into the crate is **synthetic**: see
//! [`synthetic`] for exactly what it invents and what it borrows. It is
//! enough to place, route and write a bitstream end to end, and it is not
//! the silicon. A real iCE40 architecture is an [`Arch`] parsed from a
//! file; nothing in [`mod@super::place`], [`mod@super::route`] or
//! [`mod@super::bitstream`] knows which one it is holding.
//!
//! # Dropping in a real database
//!
//! Everything the flow needs is in the format above, so a database
//! derived from Project IceStorm's chip files replaces the built-in one
//! by parsing a file. What such a converter has to emit:
//!
//! 1. The grid (`.device` / the `[0-9]+ [0-9]+` header of an icebox chip
//!    database) and one `tile` line per tile with its real type.
//! 2. One `tiletype` per tile type with IceStorm's real bitmap size
//!    (`logic_tile` 16 x 54, `io_tile` 16 x 18, `ramb_tile` / `ramt_tile`
//!    16 x 42 on the 1k dice).
//! 3. Every wire icebox names for that tile type (`local_g0_0`,
//!    `sp4_h_r_0`, `sp12_v_b_0`, `lutff_0/in_0`, `glb_netwk_0`, ...) with
//!    the span icebox's `.wire` segments imply.
//! 4. Every entry of icebox's `.buffer` and `.routing` blocks as a `pip`
//!    line, with the `B<row>[<col>]` positions it lists as the pip's
//!    config bits.
//! 5. One `bel` per `lutff_*`, `io_*`, `ram` and `gb` with the wire each
//!    pin reaches, and `config` lines for `LC_<n>` (the sixteen
//!    `LUT_INIT` bits, the flip-flop enable and set/reset bits, the carry
//!    enable) and for `IoCtrl` / `IOB_<n>` (`PIN_TYPE`, `PULLUP`).
//! 6. `pinmap` lines from the package file, one per package pin.
//!
//! None of that is invented here, and none of it is on this machine.
//! Until it is supplied, the bitstream this module's data produces is
//! structurally a bitstream and electrically meaningless.

mod parse;
pub mod synthetic;

pub use parse::{ARCH_SYNTAX, ARCH_UNKNOWN};

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::diag::Diagnostics;
use crate::source::SourceMap;

/// One bit of a tile's configuration bitmap.
///
/// `row` and `col` index the tile type's `bits <rows> <cols>` bitmap,
/// which is the same addressing IceStorm's `.asc` files use: a
/// `logic_tile` line `B3[20]` is `ConfigBit { row: 3, col: 20 }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigBit {
    /// The row of the tile bitmap, from 0.
    pub row: u32,
    /// The column of the tile bitmap, from 0.
    pub col: u32,
}

impl ConfigBit {
    /// A bit at `(row, col)`.
    pub fn new(row: u32, col: u32) -> Self {
        ConfigBit { row, col }
    }
}

/// One wire declared by a tile type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireDecl {
    /// The wire's name inside its tile type.
    pub name: String,
    /// How far east the wire reaches from the tile it starts in.
    pub dx: i32,
    /// How far north the wire reaches from the tile it starts in.
    pub dy: i32,
}

/// A reference to a wire from a pip or a bel pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireRef {
    /// The wire's name, in its own tile type, or the global's name.
    pub name: String,
    /// Column offset of the tile the wire starts in, from the tile that
    /// holds the pip or bel.
    pub dx: i32,
    /// Row offset of the tile the wire starts in.
    pub dy: i32,
    /// True when `name` is a global wire rather than a tile wire.
    pub global: bool,
}

impl WireRef {
    /// A reference to a wire starting in the referring tile.
    pub fn local(name: impl Into<String>) -> Self {
        WireRef {
            name: name.into(),
            dx: 0,
            dy: 0,
            global: false,
        }
    }

    /// A reference to a wire starting `(dx, dy)` tiles away.
    pub fn at(name: impl Into<String>, dx: i32, dy: i32) -> Self {
        WireRef {
            name: name.into(),
            dx,
            dy,
            global: false,
        }
    }

    /// A reference to a global wire.
    pub fn global(name: impl Into<String>) -> Self {
        WireRef {
            name: name.into(),
            dx: 0,
            dy: 0,
            global: true,
        }
    }

    /// The reference in the text format's syntax.
    pub fn to_text(&self) -> String {
        if self.global {
            format!("*{}", self.name)
        } else if self.dx == 0 && self.dy == 0 {
            self.name.clone()
        } else {
            format!("{}@{},{}", self.name, self.dx, self.dy)
        }
    }
}

/// One programmable interconnect point declared by a tile type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipDecl {
    /// The wire the pip reads.
    pub from: WireRef,
    /// The wire the pip drives.
    pub to: WireRef,
    /// The configuration bits that switch it on, in the tile that holds
    /// it. An empty list is a connection that is always there.
    pub bits: Vec<ConfigBit>,
}

/// How a placed cell configures the bel it sits on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigEntry {
    /// Bits set when a cell of this primitive sits on the bel, which is
    /// how a family's twenty flip-flop primitives become a mode field.
    Cell {
        /// The primitive name as the device database spells it.
        primitive: String,
        /// The bits to set.
        bits: Vec<ConfigBit>,
    },
    /// The bit carrying bit `index` of the cell's parameter `name`, which
    /// is how a LUT's truth table reaches the bitstream.
    Param {
        /// The parameter name (`LUT_INIT`, `PIN_TYPE`).
        name: String,
        /// Which bit of the parameter's value.
        index: u32,
        /// Where that bit lives in the tile.
        at: ConfigBit,
    },
}

/// One placeable element declared by a tile type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BelDecl {
    /// The bel's name inside its tile, which is the tail of every site
    /// name it produces (`X3Y5/lc0_lut`).
    pub name: String,
    /// What can be placed here: `lut`, `ff`, `carry`, `io`, `gb`,
    /// `bram`. Free-form, and compared with the role keyword of the
    /// device database's [`BelRole`](super::BelRole).
    pub kind: String,
    /// Pin role to the wire it reaches, in declaration order.
    pub pins: Vec<(String, WireRef)>,
    /// How a cell on this bel is configured, in declaration order.
    pub config: Vec<ConfigEntry>,
}

impl BelDecl {
    /// A bel with a name and a kind and nothing else.
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        BelDecl {
            name: name.into(),
            kind: kind.into(),
            pins: Vec::new(),
            config: Vec::new(),
        }
    }

    /// The wire the pin playing `role` reaches.
    pub fn pin(&self, role: &str) -> Option<&WireRef> {
        self.pins.iter().find(|(r, _)| r == role).map(|(_, w)| w)
    }
}

/// One kind of tile: what every tile of that type holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileType {
    /// The type's name, used by `tile` lines and by the members below.
    pub name: String,
    /// The keyword an `.asc` file heads this tile's bits with, without
    /// the leading dot (`logic_tile`).
    pub asc_keyword: String,
    /// Rows of the tile's configuration bitmap.
    pub bit_rows: u32,
    /// Columns of the tile's configuration bitmap.
    pub bit_cols: u32,
    /// The wires that start in a tile of this type, in file order.
    pub wires: Vec<WireDecl>,
    /// The placeable elements, in file order.
    pub bels: Vec<BelDecl>,
    /// The pips, in file order.
    pub pips: Vec<PipDecl>,
}

impl TileType {
    /// An empty tile type with the given name, `.asc` keyword and bitmap.
    pub fn new(
        name: impl Into<String>,
        asc_keyword: impl Into<String>,
        rows: u32,
        cols: u32,
    ) -> Self {
        TileType {
            name: name.into(),
            asc_keyword: asc_keyword.into(),
            bit_rows: rows,
            bit_cols: cols,
            wires: Vec::new(),
            bels: Vec::new(),
            pips: Vec::new(),
        }
    }

    /// The number of bits in one tile of this type.
    pub fn bit_count(&self) -> usize {
        self.bit_rows as usize * self.bit_cols as usize
    }

    /// True when the type declares a wire of that name.
    pub fn has_wire(&self, name: &str) -> bool {
        self.wires.iter().any(|w| w.name == name)
    }

    /// The bel of that name.
    pub fn bel(&self, name: &str) -> Option<&BelDecl> {
        self.bels.iter().find(|b| b.name == name)
    }
}

/// One family's routing architecture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arch {
    /// The architecture's own name (`ice40-synthetic`).
    pub name: String,
    /// The device family it belongs to, matching [`Device::family`].
    ///
    /// [`Device::family`]: super::Device::family
    pub family: String,
    /// The device names this architecture serves, in file order.
    pub parts: Vec<String>,
    /// Number of tile columns.
    pub width: u32,
    /// Number of tile rows.
    pub height: u32,
    /// The word an `.asc` file's `.device` line carries (`1k`), empty
    /// when the family has no such convention.
    pub asc_device: String,
    /// The wires that reach the whole die, in file order.
    pub globals: Vec<String>,
    /// The tile types, in file order.
    pub tile_types: Vec<TileType>,
    /// The tile type of each position, indexed `y * width + x`; `None`
    /// for a position that holds no tile (the corners of an IO ring).
    pub tiles: Vec<Option<usize>>,
    /// Package pin to site name, in file order.
    pub pinmap: Vec<(String, String)>,
}

impl Arch {
    /// An empty architecture of the given name, family and grid.
    pub fn new(
        name: impl Into<String>,
        family: impl Into<String>,
        width: u32,
        height: u32,
    ) -> Self {
        Arch {
            name: name.into(),
            family: family.into(),
            parts: Vec::new(),
            width,
            height,
            asc_device: String::new(),
            globals: Vec::new(),
            tile_types: Vec::new(),
            tiles: vec![None; width as usize * height as usize],
            pinmap: Vec::new(),
        }
    }

    /// The index of the tile type named `name`.
    pub fn tile_type_index(&self, name: &str) -> Option<usize> {
        self.tile_types.iter().position(|t| t.name == name)
    }

    /// The tile type at `(x, y)`, or `None` when the position is empty or
    /// outside the grid.
    pub fn tile_at(&self, x: u32, y: u32) -> Option<&TileType> {
        self.tile_index_at(x, y).map(|i| &self.tile_types[i])
    }

    /// The tile type *index* at `(x, y)`.
    pub fn tile_index_at(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.tiles[(y * self.width + x) as usize]
    }

    /// Puts a tile of type `index` at `(x, y)`, ignoring a position
    /// outside the grid.
    pub fn set_tile(&mut self, x: u32, y: u32, index: usize) {
        if x < self.width && y < self.height {
            self.tiles[(y * self.width + x) as usize] = Some(index);
        }
    }

    /// The site a package pin reaches.
    pub fn site_of_pin(&self, pin: &str) -> Option<&str> {
        self.pinmap
            .iter()
            .find(|(p, _)| p == pin)
            .map(|(_, s)| s.as_str())
    }

    /// True when this architecture describes the named part.
    pub fn serves(&self, part: &str) -> bool {
        self.parts.iter().any(|p| p == part)
    }

    /// Expands the architecture into the graph the flow works on.
    ///
    /// Every tile of every type contributes its wires as nodes, its bels
    /// as sites and its pips as edges. A pip or a pin whose wire
    /// reference falls outside the grid, or names a wire the tile there
    /// does not declare, is dropped and counted in
    /// [`RoutingGraph::dangling`]: that is what happens at the edges of
    /// the die, and it is not an error.
    pub fn build_graph(&self) -> RoutingGraph {
        RoutingGraph::build(self)
    }
}

/// One node of the routing graph: a piece of metal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    /// The wire's name inside its tile type, or the global's name.
    pub name: String,
    /// The tile the wire starts in. A global starts at `(0, 0)` by
    /// convention and covers the whole die.
    pub tile: (u32, u32),
    /// How far the wire reaches from that tile, east and north.
    pub span: (i32, i32),
    /// True when the wire reaches every tile.
    pub global: bool,
}

impl Wire {
    /// The wire's full name, which is what reports and `.asc` comments
    /// print: `X3Y5/local0`, or `GLOBAL/glb0`.
    pub fn full_name(&self) -> String {
        if self.global {
            format!("GLOBAL/{}", self.name)
        } else {
            format!("X{}Y{}/{}", self.tile.0, self.tile.1, self.name)
        }
    }

    /// The tile of this wire closest to `(x, y)`, which is what a
    /// distance heuristic must measure from: a span-12 wire is *at* every
    /// tile it crosses.
    pub fn nearest_tile(&self, x: u32, y: u32) -> (u32, u32) {
        if self.global {
            return (x, y);
        }
        (
            clamp_span(self.tile.0, self.span.0, x),
            clamp_span(self.tile.1, self.span.1, y),
        )
    }

    /// True when the wire reaches tile `(x, y)`.
    pub fn covers(&self, x: u32, y: u32) -> bool {
        self.global || self.nearest_tile(x, y) == (x, y)
    }
}

/// The coordinate of a span `origin .. origin + span` closest to `target`.
fn clamp_span(origin: u32, span: i32, target: u32) -> u32 {
    let far = i64::from(origin) + i64::from(span);
    let (lo, hi) = if far < i64::from(origin) {
        (far, i64::from(origin))
    } else {
        (i64::from(origin), far)
    };
    let clamped = i64::from(target).clamp(lo, hi);
    u32::try_from(clamped.max(0)).unwrap_or(0)
}

/// One programmable interconnect point of the graph.
///
/// # Why the bits are not here
///
/// They used to be: a `Vec<ConfigBit>` of its own, per pip. That is
/// fine for a fabric declared at 14 × 18 tiles and hopeless for a real
/// one. An `INT_L` has 3636 pips and an `xc7a50t` has 5650 `INT_L`s, so
/// a whole die is 20.6 million pips; with a vector each that is around
/// 96 bytes a pip before any routing happens, and the vectors hold
/// 5650 identical copies of the same 3636 patterns.
///
/// So the patterns are **interned**: every distinct list of bits is
/// stored once in the graph and a pip keeps a [`BitsId`] into it.
/// [`RoutingGraph::pip_bits`] reads them back. A pip is 20 bytes, and
/// [`RoutingGraph::heap_bytes`] measures what the whole graph costs
/// rather than estimating it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pip {
    /// The node the pip reads.
    pub from: NodeId,
    /// The node the pip drives.
    pub to: NodeId,
    /// The tile that holds it, which is the tile its bits live in.
    pub tile: (u32, u32),
    /// Which interned bit pattern switches it on; read it with
    /// [`RoutingGraph::bits`].
    pub bits: BitsId,
}

/// An index into [`RoutingGraph::nodes`].
pub type NodeId = u32;
/// An index into [`RoutingGraph::pips`].
pub type PipId = u32;
/// An index into a [`RoutingGraph`]'s table of configuration bit
/// patterns; see [`Pip`].
pub type BitsId = u32;

/// One placeable location of the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchSite {
    /// The site's name, `X<x>Y<y>/<bel>`.
    pub name: String,
    /// What can be placed here, from [`BelDecl::kind`].
    pub kind: String,
    /// The tile it is in.
    pub tile: (u32, u32),
    /// The tile type that declared it.
    pub tile_type: usize,
    /// The bel's name inside that tile type.
    pub bel: String,
    /// Pin role to the node it reaches, in declaration order. A pin whose
    /// wire could not be resolved is absent.
    pub pins: Vec<(String, NodeId)>,
}

impl ArchSite {
    /// The node the pin playing `role` reaches.
    pub fn pin(&self, role: &str) -> Option<NodeId> {
        self.pins.iter().find(|(r, _)| r == role).map(|(_, n)| *n)
    }
}

/// The expanded routing architecture: nodes, pips and sites, with
/// neighbour lookup in both directions.
///
/// Node and pip ids are indices, handed out in a fixed order — tiles in
/// row-major order, each tile's wires in declaration order — so two runs
/// over one architecture produce the same graph and the same routes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingGraph {
    /// Every wire of the part.
    pub nodes: Vec<Wire>,
    /// Every pip of the part.
    pub pips: Vec<Pip>,
    /// Every placeable location.
    pub sites: Vec<ArchSite>,
    /// Number of tile columns.
    pub width: u32,
    /// Number of tile rows.
    pub height: u32,
    /// How many declared pips and bel pins were dropped because their
    /// wire reference left the grid.
    pub dangling: usize,
    /// Every distinct configuration bit pattern, end to end.
    bit_pool: Vec<ConfigBit>,
    /// Where each interned pattern starts and how long it is.
    bit_spans: Vec<(u32, u32)>,
    /// Start of each node's outgoing pip list in `out_pips`.
    out_start: Vec<u32>,
    /// Outgoing pips, grouped by source node.
    out_pips: Vec<PipId>,
    /// Start of each node's incoming pip list in `in_pips`.
    in_start: Vec<u32>,
    /// Incoming pips, grouped by destination node.
    in_pips: Vec<PipId>,
}

impl RoutingGraph {
    /// Expands `arch`; see [`Arch::build_graph`].
    fn build(arch: &Arch) -> RoutingGraph {
        let mut nodes: Vec<Wire> = Vec::new();
        let mut index: HashMap<(u32, u32, String), NodeId> = HashMap::new();
        let mut dangling = 0usize;

        // Globals first, so their ids are the lowest and stable.
        for name in &arch.globals {
            let id = node_id(nodes.len());
            index.insert((0, 0, name.clone()), id);
            nodes.push(Wire {
                name: name.clone(),
                tile: (0, 0),
                span: (0, 0),
                global: true,
            });
        }
        let is_global = |name: &str| arch.globals.iter().any(|g| g == name);

        for y in 0..arch.height {
            for x in 0..arch.width {
                let Some(tile) = arch.tile_at(x, y) else {
                    continue;
                };
                for wire in &tile.wires {
                    if is_global(&wire.name) {
                        // A tile wire may not shadow a global; the
                        // declaration is ignored and counted.
                        dangling += 1;
                        continue;
                    }
                    let id = node_id(nodes.len());
                    index.insert((x, y, wire.name.clone()), id);
                    nodes.push(Wire {
                        name: wire.name.clone(),
                        tile: (x, y),
                        span: (wire.dx, wire.dy),
                        global: false,
                    });
                }
            }
        }

        let resolve = |wref: &WireRef, x: u32, y: u32| -> Option<NodeId> {
            if wref.global {
                return index.get(&(0, 0, wref.name.clone())).copied();
            }
            let ox = i64::from(x) + i64::from(wref.dx);
            let oy = i64::from(y) + i64::from(wref.dy);
            let ox = u32::try_from(ox).ok()?;
            let oy = u32::try_from(oy).ok()?;
            index.get(&(ox, oy, wref.name.clone())).copied()
        };

        // Every distinct bit pattern once. A pattern belongs to a tile
        // *type*, so every tile of that type would otherwise store its
        // own copy; on a real 7-series fabric that is thousands of
        // copies of each.
        let mut bit_pool: Vec<ConfigBit> = Vec::new();
        let mut bit_spans: Vec<(u32, u32)> = vec![(0, 0)];
        let mut interned: HashMap<Vec<ConfigBit>, BitsId> = HashMap::new();
        interned.insert(Vec::new(), 0);
        let mut intern = |bits: &[ConfigBit]| -> BitsId {
            if bits.is_empty() {
                return 0;
            }
            if let Some(id) = interned.get(bits) {
                return *id;
            }
            let id = BitsId::try_from(bit_spans.len()).expect("bit patterns fit in 2^32");
            let start = u32::try_from(bit_pool.len()).expect("bit pool fits in 2^32");
            bit_pool.extend_from_slice(bits);
            bit_spans.push((start, u32::try_from(bits.len()).unwrap_or(0)));
            interned.insert(bits.to_vec(), id);
            id
        };

        let mut pips: Vec<Pip> = Vec::new();
        let mut sites: Vec<ArchSite> = Vec::new();
        for y in 0..arch.height {
            for x in 0..arch.width {
                let Some(ti) = arch.tile_index_at(x, y) else {
                    continue;
                };
                let tile = &arch.tile_types[ti];
                for bel in &tile.bels {
                    let mut pins = Vec::new();
                    for (role, wref) in &bel.pins {
                        match resolve(wref, x, y) {
                            Some(node) => pins.push((role.clone(), node)),
                            None => dangling += 1,
                        }
                    }
                    sites.push(ArchSite {
                        name: format!("X{x}Y{y}/{}", bel.name),
                        kind: bel.kind.clone(),
                        tile: (x, y),
                        tile_type: ti,
                        bel: bel.name.clone(),
                        pins,
                    });
                }
                for pip in &tile.pips {
                    let (Some(from), Some(to)) = (resolve(&pip.from, x, y), resolve(&pip.to, x, y))
                    else {
                        dangling += 1;
                        continue;
                    };
                    pips.push(Pip {
                        from,
                        to,
                        tile: (x, y),
                        bits: intern(&pip.bits),
                    });
                }
            }
        }

        let (out_start, out_pips) = csr(nodes.len(), &pips, |p| p.from);
        let (in_start, in_pips) = csr(nodes.len(), &pips, |p| p.to);
        RoutingGraph {
            nodes,
            pips,
            sites,
            width: arch.width,
            height: arch.height,
            dangling,
            bit_pool,
            bit_spans,
            out_start,
            out_pips,
            in_start,
            in_pips,
        }
    }

    /// The pips leaving `node`.
    pub fn outgoing(&self, node: NodeId) -> &[PipId] {
        let i = node as usize;
        &self.out_pips[self.out_start[i] as usize..self.out_start[i + 1] as usize]
    }

    /// The pips arriving at `node`.
    pub fn incoming(&self, node: NodeId) -> &[PipId] {
        let i = node as usize;
        &self.in_pips[self.in_start[i] as usize..self.in_start[i + 1] as usize]
    }

    /// The wire a node id names.
    pub fn wire(&self, node: NodeId) -> &Wire {
        &self.nodes[node as usize]
    }

    /// The pip a pip id names.
    pub fn pip(&self, pip: PipId) -> &Pip {
        &self.pips[pip as usize]
    }

    /// The bits an interned pattern holds; see [`Pip`].
    ///
    /// Pattern 0 is always the empty one, which is what the model means
    /// by a connection that is always there.
    pub fn bits(&self, id: BitsId) -> &[ConfigBit] {
        let (start, len) = self.bit_spans[id as usize];
        &self.bit_pool[start as usize..start as usize + len as usize]
    }

    /// The bits that switch a pip on.
    pub fn pip_bits(&self, pip: PipId) -> &[ConfigBit] {
        self.bits(self.pips[pip as usize].bits)
    }

    /// How many distinct bit patterns the pips share between them.
    pub fn bit_patterns(&self) -> usize {
        self.bit_spans.len()
    }

    /// How many bytes the graph holds, counting every allocation it owns.
    ///
    /// This is a measurement and not an estimate: it is what decides how
    /// much of a real fabric fits, so it is computed from the vectors
    /// themselves rather than written down in a comment that can drift.
    /// A [`Wire`]'s name is counted as its own bytes plus its `String`
    /// header, which is where most of what is left now goes.
    pub fn heap_bytes(&self) -> usize {
        use std::mem::size_of;
        let mut bytes = self.nodes.capacity() * size_of::<Wire>();
        for node in &self.nodes {
            bytes += node.name.capacity();
        }
        bytes += self.pips.capacity() * size_of::<Pip>();
        bytes += self.bit_pool.capacity() * size_of::<ConfigBit>();
        bytes += self.bit_spans.capacity() * size_of::<(u32, u32)>();
        bytes += (self.out_start.capacity() + self.in_start.capacity()) * size_of::<u32>();
        bytes += (self.out_pips.capacity() + self.in_pips.capacity()) * size_of::<PipId>();
        for site in &self.sites {
            bytes += size_of::<ArchSite>()
                + site.name.capacity()
                + site.kind.capacity()
                + site.bel.capacity()
                + site.pins.capacity() * size_of::<(String, NodeId)>();
            for (role, _) in &site.pins {
                bytes += role.capacity();
            }
        }
        bytes
    }

    /// The site of that name.
    pub fn site(&self, name: &str) -> Option<&ArchSite> {
        self.site_index(name).map(|i| &self.sites[i])
    }

    /// The index of the site of that name.
    pub fn site_index(&self, name: &str) -> Option<usize> {
        self.sites.iter().position(|s| s.name == name)
    }

    /// How many sites of each kind the part has, sorted by kind.
    pub fn site_counts(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for site in &self.sites {
            *counts.entry(site.kind.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }
}

/// Narrows a node count to the id type.
///
/// The graph is built from an architecture whose grid and tile types are
/// bounded by the file that declares them; a part with more than four
/// billion wires is not one this crate can hold, and saturating keeps the
/// conversion lossless for every architecture that fits.
fn node_id(len: usize) -> NodeId {
    NodeId::try_from(len).expect("routing graph exceeds 2^32 nodes")
}

/// Groups pips by one endpoint into compressed sparse row form.
fn csr(nodes: usize, pips: &[Pip], key: impl Fn(&Pip) -> NodeId) -> (Vec<u32>, Vec<PipId>) {
    let mut counts = vec![0u32; nodes + 1];
    for pip in pips {
        counts[key(pip) as usize + 1] += 1;
    }
    for i in 1..counts.len() {
        counts[i] += counts[i - 1];
    }
    let mut cursor = counts.clone();
    let mut out = vec![0 as PipId; pips.len()];
    for (i, pip) in pips.iter().enumerate() {
        let slot = &mut cursor[key(pip) as usize];
        out[*slot as usize] = PipId::try_from(i).expect("pip count fits in 2^32");
        *slot += 1;
    }
    (counts, out)
}

static BUILTIN: OnceLock<Vec<Arch>> = OnceLock::new();

/// The architectures compiled into the crate.
///
/// There is exactly one, and it is [`synthetic`]: an iCE40-*like* fabric,
/// not iCE40. It is parsed from the text [`synthetic::ice40_like`]
/// renders, which is the same path a real database takes, so the loader
/// is exercised by every use of it.
pub fn builtin_architectures() -> &'static [Arch] {
    BUILTIN.get_or_init(|| {
        let text = synthetic::ice40_like().to_text();
        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let file = map
            .add("ice40-synthetic.arch", &text)
            .expect("the built-in architecture fits in a source map");
        let archs = Arch::parse(&text, file, &mut diags);
        assert!(
            !diags.has_errors(),
            "the built-in architecture is malformed:\n{}",
            diags.render(&map)
        );
        archs
    })
}

/// The built-in architecture serving `device`, by device name.
///
/// ```
/// assert!(reticle::fpga::arch::architecture_for("ice40-hx1k-tq144").is_some());
/// assert!(reticle::fpga::arch::architecture_for("ecp5-45f-CABGA381").is_none());
/// ```
pub fn architecture_for(device: &str) -> Option<&'static Arch> {
    builtin_architectures().iter().find(|a| a.serves(device))
}

static BUILTIN_GRAPHS: OnceLock<Vec<Arc<RoutingGraph>>> = OnceLock::new();

/// The expanded graph of the built-in architecture serving `device`.
///
/// Expanding a grid into a few hundred thousand nodes and pips is not
/// free, so the built-in ones are expanded once and shared; the handle is
/// an [`Arc`] so that a result can hold on to one without copying it. A
/// caller with its own [`Arch`] uses [`Arch::build_graph`] instead.
pub fn builtin_graph_for(device: &str) -> Option<Arc<RoutingGraph>> {
    let index = builtin_architectures()
        .iter()
        .position(|a| a.serves(device))?;
    let graphs = BUILTIN_GRAPHS.get_or_init(|| {
        builtin_architectures()
            .iter()
            .map(|arch| Arc::new(arch.build_graph()))
            .collect()
    });
    graphs.get(index).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_references_render_back() {
        assert_eq!(WireRef::local("a").to_text(), "a");
        assert_eq!(WireRef::at("a", -4, 0).to_text(), "a@-4,0");
        assert_eq!(WireRef::global("g").to_text(), "*g");
    }

    #[test]
    fn a_span_is_at_every_tile_it_crosses() {
        let wire = Wire {
            name: "sp4e".to_owned(),
            tile: (3, 5),
            span: (4, 0),
            global: false,
        };
        assert_eq!(wire.nearest_tile(9, 5), (7, 5));
        assert_eq!(wire.nearest_tile(1, 5), (3, 5));
        assert_eq!(wire.nearest_tile(5, 5), (5, 5));
        assert!(wire.covers(5, 5));
        assert!(!wire.covers(9, 5));
        assert_eq!(wire.full_name(), "X3Y5/sp4e");

        let west = Wire {
            name: "sp4w".to_owned(),
            tile: (6, 2),
            span: (-4, 0),
            global: false,
        };
        assert_eq!(west.nearest_tile(0, 2), (2, 2));
        assert!(west.covers(3, 2));

        let global = Wire {
            name: "glb0".to_owned(),
            tile: (0, 0),
            span: (0, 0),
            global: true,
        };
        assert!(global.covers(12, 12));
        assert_eq!(global.full_name(), "GLOBAL/glb0");
        assert_eq!(global.nearest_tile(9, 9), (9, 9));
    }

    /// A three-tile architecture, small enough to check node by node.
    fn tiny() -> Arch {
        let mut arch = Arch::new("tiny", "test", 3, 1);
        arch.globals.push("glb".to_owned());
        let mut logic = TileType::new("logic", "logic_tile", 2, 4);
        logic.wires.push(WireDecl {
            name: "local".to_owned(),
            dx: 0,
            dy: 0,
        });
        logic.wires.push(WireDecl {
            name: "sp2e".to_owned(),
            dx: 2,
            dy: 0,
        });
        let mut bel = BelDecl::new("lut", "lut");
        bel.pins.push(("i0".to_owned(), WireRef::local("local")));
        bel.pins.push(("o".to_owned(), WireRef::local("sp2e")));
        bel.config.push(ConfigEntry::Cell {
            primitive: "L".to_owned(),
            bits: vec![ConfigBit::new(0, 0)],
        });
        logic.bels.push(bel);
        logic.pips.push(PipDecl {
            from: WireRef::at("sp2e", -2, 0),
            to: WireRef::local("local"),
            bits: vec![ConfigBit::new(1, 1)],
        });
        logic.pips.push(PipDecl {
            from: WireRef::global("glb"),
            to: WireRef::local("local"),
            bits: vec![ConfigBit::new(1, 2)],
        });
        arch.tile_types.push(logic);
        for x in 0..3 {
            arch.set_tile(x, 0, 0);
        }
        arch.parts.push("test-part".to_owned());
        arch.pinmap.push(("1".to_owned(), "X0Y0/lut".to_owned()));
        arch
    }

    #[test]
    fn the_graph_expands_tiles_and_drops_what_leaves_the_grid() {
        let arch = tiny();
        let graph = arch.build_graph();
        // One global plus two wires in each of three tiles.
        assert_eq!(graph.nodes.len(), 1 + 6);
        assert_eq!(graph.nodes[0].name, "glb");
        assert!(graph.nodes[0].global);
        assert_eq!(graph.sites.len(), 3);
        assert_eq!(graph.sites[0].name, "X0Y0/lut");
        assert_eq!(graph.sites[0].kind, "lut");
        // The span-2 pip resolves only in the tile at x = 2.
        let spans: Vec<_> = graph
            .pips
            .iter()
            .filter(|p| !graph.wire(p.from).global)
            .collect();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].tile, (2, 0));
        assert_eq!(graph.wire(spans[0].from).tile, (0, 0));
        // Two of the three tiles could not resolve theirs.
        assert_eq!(graph.dangling, 2);
        // Every tile has the global pip.
        assert_eq!(graph.pips.len(), 4);
        assert_eq!(graph.outgoing(0).len(), 3);
        let local_x2 = graph
            .nodes
            .iter()
            .position(|w| w.name == "local" && w.tile == (2, 0))
            .unwrap();
        assert_eq!(graph.incoming(node_id(local_x2)).len(), 2);
        assert_eq!(graph.site_counts(), vec![("lut".to_owned(), 3)]);
        assert_eq!(graph.site("X1Y0/lut").unwrap().tile, (1, 0));
        assert!(graph.site("nope").is_none());
        assert_eq!(arch.site_of_pin("1"), Some("X0Y0/lut"));
        assert!(arch.serves("test-part"));
        assert_eq!(
            graph.sites[0].pin("i0"),
            graph.site("X0Y0/lut").unwrap().pin("i0")
        );
        assert!(graph.sites[0].pin("nope").is_none());
    }

    #[test]
    fn the_built_in_architecture_is_the_synthetic_one() {
        let arch = architecture_for("ice40-hx1k-tq144").unwrap();
        assert_eq!(arch.name, "ice40-synthetic");
        assert_eq!(arch.family, "ice40");
        assert_eq!((arch.width, arch.height), (14, 18));
        let graph = arch.build_graph();
        assert!(graph.nodes.len() > 1000, "{}", graph.nodes.len());
        let counts = graph.site_counts();
        for kind in ["lut", "ff", "carry", "io", "gb", "bram"] {
            assert!(
                counts.iter().any(|(k, n)| k == kind && *n > 0),
                "no {kind} sites in {counts:?}"
            );
        }
    }
}
