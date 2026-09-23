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
//! # NOTHING PRODUCED FROM THIS HAS BEEN LOADED INTO A PART
//!
//! Real bit positions are not the same thing as a working bitstream. What
//! is established is structural: the frame layout, the frame count, the
//! packet stream and both CRCs agree with a bitstream Vivado made for an
//! XC7A35T (see [`super::xc7`]). What is *not* established is that a
//! design taken through this loader configures anything, and the list of
//! reasons it would not yet is in `docs/fpga-xray.md` under "what remains".
//! Nothing in this crate has ever been sent down a JTAG cable.
//!
//! # The files, and what each one gives
//!
//! | File | What is taken from it |
//! |---|---|
//! | `<family>/mapping/devices.yaml` | which fabric a die uses — `xc7a35t` is the same die as `xc7a50t` |
//! | `<family>/<part>/part.json` | the IDCODE and the frame layout, column by column |
//! | `<family>/<part>/package_pins.csv` | package pin to site, for [`Arch::pinmap`] |
//! | `<family>/<fabric>/tilegrid.json` | every tile: name, type, grid position, sites, and where its bits live in the frames |
//! | `<family>/<fabric>/tileconn.json` | which wire of a tile is the same metal as which wire of its neighbour |
//! | `<family>/segbits_<type>.db` | which bits switch on which pip and which bel feature |
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
//! # And one gap in the data
//!
//! `prjxray-db` ships the *bits*, not the tile-type wire and pip lists
//! (prjxray's own `tile_type_*.json` is generated from Vivado and is not
//! in the repository). Two things follow:
//!
//! - **Which wire a bel pin reaches is not in the database.** The loader
//!   therefore declares bels with no pins. A placer can put a cell on a
//!   real `SLICEL`; a router cannot yet get a signal to it. This is the
//!   single biggest thing standing between here and a lit LED.
//! - **Whether a three-part feature `TYPE.A.B` is a pip or a bel
//!   feature has to be inferred.** The rule is in [`is_pip_feature`]: it
//!   is a pip unless `A` also heads a longer feature of the same tile
//!   type, which is what a site prefix does (`CLBLL_L.SLICEL_X0.AFF.ZINI`
//!   makes `SLICEL_X0` a site, so `CLBLL_L.SLICEL_X0.CLKINV` is a bel
//!   feature and not a pip). It classifies every `INT_L` feature as a pip
//!   and every `CLBLL_L` one as a bel feature, which is right; the three
//!   `LIOB33.DIFF.*` features are the known place it is wrong.
//!
//! # Scale
//!
//! The `xc7a50t` fabric is 18 055 tiles and, by the database's own
//! `element_counts.csv`, 7 857 396 nodes. Its `segbits` add up to about
//! 23.5 million features over the die, of which 20.5 million are pips in
//! the 5650 interconnect tiles. Expanding all of that into a
//! [`RoutingGraph`](super::arch::RoutingGraph) is not something this
//! crate's representation can hold in a sane amount of memory, so
//! [`XrayOptions::region`] says which rectangle of tiles gets wires, pips
//! and bels, and [`XrayOptions::max_pips`] stops with the numbers rather
//! than with an allocation failure. The frame map always covers the whole
//! part, so the bitstream is a whole-part bitstream whatever the region.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;

use super::arch::{Arch, ConfigBit, PipDecl, TileType, WireDecl, WireRef};
use super::xc7::{FrameMap, Part, TileBits};
use crate::ir::memfile::FileProvider;
use crate::json::Json;

mod parse;

pub use parse::{fabric_of, family_directory, is_pip_feature};

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
    /// The largest number of pips the loader will build before refusing
    /// with [`XrayError::TooLarge`]. It counts first and allocates after,
    /// so the refusal costs nothing.
    pub max_pips: usize,
}

