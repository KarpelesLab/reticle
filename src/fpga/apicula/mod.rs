//! Reading a **real** Gowin fabric: Project Apicula's chip database as an
//! [`Arch`].
//!
//! # What this is
//!
//! [`Project Apicula`] is the open reverse-engineered database for Gowin's
//! FPGAs, and unlike Project X-Ray it **ships prebuilt**: the PyPI package
//! `apycula` carries one file per device, `<device>.msgpack.xz`, so no
//! vendor IDE has to be run to get one. The loader below turns one of
//! those into the same [`Arch`] the placer, the router and the bitstream
//! writer already work on, and [`super::gowin`] is the `.fs` container
//! that goes with it.
//!
//! [`Project Apicula`]: https://github.com/YosysHQ/apicula
//!
//! **Reticle does not fetch it.** Every byte comes through a
//! [`FileProvider`] the caller hands in, and the caller says where the
//! file is. `docs/fpga-gowin.md` gives the exact command that obtains it.
//! The database is not in this repository and must not be; without it
//! every test here skips and says so.
//!
//! # Nothing Gowin has been loaded into a part
//!
//! The board this was written for — a Sipeed Tang Primer 20K, a
//! GW2A-LV18PG256C8/I7 — is on the machine and `reticle program
//! --probe` reads its IDCODE, `0x0000081b`. **That is the whole of what
//! has touched the hardware.** Nothing here has configured anything, and
//! the configuration sequence that would is not written.
//!
//! What *is* established, and against what, is in `docs/fpga-gowin.md`.
//! The short form: the container agrees byte for byte with a reference
//! `.fs` that Apicula's own `gowin_pack` produced for this device, the
//! fabric's measured size comes out of [`ApiculaStats`] rather than out of
//! a note, and the logic that decides what a bel's configuration bits are
//! is **not written**, because the names of the attributes it would need
//! are not in the database. See *What the database does not name*.
//!
//! # The database
//!
//! One MessagePack map inside an xz stream. The map is a Python
//! dataclass written out field by field — `apycula/chipdb.py`'s `Device`
//! — so its keys are that class's field names, and a great many of its
//! inner maps are keyed on *tuples*, which is why [`crate::msgpack`]
//! keeps a map as pairs of whole values rather than as a string map.
//!
//! For `GW2A-18` the file is 375 KB compressed, 5 993 401 bytes open, and
//! the root map has 38 entries. [`ApiculaDatabase::top_level_keys`]
//! reports them; `docs/fpga-gowin.md` says what each one holds and which
//! of them this loader reads.
//!
//! The eleven this loader reads:
//!
//! | Key | What is taken from it |
//! |---|---|
//! | `grid` | the tile type of every tile, as rows of integers |
//! | `tiles` | per tile *type*: its bitmap size, its pips, its clock pips and its bels |
//! | `tile_types` | the letter that names a type's function (`C` logic, `I` IO, `B` block RAM, `D` DSP, `P` PLL, `M` logic with distributed RAM) |
//! | `corner_tiles_io` | which edge each of the four corner tiles counts as, which is what makes the `IOLOC` names come out right |
//! | `packages` | part number to `(package, device, speed grade)` |
//! | `pinout` | package ball to `IOLOC`, which with the above gives [`Arch::pinmap`] |
//! | `pin_bank` | which IO bank an `IOLOC` is in |
//! | `cmd_hdr`, `cmd_ftr` | the `.fs` file's header and footer, byte for byte |
//! | `const` | the bits every tile of a type always sets |
//! | `nodes` | the always-connected networks — counted, not used; see below |
//!
//! # The three mismatches with Reticle's model
//!
//! ## 1. The bitmap fits exactly, which the 7 series did not
//!
//! A Gowin die bitmap **is** the per-tile bitmaps laid out in a grid, so a
//! tile's `ConfigBit { row, col }` is that tile's own bit and there is no
//! frame address anywhere. That is precisely what
//! [`TileType::bit_rows`](super::arch::TileType::bit_rows) and
//! `bit_cols` mean, and it is a closer fit than the 7 series managed. The
//! one wrinkle is that a tile's size depends on where it is — a GW2A-18's
//! rows are 24, 26 or 28 bits tall and its columns 60 or 68 wide — so the
//! die's geometry comes out beside the architecture as a
//! [`DieLayout`](super::gowin::DieLayout), the way a
//! [`FrameMap`](super::xc7::FrameMap) does for the 7 series.
//!
//! ## 2. An inter-tile wire is named per tile, and the rule is arithmetic
//!
//! A Gowin wire that leaves its tile has a **different name in each tile
//! it crosses**: `E210` in the tile it starts in, `E211` one tile east,
//! `E212` two. The 7 series has the same problem and needs a table
//! (`tileconn.json`) to solve it; here it is arithmetic, which
//! [`InterTileWire`] implements from `chipdb.py::wire2global`.
//!
//! So this loader does something the 7-series one could not: it declares
//! the wire **once, under its root name, with a span**, and every pip that
//! calls it by a later segment refers to it with
//! [`WireRef::at`](super::arch::WireRef::at). `Arch`'s span model is
//! exactly right for this, and the cost is **no extra graph edges at all**
//! — where the 7-series loader spends a third of its 30.9 million edges on
//! joins, this one spends none.
//!
//! **One thing is given up for that, deliberately.** At the die's edge a
//! wire's root would fall outside the grid, and Gowin reflects it back and
//! flips its direction, so `S212` in the top row is the same metal as
//! `N21` two rows down. That depends on the tile's *position*, and a pip
//! belongs to a tile *type*, so it cannot be written down once per type.
//! Rather than get it wrong, this loader always names the unreflected
//! root — which for a wrapped wire is off the grid, so the pip is dropped
//! and counted in
//! [`RoutingGraph::dangling`](super::arch::RoutingGraph::dangling). **A
//! wrapped wire is therefore missing, never wrong**, which is the only
//! acceptable way round: a wrong join shorts two nets and no structural
//! check would notice. On a GW2A-18 that is 16 872 of 517 440 inter-tile
//! wire references, 3.3%, all of them within eight tiles of an edge;
//! [`ApiculaStats::edge_wraps`] counts them.
//!
//! ## 3. The `nodes` table needs a pip that belongs to a tile, not a type
//!
//! `nodes` is 14 748 named networks with 89 583 members, each member a
//! `(row, col, wire)`, and it is where the global clock network, the
//! high-speed clock spines and the wiring that reaches into block RAM and
//! DSP cells live. `Arch` can say this: a node is a
//! [global wire](super::arch::Arch::globals) and a membership is a
//! zero-bit pip joining a tile's local wire to it.
//!
//! What it cannot say is *which tiles*. A membership belongs to one tile
//! and a pip is declared by a tile type, and those memberships touch all
//! 3080 tiles of a GW2A-18, so writing them down would need one tile type
//! per tile — 3080 copies of pip lists that run to 2624 entries each.
//! **So this loader does not emit them**, counts them in
//! [`ApiculaStats::nodes`] and [`ApiculaStats::node_members`], and leaves
//! them reachable through [`ApiculaDatabase::nodes`] for whatever comes
//! next. The consequence is plain and worth stating: **nothing clocked,
//! and nothing that uses a block RAM or a DSP, can route.** Teaching
//! `Arch` a per-tile pip is what would fix it.
//!
//! # What the database does not name
//!
//! This is the single biggest gap, and it is not a gap in the loader.
//!
//! A Gowin bel's configuration is a set of *attribute values* —
//! `IO_TYPE=LVCMOS33`, `PULLMODE=NONE`, `REGMODE`, `SRMODE` — and the
//! database encodes them in two steps. `logicinfo[<table>]` maps
//! `(attribute id, value id)` to a small integer code, and
//! `shortval[<ttyp>][<table>]` / `longval[<ttyp>][<table>]` map tuples of
//! those codes to the bits to set. [`fuses_for`] implements the second
//! step, including the rule that makes a Gowin bitstream not a blank
//! sheet: a row whose codes are all *negative* applies when nothing is
//! set, so its bits are on until a design turns them off.
//!
//! **The attribute and value *names* are not in the file.** They are
//! Python dictionaries in `apycula/attrids.py`, 62 KB of them. Without
//! those, a code is a number and there is no way to say "this buffer is
//! LVCMOS33". So:
//!
//! - the **lookup table** is here and tested ([`fuses_for`]);
//! - the **names** are not, so no bel is configured except the lookup
//!   tables and the lookup table for a LUT, which needs no names;
//! - which means **an IO buffer gets its pins and no bits**, and a
//!   bitstream from this loader would route a signal to a pad and leave
//!   the pad unconfigured.
//!
//! Transcribing the tables this needs — with provenance, the way
//! `super::xray`'s `sites.rs` does for Xilinx pin names — is the first
//! thing phase two has to do.
//!
//! The one bel that *is* configured is the LUT4, because its bits are
//! named by position rather than by attribute: `bels["LUT<n>"].flags[i]`
//! is where bit `i` of the sixteen-bit `INIT` lives, and the fuse is set
//! when that bit is **zero** (`gowin_pack.py::get_LUT4_fuses`: `if lutbit
//! == '0': bits.update(lutmap[bitnum])`). That is
//! [`ConfigEntry::ParamZero`](super::arch::ConfigEntry::ParamZero), which
//! exists because Project X-Ray stores a flip-flop's `INIT` the same way
//! round. Getting the polarity backwards inverts every lookup table in
//! the design.
//!
//! # Scale
//!
//! Measured, by [`ApiculaStats`], not estimated:
//!
//! | | GW2A-18 |
//! |---|---|
//! | grid | 55 x 56 = 3080 tiles, 74 tile types |
//! | die bitmap | 1342 x 3376 = 4 530 592 bits, 422 bytes a row with no padding |
//! | pips over the die | 8 095 135, plus 5198 clock pips |
//! | distinct bit patterns among them | 10 915 |
//! | bels over the die | 54 111 |
//! | LUT4 / flip-flop / ALU / distributed RAM | 20 736 / 15 552 / 15 552 / 648 |
//! | IO buffers / block RAMs / DSPs / PLLs | 384 / 46 / 12 / 4 |
//! | `nodes` | 14 748 networks, 89 583 members |
//! | package pins (PBGA256) | 207 |
//!
//! And the graph the whole die expands into, also measured, by
//! `tests/fpga_gowin.rs::the_whole_die_is_a_graph_this_crate_can_hold`:
//!
//! | | |
//! |---|---|
//! | nodes | 626 001 |
//! | edges declared | 8 100 333 |
//! | of those, kept | 7 872 211 (97%) |
//! | dropped: a reference to a wire the tile there has not got | 228 122 |
//! | distinct bit patterns, shared by all of them | 9 939 |
//! | [`RoutingGraph::heap_bytes`](super::arch::RoutingGraph::heap_bytes) | 296 MiB |
//!
//! Eight million pips is a fifth of the 7 series' `xc7a50t`, and 296 MiB
//! is a fifth of that fabric's 1386 MiB — partly because the die is
//! smaller and partly because declaring an inter-tile wire once with a
//! span costs no join edges, where the 7-series loader spends a third of
//! its edges on them. [`ApiculaOptions::region`] still says which
//! rectangle of tiles gets wires, pips and bels, and
//! [`ApiculaOptions::max_pips`] still makes the loader count first and
//! allocate after: an eleven-by-eleven region in the middle of the die is
//! 121 tiles and 321 242 edges.

