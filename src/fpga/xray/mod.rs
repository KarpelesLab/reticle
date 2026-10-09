//! Reading a **real** fabric: Project X-Ray's chip database as an
//! [`Arch`].
//!
//! # What this is
//!
//! [`super::arch::synthetic`] builds a fabric that is *shaped* like an
//! iCE40 and programs nothing, because the database that says where the
//! real bits are was not on the machine. This module is the other side of
//! that sentence: [`f4pga/prjxray-db`] is such a database for the Xilinx
//! 7 series, it is CC0-1.0 (public domain), and the loader below turns it
//! into the same [`Arch`] the placer, the router and the bitstream writer
//! already work on.
//!
//! [`f4pga/prjxray-db`]: https://github.com/f4pga/prjxray-db
//!
//! **Reticle does not fetch it.** The library is sans-I/O: every byte
//! comes through a [`FileProvider`] the caller hands in, and the caller
//! says where the database is. `docs/fpga-xray.md` gives the exact
//! command that obtains it. The database is not in this repository and
//! must not be; without it every test here skips and says so.
//!
//! # Two designs from this loader have reached a part
//!
//! On 2026-09-24 a lookup table and three pins, placed and routed through
//! this loader, ran on a Basys 3 and drove an LED from two switches — a
//! person watched it work. The same day a **clocked** design did the rest
//! of the journey: `examples/basys3/blink.v`, a pad clock through a
//! `BUFG` and the global clock column into twenty-six flip-flops, routed
//! completely and configured the same board with `DONE` high and no CRC
//! error, and a person watched its LED blink at the rate it was written
//! for.
//!
//! That is the whole of what has been tried on silicon. Real bit
//! positions are still not the same thing as a working bitstream for any
//! *other* shape of design: what is established beyond those two is
//! structural — the frame layout, the frame count, the packet stream and
//! both CRCs agree with a bitstream Vivado made for an XC7A35T (see
//! [`super::xc7`]) — and the list of what is untried, headed by a carry
//! chain, is in `docs/fpga-xray.md` under "what remains".
//!
//! # The files, and what each one gives
//!
//! | File | What is taken from it |
//! |---|---|
//! | `<family>/mapping/devices.yaml` | which fabric a die uses — `xc7a35t` is the same die as `xc7a50t` |
//! | `<family>/<part>/part.json` | the IDCODE and the frame layout, column by column |
//! | `<family>/<part>/package_pins.csv` | package pin to site, for [`Arch::pinmap`] |
//! | `<family>/<fabric>/tilegrid.json` | every tile: name, type, grid position, sites, where its bits live in the frames, and for a tile type with no `segbits` file of its own the [`TileAlias`] onto one that has |
//! | `<family>/<fabric>/tileconn.json` | which wire of a tile is the same metal as which wire of its neighbour |
//! | `<family>/segbits_<type>.db` | which bits switch on which pip and which bel feature |
//! | `<family>/ppips_<type>.db` | the fixed, unprogrammable wiring inside a tile — which is how a site pin reaches the interconnect |
//!
//! `part.yaml` is ignored: it says the same as `part.json` and would need
//! a YAML parser. `devices.yaml` is read by a deliberately narrow reader
//! ([`fabric_of`]) that accepts only the two-line shape that file has and
//! refuses anything else, rather than by pretending to parse YAML.
//!
//! # The two mismatches with Reticle's model
//!
//! Both are real and neither is papered over.
//!
//! 1. **A 7-series wire is tile-local; a node is wires joined across tile
//!    boundaries.** [`Arch`] describes a wire by a *span*: `sp4e` starts
//!    here and reaches four tiles east, and every tile it crosses sees the
//!    same node. A 7-series long line has a different *name* in each tile
//!    it passes through, and `tileconn.json` says which name equals which.
//!    That cannot be written as a span, so the loader gives every wire
//!    span `0 0` and emits each `tileconn` pair as a pair of pips with no
//!    configuration bits — which the model already means as "a connection
//!    that is always there". It is faithful, and it costs one graph edge
//!    per join per direction; [`XrayStats::joins`] counts them.
//!
//!    A `tileconn` entry names **two tile types**, and the far end of
//!    each pip carries that type — [`WireRef::at_in`] — because the
//!    offset and the wire name alone are not enough: a tall `CMT_TOP` is
//!    cut into four types that all name their wires `CMT_TOP_*`, and the
//!    file pairs two *different* `CMT_FIFO` lanes with the same
//!    `CMT_TOP_EE4A0_0` at the same offset, one per half. Dropping the
//!    type welded those two lanes into one node, so a route could enter
//!    the column on one interconnect row and leave it three rows away,
//!    with every signal routed and every bit decoding. See
//!    `docs/fpga-xray.md` and `tests/fpga_xray_joins.rs`.
//! 2. **A tile's bits live at a frame address, not at a position in a
//!    per-tile bitmap.** [`Arch`] has nowhere to put a frame address, so
//!    the mapping comes out beside the architecture as a
//!    [`FrameMap`] and the two travel together in
//!    [`XrayFabric`]. Nothing is lost — a tile's `ConfigBit { row, col }`
//!    *is* frame `row` of the tile and bit `col` of its words — but an
//!    `.arch` file written out with [`Arch::to_text`] cannot carry it, so
//!    a real 7-series architecture does not round-trip through the text
//!    format the way the synthetic one does.
//!
//! # And what the data does not name
//!
//! `prjxray-db` ships the *bits*, not the tile-type wire and pip lists
//! (prjxray's own `tile_type_*.json` is generated from Vivado and is not
//! in the repository). Two things follow:
//!
//! - **A site pin has no name in the database.** The wiring *is* there:
//!   `ppips_<type>.db` records the fixed connection between a site pin's
//!   wire and the interconnect, and that is what
//!   [`PpipKind::Always`] is read for. What is missing is only the pin's
//!   *name* — `A1`, `O6`, `I`, `O` — and which of those wires carries
//!   it. `src/fpga/xray/sites.rs` supplies that from Xilinx's public
//!   user guides and says, entry by entry, where each line came from.
//! - **Whether a three-part feature `TYPE.A.B` is a pip or a bel
//!   feature has to be inferred.** The rule is in [`is_pip_feature`]: it
//!   is a pip unless `A` also heads a longer feature of the same tile
//!   type, which is what a site prefix does (`CLBLL_L.SLICEL_X0.AFF.ZINI`
//!   makes `SLICEL_X0` a site, so `CLBLL_L.SLICEL_X0.CLKINV` is a bel
//!   feature and not a pip). It classifies every `INT_L` feature as a pip
//!   and every `CLBLL_L` one as a bel feature, which is right; the three
//!   `LIOB33.DIFF.*` features are the known place it is wrong, and
//!   because `DIFF` carries no `_X<n>` or `_Y<n>` suffix it at least
//!   never claims to be a site.
//!
//! # Scale
//!
//! The `xc7a50t` fabric is 18 055 tiles and, by the database's own
//! `element_counts.csv`, 7 857 396 nodes. Its `segbits` add up to about
//! 23.5 million features over the die, of which 20.6 million are pips in
//! the 5650 interconnect tiles.
//!
//! Phase one could not expand that into a
//! [`RoutingGraph`](super::arch::RoutingGraph) at all, because every pip
//! owned a `Vec<ConfigBit>` and the same 3636 patterns were copied into
//! each of 5650 interconnect tiles. The patterns are interned now (see
//! [`Pip`](super::arch::Pip)) and the whole die does fit — **measured**,
//! not estimated:
//!
//! | | |
//! |---|---|
//! | graph edges declared | 44 304 488 |
//! | graph edges kept | 30 918 986 (20.6 M programmable, the rest fixed wiring and tile joins) |
//! | nodes | 6 081 818 |
//! | distinct bit patterns | 9774 |
//! | [`RoutingGraph::heap_bytes`](super::arch::RoutingGraph::heap_bytes) | 1386 MiB |
//! | peak resident while building | 2.0 GiB |
//! | time to build | about 8 s in a release build |
//!
//! Two gigabytes is still more than a small design should pay, so
//! [`XrayOptions::region`] still says which rectangle of tiles gets
//! wires, pips and bels, and [`XrayOptions::max_pips`] still stops with
//! the numbers rather than with an allocation failure — the default is
//! now set from the measurement above. The frame map always covers the
//! whole part, so the bitstream is a whole-part bitstream whatever the
//! region.
//!
//! What is left is the nodes: a [`Wire`](super::arch::Wire) owns its
//! name as a `String`, and six million of those are most of what
//! remains. Interning those too is the next thing worth doing, and it
//! was not needed to route the milestone.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;

use super::arch::{Arch, ConfigBit, PipDecl, TileType, WireDecl, WireRef};
use super::xc7::{FrameMap, Part, TileBits};
use crate::ir::memfile::FileProvider;
use crate::json::Json;

mod bram;
mod carry;
mod cmt;
mod dsp;
mod lutram;
mod parse;
mod sites;

pub use bram::{
    BlockRamError, BlockRamReport, BlockRamTables, TiePolicy, mode_features, tie_policy,
};
pub use carry::{CarryPacking, legalise_carries};
pub use cmt::{ClockManagerError, Counter, PllSettings, counter, pll_registers};
pub use dsp::{DSP_PRIMITIVE, dsp_refusal};
pub use parse::{Ppip, PpipKind, fabric_of, family_directory, is_pip_feature};

/// Reads one `segbits_<type>.db`: a feature name and its bits per line.
///
/// Project U-Ray writes the same format, which is why this is public:
/// [`super::uray`] reads its files through it.
///
/// # Errors
///
/// [`XrayError::Malformed`] with the line number for a bit that is not
/// `<frame>_<bit>`.
pub fn parse_segbits(text: &str, path: &str) -> Result<FeatureSet, XrayError> {
    parse::segbits(text, path)
}
pub use sites::{SiteCoverage, io_standards};

/// Why a Project X-Ray database could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XrayError {
    /// A file the loader needs is not there.
    Missing {
        /// The path it looked for, relative to the database root.
        path: String,
        /// What it was for.
        what: String,
    },
    /// A file is there but does not hold what it should.
    Malformed {
        /// The path.
        path: String,
        /// What is wrong.
        message: String,
    },
    /// The database describes a different part from the one asked for.
    WrongPart {
        /// The device the flow asked for.
        device: String,
        /// What the loader could say about why.
        message: String,
    },
    /// The IO standard asked for is not one this flow has bits for.
    NoSuchIoStandard {
        /// What was asked for.
        wanted: String,
        /// What there is.
        known: Vec<String>,
    },
    /// The region asked for is bigger than the graph representation can
    /// hold, with the numbers that say so.
    TooLarge {
        /// How many pips the region would need.
        pips: usize,
        /// The limit [`XrayOptions::max_pips`] set.
        limit: usize,
        /// How many tiles the region covers.
        tiles: usize,
    },
}

impl fmt::Display for XrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XrayError::Missing { path, what } => write!(
                f,
                "the chip database has no `{path}`, which is where {what} lives"
            ),
            XrayError::Malformed { path, message } => write!(f, "`{path}`: {message}"),
            XrayError::WrongPart { device, message } => {
                write!(f, "no chip database for `{device}`: {message}")
            }
            XrayError::NoSuchIoStandard { wanted, known } => write!(
                f,
                "no bits are known for the io standard `{wanted}`; this flow can configure \
                 only {}, and an io standard is a voltage, so it will not substitute one",
                known.join(", ")
            ),
            XrayError::TooLarge { pips, limit, tiles } => write!(
                f,
                "that region is {tiles} tile(s) and {pips} pip(s), past the limit of {limit}; \
                 narrow it with a region or raise the limit and expect to need the memory"
            ),
        }
    }
}

impl Error for XrayError {}