impl Default for XrayOptions {
    fn default() -> Self {
        XrayOptions {
            part: None,
            region: None,
            // Four million pips is about a gigabyte once each one owns a
            // list of configuration bits, which is as far as a desktop
            // machine goes without swapping. The whole `xc7a50t` is five
            // times that; see the module docs.
            max_pips: 4_000_000,
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
    /// Bels declared in the loaded region.
    pub bels: usize,
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
    /// bus-name order.
    pub bits: Vec<(String, TileBits)>,
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
            if let (Some(head), Some(_), Some(_)) = (parts.next(), parts.next(), parts.next()) {
                sites.insert(head.to_owned());
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
        // region uses it: the die-wide count is the measurement.
        let mut features: HashMap<String, FeatureSet> = HashMap::new();
        for name in &types {
            let path = format!(
                "{}/{}/segbits_{}.db",
                self.root,
                self.family,
                name.to_lowercase()
            );
            let Some(text) = files.read_file(&path) else {
                continue;
            };
            features.insert((*name).to_owned(), parse::segbits(&text, &path)?);
        }
        for tile in &tiles {
            if let Some(set) = features.get(&tile.tile_type) {
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

        // Count before allocating, so a region that will not fit is
        // refused with numbers instead of a dead machine.
        let mut wanted = 0usize;
        let mut in_region = 0usize;
        for tile in &tiles {
            if !region.contains(tile.grid_x, tile.grid_y) {
                continue;
            }
            in_region += 1;
            if let Some(set) = features.get(&tile.tile_type) {
                wanted += set.pip_count();
            }
        }
        if wanted > options.max_pips {
            return Err(XrayError::TooLarge {
                pips: wanted,
                limit: options.max_pips,
                tiles: in_region,
            });
        }

        let conn = self.tileconn(files)?;
        let mut arch = self.build_arch(&tiles, &features, &conn, region, &part, &mut stats);
        arch.pinmap = self.pinmap(files, &tiles, &features, region)?;
        stats.tiles_loaded = in_region;

        Ok(XrayFabric {
            part,
            arch,
            frames,
            stats,
        })
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
    fn pinmap(
        &self,
        files: &dyn FileProvider,
        tiles: &[XrayTile],
        features: &HashMap<String, FeatureSet>,
        region: GridRegion,
    ) -> Result<Vec<(String, String)>, XrayError> {
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
        for (pin, site) in parse::package_pins(&text) {
            let Some(tile) = where_is.get(site.as_str()) else {
                continue;
            };
            let set = features.get(&tile.tile_type).unwrap_or(&empty);
            let Some(bel) = parse::bel_of_site(tile, &site, set) else {
                continue;
            };
            out.push((pin, format!("X{}Y{}/{bel}", tile.grid_x, tile.grid_y)));
        }
        Ok(out)
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
    fn build_arch(
        &self,
        tiles: &[XrayTile],
        features: &HashMap<String, FeatureSet>,
        conn: &[parse::TileConn],
        region: GridRegion,
        part: &Part,
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
        let mut used: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
        for tile in tiles {
            if !region.contains(tile.grid_x, tile.grid_y) {
                continue;
            }
            let shape = tile
                .bits
                .first()
                .map_or((0, 0), |(_, b)| (b.frames, b.words * 32));
            used.entry(&tile.tile_type).or_insert(shape);
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
        let mut index_of: HashMap<&str, usize> = HashMap::new();
        for (name, (rows, cols)) in &used {
            let mut tile_type = TileType::new(*name, *name, *rows, *cols);
            let set = features.get(*name).unwrap_or(&empty);

            let mut wires: BTreeSet<&str> = BTreeSet::new();
            for (to, from, _) in set.pips() {
                wires.insert(to);
                wires.insert(from);
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

            for (to, from, feature) in set.pips() {
                tile_type.pips.push(PipDecl {
                    from: WireRef::local(from),
                    to: WireRef::local(to),
                    bits: feature.ones.clone(),
                });
            }
            for (other, dx, dy, pairs) in joins.get(*name).into_iter().flatten() {
                if !used.contains_key(other) {
                    continue;
                }
                for (mine, theirs) in pairs.iter() {
                    tile_type.pips.push(PipDecl {
                        from: WireRef::local(mine.clone()),
                        to: WireRef::at(theirs.clone(), *dx, *dy),
                        bits: Vec::new(),
                    });
                    tile_type.pips.push(PipDecl {
                        from: WireRef::at(theirs.clone(), *dx, *dy),
                        to: WireRef::local(mine.clone()),
                        bits: Vec::new(),
                    });
                }
            }

            index_of.insert(*name, arch.tile_types.len());
            arch.tile_types.push(tile_type);
        }

        // Bels are declared on the tile type, but `tilegrid.json` names
        // sites per tile, so the first tile of each type that has any
        // supplies the site types the type's bels are built from.
        let mut seeded: HashSet<&str> = HashSet::new();
        for tile in tiles {
            if !region.contains(tile.grid_x, tile.grid_y) || tile.sites.is_empty() {
                continue;
            }
            if !seeded.insert(&tile.tile_type) {
                continue;
            }
            let Some(index) = index_of.get(tile.tile_type.as_str()) else {
                continue;
            };
            let set = features.get(&tile.tile_type).unwrap_or(&empty);
            arch.tile_types[*index].bels = parse::bels_of(tile, set);
        }

        for tile in tiles {
            if !region.contains(tile.grid_x, tile.grid_y) {
                continue;
            }
            if let Some(index) = index_of.get(tile.tile_type.as_str()) {
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

/// One `tileconn` entry seen from the tile type that owns it: the type
/// at the other end, the grid delta to it, and the wire pairs.
type Join<'a> = (&'a str, i32, i32, &'a [(String, String)]);

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
    /// What the load covered and cost.
    pub stats: XrayStats,
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
}

/// Splits `xc7a35t-cpg236` into its die and its package.
fn split_device(device: &str) -> Option<(String, String)> {
    let (die, package) = device.split_once('-')?;
    if die.is_empty() || package.is_empty() {
        return None;
    }
    Some((die.to_owned(), package.to_owned()))
}

#[cfg(test)]
mod tests;