mod parse;

pub use parse::{CodeRow, Coord, InterTileWire, fuses_for, ioloc};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt;

use super::arch::{Arch, ConfigBit, ConfigEntry, PipDecl, TileType, WireDecl, WireRef};
use super::gowin::{DieLayout, FsStream, GowinError};
use super::xray::GridRegion;
use crate::ir::memfile::FileProvider;
use crate::msgpack::{Msgpack, MsgpackError};

/// Why an Apicula database could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiculaError {
    /// The database file is not where the caller said.
    Missing {
        /// The path that was tried.
        path: String,
    },
    /// The xz stream would not decode.
    Compression {
        /// What `compcol` said.
        message: String,
    },
    /// The MessagePack inside would not decode.
    Malformed(MsgpackError),
    /// The MessagePack decoded but is not the shape a `Device` has.
    NotADevice {
        /// Which field, and what was wrong with it.
        what: String,
    },
    /// No part of that name is in this database's `packages`.
    NoSuchPart {
        /// The part that was asked for.
        part: String,
        /// The parts the database does know, in order.
        known: Vec<String>,
    },
    /// The region asked for would build more graph edges than
    /// [`ApiculaOptions::max_pips`] allows. Counted before anything was
    /// allocated.
    TooLarge {
        /// The edges the region would declare.
        pips: usize,
        /// The limit.
        limit: usize,
    },
    /// The `.fs` container refused something.
    Container(GowinError),
}