/// A rectangle of the tile grid, in `tilegrid.json`'s own `grid_x` /
/// `grid_y` coordinates, both ends included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridRegion {
    /// Leftmost column.
    pub x0: u32,
    /// Topmost row.
    pub y0: u32,
    /// Rightmost column.
    pub x1: u32,
    /// Bottommost row.
    pub y1: u32,
}

impl GridRegion {
    /// A rectangle, in either corner order.
    pub fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> GridRegion {
        GridRegion {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    /// A rectangle around `(x, y)`, reaching `radius` tiles each way.
    pub fn around(x: u32, y: u32, radius: u32) -> GridRegion {
        GridRegion {
            x0: x.saturating_sub(radius),
            y0: y.saturating_sub(radius),
            x1: x.saturating_add(radius),
            y1: y.saturating_add(radius),
        }
    }

    /// True when the tile at `(x, y)` is inside.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    /// How many grid positions it covers.
    pub fn area(&self) -> usize {
        let w = usize::try_from(self.x1 - self.x0 + 1).unwrap_or(0);
        let h = usize::try_from(self.y1 - self.y0 + 1).unwrap_or(0);
        w * h
    }
}

/// How much of the fabric to load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XrayOptions {
    /// The part directory inside the family, such as `xc7a35tcpg236-1`.
    /// `None` derives it from the device name and the speed grades the
    /// database is likely to carry.
    pub part: Option<String>,
    /// Which tiles get wires, pips and bels. `None` is the whole die,
    /// which for `xc7a50t` will hit [`XrayOptions::max_pips`].
    pub region: Option<GridRegion>,
    /// The largest number of graph edges the loader will build before
    /// refusing with [`XrayError::TooLarge`]. It counts first and
    /// allocates after, so the refusal costs nothing.
    ///
    /// The number counted is the edges the architecture *declares*. Not
    /// all of them survive into the graph: one that references a tile
    /// the grid does not have there is dropped and counted in
    /// [`RoutingGraph::dangling`](super::arch::RoutingGraph::dangling).
    /// For the whole `xc7a50t` that is 44 304 488 declared and
    /// 30 918 986 kept.
    ///
    /// The default is set from a measurement and not from a guess: that
    /// graph is 1386 MiB resident, 2.0 GiB at peak, built in about
    /// eight seconds. See the module docs.
    pub max_pips: usize,
    /// Which IO standard every buffer of the design is configured for.
    ///
    /// It is one setting for the whole load rather than one per pin
    /// because the bits come from a table read off Vivado's own
    /// bitstream and only one standard has been read — see
    /// [`io_standards`]. A name that is not in that list is refused with
    /// [`XrayError::NoSuchIoStandard`]: an IO standard is a voltage, and
    /// quietly substituting a different one is how a board is damaged.
    pub io_standard: String,
}

impl Default for XrayOptions {
    fn default() -> Self {
        XrayOptions {
            part: None,
            region: None,
            // Four million pips is about a gigabyte once each one owns a
            // list of configuration bits, which is as far as a desktop
            // The whole `xc7a50t` declares 44 304 488 edges and keeps
            // 30 918 986 of them, which is about two gigabytes at peak
            // now that a pip is twenty bytes and its bits are shared. A
            // part that wants more than that is refused with the
            // numbers rather than with a dead machine.
            max_pips: 48_000_000,
            io_standard: "LVCMOS33".to_owned(),
        }
    }
}

impl XrayOptions {
    /// The defaults.
    pub fn new() -> XrayOptions {
        XrayOptions::default()
    }

    /// The same options restricted to a region.
    pub fn with_region(mut self, region: GridRegion) -> XrayOptions {
        self.region = Some(region);
        self
    }

    /// The same options with a different IO standard.
    pub fn with_io_standard(mut self, standard: impl Into<String>) -> XrayOptions {
        self.io_standard = standard.into();
        self
    }
}

/// What the load cost and covered.
///
/// This is the measurement the module docs quote, produced by the loader
/// itself rather than by a note in a file, so it cannot drift.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XrayStats {
    /// Tiles in the whole fabric.
    pub tiles: usize,
    /// Distinct tile types in the whole fabric.
    pub tile_types: usize,
    /// Tiles that got wires, pips and bels.
    pub tiles_loaded: usize,
    /// Tile types that were read from a `segbits` file.
    pub tile_types_loaded: usize,
    /// Segbit features over the whole die, tile type by tile type. This
    /// is the number that decides how phase two has to be shaped.
    pub features_die: u64,
    /// Pip features over the whole die.
    pub pip_features_die: u64,
    /// Nodes the fabric has, from the database's `element_counts.csv`
    /// when it is there.
    pub nodes_die: Option<u64>,
    /// Wires declared in the loaded region.
    pub wires: usize,
    /// Pips declared in the loaded region, joins included.
    pub pips: usize,
    /// Of those, the zero-bit pips that `tileconn.json` asked for.
    pub joins: usize,
    /// Fixed connections inside a tile, one per tile *type*, taken from
    /// the `always` lines of `ppips_<type>.db`. This is the wiring
    /// between a site pin and the interconnect, and without it a bel
    /// pin reaches nothing.
    pub fixed: usize,
    /// Bels declared in the loaded region.
    pub bels: usize,
    /// What the hand-written site tables covered.
    pub coverage: SiteCoverage,
    /// Frames the part has, pad frames included.
    pub frames: usize,
    /// Tiles with a window in the frame map, which is the whole part.
    pub mapped_tiles: usize,
}

impl XrayStats {
    /// A report, one fact per line.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "chip database:");
        let _ = writeln!(
            out,
            "  fabric: {} tile(s) of {} type(s), {} frame(s)",
            self.tiles, self.tile_types, self.frames
        );
        let _ = writeln!(
            out,
            "  die-wide features: {} ({} pip(s)){}",
            self.features_die,
            self.pip_features_die,
            match self.nodes_die {
                Some(n) => format!(", {n} node(s)"),
                None => String::new(),
            }
        );
        let _ = writeln!(
            out,
            "  loaded: {} tile(s) of {} type(s), {} wire(s), {} pip(s) ({} join(s)), {} bel(s)",
            self.tiles_loaded, self.tile_types_loaded, self.wires, self.pips, self.joins, self.bels
        );
        out.push_str(&self.coverage.to_text());
        let _ = writeln!(out, "  frame map: {} tile(s)", self.mapped_tiles);
        out
    }
}

/// One tile of `tilegrid.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XrayTile {
    /// The tile's name (`CLBLL_L_X2Y0`).
    pub name: String,
    /// Its type (`CLBLL_L`).
    pub tile_type: String,
    /// Its column in the grid.
    pub grid_x: u32,
    /// Its row in the grid.
    pub grid_y: u32,
    /// Its sites, name to site type, in name order.
    pub sites: Vec<(String, String)>,
    /// Where its bits live, one entry per configuration bus it uses, in
    /// frame-address block-type order: `CLB_IO_CLK` first, then
    /// `BLOCK_RAM`. The frame map stacks them in this order, so a block RAM
    /// tile's contents are tile rows after its configuration's.
    pub bits: Vec<(String, TileBits)>,
    /// Which other tile type's `segbits` describe this tile's bits, when
    /// `tilegrid.json` says so. See [`TileAlias`].
    pub bits_alias: Option<TileAlias>,
}

impl XrayTile {
    /// The name of the tile type whose features describe this tile.
    ///
    /// Normally the tile's own type. A tile whose bits are **aliased**
    /// onto another type ([`TileAlias`]) gets a name of its own per
    /// alias offset, because two tiles of one aliased type can need two
    /// different feature sets: see [`TileAlias`] for the whole story.
    pub fn feature_type(&self) -> String {
        match &self.bits_alias {
            Some(alias) => format!("{}_W{}", self.tile_type, alias.start_offset),
            None => self.tile_type.clone(),
        }
    }

    /// How many 32-bit words wide this tile's own bitmap is, over every
    /// configuration bus it uses.
    pub(super) fn words(&self) -> u32 {
        self.bits.iter().map(|(_, b)| b.words).max().unwrap_or(0)
    }
}

/// What [`XrayDatabase::pinmap`] answers: the package pin to site map,
/// and the balls it had to refuse, each with the reason.
type PinMap = (Vec<(String, String)>, Vec<(String, String)>);

/// What `tilegrid.json` says describes a tile's bits, when they are not
/// described by the tile's own type.
///
/// # What this is for, and what it is not
///
/// prjxray ships no `segbits_liob33_sing.db`, `segbits_lioi3_sing.db`
/// or `_r` equivalent: the single-IOB tiles at the two ends of every IO
/// bank have no `segbits` file of their own. What they have instead is
/// an `alias` member on their `bits` entry, which names the tile type
/// that *does* — `LIOB33` for a `LIOB33_SING` — and the word of that
/// type's bitmap where the aliased tile's own words begin:
///
/// ```json
/// "LIOB33_SING_X0Y0": { "bits": { "CLB_IO_CLK": {
///   "baseaddr": "0x00400000", "frames": 42, "offset": 0, "words": 2,
///   "alias": { "type": "LIOB33", "start_offset": 2,
///              "sites": { "IOB33_Y0": "IOB33_Y0" } } } } }
/// ```
///
/// A `LIOB33` is four words wide and a `LIOB33_SING` two, so the
/// hypothesis that a `_SING` tile is simply its neighbour with one half
/// unpopulated is **wrong about the shape**: the shapes differ, and
/// `start_offset` is the correction. With it the rule is uniform —
/// a feature of the aliased type at bit offset `c` is at `c -
/// 32 * start_offset` here, and a feature whose bits fall outside this
/// tile's own `32 * words` is not a feature of this tile at all.
///
/// That filter is what picks the half, and it needs nothing else.
/// **CHECKED** over the whole of `segbits_liob33.db`,
/// `segbits_riob33.db`, `segbits_lioi3.db` and `segbits_rioi3.db`:
/// every `_Y0` feature's bits lie in words 2..3 and every `_Y1`
/// feature's in words 0..1, with no exceptions in 382 features. So a
/// tile with `start_offset` 2 keeps exactly the `_Y0` half and one with
/// `start_offset` 0 exactly the `_Y1` half.
///
/// And **that is what Vivado writes**, read from three of the reference
/// designs under `artix7/harness/`: `basys3/swbut` configures
/// `LIOB33_SING_X0Y0.IOB_Y0.*` (`start_offset` 2),
/// `arty-a7/swbut` configures `RIOB33_SING_X43Y50.IOB_Y0.*`
/// (`start_offset` 2) and `arty-a7/pmod` configures
/// `LIOB33_SING_X0Y99.IOB_Y1.*` (`start_offset` 0). `docs/fpga-xray.md`
/// lists every feature each one sets.
///
/// The `sites` member says the same thing a second way — the top tile's
/// lone site is aliased `IOB33_Y0` → `IOB33_Y1` — but only for the
/// `IOB` tiles; for `LIOI3_SING` it is empty although Vivado still
/// names that tile's features `ILOGIC_Y1`. So it is not read here: the
/// bit window is the rule and the `sites` map agrees with it where it
/// says anything.
///
/// # Why an aliased tile gets its own feature-type name
///
/// Two `LIOB33_SING` tiles with different `start_offset` keep different
/// halves, and the two halves' bits are **not** at the same offsets
/// within the tile (`IOB_Y0.…SLEW.SLOW` lands at bits 41..47 after the
/// shift, `IOB_Y1`'s at 16..22). One [`super::arch::TileType`] cannot
/// describe both, so [`XrayTile::feature_type`] gives each alias offset
/// a name of its own and the two become two tile types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileAlias {
    /// The tile type whose `segbits_<type>.db` describes these bits.
    pub tile_type: String,
    /// How many 32-bit words into that type's bitmap this tile's own
    /// words begin.
    pub start_offset: u32,
}

/// One feature of a `segbits` file: a name and the bits that make it
/// true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    /// The feature's name, without the leading tile type.
    pub name: String,
    /// The bits that must be one.
    pub ones: Vec<ConfigBit>,
    /// The bits that must be zero, which a zero-filled bitstream already
    /// satisfies but a differ wants to know about.
    pub zeros: Vec<ConfigBit>,
}