impl fmt::Display for ApiculaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiculaError::Missing { path } => write!(f, "`{path}` is not there"),
            ApiculaError::Compression { message } => {
                write!(f, "the xz stream would not decode: {message}")
            }
            ApiculaError::Malformed(err) => write!(f, "the MessagePack is malformed: {err}"),
            ApiculaError::NotADevice { what } => {
                write!(f, "this is not an Apicula device database: {what}")
            }
            ApiculaError::NoSuchPart { part, known } => {
                write!(f, "no part `{part}` in this database")?;
                if let Some(first) = known.first() {
                    write!(f, "; it has {} part(s), such as `{first}`", known.len())?;
                }
                Ok(())
            }
            ApiculaError::TooLarge { pips, limit } => write!(
                f,
                "that region declares {pips} graph edge(s) and the limit is {limit}"
            ),
            ApiculaError::Container(err) => write!(f, "{err}"),
        }
    }
}

impl Error for ApiculaError {}

impl From<MsgpackError> for ApiculaError {
    fn from(err: MsgpackError) -> Self {
        ApiculaError::Malformed(err)
    }
}

impl From<GowinError> for ApiculaError {
    fn from(err: GowinError) -> Self {
        ApiculaError::Container(err)
    }
}

/// How much of the fabric to load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiculaOptions {
    /// The part number whose package pins go into [`Arch::pinmap`], such
    /// as `GW2A-LV18PG256C8/I7`. `None` loads no pin map, which is right
    /// for measuring the fabric and wrong for building anything.
    pub part: Option<String>,
    /// Which tiles get wires, pips and bels. `None` is the whole die.
    pub region: Option<GridRegion>,
    /// The largest number of graph edges the loader will declare before
    /// refusing with [`ApiculaError::TooLarge`]. It counts first and
    /// allocates after, so the refusal costs nothing.
    ///
    /// The whole GW2A-18 declares 8 100 333 edges — 8 095 135 pips and
    /// 5198 clock pips — so the default has room for it and for the
    /// larger GW5 dice the same database format covers.
    pub max_pips: usize,
}

impl Default for ApiculaOptions {
    fn default() -> Self {
        ApiculaOptions {
            part: None,
            region: None,
            max_pips: 48_000_000,
        }
    }
}

impl ApiculaOptions {
    /// The defaults.
    pub fn new() -> ApiculaOptions {
        ApiculaOptions::default()
    }

    /// The same options for a named part, so the pin map is loaded.
    pub fn with_part(mut self, part: impl Into<String>) -> ApiculaOptions {
        self.part = Some(part.into());
        self
    }

    /// The same options restricted to a region.
    pub fn with_region(mut self, region: GridRegion) -> ApiculaOptions {
        self.region = Some(region);
        self
    }
}

/// What the load cost and covered.
///
/// These are the numbers the module documentation and `docs/fpga-gowin.md`
/// quote, produced by the loader itself rather than written down by hand,
/// so they cannot drift.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiculaStats {
    /// The database's device name (`GW2A-18`).
    pub device: String,
    /// Grid rows.
    pub rows: u32,
    /// Grid columns.
    pub cols: u32,
    /// Tiles in the whole grid.
    pub tiles: usize,
    /// Distinct tile types in the whole grid.
    pub tile_types: usize,
    /// Rows of the die's configuration bitmap.
    pub bitmap_rows: u32,
    /// Columns of the die's configuration bitmap.
    pub bitmap_cols: u32,
    /// Pips over the whole die.
    pub pips_die: u64,
    /// Clock pips over the whole die, which the database keeps apart.
    pub clock_pips_die: u64,
    /// Distinct configuration bit patterns among all the tile types' pips,
    /// which is what interning them in the graph saves.
    pub bit_patterns: usize,
    /// Bels over the whole die, by the database's own bel name.
    pub bels_die: BTreeMap<String, usize>,
    /// Always-connected networks in the `nodes` table. **Not loaded**; see
    /// the module documentation.
    pub nodes: usize,
    /// Members of those networks, each a `(row, col, wire)`.
    pub node_members: usize,
    /// Pips the `hclk_pips` table holds, which are per *cell* and so are
    /// not loaded either.
    pub hclk_pips: usize,
    /// Bits the `const` tables set over the whole die, which is what an
    /// otherwise empty bitstream carries.
    pub const_bits: usize,
    /// Tiles that got wires, pips and bels.
    pub tiles_loaded: usize,
    /// Tile types that were expanded.
    pub tile_types_loaded: usize,
    /// Wires declared in the loaded tile types, summed over the tiles.
    pub wires: usize,
    /// Pips declared over the loaded tiles.
    pub pips: usize,
    /// Bels declared over the loaded tiles.
    pub bels: usize,
    /// Inter-tile wire references whose root is reflected back at the die
    /// edge, and which this loader therefore leaves unconnected rather
    /// than connecting wrongly. See the module documentation.
    pub edge_wraps: usize,
    /// Configuration entries the loader emitted, all of them lookup-table
    /// bits — see *What the database does not name*.
    pub config_entries: usize,
    /// Package pins mapped to a site.
    pub pins_mapped: usize,
    /// Package pins whose `IOLOC` named no tile with that IO buffer.
    pub pins_unmapped: usize,
    /// The JTAG IDCODE the `cmd_hdr` checks.
    pub idcode: Option<u32>,
}

impl ApiculaStats {
    /// A report, one fact per line.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "chip database: {}", self.device);
        let _ = writeln!(
            out,
            "  grid: {} x {} = {} tile(s) of {} type(s)",
            self.rows, self.cols, self.tiles, self.tile_types
        );
        let _ = writeln!(
            out,
            "  die bitmap: {} row(s) x {} column(s) = {} bit(s), {} byte(s) a row",
            self.bitmap_rows,
            self.bitmap_cols,
            u64::from(self.bitmap_rows) * u64::from(self.bitmap_cols),
            (self.bitmap_cols as usize).div_ceil(8)
        );
        let _ = writeln!(
            out,
            "  die-wide: {} pip(s) + {} clock pip(s) in {} distinct bit pattern(s)",
            self.pips_die, self.clock_pips_die, self.bit_patterns
        );
        let _ = writeln!(
            out,
            "  not loaded: {} node(s) with {} member(s), {} hclk pip(s)",
            self.nodes, self.node_members, self.hclk_pips
        );
        let _ = writeln!(out, "  always-set bits: {}", self.const_bits);
        if let Some(idcode) = self.idcode {
            let _ = writeln!(out, "  idcode: {idcode:#010x}");
        }
        let _ = writeln!(
            out,
            "  loaded: {} tile(s) of {} type(s), {} wire(s), {} pip(s), {} bel(s), \
             {} config entr(ies)",
            self.tiles_loaded,
            self.tile_types_loaded,
            self.wires,
            self.pips,
            self.bels,
            self.config_entries
        );
        let _ = writeln!(
            out,
            "  edge-wrapped wire reference(s) left unconnected: {}",
            self.edge_wraps
        );
        let _ = writeln!(
            out,
            "  package pins: {} mapped, {} not",
            self.pins_mapped, self.pins_unmapped
        );
        let mut line = String::new();
        for (name, count) in &self.bels_die {
            if !line.is_empty() {
                line.push_str(", ");
            }
            let _ = write!(line, "{name} {count}");
        }
        if !line.is_empty() {
            let _ = writeln!(out, "  bels over the die: {line}");
        }
        out
    }
}

/// One always-connected network of the `nodes` table.
///
/// Counted and handed over, not loaded: see the module documentation for
/// why a membership cannot become a pip in this model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The network's name (`PCLKL0`).
    pub name: String,
    /// What kind it is (`GLOBAL_CLK`, `HCLK`, `BSRAM_I`, `DSP_O`, ...).
    pub kind: String,
    /// Its members, each a tile and a wire in it.
    pub members: Vec<(u32, u32, String)>,
}

/// One entry of the database's `packages` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// The part number (`GW2A-LV18PG256C8/I7`).
    pub part: String,
    /// The package the pinout is keyed on (`PBGA256`).
    pub package: String,
    /// The die (`GW2A-18`).
    pub device: String,
    /// The speed grades (`C8/I7`).
    pub speed: String,
}

/// A loaded fabric: the architecture, the die's geometry and the `.fs`
/// envelope, which only mean anything together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiculaFabric {
    /// The routing architecture, over the region that was asked for.
    pub arch: Arch,
    /// The die bitmap's geometry, for the whole part.
    pub layout: DieLayout,
    /// The `.fs` header commands the database carries.
    pub header: Vec<Vec<u8>>,
    /// The `.fs` footer lines the database carries.
    pub footer: Vec<Vec<u8>>,
    /// The bits every tile of a type always sets, as `((row, col), bits)`
    /// in grid order. An empty bitstream carries these and nothing else.
    pub const_bits: Vec<((u32, u32), Vec<ConfigBit>)>,
    /// What the load covered and cost.
    pub stats: ApiculaStats,
}

impl ApiculaFabric {
    /// Checks that this database really is the part the flow asked for.
    ///
    /// # Errors
    ///
    /// [`GowinError::IdcodeMismatch`] when the IDCODEs differ, which is
    /// the check that stops a bitstream built from one die's database
    /// being labelled with another's. Note that it cannot tell a GW2A-18
    /// from a GW2A-18C or a GW2AR-18C: all three carry `0x0000081b`.
    pub fn check_idcode(&self, wanted: u32) -> Result<(), GowinError> {
        match self.stats.idcode {
            Some(found) if found != wanted => Err(GowinError::IdcodeMismatch { wanted, found }),
            _ => Ok(()),
        }
    }

    /// The die bitmap with the `const` bits set and nothing else, wrapped
    /// in the database's header and footer.
    ///
    /// This is as far towards a bitstream as this phase goes, and it is
    /// worth being exact about what it is: a structurally complete `.fs`
    /// file, with the right geometry, the right commands and the right
    /// check words, **that configures nothing**. `gowin_pack`'s own output
    /// for a design with nothing in it sets 862 bits, of which these are
    /// 462; the other 400 are its configuration of the unused IO ring,
    /// which needs the attribute names the database does not carry.
    ///
    /// # Errors
    ///
    /// [`ApiculaError::Container`] when the database's header has no
    /// load-configuration command to write the row count into.
    pub fn blank_stream(&self) -> Result<FsStream, ApiculaError> {
        let mut bitmap = self.layout.bitmap();
        for ((row, col), bits) in &self.const_bits {
            let Some((origin_row, origin_col)) = self.layout.origin(*row, *col) else {
                continue;
            };
            for bit in bits {
                bitmap.set(origin_row + bit.row, origin_col + bit.col);
            }
        }
        Ok(FsStream::new(
            self.header.clone(),
            bitmap,
            self.footer.clone(),
        )?)
    }
}

/// An Apicula chip database, decoded.
#[derive(Clone, Debug)]
pub struct ApiculaDatabase {
    device: String,
    root: Msgpack,
}

impl ApiculaDatabase {
    /// Reads `<directory>/<device>.msgpack.xz` through `files`.
    ///
    /// `device` is the *database's* name, which is not the part number: a
    /// GW2AR-18C board is served by `GW2A-18C`, and Apicula's own
    /// `examples/Makefile` builds for a Tang Primer 20K with `gowin_pack
    /// -d GW2A-18`.
    ///
    /// # Errors
    ///
    /// [`ApiculaError::Missing`] when the file is not there, and the
    /// decoding errors of [`ApiculaDatabase::from_xz`].
    pub fn open(
        directory: &str,
        device: &str,
        files: &dyn FileProvider,
    ) -> Result<ApiculaDatabase, ApiculaError> {
        let path = if directory.is_empty() {
            format!("{device}.msgpack.xz")
        } else {
            format!("{}/{device}.msgpack.xz", directory.trim_end_matches('/'))
        };
        let bytes = files
            .read_bytes(&path)
            .ok_or(ApiculaError::Missing { path })?;
        ApiculaDatabase::from_xz(device, &bytes)
    }

    /// Decodes an xz-compressed database.
    ///
    /// # Errors
    ///
    /// [`ApiculaError::Compression`] when the stream will not decode, and
    /// the errors of [`ApiculaDatabase::from_msgpack`].
    pub fn from_xz(device: &str, bytes: &[u8]) -> Result<ApiculaDatabase, ApiculaError> {
        let plain = compcol::vec::decompress_to_vec::<compcol::xz::Xz>(bytes).map_err(|err| {
            ApiculaError::Compression {
                message: format!("{err:?}"),
            }
        })?;
        ApiculaDatabase::from_msgpack(device, &plain)
    }

    /// Decodes an already uncompressed database.
    ///
    /// # Errors
    ///
    /// [`ApiculaError::Malformed`] for MessagePack that will not decode,
    /// and [`ApiculaError::NotADevice`] when the value is not a map with a
    /// `grid` of rows of tile types.
    pub fn from_msgpack(device: &str, bytes: &[u8]) -> Result<ApiculaDatabase, ApiculaError> {
        let root = Msgpack::parse(bytes)?;
        if root.pairs().is_empty() {
            return Err(ApiculaError::NotADevice {
                what: format!("the root is a {} and not a map", root.type_name()),
            });
        }
        let Some(grid) = root.get("grid") else {
            return Err(ApiculaError::NotADevice {
                what: "no `grid` field".to_owned(),
            });
        };
        if grid.array().is_empty() || grid.array()[0].array().is_empty() {
            return Err(ApiculaError::NotADevice {
                what: "`grid` is not rows of tile types".to_owned(),
            });
        }
        Ok(ApiculaDatabase {
            device: device.to_owned(),
            root,
        })
    }