/// Everything one `segbits_<type>.db` says, with the pip / bel-feature
/// split already worked out.
///
/// The split costs a pass over the file's names ([`is_pip_feature`]
/// explains why it has to be inferred at all), so it is done once here
/// rather than per tile: `INT_L` has 3636 features and the die has 5650
/// interconnect tiles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeatureSet {
    features: Vec<Feature>,
    sites: HashSet<String>,
    pip_count: usize,
}

impl FeatureSet {
    /// Works out the site prefixes and the pip count for a file's
    /// features.
    pub fn new(features: Vec<Feature>) -> FeatureSet {
        let mut sites = HashSet::new();
        for feature in &features {
            let mut parts = feature.name.split('.');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(head), Some(_), Some(_)) => {
                    sites.insert(head.to_owned());
                }
                // A block RAM half's features are all two components
                // (`RAMB18_Y0.IN_USE`, `RAMB18_Y0.INIT_00[000]`), so the
                // length rule never sees the site and would read 37 000
                // of them as pips; see `bram::is_two_part_site`.
                (Some(head), Some(_), None) if bram::is_two_part_site(head) => {
                    sites.insert(head.to_owned());
                }
                _ => {}
            }
        }
        let pip_count = features
            .iter()
            .filter(|f| is_pip_feature(&f.name, &sites))
            .count();
        FeatureSet {
            features,
            sites,
            pip_count,
        }
    }

    /// Every feature, in file order.
    pub fn features(&self) -> &[Feature] {
        &self.features
    }

    /// The features, given back.
    pub fn into_features(self) -> Vec<Feature> {
        self.features
    }

    /// The feature of that name, without its leading tile type.
    pub fn feature(&self, name: &str) -> Option<&Feature> {
        self.features.iter().find(|f| f.name == name)
    }

    /// The first components that head a feature of three parts or more,
    /// which is what makes them sites rather than wires.
    pub fn sites(&self) -> &HashSet<String> {
        &self.sites
    }

    /// How many of the features are pips.
    pub fn pip_count(&self) -> usize {
        self.pip_count
    }

    /// The pips: destination wire, source wire, and the feature that
    /// carries their bits.
    pub fn pips(&self) -> impl Iterator<Item = (&str, &str, &Feature)> {
        self.features.iter().filter_map(|feature| {
            if !is_pip_feature(&feature.name, &self.sites) {
                return None;
            }
            let (to, from) = feature.name.split_once('.')?;
            Some((to, from, feature))
        })
    }
}

/// A located Project X-Ray database.
///
/// Opening one only works out where the files are; nothing is read until
/// [`XrayDatabase::load`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XrayDatabase {
    root: String,
    family: String,
    part_dir: String,
    fabric: String,
    device: String,
}

impl XrayDatabase {
    /// Finds the database for a Reticle device name.
    ///
    /// `root` is the directory holding the family directories, which is
    /// the top of a `prjxray-db` checkout. `device` is a name from
    /// [`super::device`], such as `xc7a35t-cpg236`; the part directory is
    /// derived from it by trying the speed grades the database carries
    /// unless [`XrayOptions::part`] names one.
    ///
    /// # Errors
    ///
    /// [`XrayError::WrongPart`] when the device is not a 7-series part or
    /// no part directory for it is there, and [`XrayError::Missing`] when
    /// the family's `mapping/devices.yaml` is absent.
    pub fn open(
        files: &dyn FileProvider,
        root: &str,
        device: &str,
        options: &XrayOptions,
    ) -> Result<XrayDatabase, XrayError> {
        let root = root.trim_end_matches('/').to_owned();
        let (die, package) = split_device(device).ok_or_else(|| XrayError::WrongPart {
            device: device.to_owned(),
            message: "a 7-series device name is `<die>-<package>`, as `xc7a35t-cpg236` is"
                .to_owned(),
        })?;
        let family = family_directory(&die).ok_or_else(|| XrayError::WrongPart {
            device: device.to_owned(),
            message: format!("`{die}` is not a die Project X-Ray has a family directory for"),
        })?;

        let candidates: Vec<String> = match &options.part {
            Some(part) => vec![part.clone()],
            None => ["-1", "-2", "-3", "-1L", "-2L"]
                .iter()
                .map(|grade| format!("{die}{package}{grade}"))
                .collect(),
        };
        let part_dir = candidates
            .iter()
            .find(|candidate| {
                files
                    .read_file(&format!("{root}/{family}/{candidate}/part.json"))
                    .is_some()
            })
            .cloned()
            .ok_or_else(|| XrayError::WrongPart {
                device: device.to_owned(),
                message: format!(
                    "none of {} holds a `part.json` under `{root}/{family}`",
                    candidates.join(", ")
                ),
            })?;

        let devices_path = format!("{root}/{family}/mapping/devices.yaml");
        let text = files
            .read_file(&devices_path)
            .ok_or_else(|| XrayError::Missing {
                path: devices_path.clone(),
                what: "the die-to-fabric mapping".to_owned(),
            })?;
        let fabric = fabric_of(&text, &die).ok_or_else(|| XrayError::Malformed {
            path: devices_path,
            message: format!("it names no fabric for `{die}`"),
        })?;

        Ok(XrayDatabase {
            root,
            family: family.to_owned(),
            part_dir,
            fabric,
            device: device.to_owned(),
        })
    }

    /// The database root it was opened at.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// The family directory (`artix7`).
    pub fn family(&self) -> &str {
        &self.family
    }

    /// The part directory (`xc7a35tcpg236-1`).
    pub fn part_directory(&self) -> &str {
        &self.part_dir
    }

    /// The fabric directory (`xc7a50t`), which is not the same as the
    /// part: an `xc7a35t` is an `xc7a50t` die.
    pub fn fabric(&self) -> &str {
        &self.fabric
    }

    /// Reads `part.json`: the IDCODE and the frame layout.
    ///
    /// # Errors
    ///
    /// [`XrayError::Missing`] and [`XrayError::Malformed`].
    pub fn part(&self, files: &dyn FileProvider) -> Result<Part, XrayError> {
        let path = format!("{}/{}/{}/part.json", self.root, self.family, self.part_dir);
        let text = self.read(files, &path, "the part's frame layout")?;
        let json = Json::parse(&text).map_err(|e| XrayError::Malformed {
            path: path.clone(),
            message: e.to_string(),
        })?;
        parse::part_from_json(&json, &path, &self.part_dir)
    }

    /// Loads the fabric.
    ///
    /// # Errors
    ///
    /// [`XrayError::Missing`] for an absent file, [`XrayError::Malformed`]
    /// for one that does not hold what it should, and
    /// [`XrayError::TooLarge`] when the region asked for needs more pips
    /// than [`XrayOptions::max_pips`] allows.
    pub fn load(
        &self,
        files: &dyn FileProvider,
        options: &XrayOptions,
    ) -> Result<XrayFabric, XrayError> {
        let standard = sites::io_standard(&options.io_standard).ok_or_else(|| {
            XrayError::NoSuchIoStandard {
                wanted: options.io_standard.clone(),
                known: sites::io_standards()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            }
        })?;
        let part = self.part(files)?;
        let tiles = self.tiles(files)?;
        let mut stats = XrayStats {
            tiles: tiles.len(),
            frames: part.layout.frames(),
            nodes_die: self.node_count(files),
            ..XrayStats::default()
        };

        // The frame map is the whole part, always: the bitstream is a
        // whole-part bitstream whatever the region.
        let mut frames = FrameMap::new();
        let mut width = 0u32;
        let mut height = 0u32;
        let mut types: BTreeSet<&str> = BTreeSet::new();
        for tile in &tiles {
            width = width.max(tile.grid_x + 1);
            height = height.max(tile.grid_y + 1);
            types.insert(&tile.tile_type);
            for (_, bits) in &tile.bits {
                frames.insert((tile.grid_x, tile.grid_y), *bits);
            }
        }
        stats.tile_types = types.len();
        stats.mapped_tiles = frames.len();

        // Every tile type's features, read once, whether or not the
        // region uses it: the die-wide count is the measurement. Keyed
        // by `XrayTile::feature_type`, which splits an aliased type into
        // one per alias offset; `representative` holds one tile of each
        // so the alias can be applied.
        let block_ram_rows = block_ram_rows(&tiles);
        let mut representative: BTreeMap<String, &XrayTile> = BTreeMap::new();
        for tile in &tiles {
            representative.entry(tile.feature_type()).or_insert(tile);
        }
        let mut features: HashMap<String, FeatureSet> = HashMap::new();
        for (key, tile) in &representative {
            let row = block_ram_rows.get(tile.tile_type.as_str()).copied();
            if let Some(set) = self.features_for(files, tile, row)? {
                features.insert(key.clone(), set);
            }
        }

        // And every tile type's pseudo-pips, which are what say how a
        // site pin reaches the interconnect. They carry no bits, so
        // reading them all costs almost nothing.
        let mut fixed: HashMap<String, Vec<parse::Ppip>> = HashMap::new();
        for name in &types {
            let path = format!(
                "{}/{}/ppips_{}.db",
                self.root,
                self.family,
                name.to_lowercase()
            );
            let Some(text) = files.read_file(&path) else {
                continue;
            };
            fixed.insert((*name).to_owned(), parse::ppips(&text, &path)?);
        }
        for tile in &tiles {
            if let Some(set) = features.get(&tile.feature_type()) {
                stats.features_die += u64::try_from(set.features().len()).unwrap_or(0);
                stats.pip_features_die += u64::try_from(set.pip_count()).unwrap_or(0);
            }
        }

        let region = options.region.unwrap_or(GridRegion::new(
            0,
            0,
            width.saturating_sub(1),
            height.saturating_sub(1),
        ));

        let conn = self.tileconn(files)?;

        // Count before allocating, so a region that will not fit is
        // refused with numbers instead of a dead machine. The count is
        // of graph *edges*, which is what costs memory: the pips that
        // have bits, the fixed connections `ppips` adds, and two per
        // wire pair of every join whose other end is a tile type the
        // region also holds. That is exactly what `build_arch` declares;
        // the graph then drops the ones that fall off the grid, so this
        // is a tight upper bound rather than a loose one.
        let in_region_types: HashSet<&str> = tiles
            .iter()
            .filter(|t| region.contains(t.grid_x, t.grid_y))
            .map(|t| t.tile_type.as_str())
            .collect();
        let mut join_edges: HashMap<&str, usize> = HashMap::new();
        for c in &conn {
            if in_region_types.contains(c.types.1.as_str()) {
                *join_edges.entry(c.types.0.as_str()).or_default() += c.pairs.len() * 2;
            }
        }
        let mut wanted = 0usize;
        let mut in_region = 0usize;
        for tile in &tiles {
            if !region.contains(tile.grid_x, tile.grid_y) {
                continue;
            }
            in_region += 1;
            let name = tile.tile_type.as_str();
            if let Some(set) = features.get(&tile.feature_type()) {
                wanted += set.pip_count();
            }
            if let Some(ppips) = fixed.get(name) {
                wanted += ppips
                    .iter()
                    .filter(|p| p.kind == parse::PpipKind::Always)
                    .count();
            }
            wanted += join_edges.get(name).copied().unwrap_or(0);
        }
        if wanted > options.max_pips {
            return Err(XrayError::TooLarge {
                pips: wanted,
                limit: options.max_pips,
                tiles: in_region,
            });
        }

        let mut arch = self.build_arch(
            &tiles, &features, &fixed, &conn, region, &part, standard, &mut stats,
        );
        let (pinmap, refused) = self.pinmap(files, &tiles, &features, region)?;
        arch.pinmap = pinmap;
        arch.unplaceable_pins = refused;
        stats.tiles_loaded = in_region;

        Ok(XrayFabric {
            part,
            arch,
            frames,
            clocks: clock_column(&features),
            block_ram: BlockRamTables::new(&features, &fixed),
            stats,
        })
    }