    /// The device name this database was opened as.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// The root map's keys, in file order. Thirty-eight of them for a
    /// `GW2A-18`; `docs/fpga-gowin.md` says what each holds.
    pub fn top_level_keys(&self) -> Vec<&str> {
        self.root
            .pairs()
            .iter()
            .filter_map(|(k, _)| k.as_str())
            .collect()
    }

    /// The grid as rows of tile type numbers.
    fn grid(&self) -> Vec<Vec<u32>> {
        self.root
            .get("grid")
            .map(|g| {
                g.array()
                    .iter()
                    .map(|row| row.array().iter().filter_map(Msgpack::as_u32).collect())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Grid rows and columns.
    pub fn grid_size(&self) -> (u32, u32) {
        let grid = self.grid();
        let rows = u32::try_from(grid.len()).unwrap_or(0);
        let cols = grid
            .first()
            .map_or(0, |r| u32::try_from(r.len()).unwrap_or(0));
        (rows, cols)
    }

    /// The JTAG IDCODE the `.fs` header's `0x06` command checks.
    pub fn idcode(&self) -> Option<u32> {
        let header = self.header();
        let line = header
            .iter()
            .find(|l| l.first() == Some(&super::gowin::CMD_IDCODE))?;
        let bytes: [u8; 4] = line.get(4..8)?.try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    }

    /// The `.fs` header commands the database carries.
    pub fn header(&self) -> Vec<Vec<u8>> {
        byte_strings(self.root.get("cmd_hdr"))
    }

    /// The `.fs` footer lines the database carries.
    pub fn footer(&self) -> Vec<Vec<u8>> {
        byte_strings(self.root.get("cmd_ftr"))
    }

    /// Every part number the database knows, in file order.
    pub fn packages(&self) -> Vec<Package> {
        self.root
            .get("packages")
            .map(|table| {
                table
                    .pairs()
                    .iter()
                    .filter_map(|(part, value)| {
                        let items = value.array();
                        Some(Package {
                            part: part.as_str()?.to_owned(),
                            package: items.first()?.as_str()?.to_owned(),
                            device: items.get(1)?.as_str()?.to_owned(),
                            speed: items.get(2)?.as_str()?.to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The always-connected networks. See the module documentation for why
    /// they are handed over rather than loaded.
    pub fn nodes(&self) -> Vec<Node> {
        self.root
            .get("nodes")
            .map(|table| {
                table
                    .pairs()
                    .iter()
                    .filter_map(|(name, value)| {
                        let items = value.array();
                        Some(Node {
                            name: name.as_str()?.to_owned(),
                            kind: items.first()?.as_str().unwrap_or_default().to_owned(),
                            members: items
                                .get(1)?
                                .array()
                                .iter()
                                .filter_map(|m| {
                                    let m = m.array();
                                    Some((
                                        m.first()?.as_u32()?,
                                        m.get(1)?.as_u32()?,
                                        m.get(2)?.as_str()?.to_owned(),
                                    ))
                                })
                                .collect(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The `logicinfo` table named `table`, as `((attribute id, value id),
    /// code)`.
    ///
    /// The ids are numbers and the database does not name them; see *What
    /// the database does not name*.
    pub fn logicinfo(&self, table: &str) -> Vec<((i64, i64), i64)> {
        self.root
            .get("logicinfo")
            .and_then(|t| t.get(table))
            .map(|t| {
                t.pairs()
                    .iter()
                    .filter_map(|(key, code)| {
                        let key = key.array();
                        Some((
                            (key.first()?.as_i64()?, key.get(1)?.as_i64()?),
                            code.as_i64()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A `shortval`, `longval` or `longfuses` table for one tile type, as
    /// the `(codes, bits)` rows [`fuses_for`] takes.
    ///
    /// `kind` is one of `shortval`, `longval` or `longfuses` and `table`
    /// is the name inside it (`CLS0`, `LUT`, `IOBA`, `BANK`, `PLL`).
    pub fn code_table(&self, kind: &str, tile_type: u32, table: &str) -> Vec<CodeRow> {
        let Some(outer) = self.root.get(kind) else {
            return Vec::new();
        };
        for (ttyp, tables) in outer.pairs() {
            if ttyp.as_u32() == Some(tile_type)
                && let Some(rows) = tables.get(table)
            {
                return parse::code_table(rows);
            }
        }
        Vec::new()
    }

    /// The die bitmap's geometry: where every tile's bits sit.
    pub fn layout(&self) -> DieLayout {
        let grid = self.grid();
        let tiles = self.tile_shapes();
        let heights: Vec<u32> = grid
            .iter()
            .map(|row| row.first().and_then(|t| tiles.get(t)).map_or(0, |s| s.1))
            .collect();
        let widths: Vec<u32> = grid
            .first()
            .map(|row| {
                row.iter()
                    .map(|t| tiles.get(t).map_or(0, |s| s.0))
                    .collect()
            })
            .unwrap_or_default();
        DieLayout::new(&heights, &widths)
    }

    /// Every tile type's `(width, height)`.
    fn tile_shapes(&self) -> HashMap<u32, (u32, u32)> {
        let mut out = HashMap::new();
        if let Some(tiles) = self.root.get("tiles") {
            for (ttyp, tile) in tiles.pairs() {
                if let (Some(ttyp), Some(w), Some(h)) = (
                    ttyp.as_u32(),
                    tile.get("width").and_then(Msgpack::as_u32),
                    tile.get("height").and_then(Msgpack::as_u32),
                ) {
                    out.insert(ttyp, (w, h));
                }
            }
        }
        out
    }

    /// The tile type letter of each type number, from `tile_types`.
    fn type_letters(&self) -> HashMap<u32, String> {
        let mut out = HashMap::new();
        if let Some(groups) = self.root.get("tile_types") {
            for (letter, members) in groups.pairs() {
                let Some(letter) = letter.as_str() else {
                    continue;
                };
                for member in members.array() {
                    if let Some(ttyp) = member.as_u32() {
                        out.insert(ttyp, letter.to_owned());
                    }
                }
            }
        }
        out
    }

    /// The four corner tiles' edge letters, from `corner_tiles_io`.
    fn corner_tiles(&self) -> BTreeMap<(u32, u32), String> {
        let mut out = BTreeMap::new();
        if let Some(table) = self.root.get("corner_tiles_io") {
            for (key, edge) in table.pairs() {
                let key = key.array();
                if let (Some(r), Some(c), Some(edge)) = (
                    key.first().and_then(Msgpack::as_u32),
                    key.get(1).and_then(Msgpack::as_u32),
                    edge.as_str(),
                ) {
                    out.insert((r, c), edge.to_owned());
                }
            }
        }
        out
    }

    /// One tile type's entry in `tiles`.
    fn tile(&self, tile_type: u32) -> Option<&Msgpack> {
        self.root
            .get("tiles")?
            .pairs()
            .iter()
            .find(|(k, _)| k.as_u32() == Some(tile_type))
            .map(|(_, v)| v)
    }

    /// Turns the database into an [`Arch`], a [`DieLayout`] and the
    /// measurements.
    ///
    /// # Errors
    ///
    /// [`ApiculaError::NoSuchPart`] when `options.part` names a part the
    /// database does not have, and [`ApiculaError::TooLarge`] when the
    /// region would declare more edges than `options.max_pips` — counted
    /// before anything is allocated.
    pub fn load(&self, options: &ApiculaOptions) -> Result<ApiculaFabric, ApiculaError> {
        let grid = self.grid();
        let (rows, cols) = self.grid_size();
        let shapes = self.tile_shapes();
        let letters = self.type_letters();
        let layout = self.layout();

        let mut stats = ApiculaStats {
            device: self.device.clone(),
            rows,
            cols,
            tiles: grid.iter().map(Vec::len).sum(),
            bitmap_rows: layout.rows(),
            bitmap_cols: layout.cols(),
            idcode: self.idcode(),
            ..ApiculaStats::default()
        };

        // Die-wide measurements first, over every tile of the grid, so the
        // report says what the part is and not only what was loaded.
        let mut patterns: BTreeSet<Vec<(u32, u32)>> = BTreeSet::new();
        let mut per_type: HashMap<u32, TypeFacts> = HashMap::new();
        for ttyp in grid.iter().flatten().copied() {
            per_type.entry(ttyp).or_insert_with(|| {
                let facts = self.type_facts(ttyp);
                for pattern in &facts.patterns {
                    patterns.insert(pattern.clone());
                }
                facts
            });
        }
        stats.tile_types = per_type.len();
        stats.bit_patterns = patterns.len();
        for ttyp in grid.iter().flatten() {
            let facts = &per_type[ttyp];
            stats.pips_die += facts.pips as u64;
            stats.clock_pips_die += facts.clock_pips as u64;
            for (name, count) in &facts.bels {
                *stats.bels_die.entry(name.clone()).or_default() += count;
            }
        }
        for node in self.nodes() {
            stats.nodes += 1;
            stats.node_members += node.members.len();
        }
        if let Some(table) = self.root.get("hclk_pips") {
            for (_, dests) in table.pairs() {
                for (_, sources) in dests.pairs() {
                    stats.hclk_pips += sources.pairs().len();
                }
            }
        }

        // The always-set bits, over the grid.
        let const_by_type = self.const_by_type();
        let mut const_bits = Vec::new();
        for (row, line) in grid.iter().enumerate() {
            for (col, ttyp) in line.iter().enumerate() {
                if let Some(bits) = const_by_type.get(ttyp) {
                    stats.const_bits += bits.len();
                    const_bits.push((
                        (
                            u32::try_from(row).unwrap_or(0),
                            u32::try_from(col).unwrap_or(0),
                        ),
                        bits.clone(),
                    ));
                }
            }
        }

        // Count the edges the region would declare before allocating any.
        let region = options.region;
        let inside = |x: u32, y: u32| region.is_none_or(|r| r.contains(x, y));
        let mut declared = 0usize;
        for (row, line) in grid.iter().enumerate() {
            for (col, ttyp) in line.iter().enumerate() {
                let (x, y) = (
                    u32::try_from(col).unwrap_or(0),
                    u32::try_from(row).unwrap_or(0),
                );
                if inside(x, y) {
                    let facts = &per_type[ttyp];
                    declared += facts.pips + facts.clock_pips;
                }
            }
        }
        if declared > options.max_pips {
            return Err(ApiculaError::TooLarge {
                pips: declared,
                limit: options.max_pips,
            });
        }

        // Build the architecture.
        let mut arch = Arch::new(format!("{}-apicula", self.device), "gowin", cols, rows);
        arch.parts = options.part.iter().cloned().collect();
        let mut type_index: HashMap<u32, usize> = HashMap::new();
        let mut used: BTreeSet<u32> = BTreeSet::new();
        for (row, line) in grid.iter().enumerate() {
            for (col, ttyp) in line.iter().enumerate() {
                let (x, y) = (
                    u32::try_from(col).unwrap_or(0),
                    u32::try_from(row).unwrap_or(0),
                );
                if inside(x, y) {
                    used.insert(*ttyp);
                }
            }
        }
        for ttyp in &used {
            let shape = shapes.get(ttyp).copied().unwrap_or((0, 0));
            let letter = letters.get(ttyp).map_or("t", String::as_str);
            let name = format!("{letter}{ttyp}");
            let mut tile_type = TileType::new(name.clone(), name, shape.1, shape.0);
            self.fill_tile_type(*ttyp, &mut tile_type, &mut stats);
            type_index.insert(*ttyp, arch.tile_types.len());
            arch.tile_types.push(tile_type);
        }
        stats.tile_types_loaded = arch.tile_types.len();
        for (row, line) in grid.iter().enumerate() {
            for (col, ttyp) in line.iter().enumerate() {
                let (x, y) = (
                    u32::try_from(col).unwrap_or(0),
                    u32::try_from(row).unwrap_or(0),
                );
                if !inside(x, y) {
                    continue;
                }
                let Some(index) = type_index.get(ttyp) else {
                    continue;
                };
                arch.set_tile(x, y, *index);
                stats.tiles_loaded += 1;
                let tile_type = &arch.tile_types[*index];
                stats.wires += tile_type.wires.len();
                stats.pips += tile_type.pips.len();
                stats.bels += tile_type.bels.len();
                stats.config_entries +=
                    tile_type.bels.iter().map(|b| b.config.len()).sum::<usize>();
                // An inter-tile reference is wrapped when its root falls
                // off the grid, which is exactly when this loader leaves
                // it unconnected. Counting it here, per tile, is the only
                // place the position is known.
                for wire in &per_type[ttyp].inter_wires {
                    let (dx, dy) = wire.root_offset();
                    let rx = i64::from(x) + i64::from(dx);
                    let ry = i64::from(y) + i64::from(dy);
                    if rx < 0 || ry < 0 || rx >= i64::from(cols) || ry >= i64::from(rows) {
                        stats.edge_wraps += 1;
                    }
                }
            }
        }

        // The package pins.
        if let Some(part) = &options.part {
            let packages = self.packages();
            let Some(package) = packages.iter().find(|p| &p.part == part) else {
                return Err(ApiculaError::NoSuchPart {
                    part: part.clone(),
                    known: packages.into_iter().map(|p| p.part).collect(),
                });
            };
            let sites = self.ioloc_sites(&grid, rows, cols);
            for (ball, ioloc) in self.pinout(&package.device, &package.package) {
                match sites.get(&ioloc) {
                    Some(site) => {
                        stats.pins_mapped += 1;
                        arch.pinmap.push((ball, site.clone()));
                    }
                    None => stats.pins_unmapped += 1,
                }
            }
        }

        Ok(ApiculaFabric {
            arch,
            layout,
            header: self.header(),
            footer: self.footer(),
            const_bits,
            stats,
        })
    }

    /// The `const` bits of each tile type.
    fn const_by_type(&self) -> HashMap<u32, Vec<ConfigBit>> {
        let mut out = HashMap::new();
        if let Some(table) = self.root.get("const") {
            for (ttyp, bits) in table.pairs() {
                if let Some(ttyp) = ttyp.as_u32() {
                    out.insert(
                        ttyp,
                        parse::coords(bits)
                            .into_iter()
                            .map(|(r, c)| ConfigBit::new(r, c))
                            .collect(),
                    );
                }
            }
        }
        out
    }

    /// `IOLOC` plus its half letter to the site name it reaches.
    ///
    /// Built forwards — over every tile of the IO ring, naming it and
    /// looking for an `IOB<half>` bel — and then keyed by the name, which
    /// is the direction that cannot go wrong. Reading a name and working
    /// out where it points would have to guess at the corners, where a
    /// tile is on two edges at once.
    fn ioloc_sites(&self, grid: &[Vec<u32>], rows: u32, cols: u32) -> BTreeMap<String, String> {
        let corners = self.corner_tiles();
        let mut out = BTreeMap::new();
        for (row, line) in grid.iter().enumerate() {
            for (col, ttyp) in line.iter().enumerate() {
                let (x, y) = (
                    u32::try_from(col).unwrap_or(0),
                    u32::try_from(row).unwrap_or(0),
                );
                let Some(name) = ioloc(rows, cols, &corners, y, x) else {
                    continue;
                };
                let Some(tile) = self.tile(*ttyp) else {
                    continue;
                };
                let Some(bels) = tile.get("bels") else {
                    continue;
                };
                for (bel, _) in bels.pairs() {
                    let Some(bel) = bel.as_str() else { continue };
                    if let Some(half) = bel.strip_prefix("IOB") {
                        out.insert(format!("{name}{half}"), format!("X{x}Y{y}/{bel}"));
                    }
                }
            }
        }
        out
    }

    /// The `(ball, IOLOC)` pairs of one package.
    fn pinout(&self, device: &str, package: &str) -> Vec<(String, String)> {
        let Some(table) = self
            .root
            .get("pinout")
            .and_then(|p| p.get(device))
            .and_then(|d| d.get(package))
        else {
            return Vec::new();
        };
        table
            .pairs()
            .iter()
            .filter_map(|(ball, value)| {
                Some((
                    ball.as_str()?.to_owned(),
                    value.array().first()?.as_str()?.to_owned(),
                ))
            })
            .collect()
    }

    /// The per-tile-type counts the die-wide report needs, gathered once.
    fn type_facts(&self, tile_type: u32) -> TypeFacts {
        let mut facts = TypeFacts::default();
        let Some(tile) = self.tile(tile_type) else {
            return facts;
        };
        let mut inter: BTreeSet<String> = BTreeSet::new();
        for (key, into) in [("pips", true), ("clock_pips", false)] {
            let Some(table) = tile.get(key) else { continue };
            for (dst, src, bits) in parse::pip_table(table) {
                if into {
                    facts.pips += 1;
                } else {
                    facts.clock_pips += 1;
                }
                facts.patterns.push(bits);
                for name in [dst, src] {
                    if InterTileWire::parse(name).is_some() {
                        inter.insert(name.to_owned());
                    }
                }
            }
        }
        facts.inter_wires = inter
            .iter()
            .filter_map(|n| InterTileWire::parse(n))
            .collect();
        if let Some(bels) = tile.get("bels") {
            for (name, _) in bels.pairs() {
                if let Some(name) = name.as_str() {
                    *facts.bels.entry(name.to_owned()).or_default() += 1;
                }
            }
        }
        facts
    }

    /// Fills in one tile type's wires, pips and bels.
    fn fill_tile_type(&self, tile_type: u32, into: &mut TileType, _stats: &mut ApiculaStats) {
        let Some(tile) = self.tile(tile_type) else {
            return;
        };

        // Wires first: an inter-tile name is declared under its root, once
        // and with its span; everything else is a tile-local wire.
        let mut declared: BTreeSet<String> = BTreeSet::new();
        let mut declare = |name: &str, wires: &mut Vec<WireDecl>| {
            let (decl_name, dx, dy) = match InterTileWire::parse(name) {
                Some(wire) => {
                    let (dx, dy) = wire.reach();
                    (wire.root_name(), dx, dy)
                }
                None => (name.to_owned(), 0, 0),
            };
            if declared.insert(decl_name.clone()) {
                wires.push(WireDecl {
                    name: decl_name,
                    dx,
                    dy,
                });
            }
        };

        let mut names: Vec<String> = Vec::new();
        for key in ["pips", "clock_pips"] {
            if let Some(table) = tile.get(key) {
                for (dst, src, _) in parse::pip_table(table) {
                    names.push(dst.to_owned());
                    names.push(src.to_owned());
                }
            }
        }
        if let Some(bels) = tile.get("bels") {
            for (_, bel) in bels.pairs() {
                if let Some(ports) = bel.get("portmap") {
                    for (_, wire) in ports.pairs() {
                        if let Some(wire) = wire.as_str()
                            && !wire.is_empty()
                        {
                            names.push(wire.to_owned());
                        }
                    }
                }
            }
        }
        for name in &names {
            declare(name, &mut into.wires);
        }

        for key in ["pips", "clock_pips"] {
            let Some(table) = tile.get(key) else { continue };
            for (dst, src, bits) in parse::pip_table(table) {
                into.pips.push(PipDecl {
                    from: wire_ref(src),
                    to: wire_ref(dst),
                    bits: bits
                        .into_iter()
                        .map(|(r, c)| ConfigBit::new(r, c))
                        .collect(),
                });
            }
        }

        if let Some(bels) = tile.get("bels") {
            for (name, bel) in bels.pairs() {
                let Some(name) = name.as_str() else { continue };
                if let Some(decl) = self.bel_decl(name, bel) {
                    into.bels.push(decl);
                }
            }
        }
    }

    /// One bel of a tile type.
    fn bel_decl(&self, name: &str, bel: &Msgpack) -> Option<super::arch::BelDecl> {
        let kind = bel_kind(name);
        let mut decl = super::arch::BelDecl::new(name, kind);
        let roles = bel_roles(name);
        if let Some(ports) = bel.get("portmap") {
            for (port, wire) in ports.pairs() {
                let (Some(port), Some(wire)) = (port.as_str(), wire.as_str()) else {
                    continue;
                };
                if wire.is_empty() {
                    continue;
                }
                let role = match roles {
                    // A bel Reticle places cells onto gets the model's own
                    // role names, and only the ports that have one.
                    Some(table) => match table.iter().find(|(p, _)| *p == port) {
                        Some((_, role)) => (*role).to_owned(),
                        None => continue,
                    },
                    // A bel nothing is placed on keeps the database's own
                    // port names, lowercased, because there is no role to
                    // map them to and dropping them would lose the
                    // fabric's shape.
                    None => port.to_ascii_lowercase(),
                };
                decl.pins.push((role, wire_ref(wire)));
            }
        }
        // The one bel whose bits the database names by position: a LUT4's
        // sixteen `INIT` bits, each set when the bit is ZERO.
        if name.starts_with("LUT")
            && let Some(flags) = bel.get("flags")
        {
            {
                for (index, bits) in flags.pairs() {
                    let Some(index) = index.as_u32() else {
                        continue;
                    };
                    for (row, col) in parse::coords(bits) {
                        decl.config.push(ConfigEntry::ParamZero {
                            name: "INIT".to_owned(),
                            index,
                            at: ConfigBit::new(row, col),
                        });
                    }
                }
            }
        }
        Some(decl)
    }
}

/// A wire reference: a tile-local name, or an inter-tile wire's root at
/// the offset its segment implies.
fn wire_ref(name: &str) -> WireRef {
    match InterTileWire::parse(name) {
        Some(wire) => {
            let (dx, dy) = wire.root_offset();
            WireRef::at(wire.root_name(), dx, dy)
        }
        None => WireRef::local(name),
    }
}

/// What kind of thing a bel is, by the database's own name for it.
///
/// The three Reticle places cells onto are `lut`, `ff` and `io`; the rest
/// are named so the fabric's shape is on record, and nothing is placed on
/// them. `src/fpga/devices/gowin.dev` says, primitive by primitive, why.
fn bel_kind(name: &str) -> &'static str {
    if name.starts_with("LUT") {
        return "lut";
    }
    if name.starts_with("DFF") {
        return "ff";
    }
    if name.starts_with("IOB") {
        return "io";
    }
    if name.starts_with("IOLOGIC") {
        return "iologic";
    }
    if name.starts_with("ALU54D") {
        return "dsp";
    }
    if name.starts_with("ALU") {
        return "carry";
    }
    if name == "RAM16" {
        return "lutram";
    }
    if name.starts_with("BSRAM") {
        return "bram";
    }
    if name.starts_with("DSP")
        || name.starts_with("MULT")
        || name.starts_with("PADD")
        || name.starts_with("MULTALU")
        || name.starts_with("MULTADDALU")
    {
        return "dsp";
    }
    if name.starts_with("RPLL") {
        return "pll";
    }
    if name.starts_with("CLKDIV") {
        return "clkdiv";
    }
    if name.starts_with("BANK") {
        return "bank";
    }
    if name == "OSC" {
        return "osc";
    }
    "other"
}

/// The `(database port, Reticle role)` table for a bel Reticle places
/// cells onto, or `None` for one it does not.
fn bel_roles(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    if name.starts_with("LUT") {
        // A Gowin LUT4's four inputs and its output, from the database's
        // own portmap: `{'F': 'F0', 'I0': 'A0', 'I1': 'B0', ...}`.
        return Some(&[
            ("I0", "i0"),
            ("I1", "i1"),
            ("I2", "i2"),
            ("I3", "i3"),
            ("F", "o"),
        ]);
    }
    if name.starts_with("DFF") {
        // NOTE the absence of `D`. The database's flip-flop portmap is
        // `{'Q', 'CLK', 'LSR', 'CE'}` and no more: a flip-flop's data
        // input comes from the LUT beside it *inside* the slice and has
        // no tile wire of its own, exactly as a 7-series `AFF`'s does
        // not. Until packing can put a LUT and a flip-flop on one slice,
        // nothing sequential will route.
        return Some(&[("CLK", "clk"), ("Q", "q"), ("CE", "en"), ("LSR", "rst")]);
    }
    if name.starts_with("IOB") {
        // `O` is the buffer's output *into the fabric*, so it is the
        // model's `din`; `I` is what the fabric drives out, so `dout`.
        // There is no `pad` pin: a package ball is not a wire a router can
        // reach, which is the same thing `super::xray` says about the
        // 7-series IO.
        return Some(&[("O", "din"), ("I", "dout"), ("OE", "oe")]);
    }
    None
}

/// The per-tile-type facts the report needs, gathered once per type.
#[derive(Clone, Debug, Default)]
struct TypeFacts {
    pips: usize,
    clock_pips: usize,
    patterns: Vec<Vec<(u32, u32)>>,
    bels: BTreeMap<String, usize>,
    inter_wires: Vec<InterTileWire>,
}

/// Reads an array of `bin` values as byte strings, which is what `cmd_hdr`
/// and `cmd_ftr` are.
fn byte_strings(value: Option<&Msgpack>) -> Vec<Vec<u8>> {
    value
        .map(|v| {
            v.array()
                .iter()
                .map(|line| match line {
                    Msgpack::Bin(bytes) => bytes.clone(),
                    // A writer that uses `array of uint` instead of `bin`
                    // is accepted, since the database's own encoder has
                    // changed representation before.
                    other => other
                        .array()
                        .iter()
                        .filter_map(|b| u8::try_from(b.as_u64()?).ok())
                        .collect(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