    /// Reads a bitstream back into the database's own feature names.
    ///
    /// This is the inverse of everything else here, and it exists so
    /// that a bitstream can be *checked against another bitstream*
    /// rather than against a story about what it should contain: decode
    /// Vivado's file and decode Reticle's, and the difference is a list
    /// of feature names a human can read.
    ///
    /// A feature is reported when every bit its `segbits` line says must
    /// be one is one and every bit it says must be zero is zero. A
    /// feature with no `one` bits at all is never reported, because it
    /// would match an empty bitstream everywhere; `IN_TERM.NONE` is such
    /// a feature and its absence from the output means nothing.
    ///
    /// [`Decoded::features`] is sorted by tile name and then by feature
    /// name, so two decodings can be compared line by line, and the rest
    /// of [`Decoded`] accounts for every set bit: how many a named
    /// feature explains, how many fall in a tile whose bits nothing
    /// names, and how many fall outside every tile the grid has. That
    /// accounting is the honest part. A decoding that names forty
    /// features and leaves two thousand bits unexplained has not
    /// understood the bitstream, and saying so is the point.
    ///
    /// # Errors
    ///
    /// Those of [`XrayDatabase::tiles`] and of reading the `segbits`
    /// files.
    pub fn decode(
        &self,
        files: &dyn FileProvider,
        data: &super::xc7::FrameData,
    ) -> Result<Decoded, XrayError> {
        let tiles = self.tiles(files)?;

        // Where the set bits are, by frame address, so a tile only has
        // to look at frames that hold something.
        let mut set: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for (position, address) in data.layout().order().iter().enumerate() {
            let Some(address) = address else { continue };
            let Some(frame) = data.frame(position) else {
                continue;
            };
            let mut here = Vec::new();
            for (word, value) in frame.iter().enumerate() {
                if *value == 0 {
                    continue;
                }
                for bit in 0..32u32 {
                    if (*value >> bit) & 1 == 1 {
                        here.push((u32::try_from(word).unwrap_or(0), bit));
                    }
                }
            }
            if !here.is_empty() {
                set.insert(address.to_u32(), here);
            }
        }

        let total: usize = set.values().map(Vec::len).sum();

        // Every set bit, so that one a feature explains can be struck
        // off and whatever is left can be counted rather than glossed.
        let mut unexplained: HashSet<(u32, u32, u32)> = HashSet::new();
        for (address, here) in &set {
            for (word, bit) in here {
                unexplained.insert((*address, *word, *bit));
            }
        }

        let block_ram_rows = block_ram_rows(&tiles);
        let mut features: HashMap<String, FeatureSet> = HashMap::new();
        let mut out = Vec::new();
        let mut tiles_touched = 0usize;
        let mut tiles_unnamed: Vec<String> = Vec::new();
        for tile in &tiles {
            // The tile's own bits, in the tile-local coordinates a
            // `segbits` line uses, remembering where each came from so
            // a matched feature can strike it off.
            let mut ones: HashMap<ConfigBit, (u32, u32, u32)> = HashMap::new();
            // The windows stack, as `xc7::FrameMap` stacks them.
            let mut first_row = 0u32;
            for (_, window) in &tile.bits {
                let base_row = first_row;
                first_row += window.frames;
                for row in 0..window.frames {
                    let Some(address) = window.baseaddr.checked_add(row) else {
                        continue;
                    };
                    let Some(here) = set.get(&address) else {
                        continue;
                    };
                    for (word, bit) in here {
                        if *word < window.offset || *word >= window.offset + window.words {
                            continue;
                        }
                        ones.insert(
                            ConfigBit::new(base_row + row, (*word - window.offset) * 32 + *bit),
                            (address, *word, *bit),
                        );
                    }
                }
            }
            if ones.is_empty() {
                continue;
            }
            tiles_touched += 1;

            let key = tile.feature_type();
            if !features.contains_key(&key) {
                let row = block_ram_rows.get(tile.tile_type.as_str()).copied();
                let set = self.features_for(files, tile, row)?.unwrap_or_default();
                features.insert(key.clone(), set);
            }
            let Some(known) = features.get(&key) else {
                continue;
            };
            if known.features().is_empty() {
                tiles_unnamed.push(format!("{} ({})", tile.name, tile.tile_type));
                continue;
            }
            for feature in known.features() {
                if feature.ones.is_empty() {
                    continue;
                }
                if feature.ones.iter().all(|b| ones.contains_key(b))
                    && feature.zeros.iter().all(|b| !ones.contains_key(b))
                {
                    out.push((tile.name.clone(), feature.name.clone()));
                    for bit in &feature.ones {
                        if let Some(where_it_is) = ones.get(bit) {
                            unexplained.remove(where_it_is);
                        }
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(Decoded {
            features: out,
            bits: total,
            unexplained: unexplained.len(),
            tiles: tiles_touched,
            tiles_without_a_segbits_file: tiles_unnamed.len(),
            tiles_with_no_segbits_file: {
                tiles_unnamed.sort();
                tiles_unnamed.dedup();
                tiles_unnamed
            },
        })
    }

    /// Every feature of one tile type: `segbits_<type>.db`, and for a tile
    /// type with a `BLOCK_RAM` window also `segbits_<type>.block_ram.db`,
    /// whose frame numbers count from that window's first frame and are
    /// moved to the tile rows the frame map stacks it at
    /// (`block_ram_row`). `None` when the type has no `segbits` file.
    ///
    /// The contents features (`RAMB18_Y0.INIT_00[000]` and so on) are
    /// ordinary one-bit features with an index, so they become
    /// [`ConfigEntry::Param`](super::arch::ConfigEntry::Param) entries on
    /// the block RAM's bel exactly the way a lookup table's `INIT[63:0]`
    /// does, and a `RAMB18E1` cell's `INIT_00` reaches the frames with no
    /// code that knows what a block RAM is. Which frame bit carries which
    /// `INIT` bit is therefore the database's statement, not this
    /// crate's: prjxray's fuzzer `026-bram-data` measured it by giving
    /// Vivado random 256-bit `INIT_xx` values and solving for the bits.
    fn features_of(
        &self,
        files: &dyn FileProvider,
        tile_type: &str,
        block_ram_row: Option<u32>,
    ) -> Result<Option<FeatureSet>, XrayError> {
        let base = format!(
            "{}/{}/segbits_{}",
            self.root,
            self.family,
            tile_type.to_lowercase()
        );
        let path = format!("{base}.db");
        let Some(text) = files.read_file(&path) else {
            return Ok(None);
        };
        let set = parse::segbits(&text, &path)?;
        let Some(row) = block_ram_row else {
            return Ok(Some(set));
        };
        let path = format!("{base}.block_ram.db");
        let Some(text) = files.read_file(&path) else {
            return Ok(Some(set));
        };
        let contents = parse::segbits(&text, &path)?;
        let mut all = set.into_features();
        for mut feature in contents.into_features() {
            for bit in feature.ones.iter_mut().chain(feature.zeros.iter_mut()) {
                bit.row += row;
            }
            all.push(feature);
        }
        Ok(Some(FeatureSet::new(all)))
    }

    /// Every feature of the tile type `tile` belongs to, which is
    /// [`XrayTile::feature_type`] and not always the tile's own type.
    ///
    /// A tile whose own type has a `segbits` file is read from it. One
    /// that does not but carries a [`TileAlias`] is read from the
    /// aliased type's file, with every bit moved by the alias offset and
    /// every feature that then falls outside this tile's own bitmap
    /// dropped — which is what leaves exactly the half the tile has.
    /// [`TileAlias`] is the whole account, including what measured it.
    ///
    /// The tile's own file wins where there is one, so a database that
    /// grows a `segbits_liob33_sing.db` is believed over the alias
    /// without a code change.
    ///
    /// # Errors
    ///
    /// Those of [`XrayDatabase::features_of`].
    fn features_for(
        &self,
        files: &dyn FileProvider,
        tile: &XrayTile,
        block_ram_row: Option<u32>,
    ) -> Result<Option<FeatureSet>, XrayError> {
        if let Some(set) = self.features_of(files, &tile.tile_type, block_ram_row)? {
            return Ok(Some(set));
        }
        let Some(alias) = &tile.bits_alias else {
            return Ok(None);
        };
        let Some(set) = self.features_of(files, &alias.tile_type, block_ram_row)? else {
            return Ok(None);
        };
        let shift = alias.start_offset * 32;
        let limit = shift + tile.words() * 32;
        let mut kept = Vec::new();
        for mut feature in set.into_features() {
            if feature
                .ones
                .iter()
                .chain(feature.zeros.iter())
                .any(|bit| bit.col < shift || bit.col >= limit)
            {
                continue;
            }
            for bit in feature.ones.iter_mut().chain(feature.zeros.iter_mut()) {
                bit.col -= shift;
            }
            kept.push(feature);
        }
        Ok(Some(FeatureSet::new(kept)))
    }

    /// A region that covers the tiles the given package pins sit in,
    /// with `margin` tiles of fabric around them.
    ///
    /// This is how a flow picks a region without a human choosing
    /// coordinates: the design's constrained pins say where on the die
    /// it has to be, and the logic that serves them has to be near. It
    /// is a convenience over [`XrayDatabase::tiles`], not a placement
    /// decision — the region it gives is where the loader will *look*,
    /// and a design that does not fit in it fails to place rather than
    /// spilling outside it.
    ///
    /// `None` when none of the pins names a site the grid has.
    ///
    /// # Errors
    ///
    /// Those of [`XrayDatabase::tiles`].
    pub fn region_for_pins(
        &self,
        files: &dyn FileProvider,
        pins: &[String],
        margin: u32,
    ) -> Result<Option<GridRegion>, XrayError> {
        let tiles = self.tiles(files)?;
        let path = format!(
            "{}/{}/{}/package_pins.csv",
            self.root, self.family, self.part_dir
        );
        let text = self.read(files, &path, "the package pin map")?;
        let wanted: HashSet<&str> = pins.iter().map(String::as_str).collect();
        let sites: HashSet<String> = parse::package_pins(&text)
            .into_iter()
            .filter(|(pin, _)| wanted.contains(pin.as_str()))
            .map(|(_, site)| site)
            .collect();

        let mut region: Option<GridRegion> = None;
        for tile in &tiles {
            if !tile.sites.iter().any(|(name, _)| sites.contains(name)) {
                continue;
            }
            let here = GridRegion::around(tile.grid_x, tile.grid_y, margin);
            region = Some(match region {
                None => here,
                Some(r) => GridRegion {
                    x0: r.x0.min(here.x0),
                    y0: r.y0.min(here.y0),
                    x1: r.x1.max(here.x1),
                    y1: r.y1.max(here.y1),
                },
            });
        }
        Ok(region)
    }

    /// The same region, grown to hold the nearest tile with a site of
    /// type `site_type`.
    ///
    /// A clock is why this exists. [`XrayDatabase::region_for_pins`]
    /// puts the fabric where the design's pins are, which is right for
    /// combinational logic and wrong for anything with a global buffer:
    /// a 7-series `BUFGCTRL` sits in one column in the middle of the
    /// die, far from any pad, and a clock that cannot reach it cannot be
    /// routed. Growing the rectangle to the *nearest* such tile, rather
    /// than to all of them, is what keeps the region a band across the
    /// die instead of the whole die.
    ///
    /// `None` when the part has no site of that type at all.
    ///
    /// # Errors
    ///
    /// Those of [`XrayDatabase::tiles`].
    pub fn region_with_site_type(
        &self,
        files: &dyn FileProvider,
        region: GridRegion,
        site_type: &str,
    ) -> Result<Option<GridRegion>, XrayError> {
        let tiles = self.tiles(files)?;
        let mut best: Option<(u64, u32, u32)> = None;
        for tile in &tiles {
            if !tile.sites.iter().any(|(_, kind)| kind == site_type) {
                continue;
            }
            let dx = u64::from(
                region
                    .x0
                    .saturating_sub(tile.grid_x)
                    .max(tile.grid_x.saturating_sub(region.x1)),
            );
            let dy = u64::from(
                region
                    .y0
                    .saturating_sub(tile.grid_y)
                    .max(tile.grid_y.saturating_sub(region.y1)),
            );
            let distance = dx + dy;
            if best.is_none_or(|(b, _, _)| distance < b) {
                best = Some((distance, tile.grid_x, tile.grid_y));
            }
        }
        Ok(best.map(|(_, x, y)| GridRegion {
            x0: region.x0.min(x),
            y0: region.y0.min(y),
            x1: region.x1.max(x),
            y1: region.y1.max(y),
        }))
    }

    /// The same region, grown so that every clock region it overlaps has
    /// its `HCLK` row inside it.
    ///
    /// A flip-flop is clocked from the global network through its own clock
    /// region's horizontal clock row: the `HCLK` tile drives the leaves,
    /// and the leaves drive `GCLK_B` in every interconnect tile of the
    /// region. A region loaded around the design's pins can cover some rows
    /// of a clock region without its `HCLK` row, and then nothing placed in
    /// those rows can reach the network at all. That happened on
    /// `examples/basys3/iso7816_terminal.v`: the host UART's synchroniser
    /// landed at `X14Y51`, the clock region of rows 0-51 had no `HCLK` row
    /// loaded, and the router brought the clock down from the region above
    /// through `GFAN` and a chain of bounce wires — the kind of path that
    /// gave a hold violation on the part.
    ///
    /// The clock regions are read off the grid rather than assumed: each
    /// `HCLK_L`/`HCLK_R` row is one region's centre, and a region's
    /// boundary is midway between two centres — on this part rows 26, 78
    /// and 130, with the `BRKH` break rows at 52 and 104, which is where the
    /// midpoints fall.
    ///
    /// # Errors
    ///
    /// Those of [`XrayDatabase::tiles`].
    pub fn region_with_clock_rows(
        &self,
        files: &dyn FileProvider,
        region: GridRegion,
    ) -> Result<GridRegion, XrayError> {
        let tiles = self.tiles(files)?;
        let rows: BTreeSet<u32> = tiles
            .iter()
            .filter(|tile| matches!(tile.tile_type.as_str(), "HCLK_L" | "HCLK_R"))
            .map(|tile| tile.grid_y)
            .collect();
        let rows: Vec<u32> = rows.into_iter().collect();
        let mut out = region;
        for (index, row) in rows.iter().enumerate() {
            let top = index
                .checked_sub(1)
                .map_or(0, |before| u32::midpoint(rows[before], *row) + 1);
            let bottom = rows
                .get(index + 1)
                .map_or(u32::MAX, |after| u32::midpoint(*row, *after));
            if region.y0 <= bottom && region.y1 >= top {
                out.y0 = out.y0.min(*row);
                out.y1 = out.y1.max(*row);
            }
        }
        Ok(out)
    }

    /// Every tile of `tilegrid.json`, in name order.
    ///
    /// # Errors
    ///
    /// [`XrayError::Missing`] and [`XrayError::Malformed`].
    pub fn tiles(&self, files: &dyn FileProvider) -> Result<Vec<XrayTile>, XrayError> {
        let path = format!("{}/{}/tilegrid.json", self.root, self.fabric_dir());
        let text = self.read(files, &path, "the tile grid")?;
        let json = Json::parse(&text).map_err(|e| XrayError::Malformed {
            path: path.clone(),
            message: e.to_string(),
        })?;
        parse::tilegrid(&json, &path)
    }

    /// `tileconn.json`, as (tile type, tile type, delta, wire pairs).
    fn tileconn(&self, files: &dyn FileProvider) -> Result<Vec<parse::TileConn>, XrayError> {
        let path = format!("{}/{}/tileconn.json", self.root, self.fabric_dir());
        let text = self.read(files, &path, "the tile-to-tile wire joins")?;
        let json = Json::parse(&text).map_err(|e| XrayError::Malformed {
            path: path.clone(),
            message: e.to_string(),
        })?;
        parse::tileconn(&json, &path)
    }

    /// The package pin to site map, in the architecture's own site
    /// names.
    ///
    /// A package pin names a site (`IOB_X0Y26`) and the architecture
    /// names a bel (`IOB_Y0`), because a bel belongs to a tile type and
    /// a site name does not; [`parse::bel_of_site`] is the translation,
    /// and without it a constrained pin resolves to nothing. Pins whose
    /// site is outside the loaded region are dropped, since there is no
    /// site for them to name.
    ///
    /// The second list is every ball the package has that this flow
    /// cannot place, each with the reason, which becomes the body of the
    /// diagnostic a design constraining such a ball gets
    /// ([`Arch::unplaceable_pins`]). A ball is unplaceable when nothing
    /// in the database describes the bits of the tile it sits in — the
    /// state the `_SING` IO tiles were in until the alias of
    /// [`TileAlias`] was read — or when the tile type has features but
    /// none of them claims this ball's site, which is what a
    /// multi-gigabit transceiver or an analogue-monitor ball looks like.
    /// Saying which is the whole point: "nothing is there" and "this
    /// flow has no table for it" are different problems and one of them
    /// is ours.
    fn pinmap(
        &self,
        files: &dyn FileProvider,
        tiles: &[XrayTile],
        features: &HashMap<String, FeatureSet>,
        region: GridRegion,
    ) -> Result<PinMap, XrayError> {
        let path = format!(
            "{}/{}/{}/package_pins.csv",
            self.root, self.family, self.part_dir
        );
        let text = self.read(files, &path, "the package pin map")?;
        let mut where_is: HashMap<&str, &XrayTile> = HashMap::new();
        for tile in tiles {
            if !region.contains(tile.grid_x, tile.grid_y) {
                continue;
            }
            for (name, _) in &tile.sites {
                where_is.insert(name.as_str(), tile);
            }
        }
        let empty = FeatureSet::default();
        let mut out = Vec::new();
        let mut refused = Vec::new();
        for (pin, site) in parse::package_pins(&text) {
            let Some(tile) = where_is.get(site.as_str()) else {
                continue;
            };
            let set = features.get(&tile.feature_type()).unwrap_or(&empty);
            if set.features().is_empty() {
                refused.push((
                    pin,
                    format!(
                        "it is site `{site}` of tile `{}`, and nothing in this database \
                         describes the bits of a `{}` — there is no `segbits_{}.db` and \
                         `tilegrid.json` gives the tile no alias onto a type that has one",
                        tile.name,
                        tile.tile_type,
                        tile.tile_type.to_lowercase()
                    ),
                ));
                continue;
            }
            let Some(bel) = parse::bel_of_site(tile, &site, set) else {
                continue;
            };
            if !set.sites().contains(&bel) {
                // `bel_of_site` falls back to the site's own name when no
                // feature prefix claims it, which places the pin on a bel
                // `bels_of` never declared. Refuse it here, where the tile
                // is still in hand to say so, instead of leaving placement
                // to report a site that does not exist.
                refused.push((
                    pin,
                    format!(
                        "it is site `{site}` of tile `{}`, a `{}`, and no feature of that \
                         tile type names that site, so this flow has no bel for it",
                        tile.name, tile.tile_type
                    ),
                ));
                continue;
            }
            out.push((pin, format!("X{}Y{}/{bel}", tile.grid_x, tile.grid_y)));
        }
        refused.sort();
        Ok((out, refused))
    }

    /// The node count the database's own `element_counts.csv` states, for
    /// the record; `None` when that file is not there.
    fn node_count(&self, files: &dyn FileProvider) -> Option<u64> {
        let path = format!("{}/{}/element_counts.csv", self.root, self.family);
        let text = files.read_file(&path)?;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("nodes,") {
                return rest.trim().parse().ok();
            }
        }
        None
    }

    fn fabric_dir(&self) -> String {
        format!("{}/{}", self.family, self.fabric)
    }

    fn read(&self, files: &dyn FileProvider, path: &str, what: &str) -> Result<String, XrayError> {
        files.read_file(path).ok_or_else(|| XrayError::Missing {
            path: path.to_owned(),
            what: what.to_owned(),
        })
    }

    /// Turns the parsed files into an [`Arch`].
    #[allow(clippy::too_many_arguments)]
    fn build_arch(
        &self,
        tiles: &[XrayTile],
        features: &HashMap<String, FeatureSet>,
        fixed: &HashMap<String, Vec<parse::Ppip>>,
        conn: &[parse::TileConn],
        region: GridRegion,
        part: &Part,
        standard: &sites::IoStandard,
        stats: &mut XrayStats,
    ) -> Arch {
        let mut width = 0u32;
        let mut height = 0u32;
        for tile in tiles {
            width = width.max(tile.grid_x + 1);
            height = height.max(tile.grid_y + 1);
        }

        // Which tile types the region actually uses, and each one's
        // bitmap shape. `tilegrid.json` keeps `frames` and `words`
        // constant within a type and varies only `offset`, which is the
        // tile's column inside the frame and belongs to the frame map.
        // A tile on two configuration buses — a block RAM, whose
        // contents are on `BLOCK_RAM` — stacks its windows, so its
        // bitmap is their frames added up; see `xc7::FrameMap`.
        // A tile whose bits are aliased gets a tile type per alias
        // offset, so the key here is `XrayTile::feature_type` and the
        // *database's* type is carried alongside: it is what names the
        // `ppips` file, the `sites` tables and the `tileconn` joins,
        // none of which the alias touches. See [`TileAlias`].
        let mut used: BTreeMap<String, (u32, u32, &str)> = BTreeMap::new();
        for tile in tiles {
            if !region.contains(tile.grid_x, tile.grid_y) {
                continue;
            }
            let rows = tile.bits.iter().map(|(_, b)| b.frames).sum();
            let cols = tile
                .bits
                .iter()
                .map(|(_, b)| b.words * 32)
                .max()
                .unwrap_or(0);
            used.entry(tile.feature_type())
                .or_insert((rows, cols, tile.tile_type.as_str()));
        }
        // Which tile types of the region each database type stands for:
        // one of itself, or one per alias offset. A join names the far
        // end's database type, and the tile there may be any of them.
        let mut variants: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (key, (_, _, db_type)) in &used {
            variants.entry(db_type).or_default().push(key.as_str());
        }

        // Which joins each tile type owns. A `tileconn` entry names two
        // types and a delta; the pips go on the first of the two, one
        // each way, because a join is metal and metal has no direction.
        let mut joins: HashMap<&str, Vec<Join<'_>>> = HashMap::new();
        for c in conn {
            joins.entry(c.types.0.as_str()).or_default().push((
                c.types.1.as_str(),
                c.delta.0,
                c.delta.1,
                &c.pairs,
            ));
        }

        let mut arch = Arch::new(
            format!("{}-{}", self.family, self.fabric),
            "xc7",
            width,
            height,
        );
        arch.parts.push(self.device.clone());
        arch.asc_device = part.name.clone();

        let empty = FeatureSet::default();
        let no_ppips: Vec<parse::Ppip> = Vec::new();
        let mut index_of: HashMap<&str, usize> = HashMap::new();
        for (key, (rows, cols, name)) in &used {
            let key = key.as_str();
            let mut tile_type = TileType::new(key, key, *rows, *cols);
            let set = features.get(key).unwrap_or(&empty);
            let ppips = fixed.get(*name).unwrap_or(&no_ppips);

            // A fixed path through a site is declared here rather than
            // taken from `ppips`, because it is not free: see
            // `sites::pass_throughs`. Where the two describe the same
            // pair, this one wins, or the router would find a copy of
            // the hop with no bits on it and turn nothing on.
            let passes = sites::pass_throughs(name);
            let overridden: HashSet<(&str, &str)> = passes
                .iter()
                .map(|p| (p.to.as_str(), p.from.as_str()))
                .collect();

            let mut wires: BTreeSet<&str> = BTreeSet::new();
            for (to, from, _) in set.pips() {
                wires.insert(to);
                wires.insert(from);
            }
            for ppip in ppips.iter().filter(|p| p.kind == parse::PpipKind::Always) {
                wires.insert(ppip.to.as_str());
                wires.insert(ppip.from.as_str());
            }
            for pass in &passes {
                wires.insert(pass.to.as_str());
                wires.insert(pass.from.as_str());
            }
            for (_, _, _, pairs) in joins.get(*name).into_iter().flatten() {
                for (mine, _) in pairs.iter() {
                    wires.insert(mine.as_str());
                }
            }
            // The other end of a join this type does not own still has
            // to exist as a wire of this type.
            for c in conn.iter().filter(|c| c.types.1 == **name) {
                for (_, theirs) in &c.pairs {
                    wires.insert(theirs.as_str());
                }
            }
            for wire in &wires {
                tile_type.wires.push(WireDecl {
                    name: (*wire).to_owned(),
                    dx: 0,
                    dy: 0,
                });
            }

            // A wire of a clock row costs bits merely to be touched: the
            // buffer sitting on it has an enable, and prjxray names it
            // with a one-component feature that is neither a pip nor a
            // bel feature. Worked out once per tile type, since it is a
            // property of the wire and not of the pip.
            // See [`sites::wire_enable_features`].
            let mut enables: HashMap<&str, Vec<ConfigBit>> = HashMap::new();
            for wire in &wires {
                let mut bits = Vec::new();
                for name in sites::wire_enable_features(wire) {
                    if let Some(feature) = set.feature(&name) {
                        bits.extend(feature.ones.iter().copied());
                    }
                }
                if !bits.is_empty() {
                    enables.insert(wire, bits);
                }
            }
            let enable_bits = |a: &str, b: &str| -> Vec<ConfigBit> {
                let mut bits = Vec::new();
                for wire in [a, b] {
                    if let Some(extra) = enables.get(wire) {
                        bits.extend(extra.iter().copied());
                    }
                }
                bits
            };

            for (to, from, feature) in set.pips() {
                if bram::is_cascade_input(name, from) {
                    continue;
                }
                let mut bits = feature.ones.clone();
                let extra = enable_bits(to, from);
                if !extra.is_empty() {
                    stats.coverage.wire_enables += 1;
                    bits.extend(extra);
                }
                tile_type.pips.push(PipDecl {
                    from: WireRef::local(from),
                    to: WireRef::local(to),
                    bits,
                });
            }
            // The fixed wiring inside a tile: a site pin reaching its
            // interconnect wires. Only `always` is metal; see
            // [`parse::PpipKind`] for why the other two are not.
            for ppip in ppips.iter().filter(|p| p.kind == parse::PpipKind::Always) {
                if overridden.contains(&(ppip.to.as_str(), ppip.from.as_str())) {
                    continue;
                }
                let bits = enable_bits(&ppip.to, &ppip.from);
                if !bits.is_empty() {
                    stats.coverage.wire_enables += 1;
                }
                tile_type.pips.push(PipDecl {
                    from: WireRef::local(ppip.from.clone()),
                    to: WireRef::local(ppip.to.clone()),
                    bits,
                });
                stats.fixed += 1;
            }
            for pass in &passes {
                let mut bits = Vec::new();
                let mut missing = false;
                for feature in &pass.features {
                    match set.feature(feature) {
                        Some(f) => bits.extend(f.ones.iter().copied()),
                        None => missing = true,
                    }
                }
                if missing {
                    // Better no path than a path that turns on half a
                    // site: the design fails to route and says so.
                    stats.coverage.pass_throughs_unresolved += 1;
                    continue;
                }
                let extra = enable_bits(&pass.to, &pass.from);
                if !extra.is_empty() {
                    stats.coverage.wire_enables += 1;
                    bits.extend(extra);
                }
                tile_type.pips.push(PipDecl {
                    from: WireRef::local(pass.from.clone()),
                    to: WireRef::local(pass.to.clone()),
                    bits,
                });
                stats.coverage.pass_throughs += 1;
            }
            for (other, dx, dy, pairs) in joins.get(*name).into_iter().flatten() {
                // `tileconn.json` names the far end's *database* type,
                // and an aliased type stands for one tile type per alias
                // offset. The join is declared once for each, and at
                // most one of them resolves, because the tile at the
                // offset is of exactly one type: see `WireRef::at_in`.
                let Some(ends) = variants.get(other) else {
                    continue;
                };
                for (mine, theirs) in pairs.iter() {
                    for other in ends {
                        // Only `mine` is a wire of this tile type, so only
                        // its enable can be charged here; the other end's
                        // belongs to the tile at the other end and is
                        // charged by that type's own joins.
                        let bits = enables.get(mine.as_str()).cloned().unwrap_or_default();
                        if !bits.is_empty() {
                            stats.coverage.wire_enables += 2;
                        }
                        // The far end names its tile *type* as well as its
                        // offset. Without that, a join lands on whatever
                        // tile sits at the offset and happens to have a wire
                        // of that name — and `tileconn.json` pairs the same
                        // name at the same offset for two different types
                        // wherever a tall tile is cut into an upper and a
                        // lower half. See `WireRef::tile_type`.
                        tile_type.pips.push(PipDecl {
                            from: WireRef::local(mine.clone()),
                            to: WireRef::at_in(theirs.clone(), *dx, *dy, *other),
                            bits: bits.clone(),
                        });
                        tile_type.pips.push(PipDecl {
                            from: WireRef::at_in(theirs.clone(), *dx, *dy, *other),
                            to: WireRef::local(mine.clone()),
                            bits,
                        });
                    }
                }
            }

            index_of.insert(key, arch.tile_types.len());
            arch.tile_types.push(tile_type);
        }

        // Bels are declared on the tile type, but `tilegrid.json` names
        // sites per tile, so the first tile of each type that has any
        // supplies the site types the type's bels are built from.
        let mut seeded: HashSet<String> = HashSet::new();
        for tile in tiles {
            if !region.contains(tile.grid_x, tile.grid_y) || tile.sites.is_empty() {
                continue;
            }
            let key = tile.feature_type();
            if !seeded.insert(key.clone()) {
                continue;
            }
            let Some(index) = index_of.get(key.as_str()) else {
                continue;
            };
            let set = features.get(&key).unwrap_or(&empty);
            // A pin may only name a wire the tile type really declares,
            // so the wire list is handed in and a pin that misses it is
            // counted rather than left to dangle.
            let declared: HashSet<&str> = arch.tile_types[*index]
                .wires
                .iter()
                .map(|w| w.name.as_str())
                .collect();
            let bels = parse::bels_of(tile, set, &declared, Some(standard), &mut stats.coverage);
            arch.tile_types[*index].bels = bels;
        }

        for tile in tiles {
            if !region.contains(tile.grid_x, tile.grid_y) {
                continue;
            }
            if let Some(index) = index_of.get(tile.feature_type().as_str()) {
                arch.set_tile(tile.grid_x, tile.grid_y, *index);
            }
        }

        stats.tile_types_loaded = arch.tile_types.len();
        let (mut wires, mut pips, mut bels, mut joined) = (0usize, 0usize, 0usize, 0usize);
        for y in 0..arch.height {
            for x in 0..arch.width {
                let Some(index) = arch.tile_index_at(x, y) else {
                    continue;
                };
                let t = &arch.tile_types[index];
                wires += t.wires.len();
                pips += t.pips.len();
                bels += t.bels.len();
                joined += t.pips.iter().filter(|p| p.bits.is_empty()).count();
            }
        }
        stats.wires = wires;
        stats.pips = pips;
        stats.bels = bels;
        stats.joins = joined;
        arch
    }
}

/// The tile row each tile type's `BLOCK_RAM` window starts at, once the
/// windows are stacked in [`XrayTile::bits`] order; only the types that
/// have such a window are present. `tilegrid.json` gives every tile of a
/// type the same window shapes, so the first tile of a type decides.
fn block_ram_rows(tiles: &[XrayTile]) -> HashMap<&str, u32> {
    let mut out = HashMap::new();
    for tile in tiles {
        if out.contains_key(tile.tile_type.as_str()) {
            continue;
        }
        let mut row = 0u32;
        for (bus, window) in &tile.bits {
            if bus == "BLOCK_RAM" {
                out.insert(tile.tile_type.as_str(), row);
                break;
            }
            row += window.frames;
        }
    }
    out
}

/// One `tileconn` entry seen from the tile type that owns it: the type
/// at the other end, the grid delta to it, and the wire pairs.
type Join<'a> = (&'a str, i32, i32, &'a [(String, String)]);

/// What [`XrayDatabase::decode`] made of a bitstream.
///
/// The three counts after [`Decoded::features`] are what keeps the
/// decoding honest: a list of feature names on its own looks like
/// understanding, and the difference between [`Decoded::bits`] and
/// [`Decoded::unexplained`] says how much of it there really is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// Tile name and feature name, sorted, one per feature the bits
    /// satisfy.
    pub features: Vec<(String, String)>,
    /// How many bits the image sets in all.
    pub bits: usize,
    /// How many of those no named feature accounts for. A bit is
    /// accounted for when it is one of the `one` bits of a feature that
    /// matched.
    pub unexplained: usize,
    /// How many tiles hold at least one set bit.
    pub tiles: usize,
    /// Of those, how many have no `segbits` file at all, so nothing
    /// their bits say could have been named.
    pub tiles_without_a_segbits_file: usize,
    /// **Which** ones, `name (type)`, sorted. The count on its own cannot
    /// be judged: "1 tile with no segbits file" could be a harmless
    /// configuration tile or the one holding the bits that matter, and
    /// bits in these tiles are left out of [`Decoded::unexplained`]
    /// rather than counted — so this list is the only thing that says
    /// whether a clean `unexplained` means what it appears to.
    pub tiles_with_no_segbits_file: Vec<String>,
}

impl Decoded {
    /// A report, one fact per line.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "decoded: {} bit(s) over {} tile(s) into {} feature(s); \
             {} bit(s) unexplained, {} tile(s) with no segbits file",
            self.bits,
            self.tiles,
            self.features.len(),
            self.unexplained,
            self.tiles_without_a_segbits_file
        );
        // Name them. A count cannot be judged; a name can be looked up.
        for which in &self.tiles_with_no_segbits_file {
            let _ = writeln!(
                out,
                "  no segbits for {which}; its bits are not counted above"
            );
        }
        out
    }

    /// Every feature this decoding found in one tile.
    pub fn at(&self, tile: &str) -> Vec<&str> {
        self.features
            .iter()
            .filter(|(t, _)| t == tile)
            .map(|(_, f)| f.as_str())
            .collect()
    }

    /// Global clocks this image **distributes** with nothing configured
    /// to drive them, one line each.
    ///
    /// # Why a decoding needs this on top of "every bit decodes"
    ///
    /// "Every bit decodes" is a statement about the bits that are
    /// *there*. This defect is a bit that is **absent**: a `GCLK<n>` can
    /// be enabled the whole length of the buffer column, made active in a
    /// clock row, muxed onto the horizontal network and taken by `BUFHCE`s
    /// to leaf tiles — and the `BUFGCTRL` at the head of it left with no
    /// `IN_USE`, no polarity bits and no input mux. A `BUFGCTRL` that is
    /// not in use does not drive, so that clock carries nothing, and every
    /// flip-flop on it never ticks. Nothing about the image is
    /// *unexplained*; what is wrong is what it does not say.
    ///
    /// It is the same shape as a constant on a pin nothing drives, in the
    /// clock network instead of the fabric, and it was found the same way:
    /// by a person with a board and two bitstreams. Two images of one
    /// design differing by twenty-six lines of logic read a smartcard's
    /// ATR cleanly and garbled it; decoded, the whole difference outside
    /// the placement churn was the clock, and the garbling one contained
    ///
    /// ```text
    /// CLK_BUFG_TOP_R_X60Y53   CLK_BUFG_CK_GCLK28.CLK_BUFG_BUFGCTRL12_O
    /// ```
    ///
    /// as the **only** feature in that tile: `GCLK28` driven from the
    /// output of a buffer with nothing configuring it and nothing routed
    /// to its input, then distributed over the die. The working image had
    /// `BUFGCTRL_X0Y12` fully configured — `IN_USE`,
    /// `IS_IGNORE1_INVERTED`, `ZINV_CE0`, `ZINV_S0` and
    /// `CLK_BUFG_BUFGCTRL12_I0.CLK_BUFG_BOT_R_CK_MUXED24` — driving
    /// `GCLK12`.
    ///
    /// # What it reads, and what it does not
    ///
    /// Everything here is the database's own feature spelling, and the
    /// numbering is **tile-local**: `CLK_BUFG_TOP_R`'s `BUFGCTRL12` is
    /// the die's `BUFGCTRL_X0Y28` and drives `GCLK28`, and the pip name
    /// pairs the two, so nothing here has to know which half of the
    /// column it is in.
    ///
    /// - a clock is **distributed** when a `GCLK<n>_ENABLE_*`, a
    ///   `..._CK_GCLK<n>_TOP`/`_BOT` rebuffer pip, a
    ///   `CLK_HROW_R_CK_GCLK<n>_ACTIVE` or a mux output selecting
    ///   `CLK_HROW_R_CK_GCLK<n>` mentions it;
    /// - it is **driven** when some `CLK_BUFG_*` tile has both
    ///   `CLK_BUFG_CK_GCLK<n>.CLK_BUFG_BUFGCTRL<m>_O` and that same
    ///   tile's `BUFGCTRL.BUFGCTRL_X0Y<m>.IN_USE`.
    ///
    /// An empty answer does **not** mean the clock network is right. This
    /// says nothing about a `BUFGCTRL` that is in use but whose input mux
    /// selects nothing, about a `BUFHCE` in use with no clock reaching it,
    /// or about a clock that is distributed correctly and simply goes to
    /// the wrong flip-flops. It catches one mistake, and it catches it
    /// from a file rather than from a story.
    #[must_use]
    pub fn clocks_without_a_driver(&self) -> Vec<String> {
        // Which `GCLK<n>` each tile drives from which tile-local buffer,
        // and which tile-local buffers that tile says are in use.
        let mut driven: BTreeSet<u32> = BTreeSet::new();
        let mut headless: BTreeMap<u32, String> = BTreeMap::new();
        let mut distributed: BTreeMap<u32, String> = BTreeMap::new();
        for (tile, feature) in &self.features {
            if let Some(rest) = feature.strip_prefix("CLK_BUFG_CK_GCLK") {
                let Some((gclk, buffer)) = rest.split_once(".CLK_BUFG_BUFGCTRL") else {
                    continue;
                };
                let Some(buffer) = buffer.strip_suffix("_O") else {
                    continue;
                };
                let (Ok(gclk), Ok(_)) = (gclk.parse::<u32>(), buffer.parse::<u32>()) else {
                    continue;
                };
                let in_use = format!("BUFGCTRL.BUFGCTRL_X0Y{buffer}.IN_USE");
                if self.features.iter().any(|(t, f)| t == tile && *f == in_use) {
                    driven.insert(gclk);
                } else {
                    headless.insert(
                        gclk,
                        format!(
                            "{tile} drives GCLK{gclk} from BUFGCTRL{buffer}'s output and that \
                             tile has no `{in_use}`"
                        ),
                    );
                }
                continue;
            }
            // Where a clock is distributed to, in the four spellings the
            // database uses for it.
            let mentions = feature
                .strip_prefix("GCLK")
                .and_then(|r| r.split_once('_'))
                .filter(|(_, tail)| tail.starts_with("ENABLE_"))
                .map(|(n, _)| n)
                .or_else(|| {
                    feature
                        .rsplit_once("_CK_GCLK")
                        .map(|(_, tail)| tail.split(['_', '.']).next().unwrap_or(tail))
                });
            if let Some(n) = mentions
                && let Ok(n) = n.parse::<u32>()
            {
                distributed.entry(n).or_insert_with(|| tile.clone());
            }
        }
        let mut out = Vec::new();
        for (gclk, tile) in &distributed {
            if driven.contains(gclk) {
                continue;
            }
            match headless.get(gclk) {
                Some(why) => out.push(why.clone()),
                None => out.push(format!(
                    "GCLK{gclk} is distributed (first seen at {tile}) and no `CLK_BUFG_*` tile \
                     drives it from a `BUFGCTRL` at all"
                )),
            }
        }
        // A buffer driving a clock nothing distributes is the harmless
        // way round and is not reported; one with no `IN_USE` is not.
        for (gclk, why) in &headless {
            if !distributed.contains_key(gclk) {
                out.push(why.clone());
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// A loaded fabric: the part, the architecture and the frame map, which
/// only mean anything together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XrayFabric {
    /// The part: its IDCODE and its frame layout.
    pub part: Part,
    /// The routing architecture, over the region that was asked for.
    pub arch: Arch,
    /// Where every tile's bits live in the frames, for the whole part.
    pub frames: FrameMap,
    /// The global clock column's rebuffer enables, which belong to a
    /// whole column of tiles rather than to any one pip; see
    /// [`XrayFabric::enable_global_clocks`].
    pub clocks: ClockColumn,
    /// The block RAM mode features and the interconnect's `VCC_WIRE`
    /// defaults, for [`XrayFabric::configure_block_rams`].
    pub block_ram: BlockRamTables,
    /// What the load covered and cost.
    pub stats: XrayStats,
}

/// The `CLK_BUFG_REBUF` buffer enables, by global clock track.
///
/// Every other bit this loader knows about hangs off a pip or off a bel,
/// and a router that takes the pip or a placer that fills the bel turns
/// it on. These do not: a global clock track is cut at every rebuffer of
/// its column, each cut has a buffer enable at each side, and a route
/// crosses exactly one of the rebuffers. So the bits are kept here,
/// beside the architecture, and switched on by
/// [`XrayFabric::enable_global_clocks`] once the routing is known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockColumn {
    /// The tile type that holds the rebuffers, empty when the database
    /// has none.
    pub tile_type: String,
    /// Track number to the bits of that track's two enables, which are
    /// switched on together.
    pub enables: BTreeMap<u32, Vec<ConfigBit>>,
}

impl XrayFabric {
    /// Checks that this database really is the part the flow asked for.
    ///
    /// # Errors
    ///
    /// [`super::xc7::Xc7Error::IdcodeMismatch`] when the IDCODEs differ,
    /// which is the check that stops a bitstream built from one die's
    /// database being labelled with another's.
    pub fn check_idcode(&self, wanted: u32) -> Result<(), super::xc7::Xc7Error> {
        if self.part.idcode != wanted {
            return Err(super::xc7::Xc7Error::IdcodeMismatch {
                wanted,
                found: self.part.idcode,
            });
        }
        Ok(())
    }

    /// Switches on the rebuffer enables of every global clock track the
    /// routing uses, over the whole height of its column, and says how
    /// many bits that was.
    ///
    /// # Why this is not a pip's business
    ///
    /// A 7-series global clock leaves its `BUFGCTRL` onto one of 32
    /// vertical tracks, and the track is **cut** at every
    /// `CLK_BUFG_REBUF` tile of the column. Each cut has a buffer enable
    /// on each side (`sites::global_clock_enable` derives which, and says
    /// how that was measured), and a
    /// route crosses exactly one rebuffer — the one between the buffer
    /// and the clock row it is headed for. Every other rebuffer that
    /// sees the same track is a tile the route never enters, so no pip of
    /// it is taken and nothing charges its bits.
    ///
    /// # What this sets, and whose rule it is
    ///
    /// Both enables of the track, in **every** rebuffer of the column.
    /// That is nextpnr-xilinx's rule — `write_clocking` emits
    /// `GCLK<n>_ENABLE_ABOVE` and `GCLK<n>_ENABLE_BELOW` in every
    /// `CLK_BUFG_REBUF` tile for every track the design uses — and it is
    /// deliberately broader than Vivado's. Vivado marks the two ends of
    /// each *live* segment and no more: for the four designs in
    /// `artix7/harness/`, which all drive a clock from the middle of the
    /// column up to the row at `Y130`, that is eleven features and the
    /// rebuffers below the buffer keep one enable each.
    ///
    /// The broader rule was chosen because the two are not symmetric in
    /// their failure: an enable set on a track segment nothing uses is a
    /// buffer driving a track that is already driven by the same clock,
    /// which is what nextpnr does on this very board, while an enable
    /// *missing* under a clock is a clock that arrives nowhere and a part
    /// that configures, reports `DONE` and does nothing. Neither what the
    /// two words mean nor whether the far end is needed is established
    /// here; what is established is that two independent tools set it.
    ///
    /// # Errors
    ///
    /// [`super::bitstream::BitstreamError`] when a rebuffer's bits fall
    /// outside the tile's bitmap, which would mean the `segbits` file and
    /// `tilegrid.json` disagree about the tile type's shape.
    pub fn enable_global_clocks(
        &self,
        graph: &super::arch::RoutingGraph,
        routing: &super::Routing,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<usize, super::bitstream::BitstreamError> {
        if self.clocks.enables.is_empty() {
            return Ok(0);
        }
        let is_rebuffer = |x: u32, y: u32| -> bool {
            self.arch
                .tile_index_at(x, y)
                .is_some_and(|index| self.arch.tile_types[index].name == self.clocks.tile_type)
        };

        // The columns and tracks a route crossed a rebuffer of.
        let mut used: BTreeSet<(u32, u32)> = BTreeSet::new();
        for route in routing.routes() {
            for id in &route.pips {
                let pip = graph.pip(*id);
                if !is_rebuffer(pip.tile.0, pip.tile.1) {
                    continue;
                }
                for node in [pip.from, pip.to] {
                    if let Some((_, track)) = sites::global_clock_track(&graph.wire(node).name) {
                        used.insert((pip.tile.0, track));
                    }
                }
            }
        }

        let mut count = 0usize;
        for (x, track) in used {
            let Some(enables) = self.clocks.enables.get(&track) else {
                continue;
            };
            for y in 0..self.arch.height {
                if !is_rebuffer(x, y) {
                    continue;
                }
                for bit in enables {
                    bits.set((x, y), *bit)?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// A [`super::route::RouteOptions::node_base`] vector that makes the
    /// global clock network cheap, so a clock goes on it — the 7-series
    /// counterpart of `TrellisFabric::clock_node_costs`, for the same
    /// reason.
    ///
    /// # What happened without it
    ///
    /// An interconnect tile's `CLK0`/`CLK1` take twelve `GCLK_B` wires
    /// **and** `FAN_BOUNCE`, `ER1END`, `WR1END` and `SR1END`
    /// (`segbits_int_l.db`). A router with no preference reaches a clock pin
    /// whichever way is cheapest, and once part of the clock's tree is
    /// built, a neighbouring tile's ordinary wires are cheaper than going
    /// back to the network. On `examples/basys3/ssd1306_console.v` the
    /// flip-flops of the status shift register were clocked through
    /// **80 to 103** `FAN_BOUNCE`/`BYP_BOUNCE` hops, daisy-chained from
    /// tile to tile, while a flip-flop one row away was on `GCLK_B0`. The
    /// skew between those two kinds of path was larger than a flip-flop's
    /// clock-to-out, so `shifter[127]` captured `shifter[123]`'s **new**
    /// value — a hold violation — and every character of the status line
    /// read some of its bits from the next nibble. Which bits depended on
    /// the placement. It routed, verified and decoded cleanly: only the
    /// part showed it.
    ///
    /// # Why it is not sound on its own here
    ///
    /// On an ECP5 the network is a one-way funnel and a cheap network is
    /// all it takes. **This one is not.** The clock row takes interconnect
    /// inputs (`CLK_HROW_WW2END2`, a buffer tile's `CLK_BUFG_IMUX`) and a
    /// break tile joins general routing onto the vertical tracks
    /// (`BRKH_CLK_R_CK_GCLK26 -> CLK_BUFG_CK_GCLK26`), and `GCLK_B` feeds
    /// `GFAN` and so every `IMUX`. With only this, a flip-flop's output
    /// rode 1322 pips of clock wire and took the two ground fans a block
    /// RAM's write enable needed. So it goes with
    /// [`XrayFabric::clock_network_nodes`] as a barrier: only a signal a
    /// global buffer drives may enter the network at all.
    #[must_use]
    pub fn clock_node_costs(&self, graph: &super::arch::RoutingGraph, preference: f32) -> Vec<f32> {
        let mut out = vec![1.0f32; graph.nodes.len()];
        for (index, wire) in graph.nodes.iter().enumerate() {
            if is_clock_wire(&wire.name) {
                out[index] = preference;
            }
        }
        out
    }

    /// The global clock network's nodes, for
    /// [`super::route::RouteOptions::network`]: what a signal no global
    /// buffer drives may not enter. See [`XrayFabric::clock_node_costs`].
    #[must_use]
    pub fn clock_network_nodes(&self, graph: &super::arch::RoutingGraph) -> Vec<bool> {
        graph
            .nodes
            .iter()
            .map(|wire| is_clock_wire(&wire.name))
            .collect()
    }

    /// Whether each placed clock pin's clock arrived from the global
    /// network, for every clock a global buffer drives.
    ///
    /// [`XrayFabric::clock_node_costs`] is a preference and a preference
    /// can be lost, and a clock off the network fails as skew rather than
    /// as a broken bitstream — nothing else would notice — so
    /// `reticle fpga --bitstream` refuses on what this finds.
    ///
    /// A pin is on the network when the **first programmable pip** on its
    /// own path back through the route comes from a clock-network wire:
    /// `CLK1 <- GCLK_B0_EAST`, not `CLK1 <- FAN_BOUNCE5`. It is asked of
    /// each pin's own path, because one slice of a tile can be on the
    /// network while its neighbour is not. A clock no global buffer drives
    /// is counted in [`XrayClockUse::unbuffered`] and not judged: there is
    /// no network for it to have been on.
    #[must_use]
    pub fn clock_network_use(
        &self,
        netlist: &super::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        routing: &super::Routing,
    ) -> XrayClockUse {
        let mut out = XrayClockUse::default();
        for (index, instance) in netlist.instances.iter().enumerate() {
            let roles: &[&str] = match instance.kind.as_str() {
                "ff" => &["clk"],
                "lutram" => &["wclk"],
                "bram" => &["p0_clk", "p1_clk"],
                _ => continue,
            };
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            for role in roles {
                let Some(signal) = instance
                    .pins
                    .iter()
                    .map(|pin| &netlist.pins[*pin])
                    .find(|pin| pin.role == *role)
                    .and_then(|pin| pin.signal)
                else {
                    continue;
                };
                let (Some(pin), Some(route)) = (site.pin(role), routing.route(signal)) else {
                    continue;
                };
                if !is_clock_wire(&graph.wire(route.source).name) {
                    out.unbuffered += 1;
                    continue;
                }
                let mut node = pin;
                let mut on_network = false;
                for _ in 0..32 {
                    let Some(id) = route.pips.iter().find(|id| graph.pip(**id).to == node) else {
                        break;
                    };
                    let pip = graph.pip(*id);
                    if !graph.pip_bits(*id).is_empty() {
                        on_network = is_clock_wire(&graph.wire(pip.from).name);
                        break;
                    }
                    node = pip.from;
                }
                if on_network {
                    out.on_network += 1;
                } else {
                    out.off_network
                        .push(format!("{} ({role}) on {}", instance.name, site.name));
                }
            }
        }
        out
    }

    /// Switches on the weak pull-up of every IO buffer whose cell asks for
    /// one — `set_io -pullup yes`, which the IO pass records as the
    /// cell's `pullup` attribute — and returns how many pads it pulled.
    ///
    /// # Why this is a pass and not a bel entry
    ///
    /// A bel's [`ConfigEntry`](super::arch::ConfigEntry) list is selected by primitive name and by
    /// the cell's *parameters*, and a pull is neither: on the 7 series it
    /// is a constraint (`set_property PULLUP TRUE` in an XDC), not a
    /// parameter of `IBUF` or `IOBUF`, and putting one on the cell would
    /// hand Vivado an instance parameter its library does not have. So
    /// the request stays an attribute and this reads it, once placement
    /// says which pad each buffer is on.
    ///
    /// # The bits
    ///
    /// `segbits_liob33.db` writes `PULLTYPE` as a three-bit field, for
    /// `IOB_Y0`: `NONE` is `!38_92 38_94 !39_93`, `PULLUP` is
    /// `!38_92 38_94 39_93`, `KEEPER` is `38_92 38_94 !39_93`, and
    /// `PULLDOWN` is **all three clear**. Every buffer recipe already sets
    /// `PULLTYPE.NONE`, so adding `PULLTYPE.PULLUP`'s ones gives exactly
    /// the `PULLUP` pattern, and `NONE` no longer decodes because its
    /// `!39_93` fails. The bits are the ones the loader already attached
    /// to the bel as the inert entry `ConfigEntry::Cell { primitive:
    /// "PULLTYPE.PULLUP", .. }` (see `parse::bels_of`), so nothing here
    /// names a bit position.
    ///
    /// What this has been checked against: three of the four Vivado
    /// harness designs set `PULLTYPE.PULLUP` on some pad and Vivado's
    /// `design.json` names it; that is the feature's name and bits, not
    /// its effect. **No pull-up from this flow has been seen on a part.**
    ///
    /// # Errors
    ///
    /// Those of [`super::bitstream::Bitstream::set`].
    pub fn apply_pullups(
        &self,
        design: &crate::ir::Design,
        top: crate::ir::ModuleId,
        graph: &super::arch::RoutingGraph,
        netlist: &super::Netlist,
        placement: &super::place::Placement,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<usize, super::bitstream::BitstreamError> {
        let Some(module) = design.modules.get(top) else {
            return Ok(0);
        };
        let mut pulled = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "io" {
                continue;
            }
            let wants = module
                .cells
                .get(instance.cell)
                .and_then(|cell| cell.attrs.get("pullup"))
                .and_then(crate::ir::AttrValue::as_int)
                == Some(1);
            if !wants {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(bel) = self.arch.tile_types[site.tile_type].bel(&site.bel) else {
                continue;
            };
            let pullup = bel.config.iter().find_map(|entry| match entry {
                super::arch::ConfigEntry::Cell { primitive, bits }
                    if primitive == PULLUP_FEATURE =>
                {
                    Some(bits)
                }
                _ => None,
            });
            let Some(pullup) = pullup else {
                continue;
            };
            for bit in pullup {
                bits.set(site.tile, *bit)?;
            }
            pulled += 1;
        }
        Ok(pulled)
    }
}

/// The tail of the `segbits` feature that turns an IO buffer's weak
/// pull-up on; see [`XrayFabric::apply_pullups`].
const PULLUP_FEATURE: &str = "PULLTYPE.PULLUP";

/// The global clock rebuffer enables a family's `segbits` files hold.
///
/// One tile type has them and `artix7` has one such type
/// (`CLK_BUFG_REBUF`), so the first in name order wins and a family with
/// none gives an empty [`ClockColumn`], which makes
/// [`XrayFabric::enable_global_clocks`] a no-op.
fn clock_column(features: &HashMap<String, FeatureSet>) -> ClockColumn {
    let mut names: Vec<&String> = features.keys().collect();
    names.sort();
    for name in names {
        let mut enables: BTreeMap<u32, Vec<ConfigBit>> = BTreeMap::new();
        for feature in features[name].features() {
            if let Some(track) = sites::global_clock_enable_track(&feature.name) {
                enables
                    .entry(track)
                    .or_default()
                    .extend(feature.ones.iter().copied());
            }
        }
        if !enables.is_empty() {
            return ClockColumn {
                tile_type: name.clone(),
                enables,
            };
        }
    }
    ClockColumn::default()
}

/// Splits `xc7a35t-cpg236` into its die and its package.
fn split_device(device: &str) -> Option<(String, String)> {
    let (die, package) = device.split_once('-')?;
    if die.is_empty() || package.is_empty() {
        return None;
    }
    Some((die.to_owned(), package.to_owned()))
}

/// How the clock pins of a placed 7-series design were reached; see
/// [`XrayFabric::clock_network_use`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XrayClockUse {
    /// Clock pins whose clock came off the global network.
    pub on_network: usize,
    /// Clock pins a global buffer's clock reached through general
    /// interconnect instead, as `<instance> (<role>) on <site>`.
    pub off_network: Vec<String>,
    /// Clock pins whose clock no global buffer drives.
    pub unbuffered: usize,
}

/// The default [`XrayFabric::clock_node_costs`] preference: the same
/// twentieth of an ordinary wire the ECP5 flow uses.
pub const CLOCK_PREFERENCE: f32 = 0.05;

/// Whether a wire belongs to the global clock network.
///
/// **Not a tile prefix.** The clock tiles are crossed by ordinary
/// routing: `HCLK_NN6A`, `HCLK_LV`, `CLK_FEED_EE2A`, `CLK_HROW_WW4END0` and
/// a carry chain's `HCLK_CLB_COUT0_L` are general interconnect that happens
/// to pass through an `HCLK` or clock-row tile, and a rule of "starts with
/// `HCLK_`" barred every one of them, so that no signal of
/// `examples/basys3/ssd1306_console.v` could route at all. What marks a
/// clock wire is in the rest of the name:
///
/// - `GCLK`: the 32 vertical tracks (`CLK_BUFG_CK_GCLK`,
///   `CLK_FEED_R_CK_GCLK`, `CLK_BUFG_REBUF_R_CK_GCLK<n>_TOP`), the clock
///   row's view of them (`CLK_HROW_R_CK_GCLK`) and the twelve an
///   interconnect tile takes (`GCLK_B<n>`, `GCLK_L_B<n>`);
/// - `BUFHCLK`: the horizontal buffers' outputs and every feedthrough and
///   column tile that carries them (`HCLK_CK_BUFHCLK`,
///   `HCLK_CLB_CK_BUFHCLK`, `HCLK_FEEDTHRU_1_CK_BUFHCLK`, ...);
/// - `HCLK_LEAF_CLK`: the leaf drivers onto `GCLK_B`;
/// - `CK_MUX_OUT` and `CK_HCLK_OUT`: the clock row's own path;
/// - a `BUFGCTRL`'s output, `CLK_BUFG_BUFGCTRL<n>_O`.
///
/// Three things that match and are left out: the clock row's `*TEST*`
/// wires; a block RAM's `REGCLK<x>` pins, which contain `GCLK` by accident;
/// and `I2GCLK`, the dedicated path from a clock-capable pad to a buffer,
/// whose signal an input buffer drives and not a global buffer — barring it
/// would keep a pad's clock off the only road to its `BUFG`. These are the
/// wires the clock of that design took on the network, read off its route.
fn is_clock_wire(name: &str) -> bool {
    if ["TEST", "REGCLK", "I2GCLK"]
        .iter()
        .any(|part| name.contains(part))
    {
        return false;
    }
    [
        "GCLK",
        "BUFHCLK",
        "HCLK_LEAF_CLK",
        "CK_MUX_OUT",
        "CK_HCLK_OUT",
    ]
    .iter()
    .any(|part| name.contains(part))
        || (name.starts_with("CLK_BUFG_BUFGCTRL") && name.ends_with("_O"))
}

#[cfg(test)]
mod tests;
