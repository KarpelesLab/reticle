//! Project Trellis' Lattice ECP5 database, turned into an [`Arch`].
//!
//! # What this builds
//!
//! The part's geometry, its interconnect, its lookup tables and its pads:
//!
//! - the tile grid of an ECP5 and every position's rectangle of
//!   configuration memory;
//! - **the routing graph**: 467 global wires, 1 095 958 tile wires and
//!   8 270 828 graph edges for the whole LFE5U-12F die — 8 211 900 from
//!   `bits.db`'s `.mux` and `.fixed_conn` records and 58 928 the clock
//!   network needs and the database does not state. It costs about 350 MiB
//!   and under a second to build, which is why there is no region option
//!   here where [`super::xray`] needs one;
//! - the eight lookup tables and the eight flip-flops of every logic tile,
//!   with the truth table and input ties
//!   [`TrellisFabric::configure_logic`] writes and the settings
//!   [`TrellisFabric::configure_registers`] does;
//! - **the global clock network**: sixteen networks, four quadrants, four
//!   tap columns and eight spines from `globals.json`, the 56 `DCC` buffers
//!   as `gb` bels, and the three joins between them that `bits.db` states
//!   nowhere — see [`ClockNetwork`], which is the part of this backend that
//!   needed a file of its own;
//! - and an `io` bel for every PIO of the **top** and **right** edges, with
//!   the bits that make one an input or an output, the settings an input
//!   needs, and the bits that tie an output's data to a constant.
//!
//! A design that routes and a design with a clock in it, both built by
//! this, have been loaded into a real part; `docs/fpga-trellis.md` says
//! what that settled and what it did not.
//!
//! # The five structural surprises
//!
//! **A grid position owns several rectangles of configuration memory.**
//! Project Trellis splits one position of the fabric into up to six tiles
//! with separate bit regions that share one wire namespace — on this part
//! 480 of the 3723 positions of this part's grid have more than one, and
//! the most any has is six.
//! [`super::ecp5::Ecp5FrameMap`] is what holds them, a [`ConfigBit`]'s
//! `row` addresses a position's windows end to end, and the order is the
//! order Project Trellis' own tile names sort in, so it is the same on
//! every run. Nothing about it reaches [`Arch`].
//!
//! **A wire belongs to the position a prefix points at, and that position
//! has to declare it.** `S1E1_JA0` in a `PIOT0`'s `bits.db` is the `JA0` of
//! the tile one row south and one column east; see
//! [`parse::globalise_ref`]. So the loader cannot decide a tile type's wire
//! list by reading that type alone: it walks every reference on the die
//! first and gives each name to the *composition* it lands on. Doing it the
//! other way round leaves a few thousand references resolving to nothing,
//! and a reference that resolves to nothing is a pip that quietly vanishes.
//!
//! This is also why there are no join pips here where [`super::xray`]
//! spends a third of its edges on them: the mapping from a neighbour's
//! spelling to the wire it means is arithmetic, so a wire is one node and
//! every reference reaches it.
//!
//! **A `.mux` source with no bits is still a connection**, and believing
//! otherwise disconnects every lookup table in the part. See
//! [`parse::TileDatabase::arcs`], which is where that was got wrong.
//!
//! **A pad's bits are not at its buffer.** For a PIO on the top edge:
//!
//! | What | Tile | Position |
//! |---|---|---|
//! | the three wires the buffer presents to the fabric | `PIOT0` | the ball's own |
//! | the pad: standard, hysteresis, pull | `PIOT0` (side A) or `PIOT1` (side B) | row 0, side B one column east |
//! | a second copy of the standard | `PICT0` / `PICT1` | row 1, same column |
//! | the constants that tie the data and enable wires | `CIB` | row 1, one column east |
//! | the bank's VCCIO rail | `BANKREF<bank>` | wherever the bank's is |
//!
//! and on the right edge the answers are all different: the pad tile is one
//! row *south*, the second copy is at the ball's row for sides A and B and
//! two rows south for C and D, and the `CIB` is one column west. The
//! **left** edge takes the right edge's row arithmetic unchanged with the
//! column and the `CIB`'s direction flipped, which is a measurement and not
//! a symmetry; the **bottom** edge takes the top edge's column rule with
//! the two tiles collapsed into one, so there is no second copy to write.
//! That is nextpnr's own rule (`get_pio_tile`, `get_pic_tile`), and all
//! four edges were checked against bitstreams Lattice's own packer wrote
//! for this board rather than taken on trust — see `docs/fpga-trellis.md`.
//!
//! Almost none of those tiles is the tile a bel could carry
//! [`ConfigEntry`](super::arch::ConfigEntry)s in. So an IO's bits are not a
//! bel's here; they are [`TrellisFabric::configure_io`]'s, computed from
//! the placement, and a lookup table's are
//! [`TrellisFabric::configure_logic`]'s for a different reason its own
//! documentation gives. That is the same division `super::apicula`'s
//! `Periphery` makes, and it is why `src/fpga/bitstream.rs` needs no edit
//! for this vendor either.
//!
//! **The bel goes where the buffer is, not where the bits are.** Before
//! anything routed the distinction did not exist and the bel was put at the
//! bits; a routed design notices at once, because a bel's pins resolve in
//! the tile the bel sits in and `PADDOB_PIO` exists at every top-edge
//! position.
//!
//! **A bidirectional pad is a base type and a routed wire, and nothing
//! else.** `PIO<s>.BASE_TYPE = BIDIR_<standard>` is its own pattern — neither
//! the input's nor the output's nor their union — and the `CIB` mux that ties
//! the tristate wire low for every ordinary output is left *alone*, because
//! the router drove that wire and the tie and the route are one mux. The
//! field that could have governed the tristate, `PIO<s>.TRIMUX_TSREG`,
//! defaults to `PADDT`, which is the fabric-driven wire, and so costs no
//! bits; `docs/fpga-trellis.md` records that being checked against eight
//! bidirectional pads in Great Scott Gadgets' own `analyzer.bit` rather than
//! assumed. The pull mode is what decides what a *released* pad reads, and
//! the database's default for it is a pull-down: see [`PULL_NONE`].
//!
//! **A clock's wires carry the same name in every tile they cross.** That
//! is the one place the arithmetic above stops working: `G_HPBX0000` is a
//! logic tile's branch wire and so is its neighbour's, and the tap driver
//! twenty columns away spells its output `R_HPBX0000` with nothing to say
//! the three are one piece of metal. `globals.json` is the only statement
//! of where they go, and [`ClockNetwork`] is what is built out of it.
//!
//! # Obtaining the database
//!
//! Nothing here reads a file: a [`crate::ir::memfile::FileProvider`] is handed in,
//! rooted at the directory holding `devices.json`. `reticle fetch
//! prjtrellis-db` puts one in the per-user cache; `docs/fpga-trellis.md`
//! has the command and what is downloaded.
pub mod parse;
pub mod sites;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::ir::memfile::FileProvider;

use super::arch::{Arch, BelDecl, ConfigBit, TileType};
use super::ecp5::{Cram, Ecp5Error, Ecp5FrameMap, Ecp5Stream, FrameFormat};

use parse::{DeviceInfo, Pinout, TileDatabase, TileEntry};

/// Why a Project Trellis database could not be read, or could not be
/// turned into something a design can be compiled against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrellisError {
    /// A file the database must have is not there.
    Missing {
        /// Its path, relative to the database root.
        path: String,
    },
    /// A file is there and is not the shape it has to be.
    Malformed {
        /// Its path, relative to the database root.
        path: String,
        /// What was wrong with it.
        what: String,
    },
    /// `devices.json` describes no part of that name.
    NoSuchDevice {
        /// The part that was asked for.
        wanted: String,
        /// The ECP5 parts the file does describe.
        known: Vec<String>,
    },
    /// The part's `iodb.json` describes no package of that name.
    NoSuchPackage {
        /// The package that was asked for.
        wanted: String,
        /// The packages the file does describe.
        known: Vec<String>,
    },
    /// A tile type's `bits.db` has no field a pad's configuration needs.
    ///
    /// This is a database that is present and does not say what it has to
    /// say, which is different from one that is missing: it means the
    /// version pinned here is not the version this code was written
    /// against.
    NoSuchField {
        /// The tile type whose `bits.db` was read.
        tile: String,
        /// The field that is not in it.
        field: String,
        /// The value that was wanted, for an enumerated field.
        value: String,
    },
    /// An IO standard no `bits.db` of this part offers.
    NoSuchIoStandard {
        /// What was asked for.
        wanted: String,
        /// What a `PIO<side>.BASE_TYPE` field does offer.
        known: Vec<String>,
    },
    /// Something a placed cell asks for that this backend will not write,
    /// with the reason it will not.
    ///
    /// The point of the variant is that it is never a silent
    /// approximation: a setting whose bits cannot be placed with certainty
    /// is refused, because writing it into the wrong one of a tile's shared
    /// muxes would change every other cell of that tile.
    Unsupported {
        /// What was asked for and why it is refused.
        what: String,
    },
    /// A bit did not fit the position it was located in, which means the
    /// frame map and the [`Arch`] disagree about a tile's shape.
    Bits(String),
}

impl fmt::Display for TrellisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrellisError::Missing { path } => write!(
                f,
                "the Project Trellis database has no `{path}`; `reticle fetch prjtrellis-db` \
                 downloads a complete one"
            ),
            TrellisError::Malformed { path, what } => {
                write!(f, "`{path}` is not the shape it has to be: {what}")
            }
            TrellisError::NoSuchDevice { wanted, known } => write!(
                f,
                "`devices.json` describes no ECP5 called `{wanted}`; it has {}",
                known.join(", ")
            ),
            TrellisError::NoSuchPackage { wanted, known } => write!(
                f,
                "this part's `iodb.json` describes no package called `{wanted}`; it has {}",
                known.join(", ")
            ),
            TrellisError::NoSuchField { tile, field, value } => write!(
                f,
                "`{tile}`'s bits.db has no `{field}` = `{value}`, so this copy of the database \
                 is not the one this code was written against"
            ),
            TrellisError::NoSuchIoStandard { wanted, known } => write!(
                f,
                "no IO standard called `{wanted}`; this part offers {}",
                known.join(", ")
            ),
            TrellisError::Unsupported { what } => f.write_str(what),
            TrellisError::Bits(what) => f.write_str(what),
        }
    }
}

impl Error for TrellisError {}

impl From<super::bitstream::BitstreamError> for TrellisError {
    fn from(err: super::bitstream::BitstreamError) -> TrellisError {
        TrellisError::Bits(err.to_string())
    }
}

/// What to load, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrellisOptions {
    /// The package whose ball map becomes [`Arch::pinmap`].
    ///
    /// `iodb.json`'s spelling, which is upper case and has no hyphen:
    /// `CABGA256`, not `caBGA256`. A Cynthion r1.4 is `CABGA256`.
    pub package: String,
    /// The IO standard an output pad is configured for.
    ///
    /// The `BASE_TYPE` value is built from it: an output becomes
    /// `OUTPUT_<standard>`. A Cynthion's LEDs are on bank 1, whose VCCIO
    /// is 3.3 V, so `LVCMOS33`.
    pub io_standard: String,
}

impl Default for TrellisOptions {
    fn default() -> TrellisOptions {
        TrellisOptions {
            package: "CABGA256".to_owned(),
            io_standard: "LVCMOS33".to_owned(),
        }
    }
}

impl TrellisOptions {
    /// The defaults: a Cynthion's package and its LED bank's standard.
    #[must_use]
    pub fn new() -> TrellisOptions {
        TrellisOptions::default()
    }
}

/// What was read, before anything is turned into an [`Arch`].
///
/// Opening is separated from loading the way
/// [`super::apicula::ApiculaDatabase`] separates them, and for the same
/// reason: the part's identity and its package list are wanted by a
/// caller that has not decided what to build yet.
#[derive(Clone, Debug)]
pub struct TrellisDatabase {
    device: DeviceInfo,
    tiles: Vec<TileEntry>,
    pinouts: Vec<Pinout>,
    /// Which IO bank each PIO position belongs to, from `iodb.json`'s
    /// `pio_metadata`. A property of the die, not of a package.
    banks: BTreeMap<(u32, u32, String), u32>,
    /// The clock network's geometry, from `globals.json`.
    globals: parse::Globals,
    types: BTreeMap<String, TileDatabase>,
}

/// The file every complete copy of the database has.
pub const PROBE_FILE: &str = "devices.json";

/// Reads the part of a Project Trellis database that describes one ECP5.
///
/// `root` is the directory holding `devices.json`. What is read is that
/// file, the part's `tilegrid.json` and `iodb.json`, and one `bits.db` per
/// tile type the grid actually uses — 185 of them on this part, which is
/// every type the family has.
///
/// `globals.json` is read too: it is the clock network's geometry, and
/// [`ClockNetwork`] says what could not be worked out without it.
///
/// # Errors
///
/// [`TrellisError::Missing`] for a file that is not there,
/// [`TrellisError::Malformed`] for one that is not the shape it has to be,
/// and [`TrellisError::NoSuchDevice`] when `devices.json` has no such
/// part.
pub fn open(
    files: &dyn FileProvider,
    root: &str,
    device: &str,
) -> Result<TrellisDatabase, TrellisError> {
    let root = root.trim_end_matches('/');
    // An empty root means "the provider's own names are the database's",
    // which is what the tests below and a caller with an in-memory copy
    // want; prefixing it would ask for `/devices.json`.
    let at = |rest: &str| -> String {
        if root.is_empty() {
            rest.to_owned()
        } else {
            format!("{root}/{rest}")
        }
    };
    let read = |path: String| -> Result<String, TrellisError> {
        files.read_file(&path).ok_or(TrellisError::Missing { path })
    };

    let devices = parse::devices(&read(at(PROBE_FILE))?)?;
    let Some(info) = devices.iter().find(|d| d.name == device).cloned() else {
        return Err(TrellisError::NoSuchDevice {
            wanted: device.to_owned(),
            known: devices
                .iter()
                .filter(|d| d.family == "ECP5")
                .map(|d| d.name.clone())
                .collect(),
        });
    };

    let grid_path = at(&format!("{}/{}/tilegrid.json", info.family, info.name));
    let tiles = parse::tilegrid(&read(grid_path.clone())?, &grid_path)?;

    let io_path = at(&format!("{}/{}/iodb.json", info.family, info.name));
    let io_text = read(io_path.clone())?;
    let pinouts = parse::iodb(&io_text, &io_path)?;
    let banks = parse::pio_banks(&io_text, &io_path)?;

    let globals_path = at(&format!("{}/{}/globals.json", info.family, info.name));
    let globals = parse::globals(&read(globals_path.clone())?, &globals_path)?;

    // One `bits.db` per type the grid uses. They are shared by the whole
    // family — `<family>/tiledata/<type>/bits.db` — which is why they are
    // keyed by type and not by part.
    let mut types: BTreeMap<String, TileDatabase> = BTreeMap::new();
    for tile in &tiles {
        if types.contains_key(&tile.ty) {
            continue;
        }
        let path = at(&format!("{}/tiledata/{}/bits.db", info.family, tile.ty));
        let text = read(path.clone())?;
        types.insert(tile.ty.clone(), TileDatabase::parse(&text, &path)?);
    }

    Ok(TrellisDatabase {
        device: info,
        tiles,
        pinouts,
        banks,
        globals,
        types,
    })
}

impl TrellisDatabase {
    /// What `devices.json` says about the part.
    #[must_use]
    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// The packages its `iodb.json` describes, in file order.
    #[must_use]
    pub fn packages(&self) -> Vec<&str> {
        self.pinouts.iter().map(|p| p.package.as_str()).collect()
    }

    /// How many tiles the grid has, and how many distinct types they use.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        (self.tiles.len(), self.types.len())
    }

    /// One tile type's `bits.db`.
    #[must_use]
    pub fn tile_database(&self, ty: &str) -> Option<&TileDatabase> {
        self.types.get(ty)
    }

    /// Turns the database into a fabric: an [`Arch`], the shape of the
    /// configuration memory, and where every pad's bits are.
    ///
    /// # Errors
    ///
    /// [`TrellisError::NoSuchPackage`] when the part's `iodb.json` has no
    /// such package, [`TrellisError::NoSuchIoStandard`] when no
    /// `BASE_TYPE` field offers an output in that standard, and
    /// [`TrellisError::NoSuchField`] when a field a pad needs is not in
    /// the `bits.db` this copy of the database ships.
    pub fn load(&self, options: &TrellisOptions) -> Result<TrellisFabric, TrellisError> {
        let width = self.device.width();
        let height = self.device.height();
        let prefix = self.device.chip_prefix();

        // ---- the configuration memory, position by position ----
        //
        // `tilegrid` is sorted by Lattice tile name, so a position's
        // windows are pushed in the same order on every run, and a
        // `ConfigBit`'s row means the same thing twice.
        let mut frames = Ecp5FrameMap::new();
        let mut members: BTreeMap<(u32, u32), Vec<String>> = BTreeMap::new();
        // Where each of a position's Lattice tiles starts in that
        // position's combined frame numbering, gathered in the same pass.
        // It used to be re-derived per position by filtering the whole
        // tile list, which is the grid squared for an answer this loop
        // already has in order.
        let mut windows_at: BTreeMap<(u32, u32), FrameWindows> = BTreeMap::new();
        for tile in &self.tiles {
            let at = (tile.col, tile.row);
            frames.push(at, tile.window);
            members.entry(at).or_default().push(tile.ty.clone());
            let (here, before) = windows_at.entry(at).or_default();
            here.push((tile.ty.clone(), *before));
            *before += tile.window.frames;
        }

        // ---- one Arch tile type per composition of a position ----
        //
        // A position's type is the list of Lattice types it holds, joined
        // with `+`. Two positions with the same list always have the same
        // geometry on this family, and that is checked rather than
        // assumed: a mismatch would silently move every bit of one of
        // them.
        let mut arch = Arch::new(
            format!("{}-trellis", self.device.name.to_lowercase()),
            "ecp5",
            width,
            height,
        );
        arch.parts.push(self.device.name.clone());
        arch.asc_device = self.device.name.clone();
        let mut type_of: BTreeMap<String, usize> = BTreeMap::new();
        // Per composition, where each of its Lattice tiles starts in the
        // position's combined frame numbering. Taken from the first
        // position of the composition and checked against every other,
        // because it is what every bit of a shared position hangs off.
        let mut layout: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
        for (at, list) in &members {
            let name = list.join("+");
            let rows = frames.bit_rows(*at);
            let cols = frames.bit_cols(*at);
            let windows = windows_at
                .get(at)
                .map_or_else(Vec::new, |(windows, _)| windows.clone());
            let index = match type_of.get(&name) {
                Some(index) => {
                    let existing = &arch.tile_types[*index];
                    if existing.bit_rows != rows
                        || existing.bit_cols != cols
                        || layout.get(&name) != Some(&windows)
                    {
                        return Err(TrellisError::Malformed {
                            path: format!(
                                "{}/{}/tilegrid.json",
                                self.device.family, self.device.name
                            ),
                            what: format!(
                                "two positions are both `{name}` and their bit regions are laid \
                                 out differently: {}x{} against {rows}x{cols}",
                                existing.bit_rows, existing.bit_cols
                            ),
                        });
                    }
                    *index
                }
                None => {
                    let index = arch.tile_types.len();
                    // The keyword is the type's name in lower case, which
                    // is only ever used by the `.asc` debugging form.
                    arch.tile_types.push(TileType::new(
                        name.clone(),
                        name.to_lowercase(),
                        rows,
                        cols,
                    ));
                    type_of.insert(name.clone(), index);
                    layout.insert(name, windows);
                    index
                }
            };
            arch.set_tile(at.0, at.1, index);
        }

        // ---- the interconnect ----
        //
        // Which position owns a wire name, worked out by walking every
        // reference on the die. A name a tile spells without a direction
        // prefix belongs to that tile's own position; a name it spells with
        // one (`S1E1_JA0`) belongs to the position the prefix points at,
        // and that position's *composition* has to declare it or the
        // reference resolves to nothing. Doing it this way round — collect
        // the references, then declare — is what makes every reference on
        // the die resolve, rather than only the ones whose owner happens to
        // mention the name itself.
        //
        // **It is done per tile *type*, not per position**, and that is
        // worth saying because doing it per position was most of the
        // loader's cost. `bits.db` belongs to the family, so one type
        // spells the same million-and-a-half references at every position
        // it sits at; what varies with the position is only *which
        // composition* an offset lands on. So the names are classified
        // once per type, grouped by the offset they point at, and a group
        // is handed to a neighbouring composition the first time that
        // (type, offset, composition) triple turns up — about a hundred
        // thousand set insertions over this die instead of three million.
        // The result is the same set of names for the same composition,
        // because a set does not count how often something was inserted.
        struct TypeRefs<'a> {
            /// Names that reach the whole die.
            globals: Vec<&'a str>,
            /// The rest, grouped by the offset `(dx, dy)` they point at.
            groups: Vec<((i32, i32), Vec<&'a str>)>,
        }
        let mut refs_of: BTreeMap<&str, TypeRefs<'_>> = BTreeMap::new();
        for (ty, db) in &self.types {
            let mut reaching = Vec::new();
            let mut groups: BTreeMap<(i32, i32), Vec<&str>> = BTreeMap::new();
            for name in db.wire_names() {
                match parse::globalise_ref(name, prefix) {
                    None => {}
                    Some(parse::WireTargetRef::Global { name }) => reaching.push(name),
                    Some(parse::WireTargetRef::Tile { dx, dy, name }) => {
                        groups.entry((dx, dy)).or_default().push(name);
                    }
                }
            }
            refs_of.insert(
                ty.as_str(),
                TypeRefs {
                    globals: reaching,
                    groups: groups.into_iter().collect(),
                },
            );
        }

        // Each position's composition, once, so the walk below never joins
        // a type list into a string again.
        let mut comp_at: BTreeMap<(u32, u32), &str> = BTreeMap::new();
        for (at, list) in &members {
            if let Some(comp) = composition_key(&type_of, list) {
                comp_at.insert(*at, comp);
            }
        }

        let mut owned: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut globals: BTreeSet<&str> = BTreeSet::new();
        let mut off_grid = 0usize;
        for refs in refs_of.values() {
            for name in &refs.globals {
                globals.insert(name);
            }
        }
        // The triples already handed over; see the paragraph above.
        let mut handed: BTreeSet<(&str, i32, i32, &str)> = BTreeSet::new();
        for (at, list) in &members {
            if !comp_at.contains_key(at) {
                continue;
            }
            for ty in list {
                let Some(refs) = refs_of.get(ty.as_str()) else {
                    continue;
                };
                for ((dx, dy), names) in &refs.groups {
                    let x = i64::from(at.0) + i64::from(*dx);
                    let y = i64::from(at.1) + i64::from(*dy);
                    let target = match (u32::try_from(x), u32::try_from(y)) {
                        (Ok(x), Ok(y)) => comp_at.get(&(x, y)).copied(),
                        _ => None,
                    };
                    let Some(target) = target else {
                        off_grid += names.len();
                        continue;
                    };
                    if !handed.insert((ty.as_str(), *dx, *dy, target)) {
                        continue;
                    }
                    let into = owned.entry(target).or_default();
                    for name in names {
                        into.insert(name);
                    }
                }
            }
        }
        arch.globals = globals.iter().map(|g| (*g).to_owned()).collect();

        // Now the declarations, per tile *type*: the wires it owns, and one
        // pip per `.mux` source and per `.fixed_conn`. A pip's reference is
        // relative, so it is the same in every tile of the type and
        // `Arch::build_graph` resolves it per position.
        let mut arcs = 0usize;
        let mut fixed = 0usize;
        // The bits a `.mux` source wants **clear**, which the pip itself
        // cannot carry; see [`TrellisFabric::dropped_clear_bits`], which is
        // what makes dropping them sound rather than merely convenient.
        let mut clears: BTreeMap<(usize, Vec<ConfigBit>), Vec<ConfigBit>> = BTreeMap::new();
        let mut clear_collisions = 0usize;
        for (name, index) in &type_of {
            let ty = &mut arch.tile_types[*index];
            if let Some(names) = owned.get(name.as_str()) {
                for wire in names {
                    ty.wires.push(super::arch::WireDecl {
                        name: (*wire).to_owned(),
                        dx: 0,
                        dy: 0,
                    });
                }
            }
            for (lattice, offset) in layout.get(name).into_iter().flatten() {
                let Some(db) = self.types.get(lattice) else {
                    continue;
                };
                for (sink, source, bits) in db.arcs() {
                    let (Some(to), Some(from)) = (
                        wire_ref(sink, prefix, &globals),
                        wire_ref(source, prefix, &globals),
                    ) else {
                        continue;
                    };
                    let set: Vec<ConfigBit> = bits
                        .iter()
                        .filter(|bit| !bit.inverted)
                        .map(|bit| ConfigBit::new(offset + bit.frame, bit.bit))
                        .collect();
                    let clear: Vec<ConfigBit> = bits
                        .iter()
                        .filter(|bit| bit.inverted)
                        .map(|bit| ConfigBit::new(offset + bit.frame, bit.bit))
                        .collect();
                    if !clear.is_empty() {
                        let mut key = set.clone();
                        key.sort_unstable();
                        match clears.entry((*index, key)) {
                            std::collections::btree_map::Entry::Vacant(slot) => {
                                slot.insert(clear);
                            }
                            std::collections::btree_map::Entry::Occupied(mut slot) => {
                                // Two sources of one tile type with the same
                                // bits set and different bits clear are
                                // indistinguishable in a finished bitstream
                                // whatever is recorded here, so only what
                                // they agree about can be checked. Six of
                                // this die's sources are like that, and
                                // `TrellisStats::clear_collisions` counts
                                // them so the bound on the check is visible
                                // rather than implied.
                                if *slot.get() != clear {
                                    clear_collisions += 1;
                                    slot.get_mut().retain(|bit| clear.contains(bit));
                                }
                            }
                        }
                    }
                    ty.pips.push(super::arch::PipDecl {
                        from,
                        to,
                        bits: set,
                    });
                    arcs += 1;
                }
                for (sink, source) in &db.fixed {
                    let (Some(to), Some(from)) = (
                        wire_ref(sink, prefix, &globals),
                        wire_ref(source, prefix, &globals),
                    ) else {
                        continue;
                    };
                    ty.pips.push(super::arch::PipDecl {
                        from,
                        to,
                        bits: Vec::new(),
                    });
                    fixed += 1;
                }
            }
        }

        // ---- the clock network's implicit joins ----
        //
        // See [`ClockNetwork`]: three hops of the network carry the same
        // wire name in two different tiles and `bits.db` never says so, so
        // they are declared here from `globals.json`'s geometry. They cost
        // no bits, which is why declaring them is not a claim about the
        // bitstream — every bit of a clock route is still a `.mux` record
        // the router charges for.
        let clocks = self.clock_network(&mut arch, &globals)?;
        let joins = clocks.joins;

        // ---- the lookup tables ----
        let mut luts: BTreeMap<(usize, String), LutBits> = BTreeMap::new();
        for (name, index) in &type_of {
            let Some(offset) = layout
                .get(name)
                .and_then(|w| w.iter().find(|(ty, _)| ty == LOGIC_TILE))
                .map(|(_, offset)| *offset)
            else {
                continue;
            };
            let Some(db) = self.types.get(LOGIC_TILE) else {
                continue;
            };
            for z in 0..LUTS_PER_TILE {
                let letter = slice_letter(z / 2);
                let half = z % 2;
                let bel = format!("SLICE{letter}.K{half}");
                let pins: Vec<(String, super::arch::WireRef)> = LUT_INPUTS
                    .iter()
                    .enumerate()
                    .map(|(k, letter)| {
                        (
                            format!("i{k}"),
                            super::arch::WireRef::local(format!("{letter}{z}_SLICE")),
                        )
                    })
                    .chain(std::iter::once((
                        "o".to_owned(),
                        super::arch::WireRef::local(format!("F{z}_SLICE")),
                    )))
                    .collect();
                // A pin whose wire the type does not own would be recorded
                // in `Netlist::off_fabric` and silently dropped, so a bel
                // that cannot be wired up is not declared at all.
                if pins
                    .iter()
                    .any(|(_, wire)| !arch.tile_types[*index].has_wire(&wire.name))
                {
                    continue;
                }
                let Some(word) = db.word_bits(&format!("{bel}.INIT")) else {
                    continue;
                };
                let mut init_zero = Vec::with_capacity(word.len());
                let mut init_one = Vec::with_capacity(word.len());
                for group in word {
                    let at = |want: bool| -> Vec<ConfigBit> {
                        group
                            .iter()
                            .filter(|bit| bit.inverted == want)
                            .map(|bit| ConfigBit::new(offset + bit.frame, bit.bit))
                            .collect()
                    };
                    // A `bits.db` bit marked `!` is set when the feature is
                    // *false*, so an inverted bit belongs to the zero list.
                    init_zero.push(at(true));
                    init_one.push(at(false));
                }
                let mut tie_high = Vec::with_capacity(LUT_INPUTS.len());
                for input in LUT_INPUTS {
                    let field = format!("SLICE{letter}.{input}{half}MUX");
                    tie_high.push(match db.enum_bits(&field, TIE_HIGH) {
                        Some(bits) => bits
                            .iter()
                            .filter(|bit| !bit.inverted)
                            .map(|bit| ConfigBit::new(offset + bit.frame, bit.bit))
                            .collect(),
                        None => Vec::new(),
                    });
                }
                let mut decl = BelDecl::new(&bel, "lut");
                decl.pins = pins;
                arch.tile_types[*index].bels.push(decl);
                luts.insert(
                    (*index, bel),
                    LutBits {
                        init_zero,
                        init_one,
                        tie_high,
                    },
                );
            }
        }

        // ---- the flip-flops ----
        //
        // A slice's flip-flop is placeable on its own on this family, which
        // is unusual: `M<z>_SLICE` is a mux output the interconnect drives
        // and `SLICE<l>.REG<n>.SD = 0` selects it over the lookup table
        // beside it, so nothing has to pack a LUT and a flop onto one site.
        // A Gowin flop's `D` and a 7-series `AFF`'s have no tile wire at
        // all, which is why neither family can place one alone.
        //
        // The settings are not a bel's `ConfigEntry`s for the same reason a
        // truth table is not: the ECP5 has **one** flip-flop primitive and
        // its behaviour is in its parameters, so a `ConfigEntry::Cell` keyed
        // on the primitive name would write one variant's bits for all
        // thirty-two. `configure_registers` reads the parameters instead.
        let mut ffs: BTreeMap<(usize, String), FfBits> = BTreeMap::new();
        for (name, index) in &type_of {
            let Some(offset) = layout
                .get(name)
                .and_then(|w| w.iter().find(|(ty, _)| ty == LOGIC_TILE))
                .map(|(_, offset)| *offset)
            else {
                continue;
            };
            let Some(db) = self.types.get(LOGIC_TILE) else {
                continue;
            };
            let at = |field: &str, value: &str| -> Vec<ConfigBit> {
                db.enum_bits(field, value)
                    .into_iter()
                    .flatten()
                    .filter(|bit| !bit.inverted)
                    .map(|bit| ConfigBit::new(offset + bit.frame, bit.bit))
                    .collect()
            };
            for z in 0..LUTS_PER_TILE {
                let letter = slice_letter(z / 2);
                let half = z % 2;
                let control = z / 2;
                let bel = format!("SLICE{letter}.FF{half}");
                let pins: Vec<(String, super::arch::WireRef)> = FF_PINS
                    .iter()
                    .map(|(role, wire)| {
                        (
                            (*role).to_owned(),
                            super::arch::WireRef::local(
                                wire.replace('#', &z.to_string())
                                    .replace('@', &control.to_string()),
                            ),
                        )
                    })
                    .collect();
                if pins
                    .iter()
                    .any(|(_, wire)| !arch.tile_types[*index].has_wire(&wire.name))
                {
                    continue;
                }
                // A tile whose database cannot say "take the data from the
                // fabric" cannot hold a flip-flop this flow could use, and
                // leaving the bel out is better than placing one that would
                // latch the lookup table beside it.
                let sd = format!("SLICE{letter}.REG{half}.SD");
                if db.enum_bits(&sd, "0").is_none() {
                    continue;
                }
                let mut decl = BelDecl::new(&bel, "ff");
                decl.pins = pins;
                arch.tile_types[*index].bels.push(decl);
                ffs.insert(
                    (*index, bel),
                    FfBits {
                        control: u32::try_from(control).unwrap_or(0),
                        sd_fabric: at(&sd, "0"),
                        regset: [
                            at(&format!("SLICE{letter}.REG{half}.REGSET"), "RESET"),
                            at(&format!("SLICE{letter}.REG{half}.REGSET"), "SET"),
                        ],
                        lsrmode_lsr: at(&format!("SLICE{letter}.REG{half}.LSRMODE"), "LSR"),
                        gsr: [
                            at(&format!("SLICE{letter}.GSR"), "ENABLED"),
                            at(&format!("SLICE{letter}.GSR"), "DISABLED"),
                        ],
                        cemux: [
                            at(&format!("SLICE{letter}.CEMUX"), "1"),
                            at(&format!("SLICE{letter}.CEMUX"), "CE"),
                        ],
                        clkmux_inv: [at("CLK0.CLKMUX", "INV"), at("CLK1.CLKMUX", "INV")],
                        lsrmux_inv: [at("LSR0.LSRMUX", "INV"), at("LSR1.LSRMUX", "INV")],
                        srmode_async: [at("LSR0.SRMODE", "ASYNC"), at("LSR1.SRMODE", "ASYNC")],
                    },
                );
            }
        }

        // ---- the distributed RAM ----
        //
        // A `TRELLIS_DPR16X4` is not a bel of the silicon: it is three
        // slices of one logic tile used together, and one bit of the tile
        // says so for all three. `DPRAM_PINS` has the wiring and its
        // provenance; what is decided here is only whether this tile type
        // can hold one, and the answer is no unless it owns every one of
        // those thirty-two wires, the three `MODE` settings exist, and all
        // six lookup tables the RAM consumes were declared above — a RAM
        // whose contents this flow could not write would place, route and
        // read nothing back.
        let mut dprams: BTreeMap<usize, DpRamBits> = BTreeMap::new();
        for (name, index) in &type_of {
            let Some(offset) = layout
                .get(name)
                .and_then(|w| w.iter().find(|(ty, _)| ty == LOGIC_TILE))
                .map(|(_, offset)| *offset)
            else {
                continue;
            };
            let Some(db) = self.types.get(LOGIC_TILE) else {
                continue;
            };
            if DPRAM_PINS
                .iter()
                .any(|(_, wire)| !arch.tile_types[*index].has_wire(wire))
            {
                continue;
            }
            if DPRAM_DATA_LUTS
                .iter()
                .chain(DPRAM_RAMW_LUTS.iter())
                .any(|bel| !luts.contains_key(&(*index, (*bel).to_owned())))
            {
                continue;
            }
            let mut mode: Vec<ConfigBit> = Vec::new();
            let mut complete = true;
            for (field, value) in DPRAM_MODES {
                match db.enum_bits(field, value) {
                    Some(bits) => {
                        for bit in bits.iter().filter(|bit| !bit.inverted) {
                            let at = ConfigBit::new(offset + bit.frame, bit.bit);
                            if !mode.contains(&at) {
                                mode.push(at);
                            }
                        }
                    }
                    None => complete = false,
                }
            }
            if !complete || mode.is_empty() {
                continue;
            }
            let mut decl = BelDecl::new(DPRAM_BEL, "lutram");
            decl.pins = DPRAM_PINS
                .iter()
                .map(|(role, wire)| {
                    (
                        (*role).to_owned(),
                        super::arch::WireRef::local((*wire).to_owned()),
                    )
                })
                .collect();
            decl.blocks = DPRAM_BLOCKS.iter().map(|b| (*b).to_owned()).collect();
            arch.tile_types[*index].bels.push(decl);
            dprams.insert(*index, DpRamBits { mode });
        }

        // ---- the block RAM ----
        //
        // A `DP16KD` is 18 kbit, true dual port, and it is **not** a slice
        // writ large: the rule a distributed RAM taught — "one tile, one
        // bit, three slices" — does not transfer and a guess by analogy
        // would have been wrong. What the database says, read rather than
        // reasoned about:
        //
        // - the EBR row of this die is nine tile types repeating,
        //   `MIB_EBR0` to `MIB_EBR8`, and they hold **four** blocks
        //   between them;
        // - exactly four of the nine — `MIB_EBR0`, `MIB_EBR2`, `MIB_EBR4`
        //   and `MIB_EBR6` — carry the 116 `.fixed_conn` records that
        //   join a block's pins to the interconnect. Those are the tiles
        //   that own a block, and the bel goes on them;
        // - a block's **fields** are spread over its own tile and the two
        //   east of it, and a single field's bits straddle the boundary.
        //   `EBR1.DP16KD.DATA_WIDTH_B` is three bits of `MIB_EBR2` and a
        //   fourth of `MIB_EBR4`; without that fourth bit the 9-bit and
        //   18-bit modes are the same pattern. So a block's bits are
        //   gathered from [`BRAM_SPAN`] positions and each one remembers
        //   which position it belongs to;
        // - which of the four a tile's block is, is read off the field
        //   names: a tile that owns a block declares `EBR<n>.<field>` for
        //   its own `n` and, where the fuzzer found a leftover bit, for
        //   `n - 1` as well, so the block is the **highest** index the
        //   tile names.
        //
        // The four blocks of a group overlap in the interconnect: the two
        // top data bits of each of a block's ports are the same wires as
        // the two bottom bits of the block two columns east. That is in
        // this model — the pins are declared and the routing graph has
        // them — and `configure_bram` refuses the one arrangement where it
        // bites. `docs/fpga-trellis.md` says what is and is not settled
        // about it.
        let mut brams: Vec<(usize, BramSite)> = Vec::new();
        // Per candidate tile type: the bel it would get, and whether every
        // position of that type turned out complete. A bel is declared only
        // when they all did, because a site whose bits this flow cannot
        // write would place and configure nothing.
        type BramCandidate = (String, Vec<(String, super::arch::WireRef)>, bool);
        let mut bram_bel: BTreeMap<usize, BramCandidate> = BTreeMap::new();
        for (at, list) in &members {
            let Some(index) = comp_at.get(at).and_then(|c| type_of.get(*c)).copied() else {
                continue;
            };
            // Which of this position's tiles owns a block RAM's wires.
            let Some(owner) = list.iter().find(|ty| {
                self.types.get(ty.as_str()).is_some_and(|db| {
                    db.fixed.iter().any(|(sink, source)| {
                        sink.ends_with(BRAM_WIRE.1) || source.ends_with(BRAM_WIRE.1)
                    })
                })
            }) else {
                continue;
            };
            let Some(db) = self.types.get(owner.as_str()) else {
                continue;
            };
            // The block's index inside its group, off the field names.
            let mut which: Option<u32> = None;
            for (field, _, _) in &db.enums {
                let Some(rest) = field.strip_prefix("EBR") else {
                    continue;
                };
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(n) = digits.parse::<u32>() {
                    which = Some(which.map_or(n, |had: u32| had.max(n)));
                }
            }
            let Some(which) = which else { continue };
            let bel = format!("EBR{which}");
            let prefix_of = format!("EBR{which}.");

            // The pins: the roles are written down, the wires are read out
            // of this tile's own fixed connections.
            let mut pins: Vec<(String, super::arch::WireRef)> = Vec::new();
            for (role, pin) in bram_pins() {
                let own = format!("{}{pin}{}", BRAM_WIRE.0, BRAM_WIRE.1);
                let joined = db
                    .fixed
                    .iter()
                    .find_map(|(sink, source)| {
                        if *sink == own {
                            Some(source.as_str())
                        } else if *source == own {
                            Some(sink.as_str())
                        } else {
                            None
                        }
                    })
                    .and_then(|name| wire_ref(name, prefix, &globals));
                if let Some(wire) = joined {
                    pins.push((role, wire));
                }
            }

            // The bits, from this position and the two east of it.
            let mut enums: Vec<(String, String, Vec<PlacedBit>)> = Vec::new();
            let mut words: Vec<(String, Vec<Vec<PlacedBit>>)> = Vec::new();
            for dx in 0..BRAM_SPAN {
                let there = (at.0 + dx, at.1);
                let Some((windows, _)) = windows_at.get(&there) else {
                    continue;
                };
                for (ty, offset) in windows {
                    let Some(db) = self.types.get(ty.as_str()) else {
                        continue;
                    };
                    let at_bit = |bit: &parse::DbBit| -> PlacedBit {
                        (there, ConfigBit::new(offset + bit.frame, bit.bit))
                    };
                    for (field, _, options) in &db.enums {
                        let Some(name) = field.strip_prefix(prefix_of.as_str()) else {
                            continue;
                        };
                        for (value, bits) in options {
                            let set: Vec<PlacedBit> = bits
                                .iter()
                                .filter(|bit| !bit.inverted)
                                .map(at_bit)
                                .collect();
                            match enums
                                .iter_mut()
                                .find(|(f, v, _)| f == name && v == value)
                            {
                                Some((_, _, had)) => had.extend(set),
                                None => enums.push((name.to_owned(), value.clone(), set)),
                            }
                        }
                    }
                    for (field, _, groups) in &db.words {
                        let Some(name) = field.strip_prefix(prefix_of.as_str()) else {
                            continue;
                        };
                        let here: Vec<Vec<PlacedBit>> = groups
                            .iter()
                            .map(|group| {
                                group
                                    .iter()
                                    .filter(|bit| !bit.inverted)
                                    .map(at_bit)
                                    .collect()
                            })
                            .collect();
                        match words.iter_mut().find(|(f, _)| f == name) {
                            // A word can straddle a tile boundary too, and
                            // `WID` does: `MIB_EBR7` holds four of its nine
                            // bits and the tile east of it the other five,
                            // each marking the ones it does not hold `-`.
                            Some((_, had)) if had.len() == here.len() => {
                                for (into, from) in had.iter_mut().zip(here) {
                                    into.extend(from);
                                }
                            }
                            Some(_) => {}
                            None => words.push((name.to_owned(), here)),
                        }
                    }
                }
            }
            enums.sort();
            words.sort();
            let site = BramSite {
                at: *at,
                bel: bel.clone(),
                index: which,
                enums,
                words,
            };
            // Complete enough to configure: every field this flow writes,
            // every pin the primitive has, and nine `WID` bits.
            let mut complete = pins.len() == bram_pins().len()
                && site.has_enum(BRAM_MODE.0)
                && site.enum_bits(BRAM_MODE.0, BRAM_MODE.1).is_some()
                && site.enum_bits(BRAM_GSR.0, BRAM_GSR.1).is_some()
                && site
                    .word(BRAM_WID)
                    .is_some_and(|w| w.len() == BRAM_WID_BITS as usize);
            for field in BRAM_WIDTH {
                complete &= site.has_enum(field);
            }
            for field in BRAM_WRITEMODE.0 {
                complete &= site.enum_bits(field, BRAM_WRITEMODE.1).is_some();
            }
            for field in BRAM_RESET.0 {
                complete &= site.enum_bits(field, BRAM_RESET.1).is_some();
            }
            for letter in BRAM_PORTS {
                for field in BRAM_TIE_LOW {
                    let field = field.replace('#', &letter.to_string());
                    complete &= site.enum_bits(&field, BRAM_INV).is_some();
                }
            }
            let entry = bram_bel.entry(index).or_insert((bel, pins, true));
            entry.2 &= complete;
            if complete {
                brams.push((index, site));
            }
        }
        for (index, (bel, pins, complete)) in &bram_bel {
            if !complete {
                continue;
            }
            let mut decl = BelDecl::new(bel, "bram");
            decl.pins = pins.clone();
            arch.tile_types[*index].bels.push(decl);
        }
        let brams: Vec<BramSite> = brams
            .into_iter()
            .filter(|(index, _)| bram_bel.get(index).is_some_and(|(_, _, done)| *done))
            .map(|(_, site)| site)
            .collect();

        // ---- the pads ----
        let Some(pinout) = self.pinouts.iter().find(|p| p.package == options.package) else {
            return Err(TrellisError::NoSuchPackage {
                wanted: options.package.clone(),
                known: self.pinouts.iter().map(|p| p.package.clone()).collect(),
            });
        };

        let mut io = Vec::new();
        let mut skipped = 0usize;
        for (ball, (row, col, side)) in &pinout.balls {
            let Some(side) = side.chars().next() else {
                continue;
            };
            let Some(edge) = Edge::of(*col, *row, width, height, side) else {
                skipped += 1;
                continue;
            };
            // The bank comes from `pio_metadata` and nothing infers it. A
            // pad whose bank the database does not state is left out for
            // the same reason as one whose tiles are missing: it could be
            // placed and then configured incompletely, which is a dark
            // pin with nothing to explain it.
            let Some(bank) = self.banks.get(&(*row, *col, side.to_string())).copied() else {
                skipped += 1;
                continue;
            };
            match IoSite::locate(
                ball,
                (*col, *row),
                side,
                edge,
                bank,
                self,
                &options.io_standard,
            ) {
                Ok(site) => io.push(site),
                Err(TrellisError::NoSuchField { .. }) => {
                    // A pad whose tiles this grid does not have where the
                    // edge's rule says they are — the last column of the
                    // top edge has no eastern neighbour — is left out
                    // rather than guessed at.
                    skipped += 1;
                }
                Err(err) => return Err(err),
            }
        }
        if io.is_empty() {
            return Err(TrellisError::NoSuchIoStandard {
                wanted: options.io_standard.clone(),
                known: self.base_type_values(),
            });
        }

        // The bels, and the ball-to-site map the placer resolves a pin
        // constraint through. A bel belongs to a tile *type*, so the
        // declarations are per type and the sites come out per position
        // from `Arch::build_graph`.
        //
        // **A pad's bel sits at the position `iodb.json` gives the ball**,
        // which is not always where its bits are: the bits follow the
        // edge's tile rule and can be a column east or two rows south,
        // while the three wires the buffer presents to the fabric —
        // `PADDO<L>_PIO`, `PADDT<L>_PIO`, `JPADDI<L>_PIO` — are always the
        // ball's own position's. Putting the bel anywhere else leaves its
        // pins resolving to another tile's wires of the same name, which
        // routes a design through metal that is not there.
        let mut sides: BTreeMap<usize, BTreeSet<char>> = BTreeMap::new();
        let mut placed = Vec::new();
        for site in &io {
            let Some(index) = arch.tile_index_at(site.bel.0, site.bel.1) else {
                skipped += 1;
                continue;
            };
            let pins = pad_pins(site.side);
            if pins
                .iter()
                .any(|(_, wire)| !arch.tile_types[index].has_wire(&wire.name))
            {
                // The tile at the ball's position does not own the wires a
                // buffer of that side presents. That is a tile rule this
                // code has got wrong, and the honest thing is to leave the
                // ball out of the map rather than place a cell whose pins
                // reach nothing.
                skipped += 1;
                continue;
            }
            sides.entry(index).or_default().insert(site.side);
            placed.push(site.clone());
        }
        let io = placed;
        for (index, letters) in sides {
            for letter in letters {
                let mut bel = BelDecl::new(format!("PIO{letter}"), "io");
                bel.pins = pad_pins(letter);
                bel.config = Vec::new();
                arch.tile_types[index].bels.push(bel);
            }
        }
        for site in &io {
            arch.pinmap.push((site.ball.clone(), site.site_name()));
        }
        arch.pinmap.sort();

        // ---- the banks the pads are in ----
        //
        // One setting per bank, in a tile no pad owns; see [`BANK_VCCIO`].
        // Resolved here rather than when a bitstream is written, so a
        // database that cannot express the rail is an error from `load`.
        let Some(voltage) = bank_voltage(&options.io_standard) else {
            return Err(TrellisError::NoSuchIoStandard {
                wanted: options.io_standard.clone(),
                known: bank_voltage_standards(),
            });
        };
        let mut bank_bits: BTreeMap<u32, ((u32, u32), Vec<ConfigBit>)> = BTreeMap::new();
        for site in &io {
            if bank_bits.contains_key(&site.bank) {
                continue;
            }
            // `BANKREF<n>`, except where Lattice spells it `BANKREF<n>A`:
            // banks 2 and 7 of this die do, and a bank whose reference tile
            // cannot be found is an error rather than a pad configured
            // without its rail.
            let plain = format!("BANKREF{}", site.bank);
            let suffixed = format!("{plain}A");
            let Some(tile) = self
                .tiles
                .iter()
                .find(|t| t.ty == plain || t.ty == suffixed)
            else {
                return Err(TrellisError::NoSuchField {
                    tile: plain,
                    field: BANK_VCCIO.to_owned(),
                    value: voltage.to_owned(),
                });
            };
            let at = (tile.col, tile.row);
            let bits = self.locate_field(at, BANK_VCCIO, voltage)?;
            bank_bits.insert(site.bank, (at, bits));
        }

        let stats = TrellisStats {
            tiles: self.tiles.len(),
            positions: members.len(),
            tile_types: arch.tile_types.len(),
            lattice_types: self.types.len(),
            most_windows: members.values().map(Vec::len).max().unwrap_or(0),
            shared_positions: members.values().filter(|v| v.len() > 1).count(),
            frames: self.device.format.frames,
            bits_per_frame: self.device.format.bits_per_frame,
            balls: pinout.balls.len(),
            pads: io.len(),
            pads_skipped: skipped,
            globals: arch.globals.len(),
            wires: arch
                .tiles
                .iter()
                .flatten()
                .map(|index| arch.tile_types[*index].wires.len())
                .sum(),
            arcs,
            fixed,
            joins,
            buffers: clocks.buffers,
            clears: clears.len(),
            clear_collisions,
            luts: luts.len(),
            ffs: ffs.len(),
            dprams: dprams.len(),
            brams: brams.len(),
            clock_networks: clocks.indices.len(),
            references_off_the_grid: off_grid,
        };

        Ok(TrellisFabric {
            arch,
            format: self.device.format,
            frames,
            idcode: self.device.idcode,
            part: self.device.name.clone(),
            package: options.package.clone(),
            io,
            luts,
            ffs,
            dprams,
            brams,
            clocks,
            clears,
            bank_bits,
            voltage: voltage.to_owned(),
            standard: options.io_standard.clone(),
            stats,
        })
    }

    /// Builds [`ClockNetwork`] and declares the joins it describes.
    ///
    /// `globals` is the set of names the loader decided reach the whole
    /// die, which is what says whether `G_URPCLK0` is a node of its own.
    ///
    /// # Errors
    ///
    /// [`TrellisError::Malformed`] when `globals.json` names a spine at a
    /// position the grid does not have, or a quadrant no spine belongs to.
    /// Both would mean the two files disagree, and a clock routed from a
    /// table that disagrees with the grid is a clock that arrives nowhere.
    fn clock_network(
        &self,
        arch: &mut Arch,
        globals: &BTreeSet<&str>,
    ) -> Result<ClockNetwork, TrellisError> {
        let path = format!("{}/{}/globals.json", self.device.family, self.device.name);
        let bad = |what: String| TrellisError::Malformed {
            path: path.clone(),
            what,
        };
        let g = &self.globals;

        // How many networks there are is read rather than assumed: it is
        // the set of `n` for which *every* quadrant offers a
        // `G_<quadrant>PCLK<n>` and a logic tile offers the branch wire the
        // network ends in. On this family that is 0..16.
        let mut indices: Vec<u32> = Vec::new();
        for n in 0..64u32 {
            if g.quadrants
                .iter()
                .all(|(q, _)| globals.contains(format!("G_{q}PCLK{n}").as_str()))
            {
                indices.push(n);
            }
        }
        if indices.is_empty() {
            // A die whose `bits.db` files declare no centre mux has no
            // network to join, and that is not an error: it is what
            // `super::xray`'s `ClockColumn::default()` is for a family with
            // no rebuffers. Nothing clocked will place, because no flip-flop
            // will find a clock, and the loader says so by reporting zero
            // networks rather than by refusing to load.
            return Ok(ClockNetwork::default());
        }

        // Which wire names each tile type owns, once. `TileType::has_wire`
        // is a linear scan and this asks about a quarter of a million
        // names.
        let owned: Vec<BTreeSet<String>> = arch
            .tile_types
            .iter()
            .map(|ty| ty.wires.iter().map(|w| w.name.clone()).collect())
            .collect();

        let mut joins = 0usize;
        for (quadrant, tap, at) in &g.spines {
            let Some((_, rect)) = g.quadrants.iter().find(|(name, _)| name == quadrant) else {
                return Err(bad(format!(
                    "spine `{quadrant}{tap}` names quadrant `{quadrant}`, which `quadrants` does \
                     not describe"
                )));
            };
            let Some((_, (lx0, lx1, _, rx1))) = g.taps.iter().find(|(col, _)| col == tap) else {
                return Err(bad(format!(
                    "spine `{quadrant}{tap}` names tap column {tap}, which `taps` does not describe"
                )));
            };
            let Some(spine_type) = arch.tile_index_at(at.0, at.1) else {
                return Err(bad(format!(
                    "spine `{quadrant}{tap}` is at (col {}, row {}) and the grid has no tile \
                     there",
                    at.0, at.1
                )));
            };
            let dx = |x: u32| i64::from(x) - i64::from(at.0);
            let dy = |y: u32| i64::from(y) - i64::from(at.1);
            let mut decls: Vec<super::arch::PipDecl> = Vec::new();
            for n in &indices {
                let hprx = format!("G_HPRX{n:02}00");
                let vptx = format!("G_VPTX{n:02}00");
                let hpbx = format!("G_HPBX{n:02}00");
                // The quadrant's own primary clock onto the spine's feed
                // wire. Everything else hangs off this.
                if owned[spine_type].contains(hprx.as_str()) {
                    decls.push(super::arch::PipDecl {
                        from: super::arch::WireRef::global(format!("G_{quadrant}PCLK{n}")),
                        to: super::arch::WireRef::local(&hprx),
                        bits: Vec::new(),
                    });
                }
                if !owned[spine_type].contains(vptx.as_str()) {
                    continue;
                }
                for y in rect.1..=rect.3 {
                    // The spine's vertical wire, as the tap column's tiles
                    // spell it. A row with no tap tile — the two IO rows —
                    // has nothing to join.
                    let Some(tap_type) = arch.tile_index_at(*tap, y) else {
                        continue;
                    };
                    if !owned[tap_type].contains(vptx.as_str()) {
                        continue;
                    }
                    let (tdx, tdy) = (
                        i32::try_from(dx(*tap)).unwrap_or(0),
                        i32::try_from(dy(y)).unwrap_or(0),
                    );
                    decls.push(super::arch::PipDecl {
                        from: super::arch::WireRef::local(&vptx),
                        to: super::arch::WireRef::at(&vptx, tdx, tdy),
                        bits: Vec::new(),
                    });
                    // And the two branch drivers onto the tiles they
                    // reach. Which side a column is on is the whole of
                    // what `taps` says.
                    for x in *lx0..=*rx1 {
                        let side = if x <= *lx1 { 'L' } else { 'R' };
                        let branch = format!("{side}_HPBX{n:02}00");
                        if !owned[tap_type].contains(branch.as_str()) {
                            continue;
                        }
                        let Some(tile_type) = arch.tile_index_at(x, y) else {
                            continue;
                        };
                        if !owned[tile_type].contains(hpbx.as_str()) {
                            continue;
                        }
                        decls.push(super::arch::PipDecl {
                            from: super::arch::WireRef::at(&branch, tdx, tdy),
                            to: super::arch::WireRef::at(
                                &hpbx,
                                i32::try_from(dx(x)).unwrap_or(0),
                                tdy,
                            ),
                            bits: Vec::new(),
                        });
                    }
                }
            }
            joins += decls.len();
            arch.tile_types[spine_type].pips.extend(decls);
        }

        // ---- the buffers ----
        //
        // Every global of this family comes out of a `DCC`, and a `DCC` is
        // the one thing on the clock path that is a **bel** rather than a
        // join: the device file has `bel DCCA gb port i=CLKI o=CLKO en=CE`,
        // so the flow inserts a cell for it and the placer needs somewhere
        // to put it.
        //
        // Which tile a buffer lives in is not guessed: it is the tile type
        // whose `.fixed_conn` declares `G_CLKI_<name>`, and the bel takes
        // that name. On this die those are `TMID_0`, `TMID_1`, `LMID_0`,
        // `RMID_0` and the four `BMID`s, each of which occupies exactly one
        // position, so one declaration is one site.
        //
        // An **ungated** buffer costs no bits at all, which is why nothing
        // here has a `ConfigEntry`: nextpnr's `write_dcc` writes
        // `DCC_<x><n>.MODE = DCCA` only when the cell has a clock enable,
        // `NONE` is the field's default, and no `DCC_*.MODE` appears in any
        // of the three bitstreams Great Scott Gadgets built for this board —
        // one of which routes two globals out of `LDCC0` and `LDCC6`.
        let mut buffers = 0usize;
        for index in 0..arch.tile_types.len() {
            let mut names: Vec<String> = arch.tile_types[index]
                .pips
                .iter()
                .filter(|pip| pip.to.global && pip.to.name.starts_with("G_CLKI_"))
                .map(|pip| pip.to.name["G_CLKI_".len()..].to_owned())
                .collect();
            names.sort();
            names.dedup();
            for name in names {
                if !globals.contains(format!("G_CLKO_{name}").as_str()) {
                    continue;
                }
                let mut bel = BelDecl::new(&name, "gb");
                bel.pins = vec![
                    (
                        "i".to_owned(),
                        super::arch::WireRef::global(format!("G_CLKI_{name}")),
                    ),
                    (
                        "o".to_owned(),
                        super::arch::WireRef::global(format!("G_CLKO_{name}")),
                    ),
                ];
                if globals.contains(format!("G_JCE_{name}").as_str()) {
                    bel.pins.push((
                        "en".to_owned(),
                        super::arch::WireRef::global(format!("G_JCE_{name}")),
                    ));
                }
                arch.tile_types[index].bels.push(bel);
                buffers += 1;
            }
        }

        Ok(ClockNetwork {
            indices,
            quadrants: g.quadrants.clone(),
            taps: g.taps.clone(),
            spines: g.spines.clone(),
            joins,
            buffers,
        })
    }

    /// Every value a `PIO<side>.BASE_TYPE` field offers, for an error
    /// message.
    fn base_type_values(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for db in self.types.values() {
            for (name, _, values) in &db.enums {
                if name.ends_with(".BASE_TYPE") {
                    for (value, _) in values {
                        if !out.contains(value) {
                            out.push(value.clone());
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }
}

/// What a load measured, so the numbers in a document cannot drift from
/// the database.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrellisStats {
    /// Tiles in `tilegrid.json`.
    pub tiles: usize,
    /// Grid positions that hold at least one.
    pub positions: usize,
    /// [`Arch`] tile types, one per composition of a position.
    pub tile_types: usize,
    /// Lattice tile types, one per `bits.db` read.
    pub lattice_types: usize,
    /// The most tiles any one position holds.
    pub most_windows: usize,
    /// How many positions hold more than one.
    pub shared_positions: usize,
    /// Frames of configuration memory.
    pub frames: u32,
    /// Bits in each.
    pub bits_per_frame: u32,
    /// Balls the package's map names.
    pub balls: usize,
    /// Pads that became a bel.
    pub pads: usize,
    /// Balls left out, because their tiles are not where their edge's rule
    /// says, or because `pio_metadata` does not say which bank they are in.
    ///
    /// **Zero on the caBGA-256**, now that all four edges are described: all
    /// 197 balls that package's map names become pads. It was 77 while the
    /// left and bottom edges were left out, and a non-zero count on a
    /// package nobody has built for is the thing to look at first.
    pub pads_skipped: usize,
    /// Wires that reach the whole die, one node each.
    pub globals: usize,
    /// Wires declared over the grid, which is one graph node each.
    pub wires: usize,
    /// Programmable connections declared, from `bits.db`'s `.mux` records.
    pub arcs: usize,
    /// Unconditional connections declared, from its `.fixed_conn` records.
    pub fixed: usize,
    /// Bitless joins the clock network needed, which `bits.db` states
    /// nowhere; see [`ClockNetwork`].
    pub joins: usize,
    /// Clock buffers that became a `gb` bel.
    pub buffers: usize,
    /// `.mux` sources that want at least one bit **clear**, which is the
    /// simplification [`TrellisFabric::dropped_clear_bits`] checks.
    pub clears: usize,
    /// How many of them share their set bits with another source of the
    /// same tile type that wants different bits clear, and so can only be
    /// checked on what the two agree about. Zero on this die.
    pub clear_collisions: usize,
    /// Lookup tables that became a bel.
    pub luts: usize,
    /// Flip-flops that became a bel.
    pub ffs: usize,
    /// Tile *types* that can hold a distributed RAM, which on this family
    /// is every composition that contains a `PLC2`. One `lutram` bel each,
    /// and so one site per logic tile of the die.
    pub dprams: usize,
    /// Block RAMs the die has, one per `DP16KD` of the part.
    ///
    /// Unlike every other count here this is a number of **sites** and not
    /// of tile types, because a block RAM's bits are a property of its
    /// position; see [`BramSite`].
    pub brams: usize,
    /// Global clock networks the die has.
    pub clock_networks: usize,
    /// Wire references that point off the grid, which is what happens at
    /// the four edges and is not an error.
    pub references_off_the_grid: usize,
}

impl TrellisStats {
    /// The measurements, one per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let mut line = |what: &str, n: u64| {
            out.push_str(&format!("{what}: {n}\n"));
        };
        line("tiles", self.tiles as u64);
        line("grid positions", self.positions as u64);
        line("positions with several tiles", self.shared_positions as u64);
        line("most tiles at one position", self.most_windows as u64);
        line("arch tile types", self.tile_types as u64);
        line("lattice tile types", self.lattice_types as u64);
        line("frames", u64::from(self.frames));
        line("bits per frame", u64::from(self.bits_per_frame));
        line(
            "configuration bits",
            u64::from(self.frames) * u64::from(self.bits_per_frame),
        );
        line("package balls", self.balls as u64);
        line("pads declared", self.pads as u64);
        line("balls left out", self.pads_skipped as u64);
        line("global wires", self.globals as u64);
        line("wires", self.wires as u64);
        line("programmable connections", self.arcs as u64);
        line("fixed connections", self.fixed as u64);
        line("clock network joins", self.joins as u64);
        line("clock networks", self.clock_networks as u64);
        line("clock buffers", self.buffers as u64);
        line("mux sources wanting a bit clear", self.clears as u64);
        line("of those, ambiguous", self.clear_collisions as u64);
        line("lookup tables", self.luts as u64);
        line("flip-flops", self.ffs as u64);
        line("distributed RAM tile types", self.dprams as u64);
        line("block RAMs", self.brams as u64);
        line(
            "references off the grid",
            self.references_off_the_grid as u64,
        );
        out
    }
}

/// Which edge of the die a pad is on, and therefore which rule says where
/// its configuration lives.
///
/// All four are here now, and each one's rule was established the same way:
/// the tile rule is nextpnr's `get_pio_tile` / `get_pic_tile`, and a rule
/// that has not been checked against a part produces a bitstream that
/// loads, asserts `DONE` and drives the wrong ball. So each has been
/// checked against bitstreams Lattice's own packer wrote for this very
/// board — the top edge against all six of its LEDs, the right edge against
/// its USER button, the left edge against the HyperRAM, the TARGET USB
/// transceiver and the three VBUS switches, the bottom edge against the SPI
/// flash and the interrupt line.
///
/// # The left edge is not the right edge mirrored
///
/// It looks like it and it is not, and the difference is the thing worth
/// stating because a mirror is exactly the guess that decodes consistently
/// against itself and still drives the wrong ball. What mirrors is the
/// *column*: the tiles are at column 0 instead of the last, and the `CIB`
/// that ties the pad's data is one column **east** instead of one west,
/// which is `E1_JA0` against `W1_JA0` and is read off the buffer's own
/// `.fixed_conn` rather than written down here. What does **not** mirror is
/// the row arithmetic: the pad tile is one row **south** on both edges, and
/// the second `BASE_TYPE` is two rows south for sides C and D on both. A
/// mirror that had flipped the rows too — pad tile one row *north* — would
/// have put every left-edge pad's bits in the wrong tile, and nothing but
/// a measurement says otherwise. The measurement is
/// `what_lattices_own_packer_writes_for_a_left_edge_pad`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Row 0. Two PIOs per position, `PIOT0` / `PIOT1`, and side B's tiles
    /// are one column east of its ball's.
    Top,
    /// The last column. Four PIOs per position; the pad tile is one row
    /// south and the second `BASE_TYPE` is at the ball's own row for sides
    /// A and B and two rows south for C and D.
    Right,
    /// Column 0. Four PIOs per position, and the same row arithmetic as
    /// [`Edge::Right`]: the pad tile is one row south, in a `PICL1*` or
    /// `MIB_CIB_LR`, and the second `BASE_TYPE` is at the ball's own row
    /// (`PICL0*`) for sides A and B and two rows south (`PICL2*`) for C
    /// and D.
    Left,
    /// The last row. Two PIOs per position, `PICB0` / `PICB1`, side B's
    /// tile one column east of its ball's — and **one tile, not two**:
    /// `PICB<n>` holds the pad's own fields and the only copy of
    /// `BASE_TYPE` there is, so [`IoSite::pic_at`] is [`IoSite::pad_at`].
    /// See the note on [`IoSite::pic_at`] and the hazard in
    /// `docs/fpga-trellis.md`: on a caBGA-256 every ball of this edge is a
    /// configuration pin.
    Bottom,
}

impl Edge {
    /// The edge a `(col, row, side)` belongs to, or `None` for a position
    /// that is on no edge at all.
    ///
    /// The order matters only at a corner, and this die has no ball at
    /// one: every left- and right-edge ball of the caBGA-256 is on a row
    /// congruent to 2 modulo 3, so none is on row 0 or on the last row,
    /// and the four corner positions hold `DUMMY_TILE_*`, `BANKREF*` or
    /// `MIB_CIB_LX` rather than a `PIO`. Rows are tried first anyway,
    /// because the top and bottom edges are the ones with two sides and a
    /// side `C` or `D` there would be a position this does not describe
    /// rather than a left- or right-edge pad.
    #[must_use]
    pub fn of(col: u32, row: u32, width: u32, height: u32, side: char) -> Option<Edge> {
        if row == 0 {
            return matches!(side, 'A' | 'B').then_some(Edge::Top);
        }
        if row + 1 == height {
            return matches!(side, 'A' | 'B').then_some(Edge::Bottom);
        }
        if col == 0 && matches!(side, 'A' | 'B' | 'C' | 'D') {
            return Some(Edge::Left);
        }
        if col + 1 == width && matches!(side, 'A' | 'B' | 'C' | 'D') {
            return Some(Edge::Right);
        }
        None
    }
}

/// One pad, with every bit that configures it already located.
///
/// # Two positions that are easy to confuse
///
/// [`IoSite::bel`] is where the **buffer** is: the position `iodb.json`
/// gives the ball, whose tile owns the three wires the buffer presents to
/// the fabric. [`IoSite::pad_at`] and the rest are where its **bits** are,
/// which the edge's tile rule decides and which is a different position on
/// three of the four edges — and the *same* position on the bottom edge,
/// where the buffer's tile is the pad's tile for side A. Before there was interconnect only the bits
/// mattered and the bel was put where they were; a routed design notices,
/// because a bel's pins resolve in the tile the bel sits in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoSite {
    /// The package ball.
    pub ball: String,
    /// `A`, `B`, `C` or `D`.
    pub side: char,
    /// Which edge, and so which tile rule was used.
    pub edge: Edge,
    /// The position of the buffer itself, from `iodb.json`. Its tile owns
    /// `PADDO<side>_PIO`, `PADDT<side>_PIO` and `JPADDI<side>_PIO`.
    pub bel: (u32, u32),
    /// The position of the tile holding `BASE_TYPE`, `HYSTERESIS` and
    /// `PULLMODE`.
    pub pad_at: (u32, u32),
    /// The position of the tile holding the second copy of `BASE_TYPE`.
    ///
    /// **Equal to [`IoSite::pad_at`] on the bottom edge**, which is not a
    /// bug and not a fallback: `PICB0` and `PICB1` are the only tiles of
    /// that edge, they hold the pad's own fields and the `DATAMUX_*` the
    /// other edges keep in a tile of their own, and no position of the
    /// bottom row or the row above it declares a second
    /// `PIO<side>.BASE_TYPE`. So the second copy is the same bits, and
    /// writing it twice writes them once.
    pub pic_at: (u32, u32),
    /// The position of the `CIB` tile holding the two constant muxes,
    /// worked out from the buffer's own fixed connections rather than
    /// assumed.
    pub cib_at: (u32, u32),
    /// The bits in the pad tile that make this an **output** in the chosen
    /// standard.
    pub output_pad_bits: Vec<ConfigBit>,
    /// The same field again in the tile the edge's rule gives.
    pub output_pic_bits: Vec<ConfigBit>,
    /// The bits in the pad tile that make this an **input**.
    pub input_pad_bits: Vec<ConfigBit>,
    /// The same field again, for an input.
    pub input_pic_bits: Vec<ConfigBit>,
    /// The bits in the pad tile that make this **bidirectional**: a driver
    /// the tristate wire can release and an input buffer at the same time.
    ///
    /// On the top edge this is eight bits where an output is six and an
    /// input five, and it is not the union of the two: `BIDIR_LVCMOS33`
    /// leaves the output's `F7B0` clear and sets the input's `F3B0` and
    /// `F4B0` as well as the output's drive bits. So a bidirectional pad
    /// is its own setting and not two settings written together, which is
    /// also why [`TrellisFabric::configure_io`] writes this instead of
    /// both.
    ///
    /// Requiring the value to exist refuses nothing that loaded before it
    /// was required, and that was checked rather than hoped: across all 185
    /// tile types, every `PIO<s>.BASE_TYPE` that declares `INPUT_<x>`
    /// declares `BIDIR_<x>` too for every **single-ended** `x`. Where the
    /// two sets differ they differ only in *differential* standards —
    /// `BLVDS25` and `MLVDS25` are inputs where `BLVDS25E` and `MLVDS25E`
    /// are the bidirectional spellings, and `LVDS`, `SLVS`, `SUBLVDS`,
    /// `LVPECL33` and `LVCMOS18D` are inputs with no bidirectional form at
    /// all — and none of those is a standard [`bank_voltage`] has a rail
    /// for, so none of them loads.
    pub bidir_pad_bits: Vec<ConfigBit>,
    /// The same field again, for a bidirectional pad. Unlike an input —
    /// whose pattern in this tile is *empty* — this costs the same two
    /// bits an output costs.
    pub bidir_pic_bits: Vec<ConfigBit>,
    /// `PIO<side>.HYSTERESIS = ON`, which Lattice's own packer writes for
    /// every single-ended input **and every single-ended bidirectional
    /// pad**, and for no output. See [`HYSTERESIS_ON`].
    pub hysteresis_bits: Vec<ConfigBit>,
    /// Every value `PIO<side>.PULLMODE` takes, with the bits that select
    /// it, in the order `bits.db` lists them. [`IoSite::pull_bits`] reads
    /// it.
    ///
    /// **This field is not cosmetic.** The database's default is `DOWN`, so
    /// a bitstream that leaves it alone leaves an internal pull-*down* on
    /// the pin. That was wrong for this board's USER button, and it is
    /// wrong again, for a second reason, on a pad that is sometimes
    /// released to high impedance: then the pull is the only thing
    /// deciding what the pin reads. See [`PULL_NONE`] and [`PULL_UP`].
    pub pull_modes: Vec<(String, Vec<ConfigBit>)>,
    /// Every value `PIO<side>.SLEWRATE` takes, with the bits that select
    /// it. [`IoSite::slew_bits`] reads it, and only a pad whose constraint
    /// asks for a slew rate is written at all: see [`SLEW_ATTR`].
    pub slew_rates: Vec<(String, Vec<ConfigBit>)>,
    /// The bits, in the `CIB` tile, that tie the output data wire to a
    /// fixed **one**.
    pub high_bits: Vec<ConfigBit>,
    /// The bits that tie it to a fixed **zero**.
    pub low_bits: Vec<ConfigBit>,
    /// The bits that tie the tristate wire to a fixed zero, which is the
    /// state in which the buffer **drives**.
    pub enable_bits: Vec<ConfigBit>,
    /// The bits that tie it to a fixed one, which releases the pad to high
    /// impedance for good.
    ///
    /// Written for a pad whose tristate the netlist gives a constant *one*
    /// — `bufif1 (pad, data, 1'b0)`, or anything that folds to it. Nothing
    /// useful asks for that, but a pad that asked to be released and was
    /// wired to drive instead is a short circuit against whatever else is
    /// on the net, so it is expressed rather than approximated.
    pub tristate_bits: Vec<ConfigBit>,
    /// The IO bank the pad is wired to, from `iodb.json`'s `pio_metadata`.
    ///
    /// A bank has one setting of its own that none of the tiles above
    /// holds — [`BANK_VCCIO`], located per bank in
    /// [`TrellisFabric::bank_bits`] — so a pad that only configures its own
    /// tiles is configured incompletely.
    pub bank: u32,
}

impl IoSite {
    /// The site name the placer resolves a pin constraint to.
    #[must_use]
    pub fn site_name(&self) -> String {
        format!("X{}Y{}/PIO{}", self.bel.0, self.bel.1, self.side)
    }

    /// The bits that select one value of `PIO<side>.PULLMODE`, or an empty
    /// slice for a value the database does not name.
    ///
    /// An empty slice is also the honest answer for the field's **default**,
    /// `DOWN`, whose pattern is two bits both of which it wants clear: there
    /// is nothing to write for it, and asking for it means leaving the field
    /// alone. See [`PULL_NONE`].
    #[must_use]
    pub fn pull_bits(&self, mode: &str) -> &[ConfigBit] {
        self.pull_modes
            .iter()
            .find(|(name, _)| name == mode)
            .map_or(&[][..], |(_, bits)| bits.as_slice())
    }

    /// The bits that select one value of `PIO<side>.SLEWRATE`, or an empty
    /// slice for a value the database does not name.
    ///
    /// Empty is also the honest answer for the field's default, `SLOW`,
    /// whose pattern is the one bit `FAST` sets and which it wants clear:
    /// asking for `SLOW` means leaving the field alone. See [`SLEW_ATTR`].
    #[must_use]
    pub fn slew_bits(&self, rate: &str) -> &[ConfigBit] {
        self.slew_rates
            .iter()
            .find(|(name, _)| name == rate)
            .map_or(&[][..], |(_, bits)| bits.as_slice())
    }

    /// Locates every bit of one pad of one of the edges [`Edge`] covers.
    fn locate(
        ball: &str,
        bel: (u32, u32),
        side: char,
        edge: Edge,
        bank: u32,
        db: &TrellisDatabase,
        standard: &str,
    ) -> Result<IoSite, TrellisError> {
        let (col, row) = bel;
        // nextpnr's own rule, from `get_pio_tile` and `get_pic_tile`.
        let (pad_at, pic_at) = match edge {
            Edge::Top => {
                let x = if side == 'B' { col + 1 } else { col };
                ((x, 0), (x, 1))
            }
            // The two long edges take the same arithmetic, and that is a
            // measurement rather than a symmetry: see [`Edge`]. Only the
            // column differs, and the `CIB` below works the column out
            // from the buffer's own fixed connection.
            Edge::Right | Edge::Left => {
                let pic_row = if matches!(side, 'A' | 'B') {
                    row
                } else {
                    row + 2
                };
                ((col, row + 1), (col, pic_row))
            }
            // **One tile, so one copy.** The bottom edge is the top edge's
            // column rule — side B one column east — with the two tiles
            // collapsed into one: `PICB<n>` declares the pad's own
            // `BASE_TYPE`, `HYSTERESIS`, `PULLMODE` and `SLEWRATE` *and*
            // the `DATAMUX_*` and `TRIMUX_TSREG` that the top edge keeps in
            // a `PICT<n>` a row away, and there is no second tile at the
            // position or at the row above it. So `pic_at == pad_at`, and
            // writing the second copy writes the same bits again.
            Edge::Bottom => {
                let x = if side == 'B' { col + 1 } else { col };
                ((x, row), (x, row))
            }
        };
        let field = format!("PIO{side}.BASE_TYPE");
        let output = format!("OUTPUT_{standard}");
        let input = format!("INPUT_{standard}");
        let bidir = format!("BIDIR_{standard}");

        // Which `CIB` ties this buffer's data and enable wires, and under
        // what name. Read off the buffer's own fixed connections — `JPADDOB
        // <- S1E1_JA0` on the top edge, `JPADDOD <- S2W1_JA3` on the right
        // — which is what nextpnr does too, by walking the pips uphill of
        // the wire. Nothing here knows that `JA0` is the top edge's answer.
        let (cib_at, data_field) = db.cib_mux(bel, &format!("JPADDO{side}"))?;
        let (enable_at, enable_field) = db.cib_mux(bel, &format!("JPADDT{side}"))?;
        if enable_at != cib_at {
            return Err(TrellisError::Malformed {
                path: format!("{}/tiledata/…/bits.db", db.device.family),
                what: format!(
                    "the data and enable wires of PIO{side} at (col {col}, row {row}) are tied in \
                     different tiles, {cib_at:?} and {enable_at:?}, which this does not describe"
                ),
            });
        }

        Ok(IoSite {
            ball: ball.to_owned(),
            side,
            edge,
            bel,
            pad_at,
            pic_at,
            cib_at,
            output_pad_bits: db.locate_field(pad_at, &field, &output)?,
            output_pic_bits: db.locate_field(pic_at, &field, &output)?,
            input_pad_bits: db.locate_field(pad_at, &field, &input)?,
            input_pic_bits: db.locate_field(pic_at, &field, &input)?,
            bidir_pad_bits: db.locate_field(pad_at, &field, &bidir)?,
            bidir_pic_bits: db.locate_field(pic_at, &field, &bidir)?,
            hysteresis_bits: db.locate_field(
                pad_at,
                &format!("PIO{side}.HYSTERESIS"),
                HYSTERESIS_ON,
            )?,
            pull_modes: PULL_MODES
                .iter()
                .map(|mode| {
                    let bits = db.locate_field(pad_at, &format!("PIO{side}.PULLMODE"), mode)?;
                    Ok(((*mode).to_owned(), bits))
                })
                .collect::<Result<Vec<_>, TrellisError>>()?,
            // **Optional, unlike everything else here.** A tile type that does
            // not declare `PIO<side>.SLEWRATE` gets no bits for it rather than
            // failing the pad: `TrellisFabric::load` reads a `NoSuchField` as
            // "this pad's tiles are not where the edge's rule says" and skips
            // the pad, which is the right answer for a base type and the wrong
            // one for an attribute a design only sometimes asks for. That the
            // real ECP5 database does declare it for every PIO of every pad
            // tile a Cynthion uses is asserted against Great Scott Gadgets' own
            // bitstreams instead, in
            // `what_lattices_own_packer_writes_for_a_slew_rate`.
            slew_rates: SLEW_RATES
                .iter()
                .map(|rate| {
                    let field = format!("PIO{side}.SLEWRATE");
                    let bits = db.locate_field(pad_at, &field, rate).unwrap_or_default();
                    ((*rate).to_owned(), bits)
                })
                .collect(),
            high_bits: db.locate_field(cib_at, &data_field, TIE_HIGH)?,
            low_bits: db.locate_field(cib_at, &data_field, TIE_LOW)?,
            enable_bits: db.locate_field(cib_at, &enable_field, TIE_LOW)?,
            tristate_bits: db.locate_field(cib_at, &enable_field, TIE_HIGH)?,
            bank,
        })
    }
}

/// The three wires an IO buffer presents to the fabric, as pin roles.
///
/// There is no `pad` pin, because a package ball is not a wire a router can
/// reach — [`super::xray`] and [`super::apicula`] both say the same about
/// their IO. The names are real ones, from the fixed connections of the
/// tile the buffer sits in.
fn pad_pins(letter: char) -> Vec<(String, super::arch::WireRef)> {
    vec![
        (
            "dout".to_owned(),
            super::arch::WireRef::local(format!("PADDO{letter}_PIO")),
        ),
        (
            "oe".to_owned(),
            super::arch::WireRef::local(format!("PADDT{letter}_PIO")),
        ),
        (
            "din".to_owned(),
            super::arch::WireRef::local(format!("JPADDI{letter}_PIO")),
        ),
    ]
}

/// The global clock network, as `globals.json` describes it.
///
/// # Why a file is needed for this and for nothing else
///
/// Every other hop of this fabric is derivable from `bits.db` alone,
/// because a wire's name carries the position it belongs to: `S1E1_JA0` is
/// the `JA0` of the tile one row south and one column east, and
/// `parse::globalise_ref` resolves it. The clock network breaks that rule
/// in one specific way: **its wires carry the same name in every tile they
/// cross, with no prefix.** A logic tile's branch wire is `G_HPBX0000` and
/// so is its neighbour's, and the tap driver twenty columns away spells its
/// output `R_HPBX0000`. Nothing in the file says the three are the same
/// metal.
///
/// So three hops of a clock's path are connections the database states
/// nowhere, and `globals.json` is the only thing that says where they go:
///
/// 1. the quadrant's primary clock onto the **spine**'s feed wire —
///    `G_HPRX<n>00` at the one position `spines` names for that quadrant
///    and tap column;
/// 2. the spine's vertical wire as the **tap column** spells it —
///    `G_VPTX<n>00` at `(tap, y)` for every row of the quadrant;
/// 3. each tap's two branch drivers onto the **tiles they reach** —
///    `L_HPBX<n>00` for the columns `taps` puts left of the tap and
///    `R_HPBX<n>00` for those right of it.
///
/// That is exactly the walk nextpnr's `Ecp5GlobalRouter` does out of band:
/// `find_tap_pip` looks up `L_`/`R_HPBX<n>00` at the tap column of the
/// sink's own row, and `find_spine_pip` looks up `G_VPTX<n>00` at the spine
/// position, both from the same three tables. The difference is that here
/// they become **pips of the graph**, so the ordinary router routes a clock
/// and the ordinary `Routing::verify` walks it back, where nextpnr needs a
/// dedicated pass. They carry **no bits**: every bit of a clock route is
/// still a `.mux` record charged to the tile that owns it, so declaring a
/// join is not a claim about the bitstream.
///
/// A fourth join is the **buffer**. Every global on this family goes
/// through a `DCC`, whose input and output are two global wires with
/// nothing between them in `bits.db`. An ungated one is a wire: nextpnr's
/// `write_dcc` writes `DCC_<x><n>.MODE = DCCA` only when the cell has a
/// clock enable, `NONE` is the field's default and costs no bits, and no
/// `DCC_*.MODE` appears anywhere in the three bitstreams Great Scott
/// Gadgets built for this board — one of which routes two globals through
/// `LDCC0` and `LDCC6`. So the buffer is a bitless join too, and a clock
/// needs no cell placed on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockNetwork {
    /// Which networks the die has, as the `n` of `G_HPBX<n>00`. Read from
    /// the database rather than assumed: it is the set for which every
    /// quadrant offers a `G_<quadrant>PCLK<n>`.
    pub indices: Vec<u32>,
    /// Quadrant name to the rectangle of positions it covers, inclusive.
    pub quadrants: Vec<(String, (u32, u32, u32, u32))>,
    /// Tap column to `(lx0, lx1, rx0, rx1)`, the columns its left and
    /// right branch drivers reach, inclusive.
    pub taps: Vec<(u32, (u32, u32, u32, u32))>,
    /// Quadrant, tap column and the position of the tile driving that
    /// spine.
    pub spines: Vec<(String, u32, (u32, u32))>,
    /// How many bitless joins were declared.
    pub joins: usize,
    /// How many clock buffers became a `gb` bel.
    pub buffers: usize,
}

impl ClockNetwork {
    /// The tap column that feeds column `x`, and which of its two branch
    /// drivers does.
    #[must_use]
    pub fn tap_of(&self, x: u32) -> Option<(u32, char)> {
        self.taps.iter().find_map(|(col, (lx0, lx1, rx0, rx1))| {
            if x >= *lx0 && x <= *lx1 {
                Some((*col, 'L'))
            } else if x >= *rx0 && x <= *rx1 {
                Some((*col, 'R'))
            } else {
                None
            }
        })
    }

    /// The quadrant a position is in.
    #[must_use]
    pub fn quadrant_of(&self, x: u32, y: u32) -> Option<&str> {
        self.quadrants
            .iter()
            .find(|(_, (x0, y0, x1, y1))| x >= *x0 && x <= *x1 && y >= *y0 && y <= *y1)
            .map(|(name, _)| name.as_str())
    }

    /// The network index a branch wire names, as in `G_HPBX0700` for 7.
    #[must_use]
    pub fn branch_index(name: &str) -> Option<u32> {
        let rest = name
            .strip_prefix("G_HPBX")
            .or_else(|| name.strip_prefix("L_HPBX"))
            .or_else(|| name.strip_prefix("R_HPBX"))?;
        rest.strip_suffix("00")?.parse().ok()
    }
}

/// One wire of the routing graph, as a name and the position it starts in.
///
/// This is the form a decoding and a routing can be compared in: the
/// database spells a wire `S1E1_JA0` relative to the tile whose `bits.db`
/// mentions it and the graph calls the same metal `JA0` at the position the
/// prefix points at, so one of the two has to be resolved into the other.
pub type ResolvedWire = (String, (u32, u32));

/// Which global clock network every placed flip-flop's clock came off, and
/// which of them did not come off one at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockUse {
    /// Network index to how many clock pins it reached: a flip-flop's, a
    /// distributed RAM's write clock, or either port of a block RAM.
    pub networks: BTreeMap<u32, usize>,
    /// The cells whose clock arrived through general interconnect instead,
    /// as `<instance> on <site>`. A block RAM appears once per port.
    pub off_network: Vec<String>,
}

/// Whether a wire belongs to the global clock network.
///
/// The names are Project Trellis' own and they are systematic, which is why
/// this is a set of stems rather than a list: `HPBX` is a horizontal
/// primary branch, `VPTX` a vertical primary tap, `HPRX` the horizontal
/// primary row that feeds one, `HPFE`/`HPFW`/`VPFN`/`VPFS` the four
/// directions a buffer's output leaves in, and `<quadrant>PCLK<n>` the
/// centre mux's output. `G_CLKI_`/`G_CLKO_` are a buffer's two sides.
///
/// It is used for one thing only — [`TrellisFabric::clock_node_costs`] — so
/// a name wrongly included costs a preference and never a connection.
fn is_clock_wire(name: &str) -> bool {
    for stem in [
        "G_HPBX", "G_VPTX", "G_HPRX", "L_HPBX", "R_HPBX", "G_HPFE", "G_HPFW", "G_VPFN", "G_VPFS",
        "G_CLKI_", "G_CLKO_",
    ] {
        if name.starts_with(stem) {
            return true;
        }
    }
    name.starts_with("G_")
        && (name.contains("PCLK") || name.contains("DCC") || name.contains("DCS"))
}

/// Where one lookup table's bits are, relative to the position it sits at.
///
/// A LUT is the one cell on this family whose configuration is a *value*
/// rather than a choice, so it cannot be a bel's
/// [`ConfigEntry`](super::arch::ConfigEntry) list the way a 7-series
/// flip-flop's `INIT` is: which bits to write depends on which of its
/// inputs the router actually reached. See
/// [`TrellisFabric::configure_logic`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LutBits {
    /// One entry per bit of the truth table, bit 0 first: the bits to set
    /// when that bit of `INIT` is a **zero**.
    ///
    /// Project Trellis stores the word inverted — `.config SLICEA.K0.INIT`
    /// defaults to sixteen ones and every group is a single `!F<n>B<n>` —
    /// so on this family this list holds the whole of it and
    /// [`LutBits::init_one`] is empty. Both are kept because the file could
    /// say otherwise and a reader should not have to know which.
    pub init_zero: Vec<Vec<ConfigBit>>,
    /// The bits to set when a bit of `INIT` is a **one**.
    pub init_one: Vec<Vec<ConfigBit>>,
    /// Per input 0 to 3: the bits of `SLICE<l>.<X><n>MUX = 1`, which tie
    /// that input high. Lattice's own packer writes them for every input a
    /// design leaves unconnected, and the field offers no `0`, which is why
    /// [`TrellisFabric::configure_logic`] folds the constant into the truth
    /// table instead of trying to tie it.
    pub tie_high: Vec<Vec<ConfigBit>>,
}

/// What the flow multiplies a clock-network node's cost by, so a clock goes
/// on the network rather than through the shortest run of data wires.
///
/// Measured rather than chosen: the general path from ball A8 to a
/// `CLK0_SLICE` of this die is seven hops and the path through the network
/// is about eighteen, so anything above about a tenth loses. See
/// [`TrellisFabric::clock_node_costs`].
pub const CLOCK_PREFERENCE: f32 = 0.05;

/// The Lattice tile type that holds a position's logic.
pub const LOGIC_TILE: &str = "PLC2";

/// How many lookup tables one [`LOGIC_TILE`] offers: four slices of two.
pub const LUTS_PER_TILE: usize = 8;

/// The letters Project Trellis gives a lookup table's four inputs, in the
/// order Reticle's `i0`..`i3` pin roles take.
pub const LUT_INPUTS: [char; 4] = ['A', 'B', 'C', 'D'];

/// The parameter a `LUT4` carries its truth table in, as the device file
/// spells it.
pub const LUT_INIT: &str = "INIT";

/// Where a flip-flop's five pins reach, as `(role, wire)` with `#` for the
/// flop's index in the tile and `@` for its slice's control set.
///
/// `bits.db` names wires and the bits that join them; it does not say that
/// `M3_SLICE` is a flip-flop's data input. That mapping is `libtrellis`'
/// own `Bels.cpp`, and `sites::bels_for` is where it is written out with
/// its provenance. What makes it checkable rather than believed is that the
/// bel is not declared unless the tile type owns every one of these names.
///
/// **`d` is `M<z>_SLICE` and not the lookup table's output**, which is the
/// whole reason a flop places on its own here; see the flip-flop section of
/// `TrellisDatabase::load`.
pub const FF_PINS: [(&str, &str); 5] = [
    ("d", "M#_SLICE"),
    ("clk", "CLK@_SLICE"),
    ("rst", "LSR@_SLICE"),
    ("en", "CE@_SLICE"),
    ("q", "Q#_SLICE"),
];

/// Where one flip-flop's settings are, relative to the position it sits at.
///
/// Every field here is what nextpnr's `write_ff` writes for a
/// `TRELLIS_FF`, and nothing else: that function is five `add_enum` calls
/// plus two conditional pairs, and [`TrellisFabric::configure_registers`]
/// is a line-for-line answer to it. A value that is the field's own default
/// costs no bits and its entry is empty, which is why all of them are kept
/// rather than only the ones a design turns out to need — an empty vector
/// is the honest record of "this costs nothing", and a missing entry would
/// be indistinguishable from a field the database does not have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FfBits {
    /// Which of the tile's two control sets this flop's slice uses, which
    /// is what `CLK<c>.CLKMUX` and `LSR<c>.LSRMUX` are indexed by once the
    /// routing says which of `CLK0`/`CLK1` carries the net.
    pub control: u32,
    /// `SLICE<l>.REG<n>.SD = 0`: read the data from the fabric's `M` wire
    /// rather than from the lookup table beside it. Always written, because
    /// this flow never packs the two together.
    pub sd_fabric: Vec<ConfigBit>,
    /// `SLICE<l>.REG<n>.REGSET`, as `[RESET, SET]`.
    pub regset: [Vec<ConfigBit>; 2],
    /// `SLICE<l>.REG<n>.LSRMODE = LSR`.
    pub lsrmode_lsr: Vec<ConfigBit>,
    /// `SLICE<l>.GSR`, as `[ENABLED, DISABLED]`.
    pub gsr: [Vec<ConfigBit>; 2],
    /// `SLICE<l>.CEMUX`, as `[1, CE]`.
    pub cemux: [Vec<ConfigBit>; 2],
    /// `CLK<c>.CLKMUX = INV`, for `c` 0 and 1. The `CLK` value is the
    /// default and costs nothing.
    pub clkmux_inv: [Vec<ConfigBit>; 2],
    /// `LSR<c>.LSRMUX = INV`, for `c` 0 and 1.
    pub lsrmux_inv: [Vec<ConfigBit>; 2],
    /// `LSR<c>.SRMODE = ASYNC`, for `c` 0 and 1.
    pub srmode_async: [Vec<ConfigBit>; 2],
}

/// The bel name a logic tile's distributed RAM is placed on.
///
/// One per `PLC2`, and it is not a slice: it is three of them. See
/// [`DPRAM_PINS`] for why, and [`DPRAM_BLOCKS`] for what it costs.
pub const DPRAM_BEL: &str = "DPR16X4";

/// The four lookup tables whose `INIT` words **are** a distributed RAM's
/// contents, one per bit of the four-bit word, in bit order.
///
/// `WD0A_SLICE` takes `WD0` and `WD0B_SLICE` takes `WD2`, so slice A holds
/// bits 0 and 1 and slice B holds bits 2 and 3. That is a `.fixed_conn` of
/// `PLC2`'s own `bits.db` and not a guess.
pub const DPRAM_DATA_LUTS: [&str; 4] = ["SLICEA.K0", "SLICEA.K1", "SLICEB.K0", "SLICEB.K1"];

/// The two lookup tables the `RAMW` slice gives up, whose `INIT` words
/// Lattice's own packer writes as sixteen zeros each.
///
/// nextpnr calls the cells that hold these down `RAMW_BLOCK` and writes
/// nothing for them; `ecp5/bitstream.cc` writes the two words when it
/// writes the `TRELLIS_RAMW` cell itself.
pub const DPRAM_RAMW_LUTS: [&str; 2] = ["SLICEC.K0", "SLICEC.K1"];

/// The lookup-table bels a distributed RAM makes unusable: six of the
/// eight a logic tile has.
///
/// Slices A and B *are* the RAM and slice C *is* its write-port register,
/// which is one bit of the tile for all three ([`DpRamBits::mode`]).
/// Slice D is untouched and still holds two lookup tables and two
/// flip-flops, which is what nextpnr's `pack_dram` leaves free too.
pub const DPRAM_BLOCKS: [&str; 6] = [
    "SLICEA.K0",
    "SLICEA.K1",
    "SLICEB.K0",
    "SLICEB.K1",
    "SLICEC.K0",
    "SLICEC.K1",
];

/// The `(pin role, wire)` table of a `TRELLIS_DPR16X4`, in the tile's own
/// spelling, with a role repeated once per wire it reaches.
///
/// Every line of this is nextpnr's `ecp5/cells.cc` — `dram_to_comb` and
/// `dram_to_ramw_split` — cross-checked against the `.fixed_conn` records
/// of `PLC2`'s `bits.db`, which is the only place the intra-tile joins
/// exist at all. The two halves:
///
/// - **the read port** is four lookup tables reading one address, so each
///   of `raddr0`..`raddr3` names **four** wires. The order is the
///   scrambled one the silicon has: `RAD[0]` is the `D` input, `RAD[1]`
///   the `B`, `RAD[2]` the `C` and `RAD[3]` the `A`. Getting that wrong
///   costs nothing a structural check would notice and every word would
///   be read from the wrong address; [`dpram_init_word`] is the same
///   permutation applied to the contents.
/// - **the write port** is the `RAMW` slice's two lookup tables used as
///   registers: `WAD[0..3]` are its first table's `D`, `B`, `C`, `A`
///   inputs and `DI[0..3]` its second table's `C`, `A`, `D`, `B`. Its
///   outputs are not pins — `WADO<n>C_SLICE` and `WDO<n>C_SLICE` reach
///   slices A and B over `.fixed_conn`s inside the tile, which is why a
///   distributed RAM has no routable wire between its halves.
///
/// `wclk` and `we` are two wires each because slices A and B have one
/// apiece, and both are fixed to the tile's `CLK1` and `LSR1` nets. That
/// is the other half of the relationship: a distributed RAM **commandeers
/// `CLK1` and `LSR1`**, and any flip-flop of the tile that wants a
/// different clock or reset has to take `CLK0`/`LSR0` instead.
pub const DPRAM_PINS: [(&str, &str); 32] = [
    // The read address, on all four of the RAM's lookup tables.
    ("raddr0", "D0_SLICE"),
    ("raddr0", "D1_SLICE"),
    ("raddr0", "D2_SLICE"),
    ("raddr0", "D3_SLICE"),
    ("raddr1", "B0_SLICE"),
    ("raddr1", "B1_SLICE"),
    ("raddr1", "B2_SLICE"),
    ("raddr1", "B3_SLICE"),
    ("raddr2", "C0_SLICE"),
    ("raddr2", "C1_SLICE"),
    ("raddr2", "C2_SLICE"),
    ("raddr2", "C3_SLICE"),
    ("raddr3", "A0_SLICE"),
    ("raddr3", "A1_SLICE"),
    ("raddr3", "A2_SLICE"),
    ("raddr3", "A3_SLICE"),
    // The read data, one lookup table output per bit.
    ("dout0", "F0_SLICE"),
    ("dout1", "F1_SLICE"),
    ("dout2", "F2_SLICE"),
    ("dout3", "F3_SLICE"),
    // The write address, on the `RAMW` slice's first lookup table.
    ("waddr0", "D4_SLICE"),
    ("waddr1", "B4_SLICE"),
    ("waddr2", "C4_SLICE"),
    ("waddr3", "A4_SLICE"),
    // The write data, on its second.
    ("din0", "C5_SLICE"),
    ("din1", "A5_SLICE"),
    ("din2", "D5_SLICE"),
    ("din3", "B5_SLICE"),
    // The write clock and the write enable, one wire per RAM slice.
    ("wclk", "WCK0_SLICE"),
    ("wclk", "WCK1_SLICE"),
    ("we", "WRE0_SLICE"),
    ("we", "WRE1_SLICE"),
];

/// How many words a `TRELLIS_DPR16X4` holds, as a number of address bits.
pub const DPRAM_ADDR_BITS: u32 = 4;

/// The cell parameter a distributed RAM's initial contents would arrive
/// in, word 0 in the lowest four bits.
///
/// `src/fpga/devices/ecp5.dev` declares no such parameter and
/// `primitives::Mapper::lower_memory` refuses a memory with initial
/// contents, so nothing in this flow sets it; a hand-written netlist can.
pub const DPRAM_INIT: &str = "INITVAL";

/// The three `MODE` settings a distributed RAM needs, which are **one bit**
/// of a `PLC2`.
pub const DPRAM_MODES: [(&str, &str); 3] = [
    ("SLICEA.MODE", "DPRAM"),
    ("SLICEB.MODE", "DPRAM"),
    ("SLICEC.MODE", "RAMW"),
];

/// A memory address as the lookup table holding it indexes its own truth
/// table.
///
/// The read address does not arrive on the inputs in order — `RAD[0]` is
/// the `D` input and `RAD[3]` the `A`, see [`DPRAM_PINS`] — so word
/// `address` of the RAM is bit `dpram_init_word(address)` of the `INIT`
/// word. This is nextpnr's `dram_to_comb` permutation written the other
/// way round, and it is its own inverse.
#[must_use]
pub fn dpram_init_word(address: usize) -> usize {
    // bit 0 of the address is `D`, which is input 3; bit 3 is `A`, which is
    // input 0; bits 1 and 2 are `B` and `C` and stay where they are.
    (address & 1) << 3 | (address & 2) | (address & 4) | (address & 8) >> 3
}

/// Where one logic tile's distributed RAM keeps its bits.
///
/// The contents and the `RAMW` slice's two zeroed words are looked up in
/// [`TrellisFabric::luts`] by the bel names in [`DPRAM_DATA_LUTS`] and
/// [`DPRAM_RAMW_LUTS`], because they are the very same `INIT` words a
/// lookup table would use — that a slice in `DPRAM` mode *is* its lookup
/// tables is the whole fact being modelled. What is left over is the mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DpRamBits {
    /// The bits that put slices A and B into `DPRAM` and slice C into
    /// `RAMW`, de-duplicated.
    ///
    /// On this die it is **one bit**, `F50B11`, because Project Trellis'
    /// `PLC2` database gives all three settings that same bit. A test
    /// asserts that rather than assuming it, since a die whose three
    /// settings were three bits would still be configured correctly by
    /// setting all of them.
    pub mode: Vec<ConfigBit>,
}

// ---------------------------------------------------------------------------
// The block RAM
// ---------------------------------------------------------------------------

/// How many address pins each port of a `DP16KD` has.
///
/// Fourteen, because the array is addressed in units of its **narrowest**
/// mode: 16384 words of one bit. A wider mode leaves the low pins out and
/// `src/fpga/devices/ecp5.dev`'s `addr` clause says how many.
pub const BRAM_ADDR_PINS: u32 = 14;

/// How many data pins each port of a `DP16KD` has, in each direction.
///
/// Eighteen: 16 kbit of data and two parity bits, which the narrow modes
/// skip.
pub const BRAM_DATA_PINS: u32 = 18;

/// The letter Lattice gives each of a `DP16KD`'s two ports, in the order
/// `src/fpga/devices/ecp5.dev` lists its `port rw` lines.
///
/// That order is what [`super::place`]'s `describe` turns into the `p0_`
/// and `p1_` prefixes of a block RAM's pin roles, so it has to agree with
/// the device file and a test asserts that it does.
///
/// **`B` comes first**, and the device file says why at length: the mapper
/// gives a read port the earliest port line, so listing B first reads on
/// port B and writes on port A, which is what `ecppack` does — and the
/// alternative cannot be configured, because the database's `EBR<n>.MODE`
/// record claims `WEAMUX`'s bit as one `DP16KD` wants clear.
pub const BRAM_PORTS: [char; 2] = ['B', 'A'];

/// The prefix and suffix an EBR's own pin wires carry in a `bits.db`.
///
/// A `PLC2` spells a slice's pins `A0_SLICE`; an EBR tile spells a block
/// RAM's `JADA0_EBR`. The `J` is Project Trellis' mark for a wire that
/// only ever joins a bel to the interconnect, and the `_EBR` is what makes
/// a tile recognisable as one that **owns** a block RAM rather than one
/// that merely holds some of its bits — which is the distinction the whole
/// model turns on, since nine tile types hold the bits of four blocks.
pub const BRAM_WIRE: (&str, &str) = ("J", "_EBR");

/// The field that puts an EBR into true dual-port mode, and the value.
///
/// `PDPW16KD` (pseudo dual port, 36 bits wide on one side) is the other
/// thing an EBR can be and this flow does not build one.
pub const BRAM_MODE: (&str, &str) = ("MODE", "DP16KD");

/// The two fields carrying each port's width, indexed the way
/// [`BRAM_PORTS`] is.
pub const BRAM_WIDTH: [&str; 2] = ["DP16KD.DATA_WIDTH_A", "DP16KD.DATA_WIDTH_B"];

/// The two fields carrying each port's read-during-write behaviour, and
/// the value `ecppack` writes for all 53 block RAMs of this board's own
/// bitstreams.
///
/// It is a don't-care for what this flow builds — a port of a block this
/// flow emits either reads or writes, never both — so matching the vendor
/// is the only reason to prefer one, and it is reason enough.
pub const BRAM_WRITEMODE: ([&str; 2], &str) = (
    ["DP16KD.WRITEMODE_A", "DP16KD.WRITEMODE_B"],
    "READBEFOREWRITE",
);

/// `EBR<n>.GSR`, and the value that keeps the global set/reset out of a
/// memory's output registers. `ecppack` writes it for every block.
pub const BRAM_GSR: (&str, &str) = ("GSR", "DISABLED");

/// The two reset fields and the value `ecppack` writes for both.
pub const BRAM_RESET: ([&str; 2], &str) = (["RESETMODE", "ASYNC_RESET_RELEASE"], "ASYNC");

/// The inverting-mux fields of the two pins a block RAM needs held **low**
/// when nothing drives them, per port, and the value that does it.
///
/// An unrouted input of this fabric reads as a **one** — the hazard
/// `docs/fpga-trellis.md` opens with — so a reset and an unused write
/// enable have to be inverted rather than left alone. That is not a
/// reading: `ecppack` writes `RSTAMUX = INV` and `RSTBMUX = INV` for every
/// one of the 53 block RAMs in this board's bitstreams and `WEBMUX = INV`
/// for every one whose B port does not write, while leaving `CEAMUX`,
/// `CEBMUX`, `OCEAMUX` and `OCEBMUX` alone — and those are exactly the
/// pins that want a **one** when nothing drives them.
pub const BRAM_TIE_LOW: [&str; 2] = ["RST#MUX", "WE#MUX"];

/// The pin roles [`BRAM_TIE_LOW`]'s fields belong to, in the same order.
pub const BRAM_TIE_LOW_PINS: [&str; 2] = ["rst", "we"];

/// The value an inverting mux takes.
pub const BRAM_INV: &str = "INV";

/// The multi-bit field naming which block-RAM initialisation block of the
/// bitstream belongs to this EBR.
///
/// A block RAM's contents are **not in the configuration memory at all**:
/// they arrive as their own `LSC_EBR_ADDRESS`/`LSC_EBR_WRITE` commands
/// (see [`super::ecp5`]), and this nine-bit word is what ties a block to
/// its data. Row 0 of the `.config` record is the **most significant**
/// bit, which is measured rather than assumed — see
/// `what_lattices_own_packer_writes_for_a_block_ram`, which reads the
/// nine bits of all 53 blocks out of `analyzer.bit` and `facedancer.bit`
/// and finds exactly the set of initialisation-block indices those files
/// carry.
pub const BRAM_WID: &str = "WID";

/// How many bits [`BRAM_WID`] has.
pub const BRAM_WID_BITS: u32 = 9;

/// The first identifier this flow gives a block RAM, which is the first
/// one `ecppack` gives.
///
/// Measured: `analyzer.bit` has nine block RAMs and its initialisation
/// blocks are 3 to 11, `facedancer.bit` has forty-four and its are 3 to
/// 46. nextpnr's `pack_ebr` says the same thing in a comment and this
/// agrees with it.
pub const BRAM_FIRST_WID: u32 = 3;

/// How many tiles east of its own an EBR's configuration reaches.
///
/// Two, so three positions in all. This is **not** by analogy with
/// anything: the EBR row of this die is nine tile types repeating
/// (`MIB_EBR0`..`MIB_EBR8`) and they hold four blocks between them, so a
/// block's fields are spread over its own tile and the two east of it and
/// a field's bits can straddle the boundary. `EBR1.DP16KD.DATA_WIDTH_B`
/// is three bits of `MIB_EBR2` and a fourth of `MIB_EBR4`, two tiles
/// away, and without that fourth bit the 9-bit and 18-bit modes are
/// indistinguishable.
pub const BRAM_SPAN: u32 = 3;

/// The `(pin role, Lattice pin)` table of a `DP16KD`.
///
/// Only the names are written down; **which wire each pin reaches is read
/// out of the tile's own `.fixed_conn` records**, which is the difference
/// [`sites`]'s header is about. `JADA0_EBR <- JC4` says the first address
/// pin of port A is the interconnect's `JC4`, and `JF0 <- JDOA0_EBR` says
/// the first read-data pin drives `JF0`; this table only has to know that
/// `ADA0` is `p0_addr0`.
///
/// The roles are `p<port>_<role>` because that is what
/// [`super::place`]'s `describe` builds from a `bram` line's `port rw`
/// entries — both ports call their clock `clk`, so the port index has to
/// be in the role.
///
/// `OCEA`/`OCEB` and `CSA0`..`CSB2` are deliberately **absent**. They are
/// six more wires the tile joins, and every one of them wants a **one**,
/// which is what an unrouted wire of this fabric already gives: `ecppack`
/// leaves all six unrouted in all 53 blocks and writes
/// `CSDECODE_A = CSDECODE_B = 111` — that is, no bits at all — to match.
#[must_use]
pub fn bram_pins() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (port, letter) in BRAM_PORTS.iter().enumerate() {
        out.push((format!("p{port}_clk"), format!("CLK{letter}")));
        out.push((format!("p{port}_en"), format!("CE{letter}")));
        out.push((format!("p{port}_we"), format!("WE{letter}")));
        out.push((format!("p{port}_rst"), format!("RST{letter}")));
        for bit in 0..BRAM_ADDR_PINS {
            out.push((format!("p{port}_addr{bit}"), format!("AD{letter}{bit}")));
        }
        for bit in 0..BRAM_DATA_PINS {
            out.push((format!("p{port}_din{bit}"), format!("DI{letter}{bit}")));
            out.push((format!("p{port}_dout{bit}"), format!("DO{letter}{bit}")));
        }
    }
    out
}

/// One bit of a block RAM's configuration: the **position** it belongs to
/// and the bit inside that position's combined frame numbering.
///
/// Every other feature of this backend writes into one position and so
/// needs no such pair. A block RAM is the first that does not: see
/// [`BRAM_SPAN`].
pub type PlacedBit = ((u32, u32), ConfigBit);

/// Where one block RAM's settings are.
///
/// Per **position**, not per tile type, which is the thing that makes this
/// unlike every other bel of this backend. A `DP16KD`'s fields live in its
/// own tile and the two east of it ([`BRAM_SPAN`]), those three positions
/// hold different compositions, and the compositions differ from block to
/// block: the easternmost tile of the fourth block of a group is
/// `MIB_EBR8` for fifty-two of this die's fifty-six blocks and an
/// `EBR_SPINE_*` or an `EBR_CMUX_*` for the other four. A frame offset
/// computed for one of those is wrong for the other, so there is nothing a
/// tile type could usefully hold.
///
/// Every field of every one of the three tiles is kept, rather than only
/// the ones this flow writes. That is on purpose: it is what lets
/// `what_lattices_own_packer_writes_for_a_block_ram` ask what `ecppack`
/// wrote for a field this flow leaves alone, which is how the rows about
/// `CSDECODE`, `CEAMUX` and `REGMODE` in `docs/fpga-trellis.md` were
/// answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramSite {
    /// The position of the tile that owns the block's **wires**, which is
    /// where its bel is.
    pub at: (u32, u32),
    /// The bel's name, `EBR<n>` for the block's index inside its group.
    pub bel: String,
    /// Which of its group's four blocks this is, which is what names
    /// every one of its fields.
    pub index: u32,
    /// Every enumerated field the three tiles declare for this block, as
    /// `(field, value, bits)` with the `EBR<n>.` prefix stripped. Bits a
    /// value wants **clear** are dropped, for the reason
    /// [`TrellisDatabase::locate_field`] gives.
    pub enums: Vec<(String, String, Vec<PlacedBit>)>,
    /// Every multi-bit field, as `(field, groups)`, one group per bit **in
    /// the order the database lists them** — which for [`BRAM_WID`] is
    /// most significant first.
    pub words: Vec<(String, Vec<Vec<PlacedBit>>)>,
}

impl BramSite {
    /// The bits one value of one enumerated field needs set.
    #[must_use]
    pub fn enum_bits(&self, field: &str, value: &str) -> Option<&[PlacedBit]> {
        self.enums
            .iter()
            .find(|(f, v, _)| f == field && v == value)
            .map(|(_, _, bits)| bits.as_slice())
    }

    /// True when the three tiles declare that field at all.
    #[must_use]
    pub fn has_enum(&self, field: &str) -> bool {
        self.enums.iter().any(|(f, _, _)| f == field)
    }

    /// The bit groups of a multi-bit field, in database order.
    #[must_use]
    pub fn word(&self, field: &str) -> Option<&[Vec<PlacedBit>]> {
        self.words
            .iter()
            .find(|(f, _)| f == field)
            .map(|(_, groups)| groups.as_slice())
    }
}

/// How many bits one word of a block RAM's initialisation stream holds.
///
/// Nine, not eight: the array is 2048 words of nine bits — 16 kbit of data
/// and 2 kbit of parity — and the stream's own packing is nine words to
/// nine bytes ([`super::ecp5::BRAM_WORDS`] and `pack_bram_row`). So a
/// **row** of the contents, which is 18 bits, is two of these.
pub const BRAM_INIT_WORD_BITS: u32 = 9;

/// What [`TrellisFabric::configure_bram`] produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BramConfig {
    /// One initialisation block per block RAM, in the order the blocks were
    /// given their [`BRAM_WID`]s. Written even when the contents are all
    /// zero, because `ecppack` writes one for every block too.
    pub blocks: Vec<super::ecp5::BramBlock>,
    /// Pairs of block RAMs that cannot both be used, as
    /// `X<x>Y<y> is 18 bits wide and X<x>Y<y> is in use`.
    ///
    /// See [`TrellisFabric::configure_bram`]: the two top data bits of each
    /// of a block's ports are the same interconnect wires as the two bottom
    /// bits of the block two columns east. A non-empty list means nothing
    /// was written and the caller must say so.
    pub overlaps: Vec<String>,
}

/// One block RAM's initialisation stream, from the `INITVAL_<nn>`
/// parameters the technology mapper put on its cell.
///
/// The layout is the device file's — `init_params` says how many
/// parameters there are, how many rows each holds and how wide a row's
/// slot is — and the only thing added here is that a row is **two** words
/// of the stream, the low one in its bottom [`BRAM_INIT_WORD_BITS`]. That
/// last step is what no vendor bitstream in this tree can confirm, because
/// all 53 of theirs are zero; [`TrellisFabric::configure_bram`] says what
/// it rests on instead.
#[must_use]
pub fn bram_init_words(
    device: &super::Device,
    primitive: &str,
    params: Option<&crate::ir::Attrs>,
) -> Vec<u16> {
    let mut words = vec![0u16; super::ecp5::BRAM_WORDS];
    let Some(shape) = device.block_rams.iter().find(|s| s.name == primitive) else {
        return words;
    };
    let Some(init) = shape.init_params.as_ref() else {
        return words;
    };
    for index in 0..init.count {
        let value = params.and_then(|p| p.get(&init.name(index)));
        if value.is_none() {
            continue;
        }
        for row in 0..init.rows {
            let at_row = u64::from(index) * u64::from(init.rows) + u64::from(row);
            let Ok(first) = usize::try_from(at_row * 2) else {
                continue;
            };
            for half in 0..2u32 {
                let mut word = 0u16;
                for bit in 0..BRAM_INIT_WORD_BITS {
                    let at = row * init.slot + half * BRAM_INIT_WORD_BITS + bit;
                    if super::bitstream::param_bit(value, at) {
                        word |= 1 << bit;
                    }
                }
                if let Some(slot) = words.get_mut(first + half as usize) {
                    *slot = word;
                }
            }
        }
    }
    words
}

/// The letter Project Trellis names slice `index` with.
fn slice_letter(index: usize) -> char {
    ['A', 'B', 'C', 'D'][index & 3]
}

/// One position's Lattice tiles with where each starts in that position's
/// combined frame numbering, and how many frames they come to in all.
type FrameWindows = (Vec<(String, u32)>, u32);

/// The composition key of a position, borrowed from the map that owns it.
fn composition_key<'a>(type_of: &'a BTreeMap<String, usize>, list: &[String]) -> Option<&'a str> {
    let name = list.join("+");
    type_of
        .get_key_value(name.as_str())
        .map(|(key, _)| key.as_str())
}

/// One wire name of a `bits.db`, as a reference an [`Arch`] pip can carry,
/// or `None` for a name that belongs to another die.
fn wire_ref(name: &str, prefix: &str, globals: &BTreeSet<&str>) -> Option<super::arch::WireRef> {
    match parse::globalise_ref(name, prefix)? {
        parse::WireTargetRef::Global { name } => globals
            .contains(name)
            .then(|| super::arch::WireRef::global(name)),
        parse::WireTargetRef::Tile { dx, dy, name } => Some(super::arch::WireRef::at(name, dx, dy)),
    }
}

/// The `CIB` field that ties a top-edge PIO's output **data** wire.
///
/// It is **not read by the loader any more**, which is the point of keeping
/// it: [`TrellisDatabase::cib_mux`] derives the field from the buffer's own
/// fixed connections, and this is what that derivation produces for the top
/// edge. `PIOT0`'s fixed connections are `JPADDOA <- S1_JA0` and `JPADDOB <-
/// S1E1_JA0`, so each side reads the `JA0` of the tile one row south of its
/// own pad tile. The right edge's answer is `CIB.JA0MUX` for sides A and B
/// and `CIB.JA3MUX` for C and D, in a tile one column *west*, which is why
/// the name could not stay a constant.
pub const DATA_MUX: &str = "CIB.JA0MUX";

/// The `CIB` field that ties a top-edge PIO's output **enable** wire.
///
/// `JB0`, by the same two fixed connections one letter along: `JPADDTA <-
/// S1_JB0` and `JPADDTB <- S1E1_JB0`. Also derived rather than read; see
/// [`DATA_MUX`].
pub const ENABLE_MUX: &str = "CIB.JB0MUX";

/// The `BANKREF` field that says what an IO bank's VCCIO rail is.
///
/// This is the one setting a pad needs that lives in **neither** of its own
/// two positions: a bank's reference tile sits at the end of its edge —
/// `BANKREF1` of this die is the single tile at grid (69, 0), forty columns
/// from some of the pads it serves. Lattice's own packer writes it for
/// every bank that holds an IO (nextpnr's `bitstream.cc`,
/// `init_io_banks`), and all three of Great Scott Gadgets' bitstreams for
/// a Cynthion r1.4 set it to `3V3` on every bank of the part.
pub const BANK_VCCIO: &str = "BANK.VCCIO";

/// The `BANKREF` value an IO standard implies, or `None` for a standard
/// whose rail this does not know.
///
/// The table is nextpnr's `get_vccio` (`ecp5/pio.cc`), restricted to the
/// single-ended standards a pad here can be given. `SSTL135` is the one
/// oddity and it is Lattice's, not nextpnr's: a 1.35 V bank is programmed
/// as `1V2`, which is why it is spelled that way below.
#[must_use]
pub fn bank_voltage(standard: &str) -> Option<&'static str> {
    Some(match standard {
        "LVCMOS33" | "LVTTL33" => "3V3",
        "LVCMOS25" => "2V5",
        "LVCMOS18" | "SSTL18_I" | "SSTL18_II" => "1V8",
        "LVCMOS15" | "SSTL15_I" | "SSTL15_II" => "1V5",
        "LVCMOS12" | "HSUL12" | "SSTL135_I" | "SSTL135_II" => "1V2",
        _ => return None,
    })
}

/// The standards [`bank_voltage`] has a rail for, for an error message.
fn bank_voltage_standards() -> Vec<String> {
    [
        "LVCMOS33",
        "LVTTL33",
        "LVCMOS25",
        "LVCMOS18",
        "LVCMOS15",
        "LVCMOS12",
        "HSUL12",
        "SSTL18_I",
        "SSTL18_II",
        "SSTL15_I",
        "SSTL15_II",
        "SSTL135_I",
        "SSTL135_II",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// The value of [`DATA_MUX`] and [`ENABLE_MUX`] that drives a fixed zero.
pub const TIE_LOW: &str = "0";

/// The value that drives a fixed one.
pub const TIE_HIGH: &str = "1";

/// `PIO<side>.PULLMODE = NONE`, and **the reason it has to be written**.
///
/// The field's default in `bits.db` is `DOWN`: with none of its bits set an
/// ECP5's input has an internal pull-down, which is Lattice's own default
/// for an unconfigured PIO. That is fine for an output and wrong for an
/// input whose board pulls it the other way. A Cynthion's USER button is
/// exactly that case — a 10 k pull-up to 3.3 V, a switch to ground, and a
/// 33 k series resistor into the ball — and an internal pull-down of the
/// order the datasheet gives would divide the released level down to well
/// under `VIH`, so the pin would read low whether or not anybody pressed
/// anything. Great Scott Gadgets' own platform file asks for `PULLMODE=NONE`
/// on that pin, `facedancer.bit` has it, and so does every input this
/// writes.
///
/// It is the default here for a **bidirectional** pad too, and there the
/// argument is stronger rather than weaker. A pad that is released to high
/// impedance has nothing but the pull deciding what it reads back; an
/// internal pull-down fights whatever the board does and does it only half
/// the time, which is the kind of fault that looks like a broken input
/// path. So the pull is always stated, and a design that wants the pin held
/// somewhere says so: `set_io -pullup yes` gives [`PULL_UP`].
pub const PULL_NONE: &str = "NONE";

/// `PIO<side>.PULLMODE = UP`, the internal pull-up.
///
/// Two bits on this family where [`PULL_NONE`] is one, and it is what
/// `set_io -pullup yes` asks for: the `.dev` file's `param_pullup
/// PULLMODE="UP"` clause puts it on the cell and
/// [`TrellisFabric::configure_io`] reads it back off there.
///
/// This is the setting that decides what a released bidirectional pad
/// reads, so on a pin with nothing but a trace on it, it decides the whole
/// observation. `testdata/fpga/cynthion/bidir_loopback.v` is that design.
pub const PULL_UP: &str = "UP";

/// Every value `PIO<side>.PULLMODE` takes, in `bits.db`'s order.
///
/// `DOWN` is the field's default and its pattern is two bits it wants
/// *clear*, so it locates to nothing at all — which is exactly why leaving
/// the field alone is not the same as not having a pull.
pub const PULL_MODES: [&str; 3] = ["DOWN", PULL_NONE, PULL_UP];

/// The cell parameter that names a pad's pull mode, which is where
/// `set_io -pullup` ends up: `param_pullup PULLMODE="UP"` on the `io` line
/// of `src/fpga/devices/ecp5.dev`.
pub const PULL_PARAM: &str = "PULLMODE";

/// `PIO<side>.HYSTERESIS = ON`, which Lattice's own packer writes for every
/// single-ended input **and every single-ended bidirectional pad**
/// (`write_io` in nextpnr's `ecp5/bitstream.cc`, whose test is `dir ==
/// "INPUT" || dir == "BIDIR"` and whose default for the attribute is `ON`)
/// and for no output.
pub const HYSTERESIS_ON: &str = "ON";

/// Every value `PIO<side>.SLEWRATE` takes, in `bits.db`'s order.
///
/// `SLOW` is the field's default and its pattern is the one bit it wants
/// *clear*, so it locates to nothing; `FAST` is that bit set. One bit, in
/// the pad tile, per PIO.
pub const SLEW_RATES: [&str; 2] = ["FAST", "SLOW"];

/// The attribute that names a pad's slew rate, which is where `set_io
/// -slew` ends up, and **why it is written at all**.
///
/// `PIO<side>.SLEWRATE` is an output edge rate: the field's database
/// default is `SLOW`, and `FAST` is one bit in the pad tile. Nothing here
/// writes it unless a constraint asks, which is exactly nextpnr's rule —
/// `ecp5/bitstream.cc`'s `write_io` has
///
/// ```text
/// if (ci->attrs.count(id_SLEWRATE) && !is_referenced(ioType_from_str(iotype)))
///     cc.tiles[pio_tile].add_enum(pio + ".SLEWRATE", str_or_default(ci->attrs, id_SLEWRATE, "SLOW"));
/// ```
///
/// so an attribute is the whole condition, the tile is the pad tile (the
/// same one `HYSTERESIS` and `PULLMODE` go in), and the direction does not
/// come into it: an input, an output and a bidirectional pad are all
/// written the same way. `LVCMOS33` is not a referenced standard, so the
/// second half of that test is always true here.
///
/// **What asks for it.** Great Scott Gadgets' platform file gives every pin
/// of all three of a Cynthion's ULPI transceivers
/// `Attrs(IO_TYPE="LVCMOS33", SLEWRATE="FAST")`, and all three of their
/// reference bitstreams have the bit set on every one of those balls —
/// `tests/fpga_trellis.rs`'s
/// `what_lattices_own_packer_writes_for_a_bidirectional_pad` reads it back
/// out of `analyzer.bit` at absolute frame positions. Until this was
/// written it was the one attribute of that resource this backend dropped
/// on the floor: the constraint parsed, reached the cell, and then nothing
/// put it in a bitstream.
pub const SLEW_ATTR: &str = "slew";

impl TrellisDatabase {
    /// The [`ConfigBit`]s of one enumerated field at one grid position, in
    /// the position's combined numbering.
    ///
    /// The field, not the tile type, is what identifies which of a
    /// position's tiles holds it. That is deliberate: the edges of this die
    /// spell the same tile a dozen ways — `PICR1`, `PICR1_DQS0`,
    /// `PICR1_DQS3`, and `MIB_CIB_LR_A` where a `PICR2` would be — and
    /// nextpnr carries a set of type names per edge to cope
    /// (`get_pio_tile`'s `pioabcd_r`). Asking which tile declares
    /// `PIOD.BASE_TYPE` gets the same answer without the table, and it
    /// **refuses** rather than guessing if two tiles of one position both
    /// declare the field.
    ///
    /// The other half of this is the whole reason
    /// [`super::ecp5::Ecp5FrameMap`] exists. A `bits.db` bit is
    /// `F<frame>B<bit>` *within its own tile*, and a position may hold
    /// several tiles; so the frame index has the frame counts of the
    /// position's earlier windows added to it, and the result is a
    /// [`ConfigBit`] whose `row` [`super::ecp5::Ecp5FrameMap::locate`]
    /// resolves back.
    ///
    /// Bits the field wants **clear** are dropped, not recorded. A
    /// bitstream is assembled by setting bits in a zeroed bitmap, so
    /// "clear" is the state a bit is already in, and there is nothing to
    /// express. This is only safe while nothing else writes to the same
    /// tile; `docs/fpga-trellis.md` says what that costs and where the
    /// cost is confined to.
    ///
    /// # Errors
    ///
    /// [`TrellisError::NoSuchField`] when no tile of the position declares
    /// the field or none of them offers that value, and
    /// [`TrellisError::Malformed`] when two of them declare it.
    pub fn locate_field(
        &self,
        at: (u32, u32),
        field: &str,
        value: &str,
    ) -> Result<Vec<ConfigBit>, TrellisError> {
        let (offset, ty, db) = self.field_owner(at, field)?;
        let bits = db
            .enum_bits(field, value)
            .ok_or_else(|| TrellisError::NoSuchField {
                tile: ty.to_owned(),
                field: field.to_owned(),
                value: value.to_owned(),
            })?;
        Ok(bits
            .iter()
            .filter(|bit| !bit.inverted)
            .map(|bit| ConfigBit::new(offset + bit.frame, bit.bit))
            .collect())
    }

    /// Which tile of a position declares an enumerated field, with its
    /// frame offset inside the position.
    fn field_owner(
        &self,
        at: (u32, u32),
        field: &str,
    ) -> Result<(u32, &str, &TileDatabase), TrellisError> {
        let mut before = 0u32;
        let mut found: Option<(u32, &str, &TileDatabase)> = None;
        for tile in self.tiles.iter().filter(|t| t.col == at.0 && t.row == at.1) {
            if let Some(db) = self.types.get(&tile.ty)
                && db.has_enum(field)
            {
                if let Some((_, other, _)) = found {
                    return Err(TrellisError::Malformed {
                        path: format!("{}/tiledata/…/bits.db", self.device.family),
                        what: format!(
                            "`{other}` and `{}` are both at (col {}, row {}) and both declare \
                             `{field}`, so which one configures the pad is ambiguous",
                            tile.ty, at.0, at.1
                        ),
                    });
                }
                found = Some((before, tile.ty.as_str(), db));
            }
            before += tile.window.frames;
        }
        found.ok_or_else(|| TrellisError::NoSuchField {
            tile: format!("the tiles at (col {}, row {})", at.0, at.1),
            field: field.to_owned(),
            value: String::new(),
        })
    }

    /// Which `CIB` tile ties a buffer's wire, and under what field name.
    ///
    /// `sink` is `JPADDO<side>` or `JPADDT<side>`, a wire of the tile the
    /// buffer sits in. Its `.fixed_conn` names the interconnect wire that
    /// drives it, with a direction prefix that says which position that wire
    /// belongs to, and the field that ties a `CIB` wire is `CIB.<wire>MUX`.
    /// That is nextpnr's own algorithm — `write_io` walks the pips uphill of
    /// the wire and takes the `CIB` wire's name — and doing it this way is
    /// what lets one piece of code serve both edges, whose answers are
    /// `JA0` one row south-east and `JA3` one column west and two rows
    /// south.
    ///
    /// # Errors
    ///
    /// [`TrellisError::NoSuchField`] when no tile of the buffer's position
    /// has such a fixed connection, or the wire it names belongs to another
    /// die.
    pub fn cib_mux(
        &self,
        bel: (u32, u32),
        sink: &str,
    ) -> Result<((u32, u32), String), TrellisError> {
        let miss = || TrellisError::NoSuchField {
            tile: format!("the tiles at (col {}, row {})", bel.0, bel.1),
            field: format!("a fixed connection driving `{sink}`"),
            value: String::new(),
        };
        for tile in self
            .tiles
            .iter()
            .filter(|t| t.col == bel.0 && t.row == bel.1)
        {
            let Some(db) = self.types.get(&tile.ty) else {
                continue;
            };
            let Some((_, source)) = db.fixed.iter().find(|(to, _)| to == sink) else {
                continue;
            };
            let Some(parse::WireTargetRef::Tile { dx, dy, name }) =
                parse::globalise_ref(source, self.device.chip_prefix())
            else {
                return Err(miss());
            };
            let x = u32::try_from(i64::from(bel.0) + i64::from(dx)).map_err(|_| miss())?;
            let y = u32::try_from(i64::from(bel.1) + i64::from(dy)).map_err(|_| miss())?;
            return Ok(((x, y), format!("CIB.{name}MUX")));
        }
        Err(miss())
    }
}

/// What reading a bitstream back through the database found.
///
/// This is the inverse of everything else in this module, and it exists for
/// the reason `super::xray`'s `Decoded` does: so that a bitstream can be
/// checked against **the database's own account of itself** rather than
/// against a story about what it should contain. Every set bit is either
/// explained by a named feature or counted in
/// [`Decoded::unexplained`], and that accounting is the honest part — a
/// decoding that names forty features and leaves two thousand bits
/// unexplained has not understood the bitstream.
///
/// It is also the only check there is on an *arc*. A pad's bits can be
/// compared with what Lattice's own packer wrote for the same ball; the
/// reference bitstreams route different designs, so there is nothing to
/// compare a route against. What can be asked is whether the bits this
/// flow wrote select, through the same `.mux` records the router read, the
/// connections the router chose — and in particular whether some *other*
/// feature written into the same tile has quietly changed one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// One entry per programmable connection the bits select, as
    /// `(position, sink, source)` in the `bits.db` spelling. Sorted.
    ///
    /// A mux whose selected source has **no** bits is not reported: it is
    /// the state of an untouched bitstream everywhere, so reporting it
    /// would say nothing. That is also why a pip with no bits cannot be
    /// checked this way.
    pub arcs: Vec<((u32, u32), String, String)>,
    /// One entry per enumerated field that is **not** at its default, as
    /// `(position, field, value)`. Sorted.
    pub enums: Vec<((u32, u32), String, String)>,
    /// One entry per multi-bit field that is not at its default, as
    /// `(position, field, value)` with the value bit 0 first. Sorted.
    pub words: Vec<((u32, u32), String, String)>,
    /// How many bits the image sets in all.
    pub bits: usize,
    /// How many of those no feature accounts for.
    pub unexplained: usize,
    /// Which ones those are, as `(tile type, position, F<frame>B<bit>)` in
    /// the tile's **own** numbering — the numbering `bits.db` uses, so an
    /// entry can be grepped for directly. Sorted, and the same length as
    /// [`Decoded::unexplained`].
    ///
    /// A count on its own is not actionable: "eight bits belong to no
    /// feature" says a tile rule is wrong and not which. These say where to
    /// look, and naming them is how the one that this field was added for
    /// was found — `PIO<s>.TERMINATION_*`, a field whose `OFF` is the empty
    /// pattern, which therefore matches every image and covers nothing.
    pub leftovers: Vec<(String, (u32, u32), String)>,
    /// How many grid positions hold at least one set bit.
    pub tiles: usize,
}

impl Decoded {
    /// A report, one fact per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "decoded: {} bit(s) over {} position(s) into {} arc(s), {} field(s) and {} word(s); \
             {} bit(s) unexplained\n",
            self.bits,
            self.tiles,
            self.arcs.len(),
            self.enums.len(),
            self.words.len(),
            self.unexplained
        );
        for (at, sink, source) in &self.arcs {
            out.push_str(&format!("  X{}Y{} {sink} <- {source}\n", at.0, at.1));
        }
        for (at, field, value) in &self.enums {
            out.push_str(&format!("  X{}Y{} {field} = {value}\n", at.0, at.1));
        }
        for (at, field, value) in &self.words {
            out.push_str(&format!("  X{}Y{} {field} = {value}\n", at.0, at.1));
        }
        for (ty, at, bit) in &self.leftovers {
            out.push_str(&format!(
                "  X{}Y{} {ty} {bit} belongs to no feature\n",
                at.0, at.1
            ));
        }
        out
    }
}

impl TrellisDatabase {
    /// The connections a decoding selects, in the same `(name, position)`
    /// pairs [`TrellisFabric::routed_arcs`] gives, so the two can be
    /// compared.
    ///
    /// A name the database spells for another die of the family resolves to
    /// nothing and is reported as such rather than skipped: a bit of this
    /// part that only makes sense on an 85F would mean the tile rules are
    /// wrong, not that there is nothing to say.
    #[must_use]
    pub fn resolved_arcs(
        &self,
        decoded: &Decoded,
    ) -> (BTreeSet<(ResolvedWire, ResolvedWire)>, Vec<String>) {
        let prefix = self.device.chip_prefix();
        let resolve = |at: (u32, u32), name: &str| -> Option<ResolvedWire> {
            match parse::globalise(name, prefix)? {
                parse::WireTarget::Global { name } => Some((name, (0, 0))),
                parse::WireTarget::Tile { dx, dy, name } => {
                    let x = u32::try_from(i64::from(at.0) + i64::from(dx)).ok()?;
                    let y = u32::try_from(i64::from(at.1) + i64::from(dy)).ok()?;
                    Some((name, (x, y)))
                }
            }
        };
        let mut out = BTreeSet::new();
        let mut problems = Vec::new();
        for (at, sink, source) in &decoded.arcs {
            match (resolve(*at, sink), resolve(*at, source)) {
                (Some(to), Some(from)) => {
                    out.insert((to, from));
                }
                _ => problems.push(format!(
                    "X{}Y{} `{sink}` <- `{source}` names a wire of another die",
                    at.0, at.1
                )),
            }
        }
        (out, problems)
    }

    /// Reads a configuration memory back into the database's own feature
    /// names; see [`Decoded`].
    ///
    /// A feature matches when every bit it wants set is set and every bit
    /// it wants clear is clear. Where several values of one field match,
    /// two rules decide between them, in this order:
    ///
    /// 1. **the reading that leaves fewest of the tile's bits
    ///    unexplained**, and
    /// 2. among those, the one with the most bits — which is what picks
    ///    `OUTPUT_LVCMOS33` over the `NONE` whose single bit it contains,
    ///    and is what `libtrellis`' own `Tile::get_config` does on its own.
    ///
    /// Rule 1 exists because a value's pattern can reach outside the field
    /// it belongs to. On the right edge four PIOs share one pad tile, and a
    /// *pseudo-differential* `PIO<s>.BASE_TYPE` — `OUTPUT_LVCMOS33D` is ten
    /// bits where `BIDIR_LVCMOS33` is eight — spells four of its bits in
    /// the **neighbouring** PIO's frames, because a differential pair needs
    /// both pads. With both halves of a pair bidirectional all ten are set,
    /// so the longest match alone reads side A back as a differential
    /// output it is not, and leaves the two bits only a bidirectional or an
    /// input pad wants belonging to nothing. Those four extra bits are
    /// explained by side B's own base type and pull mode either way, so
    /// they are no evidence about side A; the two orphans are.
    /// `docs/fpga-trellis.md`'s "What could not be read back" is the long
    /// version, including the measurement on `ecppack`'s own output.
    ///
    /// The rule is applied as a **fixed point** rather than as a ranking,
    /// because "how much does this reading leave unexplained" is a question
    /// about the whole tile and not about one field: every field and mux
    /// sink takes the longest match first, and then each in turn is allowed
    /// to change its reading for one that explains **strictly more** of the
    /// tile's set bits, until none will. Each change strictly raises the
    /// number of explained bits, which is bounded by the number of set
    /// bits, so this terminates. Two consequences are worth stating,
    /// because they are what makes the change a safe one to the most
    /// load-bearing check in this backend:
    ///
    /// - **a tile with nothing left over is never touched.** There is
    ///   nothing to improve, so the loop does not run and every reading is
    ///   the longest match, exactly as before. Every bitstream this flow
    ///   writes is of that kind — [`Decoded::unexplained`] being zero is
    ///   what `reticle fpga --bitstream` refuses to write without — so this
    ///   cannot change how any of them is read.
    /// - **it is not a licence to explain a bit twice, or to invent one.**
    ///   A reading is still only ever one of the values the image's bits
    ///   actually allow; the choice is between matches, never outside them.
    #[must_use]
    pub fn decode(&self, cram: &Cram) -> Decoded {
        /// One record whose reading the image's bits leave a choice about:
        /// a mux sink or an enumerated field, the values that match, and
        /// which of them is read back.
        struct Choice<'db> {
            /// A mux sink rather than an enumerated field. The two differ
            /// only in how they are reported.
            mux: bool,
            /// The sink's or the field's name.
            name: &'db str,
            /// The field's default, for an enumerated field that has one.
            default: Option<&'db str>,
            /// Every value whose pattern this image matches, in database
            /// order, as `(value, bits)`.
            candidates: Vec<(&'db str, &'db [parse::DbBit])>,
            /// Which of them is read back, as an index into `candidates`.
            chosen: usize,
        }

        /// The bits a pattern **accounts for**: the ones it wants set. A
        /// bit a feature wants clear is not evidence that the feature is
        /// there, so it explains nothing; `dropped_clear_bits` is the other
        /// half of that.
        fn explained(bits: &[parse::DbBit]) -> impl Iterator<Item = (u32, u32)> + '_ {
            bits.iter()
                .filter(|b| !b.inverted)
                .map(|b| (b.frame, b.bit))
        }

        /// The longest match, and the first of those in database order —
        /// rule 2 on its own.
        fn longest(candidates: &[(&str, &[parse::DbBit])]) -> usize {
            let mut best = 0;
            for (i, (_, bits)) in candidates.iter().enumerate() {
                if bits.len() > candidates[best].1.len() {
                    best = i;
                }
            }
            best
        }

        let mut out = Decoded {
            bits: cram.count_ones(),
            ..Decoded::default()
        };
        let mut positions: BTreeSet<(u32, u32)> = BTreeSet::new();
        for tile in &self.tiles {
            let Some(db) = self.types.get(&tile.ty) else {
                continue;
            };
            let at = (tile.col, tile.row);
            let w = tile.window;
            // The tile's own set bits, in its own coordinates.
            let mut ones: BTreeSet<(u32, u32)> = BTreeSet::new();
            for frame in 0..w.frames {
                for bit in 0..w.bits {
                    if cram.get(w.start_frame + frame, w.start_bit + bit) {
                        ones.insert((frame, bit));
                    }
                }
            }
            if ones.is_empty() {
                continue;
            }
            positions.insert(at);
            let matches = |bits: &[parse::DbBit]| -> bool {
                bits.iter()
                    .all(|b| ones.contains(&(b.frame, b.bit)) != b.inverted)
            };
            // How many readings account for each bit. Every bit a reading
            // accounts for is one it wants **set**, and it matched, so
            // every key here is a bit of `ones`.
            let mut covered: BTreeMap<(u32, u32), u32> = BTreeMap::new();

            // Rule 2, for every mux sink and every enumerated field.
            let mut choices: Vec<Choice<'_>> = Vec::new();
            let mut sink_seen: BTreeSet<&str> = BTreeSet::new();
            for (sink, _, _) in &db.muxes {
                if !sink_seen.insert(sink.as_str()) {
                    continue;
                }
                let candidates: Vec<(&str, &[parse::DbBit])> = db
                    .muxes
                    .iter()
                    .filter(|(other, _, bits)| other == sink && matches(bits))
                    .map(|(_, source, bits)| (source.as_str(), bits.as_slice()))
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let chosen = longest(&candidates);
                choices.push(Choice {
                    mux: true,
                    name: sink.as_str(),
                    default: None,
                    candidates,
                    chosen,
                });
            }
            for (field, default, values) in &db.enums {
                let candidates: Vec<(&str, &[parse::DbBit])> = values
                    .iter()
                    .filter(|(_, bits)| matches(bits))
                    .map(|(value, bits)| (value.as_str(), bits.as_slice()))
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let chosen = longest(&candidates);
                choices.push(Choice {
                    mux: false,
                    name: field.as_str(),
                    default: default.as_deref(),
                    candidates,
                    chosen,
                });
            }
            for choice in &choices {
                for bit in explained(choice.candidates[choice.chosen].1) {
                    *covered.entry(bit).or_default() += 1;
                }
            }

            // The multi-bit fields, bit 0 first. These are not a choice:
            // each bit group is read on its own, so there is nothing for
            // rule 1 to weigh.
            for (field, default, groups) in &db.words {
                let mut value = String::with_capacity(groups.len());
                for group in groups {
                    let one = matches(group);
                    value.push(if one { '1' } else { '0' });
                    for b in group {
                        if b.inverted != one {
                            *covered.entry((b.frame, b.bit)).or_default() += 1;
                        }
                    }
                }
                // A `.config` default is written MSB first, so it is
                // reversed to compare with a bit-0-first reading.
                let reversed: Option<String> = default.as_ref().map(|d| d.chars().rev().collect());
                if reversed.as_deref() != Some(value.as_str()) {
                    out.words.push((at, field.clone(), value));
                }
            }

            // Rule 1, as a fixed point. Only a reading that accounts for a
            // bit **nothing** currently accounts for can explain more of
            // the tile than the present one does, so a tile with nothing
            // left over skips this entirely — which is every tile of every
            // bitstream this flow has ever written.
            let mut left: BTreeSet<(u32, u32)> = ones
                .iter()
                .filter(|bit| !covered.contains_key(bit))
                .copied()
                .collect();
            while !left.is_empty() {
                let mut moved = false;
                for choice in &mut choices {
                    let cur = choice.candidates[choice.chosen].1;
                    // What `cur` alone accounts for. Switching away gives
                    // those bits up, so they are what a swap has to beat.
                    let only_cur: BTreeSet<(u32, u32)> = explained(cur)
                        .filter(|bit| covered.get(bit).copied() == Some(1))
                        .collect();
                    let mut best: Option<(usize, usize)> = None;
                    for (j, (_, bits)) in choice.candidates.iter().enumerate() {
                        // A candidate that accounts for nothing currently
                        // unaccounted for cannot explain more of the tile
                        // than the present reading: everything else it
                        // could account for is either already accounted
                        // for by something else or given up by the swap.
                        if !explained(bits).any(|bit| left.contains(&bit)) {
                            continue;
                        }
                        let gained = explained(bits)
                            .filter(|bit| left.contains(bit) || only_cur.contains(bit))
                            .count();
                        if gained <= only_cur.len() {
                            continue;
                        }
                        // Rule 2 breaks the tie, exactly as it does above.
                        if best.is_none_or(|(k, seen)| {
                            gained > seen
                                || (gained == seen && bits.len() > choice.candidates[k].1.len())
                        }) {
                            best = Some((j, gained));
                        }
                    }
                    let Some((j, _)) = best else { continue };
                    for bit in explained(cur) {
                        if let Some(n) = covered.get_mut(&bit) {
                            *n -= 1;
                            if *n == 0 {
                                covered.remove(&bit);
                                left.insert(bit);
                            }
                        }
                    }
                    choice.chosen = j;
                    for bit in explained(choice.candidates[j].1) {
                        *covered.entry(bit).or_default() += 1;
                        left.remove(&bit);
                    }
                    moved = true;
                }
                if !moved {
                    break;
                }
            }

            for choice in &choices {
                let (value, bits) = choice.candidates[choice.chosen];
                if choice.mux {
                    // A source with no bit it wants **set** is the state an
                    // untouched bitstream is in everywhere, so reporting it
                    // would say nothing — and that is true of a source with
                    // no bits at all *and* of one whose whole pattern is
                    // inverted. The centre muxes of this die have the
                    // second kind: `G_DCS0CLK1 <- G_VPFN0000` is six bits
                    // all wanted clear, so every centre mux of the part
                    // would otherwise appear to be carrying an arc as soon
                    // as anything else in its tile is written.
                    if explained(bits).next().is_some() {
                        out.arcs
                            .push((at, choice.name.to_owned(), value.to_owned()));
                    }
                } else if choice.default != Some(value) {
                    out.enums
                        .push((at, choice.name.to_owned(), value.to_owned()));
                }
            }

            for bit in &left {
                out.unexplained += 1;
                out.leftovers
                    .push((tile.ty.clone(), at, format!("F{}B{}", bit.0, bit.1)));
            }
        }
        out.tiles = positions.len();
        out.arcs.sort();
        out.enums.sort();
        out.words.sort();
        out.leftovers.sort();
        out
    }
}
/// An ECP5 fabric: an [`Arch`], where its bits live, and where its pads
/// are.
#[derive(Clone, Debug)]
pub struct TrellisFabric {
    /// The grid, its tile types' bit geometry and their interconnect, the
    /// `io` bels of the two edges [`Edge`] describes, the `lut` bels of
    /// every logic tile, and the package's ball map.
    pub arch: Arch,
    /// The shape of the configuration memory.
    pub format: FrameFormat,
    /// Which rectangles of it each position owns.
    pub frames: Ecp5FrameMap,
    /// The part's JTAG identifier.
    pub idcode: u32,
    /// The part, as `devices.json` names it.
    pub part: String,
    /// The package whose ball map [`Arch::pinmap`] holds.
    pub package: String,
    /// Every pad that became a bel, with its bits located.
    pub io: Vec<IoSite>,
    /// Where each lookup table's bits are, by `(Arch` tile type index, bel
    /// name)`, which is what a site gives.
    pub luts: BTreeMap<(usize, String), LutBits>,
    /// Where each flip-flop's settings are, keyed the same way.
    pub ffs: BTreeMap<(usize, String), FfBits>,
    /// Where a distributed RAM's mode bit is, by `Arch` tile type index.
    ///
    /// One entry per tile type that can hold a [`DPRAM_BEL`]; its contents
    /// live in [`TrellisFabric::luts`], because a slice in `DPRAM` mode
    /// *is* its lookup tables.
    pub dprams: BTreeMap<usize, DpRamBits>,
    /// Where every block RAM's settings are, one entry per `DP16KD` of the
    /// part, in position order.
    ///
    /// By position and not by tile type, which is the one thing about a
    /// block RAM that is unlike everything else this fabric holds; see
    /// [`BramSite`].
    pub brams: Vec<BramSite>,
    /// The global clock network, and the joins it needed; see
    /// [`ClockNetwork`].
    pub clocks: ClockNetwork,
    /// The bits a `.mux` source wants **clear**, by `(Arch` tile type
    /// index, the bits it wants set)`, for the sources that have any. See
    /// [`TrellisFabric::dropped_clear_bits`].
    pub clears: BTreeMap<(usize, Vec<ConfigBit>), Vec<ConfigBit>>,
    /// Per IO bank a pad is in: the `BANKREF` tile's position, and the
    /// bits that set [`BANK_VCCIO`] to the rail
    /// [`TrellisOptions::io_standard`] implies.
    ///
    /// Keyed by bank number. A bank with no pad in it is not here, which
    /// is what Lattice's own packer does too: it writes the setting for
    /// the banks a design uses and leaves the rest alone.
    pub bank_bits: BTreeMap<u32, ((u32, u32), Vec<ConfigBit>)>,
    /// The `BANKREF` value `bank_bits` was located for, so a caller can
    /// print what a bitstream will say without repeating the table.
    pub voltage: String,
    /// The IO standard every pad was located for.
    pub standard: String,
    /// What the load measured.
    pub stats: TrellisStats,
}

impl TrellisFabric {
    /// Checks that this fabric is the part on the other end of a cable.
    ///
    /// # Errors
    ///
    /// [`Ecp5Error::IdcodeMismatch`]. On this family that check matters
    /// more than it looks: the LFE5U-12F and the LFE5U-25F are the *same
    /// die* with different identifiers, so a bitstream built from one
    /// would configure the other and assert `DONE`.
    pub fn check_idcode(&self, wanted: u32) -> Result<(), Ecp5Error> {
        if self.idcode == wanted {
            Ok(())
        } else {
            Err(Ecp5Error::IdcodeMismatch {
                wanted,
                found: self.idcode,
            })
        }
    }

    /// The pad at a package ball.
    #[must_use]
    pub fn pad(&self, ball: &str) -> Option<&IoSite> {
        self.io.iter().find(|site| site.ball == ball)
    }

    /// The pad a site name belongs to.
    #[must_use]
    pub fn pad_of_site(&self, site: &str) -> Option<&IoSite> {
        let (position, bel) = site.split_once('/')?;
        let letter = bel.strip_prefix("PIO")?.chars().next()?;
        let (x, y) = position.strip_prefix('X')?.split_once('Y')?;
        let bel_at = (x.parse().ok()?, y.parse().ok()?);
        self.io
            .iter()
            .find(|candidate| candidate.bel == bel_at && candidate.side == letter)
    }

    /// Sets the bits that configure every placed IO, and returns how many
    /// pads were configured.
    ///
    /// This is the pass the module header describes: a pad's bits live in
    /// tiles the bel does not own, so they cannot be a bel's
    /// [`ConfigEntry`](super::arch::ConfigEntry)s and are computed from
    /// the placement instead. [`super::bitstream::generate`] has already
    /// done everything that *is* a pip's; this adds what is not, exactly as
    /// `super::xray`'s `enable_global_clocks` does for the 7 series.
    ///
    /// Which direction a pad is depends on the netlist and not on the
    /// database, and it is read off the *pins* rather than off the cell's
    /// `DIR` parameter, so a pad whose parameter and whose wiring disagree
    /// is configured as it is wired:
    ///
    /// | `din` (`O`) carries a signal | `dout` (`I`) carries one, or a constant | |
    /// |---|---|---|
    /// | no | yes | an **output** |
    /// | yes | no | an **input** |
    /// | yes | yes | **bidirectional** |
    ///
    /// which is the same rule nextpnr's `nxio_to_tr` applies — "`BIDIR` if
    /// the buffer's `I` has a driver" — for the one netlist shape either of
    /// them builds.
    ///
    /// **What an output gets**, which is what nextpnr's `write_io` and
    /// `tie_cib_signal` write for one and no more:
    ///
    /// 1. `PIO<side>.BASE_TYPE = OUTPUT_<standard>` in the pad tile;
    /// 2. the same field again in the tile the edge's rule gives;
    /// 3. the output-enable wire tied low, so the buffer drives;
    /// 4. the data wire tied to the constant the netlist gives the pin —
    ///    and **not tied at all** when a signal drives it, because then the
    ///    router has driven that wire and tying it would fight the route.
    ///
    /// **What an input gets**:
    ///
    /// 1. `PIO<side>.BASE_TYPE = INPUT_<standard>` in both tiles;
    /// 2. `PIO<side>.HYSTERESIS = ON`;
    /// 3. `PIO<side>.PULLMODE` — see [`PULL_NONE`], which is the
    ///    one of these that changes what a person sees;
    /// 4. no tristate tie and no data tie, neither of which nextpnr writes
    ///    for an input either.
    ///
    /// **What a bidirectional pad gets**, which is the same list asked of
    /// `write_io` again with `dir == "BIDIR"`:
    ///
    /// 1. `PIO<side>.BASE_TYPE = BIDIR_<standard>` in the pad tile — not
    ///    the input's pattern, not the output's, and not their union: see
    ///    [`IoSite::bidir_pad_bits`];
    /// 2. the same field again in the tile the edge's rule gives, which
    ///    costs the two bits an output costs and that an input does not;
    /// 3. `PIO<side>.HYSTERESIS = ON`, as for an input;
    /// 4. `PIO<side>.PULLMODE`, which on a pad that spends half its time
    ///    released is the only thing deciding what it reads then;
    /// 5. **no tristate tie**, which is the whole point: the tristate wire
    ///    is one the router drove, and tying it would be a second driver on
    ///    it. nextpnr ties it under exactly the complementary condition —
    ///    `dir != "INPUT"` *and* `T` unconnected;
    /// 6. the data wire tied only if the netlist gives that pin a constant,
    ///    as for an output.
    ///
    /// **And, for every one of the three, `PIO<side>.SLEWRATE` when a
    /// constraint asks for one** — `set_io -slew fast`. It is the one
    /// setting here whose condition is an attribute rather than a direction,
    /// which is nextpnr's own rule for it, and it is the only edge rate this
    /// family has: see [`SLEW_ATTR`].
    ///
    /// Nothing governs the tristate beyond that. The field that could —
    /// `PIO<side>.TRIMUX_TSREG`, in the second-copy tile — decides whether
    /// the tristate comes from the `PADDT` wire or from an `IOLOGIC`
    /// register, its default is `PADDT`, and `PADDT` is what a fabric-driven
    /// tristate means. nextpnr writes the field only when a packer moved the
    /// tristate into `IOLOGIC`, which nothing here does. So the default is
    /// both right and free, and `docs/fpga-trellis.md` records that it was
    /// checked rather than assumed — the same question that found
    /// [`BANK_VCCIO`] and [`PULL_NONE`] missing, asked again and answered
    /// "nothing".
    ///
    /// And, once per bank any of them is in, [`BANK_VCCIO`] in that bank's
    /// reference tile — which is none of the pad's own positions and is the
    /// one thing this pass was missing when it first put a bitstream in a
    /// part. A bidirectional pad constrains its bank's rail exactly as an
    /// output does; nextpnr's `init_io_banks` tests `dir != "INPUT"` for
    /// that, so `BIDIR` is on the output's side of it.
    ///
    /// # Errors
    ///
    /// [`super::bitstream::BitstreamError`] when a bit falls outside the
    /// position it was located in, which would mean the frame map and the
    /// `Arch` disagree.
    pub fn configure_io(
        &self,
        design: &crate::ir::Design,
        module: crate::ir::ModuleId,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<usize, super::bitstream::BitstreamError> {
        let m = design.modules.get(module);
        let mut done = 0usize;
        let mut banks: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "io" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let Some(pad) = self.pad_of_site(&graph.sites[site].name) else {
                continue;
            };
            let pin_of = |role: &str| {
                netlist
                    .pins
                    .iter()
                    .find(|pin| pin.instance == index && pin.role == role)
            };
            let din = pin_of("din");
            let dout = pin_of("dout");
            let oe = pin_of("oe");
            // An input's data wire leaves the buffer, so its pin is an
            // output of the cell; an output's arrives, so its pin is an
            // input. A pad with both is bidirectional, whichever way its
            // tristate is driven.
            let reads = din.is_some_and(|pin| pin.signal.is_some());
            let drives = dout.is_some_and(|pin| pin.signal.is_some() || pin.constant.is_some());
            let (pad_type, pic_type) = match (reads, drives) {
                (true, true) => (&pad.bidir_pad_bits, &pad.bidir_pic_bits),
                (true, false) => (&pad.input_pad_bits, &pad.input_pic_bits),
                _ => (&pad.output_pad_bits, &pad.output_pic_bits),
            };
            for bit in pad_type {
                bits.set(pad.pad_at, *bit)?;
            }
            for bit in pic_type {
                bits.set(pad.pic_at, *bit)?;
            }
            // Hysteresis and a pull mode, for every pad that reads the pin.
            // `write_io`'s own test is `dir == "INPUT" || dir == "BIDIR"`,
            // and the pull is the one of the two that changes what a person
            // sees: see `PULL_NONE`.
            // The slew rate, for every direction and only when a constraint
            // asked: nextpnr's condition is the attribute's presence and
            // nothing else. See `SLEW_ATTR`.
            if let Some(rate) = m
                .and_then(|m| m.cells.get(instance.cell))
                .and_then(|cell| cell.attrs.get(SLEW_ATTR))
                .and_then(|value| match value {
                    crate::ir::AttrValue::String(s) => Some(s.to_ascii_uppercase()),
                    _ => None,
                })
            {
                for bit in pad.slew_bits(&rate) {
                    bits.set(pad.pad_at, *bit)?;
                }
            }
            if reads {
                let mode = m
                    .and_then(|m| m.cells.get(instance.cell))
                    .and_then(|cell| cell.params.get(PULL_PARAM))
                    .and_then(|value| match value {
                        crate::ir::AttrValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or(PULL_NONE);
                for bit in pad.hysteresis_bits.iter().chain(pad.pull_bits(mode)) {
                    bits.set(pad.pad_at, *bit)?;
                }
            }
            if drives {
                // The tristate. A pin a *signal* drives is left alone —
                // that is a bidirectional pad, and the route into the wire
                // is what drives it — and one with a constant is tied to
                // the constant it was given, which for a plain output is
                // the zero that makes the buffer drive.
                let tie = match oe {
                    Some(pin) if pin.signal.is_some() => None,
                    Some(pin) if pin.constant == Some(crate::logic::Bit::One) => {
                        Some(&pad.tristate_bits)
                    }
                    _ => Some(&pad.enable_bits),
                };
                for bit in tie.into_iter().flatten() {
                    bits.set(pad.cib_at, *bit)?;
                }
                // Which constant the data pin carries, if it carries one. A
                // pin a signal drives is left alone for the same reason.
                if let Some(pin) = dout.filter(|pin| pin.signal.is_none()) {
                    let tie = match pin.constant {
                        Some(crate::logic::Bit::One) => &pad.high_bits,
                        _ => &pad.low_bits,
                    };
                    for bit in tie {
                        bits.set(pad.cib_at, *bit)?;
                    }
                }
            }
            banks.insert(pad.bank);
            done += 1;
        }
        // The bank rail, once per bank, after the pads rather than inside
        // the loop so the same bits are not written six times.
        for bank in &banks {
            let Some((at, bits_of_bank)) = self.bank_bits.get(bank) else {
                continue;
            };
            for bit in bits_of_bank {
                bits.set(*at, *bit)?;
            }
        }
        Ok(done)
    }

    /// Sets the bits that configure every placed lookup table, and returns
    /// how many there were.
    ///
    /// # Why a truth table is not a parameter bit
    ///
    /// Every other family this project has met puts a LUT's `INIT` in the
    /// bel's own [`ConfigEntry::Param`](super::arch::ConfigEntry::Param)
    /// list, and [`super::bitstream::generate`] copies the cell's parameter
    /// into it bit by bit. That does not work here, and the reason is
    /// specific:
    ///
    /// A `LUT4` reaches this pass with its unused inputs tied to a
    /// **constant**, and the slice's own input mux offers only `1` — there
    /// is no `SLICE<l>.A0MUX = 0`. So an input the design ties to zero
    /// cannot be tied at the slice at all. What Lattice's own packer does
    /// is tie every unused input high (`write_comb` writes
    /// `SLICE<l>.<X><n>MUX = 1` for each input no pip reaches) and rely on
    /// Yosys having produced a truth table that does not depend on them.
    /// This does the same thing but **derives** it rather than relying on
    /// it: the truth table is specialised at each untied input to the
    /// constant the netlist gives, so the result genuinely ignores that
    /// input and tying it high is safe whichever constant was meant.
    ///
    /// For the one design this has built that is a no-op — Reticle's
    /// technology mapper emits `INIT=0x5555` for an inverter, which already
    /// ignores B, C and D — and that is worth knowing rather than relying
    /// on.
    ///
    /// # Errors
    ///
    /// [`super::bitstream::BitstreamError`] when a bit falls outside the
    /// position, which would mean the frame map and the `Arch` disagree.
    pub fn configure_logic(
        &self,
        design: &crate::ir::Design,
        module: crate::ir::ModuleId,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<usize, super::bitstream::BitstreamError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut done = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "lut" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(lut) = self.luts.get(&(site.tile_type, site.bel.clone())) else {
                continue;
            };
            // The truth table the cell carries, as sixteen bits.
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let mut table = [false; 1 << LUT_INPUTS.len()];
            for (value, slot) in table.iter_mut().enumerate() {
                let bit = u32::try_from(value).unwrap_or(0);
                *slot = params
                    .and_then(|p| p.get(LUT_INIT))
                    .is_some_and(|v| super::bitstream::param_bit(Some(v), bit));
            }
            // Which inputs a signal reaches. The others are folded into the
            // table and then tied high.
            let mut tied: Vec<(usize, bool)> = Vec::new();
            for (k, _) in LUT_INPUTS.iter().enumerate() {
                let role = format!("i{k}");
                let pin = netlist
                    .pins
                    .iter()
                    .find(|pin| pin.instance == index && pin.role == role);
                match pin {
                    Some(pin) if pin.signal.is_some() => {}
                    Some(pin) => tied.push((k, pin.constant == Some(crate::logic::Bit::One))),
                    // A pin the cell does not have at all is unconnected,
                    // and Lattice's own packer ties those high.
                    None => tied.push((k, true)),
                }
            }
            let mut folded = table;
            for (index, value) in &tied {
                let mask = 1usize << index;
                for (address, slot) in folded.iter_mut().enumerate() {
                    let from = if *value {
                        address | mask
                    } else {
                        address & !mask
                    };
                    *slot = table[from];
                }
                table = folded;
            }
            for (bit, value) in folded.iter().enumerate() {
                let groups = if *value {
                    &lut.init_one
                } else {
                    &lut.init_zero
                };
                for at in groups.get(bit).into_iter().flatten() {
                    bits.set(site.tile, *at)?;
                }
            }
            for (input, _) in &tied {
                for at in lut.tie_high.get(*input).into_iter().flatten() {
                    bits.set(site.tile, *at)?;
                }
            }
            done += 1;
        }
        Ok(done)
    }

    /// Writes every distributed RAM's mode and contents, and returns how
    /// many were written.
    ///
    /// # Everything Lattice's own packer writes for a distributed RAM
    ///
    /// Asked in full rather than as a diff, which is how every real defect
    /// in this backend has been found. Three sources, and they agree:
    /// Project Trellis' `PLC2` database, nextpnr's `ecp5/` — `pack_dram`,
    /// `dram_to_comb`, `dram_to_ramw_split`, `write_comb` and the
    /// `TRELLIS_RAMW` arm of `write_bitstream` — and, unexpectedly, **the
    /// vendor's own bitstreams for this board**: `analyzer.bit` holds 22
    /// distributed RAMs and `facedancer.bit` 89.
    /// `tests/fpga_trellis.rs`'s
    /// `what_lattices_own_packer_writes_for_a_distributed_ram` reads all
    /// 111 of them back at absolute frame positions, and two things in the
    /// table below are its findings rather than nextpnr's: that the
    /// contents are always zero on this board, and which bels a RAM does
    /// **not** take.
    ///
    /// | | |
    /// |---|---|
    /// | `SLICEA.MODE`, `SLICEB.MODE` | `DPRAM` |
    /// | `SLICEC.MODE` | `RAMW` — and all three are **one bit**, `F50B11` |
    /// | `SLICEA.K0/K1.INIT`, `SLICEB.K0/K1.INIT` | the contents, one word per bit of the four-bit word |
    /// | `SLICEC.K0.INIT`, `SLICEC.K1.INIT` | **sixteen zeros each**, which is 32 bits nothing reads |
    /// | `SLICEA.WREMUX` | `WRE`, the default, which costs nothing |
    /// | `CLK1.CLKMUX` | `CLK`, the default, which costs nothing |
    /// | `SLICE<l>.CCU2.INJECT1_<n>` | nextpnr's `_NONE_`: leave the bits clear, and a bitstream assembled from zero already has |
    /// | An unused lookup-table input | `SLICE<l>.<X><n>MUX = 1`, which cannot happen here: a `DPRAM` slice uses all four |
    /// | Slice D | nothing. It still holds two lookup tables and two flip-flops, and the vendor uses them |
    /// | The flip-flops of slices A, B and C | nothing. `DPRAM` mode takes a slice's lookup tables, not its registers, and the vendor puts flops in 68 of the 111 |
    ///
    /// So a distributed RAM with no initial contents costs **97 bits**: one
    /// for the mode, 64 for four zeroed content words and 32 for the
    /// `RAMW` slice's two. The `INIT` bits are `!`-marked in the database,
    /// so a content bit of *zero* is a bit set in the bitstream and all
    /// ones is free — which is why an empty RAM is the expensive case.
    ///
    /// # Errors
    ///
    /// [`super::bitstream::BitstreamError`] when a bit falls outside the
    /// tile it belongs to, which would mean the grid and the database
    /// disagree.
    pub fn configure_lutram(
        &self,
        design: &crate::ir::Design,
        module: crate::ir::ModuleId,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<usize, super::bitstream::BitstreamError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut done = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "lutram" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(dpram) = self.dprams.get(&site.tile_type) else {
                continue;
            };
            for at in &dpram.mode {
                bits.set(site.tile, *at)?;
            }
            // The write-port register's two lookup tables hold nothing and
            // Lattice's own packer still writes both words as zeros. This
            // flow writes what it writes.
            for bel in DPRAM_RAMW_LUTS {
                let Some(lut) = self.luts.get(&(site.tile_type, bel.to_owned())) else {
                    continue;
                };
                for groups in &lut.init_zero {
                    for at in groups {
                        bits.set(site.tile, *at)?;
                    }
                }
            }
            // The contents, one `INIT` word per bit of the word, each
            // addressed through the permutation the read-address wiring
            // forces. `INITVAL` is the cell parameter that would carry
            // them; nothing in this flow produces one, because
            // `primitives::Mapper::lower_memory` refuses a memory with
            // initial contents, so in practice every word is zero — which
            // is also what nextpnr writes when a `TRELLIS_DPR16X4` has no
            // `INITVAL`.
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let initval = params.and_then(|p| p.get(DPRAM_INIT));
            for (bit, bel) in DPRAM_DATA_LUTS.iter().enumerate() {
                let Some(lut) = self.luts.get(&(site.tile_type, (*bel).to_owned())) else {
                    continue;
                };
                for address in 0..1usize << DPRAM_ADDR_BITS {
                    let where_in_word =
                        u32::try_from(address * DPRAM_DATA_LUTS.len() + bit).unwrap_or(u32::MAX);
                    let value = super::bitstream::param_bit(initval, where_in_word);
                    let groups = if value { &lut.init_one } else { &lut.init_zero };
                    for at in groups.get(dpram_init_word(address)).into_iter().flatten() {
                        bits.set(site.tile, *at)?;
                    }
                }
            }
            done += 1;
        }
        Ok(done)
    }

    /// Writes every block RAM's settings, and returns the initialisation
    /// blocks its contents go in.
    ///
    /// # Everything `ecppack` writes for a block RAM, in full
    ///
    /// Read out of Great Scott Gadgets' own bitstreams for this board, at
    /// the absolute frame positions this flow computes, by
    /// `what_lattices_own_packer_writes_for_a_block_ram`. There are
    /// **nine** block RAMs in `analyzer.bit`, **none** in `selftest.bit`
    /// and **forty-four** in `facedancer.bit`, and the list below is
    /// everything their packer sets for each one — not a summary of it:
    ///
    /// | | |
    /// |---|---|
    /// | `EBR<n>.MODE` | `DP16KD`, which is **five bits spread over two tiles** — one in the block's own and four in the one east of it |
    /// | `EBR<n>.DP16KD.DATA_WIDTH_A`, `..._B` | the mode's width. `facedancer.bit` is 9 on both ports in all forty-four; `analyzer.bit` has 4s as well |
    /// | `EBR<n>.DP16KD.WRITEMODE_A`, `..._B` | `READBEFOREWRITE`, in all 53 |
    /// | `EBR<n>.GSR` | `DISABLED`, in all 53 |
    /// | `EBR<n>.RESETMODE`, `EBR<n>.ASYNC_RESET_RELEASE` | `ASYNC`, in all 53 |
    /// | `EBR<n>.RSTAMUX`, `EBR<n>.RSTBMUX` | `INV`, in all 53 — the reset has to be held **low** and an unrouted wire of this fabric reads as a **one** |
    /// | `EBR<n>.WEBMUX` | `INV`, in all 53. `WEAMUX` is **never** written, because their mapping writes on port A and reads on port B. This flow is the other way round, so it writes `WEAMUX` instead |
    /// | `EBR<n>.WID` | nine bits, and the number they spell is exactly the index of one of the file's own initialisation blocks: 3 to 11 for analyzer's nine, 3 to 46 for facedancer's forty-four |
    /// | `EBR<n>.CSDECODE_A`, `..._B` | **never written**, which is `111`, which is what the three chip-select wires read when nothing drives them |
    /// | `EBR<n>.CEAMUX`, `CEBMUX`, `OCEAMUX`, `OCEBMUX` | never written. Those four pins want a **one** and get one for free |
    /// | `EBR<n>.CLKAMUX`, `CLKBMUX`, `ADA<n>MUX`, `ADB<n>MUX` | never written: the default polarity |
    /// | `EBR<n>.REGMODE_A`, `..._B` | never written, so `NOREG`: the output register is not used |
    /// | The contents | a `LSC_EBR_WRITE` block of 2048 nine-bit words per block RAM, **written even when it is all zeros** — and it is all zeros in all 53 |
    ///
    /// So an empty block RAM costs eleven to fifteen configuration bits,
    /// depending on the width, plus a 2.3 kB initialisation block in the
    /// stream. That is `ecppack`'s number as well as this flow's.
    ///
    /// # What the contents' ordering rests on, which is not a bitstream
    ///
    /// All 53 of the vendor's initialisation blocks are **zero**, exactly
    /// as all 111 of their distributed RAMs are empty, so they say nothing
    /// about where a word goes. What this writes is the layout
    /// `src/fpga/devices/ecp5.dev` states — 1024 rows of 18 bits in 20-bit
    /// slots, sixteen to an `INITVAL_<nn>` parameter, two nine-bit words to
    /// a row, the low word in bits 8..0 — and the only thing that makes it
    /// self-consistent rather than merely plausible is that the block's
    /// own 9-bit mode addresses the array in exactly those units, so word
    /// *w* of the stream is address *w* of a 9-bit port. **A part is the
    /// only thing that can settle it**; `docs/fpga-trellis.md` names the
    /// experiment.
    ///
    /// # Errors
    ///
    /// [`super::bitstream::BitstreamError`] when a bit falls outside the
    /// tile it belongs to, which would mean the grid and the database
    /// disagree.
    pub fn configure_bram(
        &self,
        design: &crate::ir::Design,
        module: crate::ir::ModuleId,
        device: &super::Device,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<BramConfig, super::bitstream::BitstreamError> {
        let mut out = BramConfig::default();
        let Some(m) = design.modules.get(module) else {
            return Ok(out);
        };
        let mut wid = BRAM_FIRST_WID;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "bram" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(bram) = self.brams.iter().find(|b| b.at == site.tile) else {
                continue;
            };
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let set = |field: &str, value: &str,
                           bits: &mut super::bitstream::Bitstream|
             -> Result<(), super::bitstream::BitstreamError> {
                for (at, bit) in bram.enum_bits(field, value).unwrap_or_default() {
                    bits.set(*at, *bit)?;
                }
                Ok(())
            };
            set(BRAM_MODE.0, BRAM_MODE.1, bits)?;
            set(BRAM_GSR.0, BRAM_GSR.1, bits)?;
            for field in BRAM_RESET.0 {
                set(field, BRAM_RESET.1, bits)?;
            }
            for field in BRAM_WRITEMODE.0 {
                set(field, BRAM_WRITEMODE.1, bits)?;
            }
            // Each port's width, out of the parameter the device file's
            // `mode` line sets. The parameter is the field's own last
            // component, so nothing here names `DATA_WIDTH_A`.
            for field in BRAM_WIDTH {
                let name = field.rsplit('.').next().unwrap_or(field);
                let Some(value) = params
                    .and_then(|p| p.get(name))
                    .and_then(crate::ir::AttrValue::as_int)
                else {
                    continue;
                };
                set(field, &value.to_string(), bits)?;
            }
            // A reset and a write enable nothing drives have to be held
            // low, and this fabric's unrouted wires read as ones.
            for (port, letter) in BRAM_PORTS.iter().enumerate() {
                for (role, field) in BRAM_TIE_LOW_PINS.iter().zip(BRAM_TIE_LOW) {
                    let driven = netlist.pins.iter().any(|pin| {
                        pin.instance == index
                            && pin.role == format!("p{port}_{role}")
                            && pin.signal.is_some()
                    });
                    if !driven {
                        set(&field.replace('#', &letter.to_string()), BRAM_INV, bits)?;
                    }
                }
            }
            // The identifier that ties this block to its contents, most
            // significant bit first.
            let Some(groups) = bram.word(BRAM_WID) else {
                continue;
            };
            for (shift, group) in groups.iter().enumerate() {
                let bit = BRAM_WID_BITS as usize - 1 - shift;
                if wid >> bit & 1 == 0 {
                    continue;
                }
                for (at, one) in group {
                    bits.set(*at, *one)?;
                }
            }
            out.blocks.push(super::ecp5::BramBlock {
                index: wid,
                words: bram_init_words(device, &instance.primitive, params),
            });
            wid += 1;
        }
        // The two top data bits of each of a block's ports are the same
        // interconnect wires as the two bottom bits of the block two
        // columns east, so a block in 18-bit mode and its eastern
        // neighbour cannot both be used. Nothing in the vendor's three
        // bitstreams exercises 18-bit mode, so there is no artefact to
        // check a model of it against, and refusing is the honest answer.
        let placed: Vec<((u32, u32), bool)> = netlist
            .instances
            .iter()
            .enumerate()
            .filter(|(_, inst)| inst.kind == "bram")
            .filter_map(|(index, inst)| {
                let site = placement.site_of(index)?;
                let wide = BRAM_WIDTH.iter().any(|field| {
                    let name = field.rsplit('.').next().unwrap_or(field);
                    m.cells
                        .get(inst.cell)
                        .and_then(|cell| cell.params.get(name))
                        .and_then(crate::ir::AttrValue::as_int)
                        == Some(i64::from(BRAM_DATA_PINS))
                });
                Some((graph.sites[site].tile, wide))
            })
            .collect();
        for (at, wide) in &placed {
            if !wide {
                continue;
            }
            let east = (at.0 + 2, at.1);
            if placed.iter().any(|(other, _)| *other == east) {
                out.overlaps.push(format!(
                    "X{}Y{} is 18 bits wide and X{}Y{} is in use",
                    at.0, at.1, east.0, east.1
                ));
            }
        }
        Ok(out)
    }

    /// A [`super::route::RouteOptions::node_base`] vector that makes the
    /// global clock network cheap, so a clock goes on it.
    ///
    /// # Why a preference is needed at all
    ///
    /// A flip-flop's clock mux (`.mux CLK0` of a `PLC2`) offers the sixteen
    /// global branch wires **and** seven ordinary interconnect wires. So
    /// the shortest path from a pad to a clock pin is through general
    /// routing — measured on this die, seven hops from `JPADDIB_PIO` on
    /// ball A8 to a `CLK0_SLICE`, against about eighteen through the
    /// network — and a router with no preference builds a clock tree out of
    /// data wires. That routes, verifies and configures; what it does not do
    /// is control skew, and a counter clocked that way is a thing nobody has
    /// a model for.
    ///
    /// # Why this is sound rather than merely convenient
    ///
    /// The clock network is a **one-way funnel**. A signal can enter it
    /// only through a buffer's `CLKI` mux or a centre mux, and every way out
    /// of it is a flip-flop's `CLK<n>`, `LSR<n>` mux — there is no pip from
    /// a branch wire back into general routing. So making it cheap cannot
    /// pull a data signal onto it: the only signal with a reason to traverse
    /// it is one that clocks or resets something.
    ///
    /// And two clocks cannot collide on it, which is the thing the per-tile
    /// naming makes worth checking. `G_HPBX0300` is a separate node in every
    /// tile although it is one piece of metal per quadrant and side, so the
    /// router's one-signal-per-node rule does not by itself stop two nets
    /// sharing a branch. It does not have to: every path onto a branch of
    /// network *n* goes through its quadrant's single `G_<quadrant>PCLK<n>`
    /// global node, and that node has capacity one.
    ///
    /// `preference` multiplies the cost of a network node. Below about a
    /// tenth the network wins for a clock with one sink; the default the
    /// flow uses is 0.05.
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

    /// Every bit a pip the router took wants **clear** and that something
    /// else has set, one line each.
    ///
    /// # Why this is the fix, and why there is no other
    ///
    /// A `bits.db` bit written `!F25B10` is one its feature wants clear. A
    /// bitstream here is assembled by setting bits in a zeroed bitmap, so
    /// there is nothing to *write* for such a bit, and the loader does not
    /// record it on the pip — a `PipDecl` says which bits switch a
    /// connection on and has no room for the ones it needs off.
    ///
    /// That is not a shortcut with a better alternative. A clear cannot be
    /// written into a map that is already clear; the only thing honouring it
    /// can mean is **noticing when another feature of the same tile has set
    /// it**. So the fix for the simplification is a check, and this is it:
    /// the bits are recorded at load time in [`TrellisFabric::clears`], and
    /// every pip a route takes is asked, against the finished image,
    /// whether the bits its source wants clear are clear.
    ///
    /// It matters most for a clock. A centre mux of this die encodes its
    /// source as a six-bit code — `G_URPCLK0 <- G_HPFE0000` is
    /// `!F1B0 F2B0 !F3B0 !F4B0 F5B0 !F6B0` — so taking one arc leaves five
    /// bits that another feature setting any of them would silently turn
    /// the arc into a different one. Over the family, 4523 of 85 379 mux
    /// source lines have an inverted bit and all of them are in the clock
    /// network's tiles, which is what
    /// `an_inverted_mux_bit_only_happens_in_the_clock_network` pins; a
    /// combinational design could not reach one and a clocked design walks
    /// through six.
    ///
    /// The pips are looked up by the bits they set rather than by name,
    /// because a pip in the graph carries resolved wire names and the
    /// database carries prefixed ones. Two sources of one tile type with
    /// the same bits set are indistinguishable in a finished bitstream
    /// anyway; [`TrellisStats::clear_collisions`] counts them and it is zero
    /// on this die.
    #[must_use]
    pub fn dropped_clear_bits(
        &self,
        graph: &super::arch::RoutingGraph,
        routing: &super::Routing,
        bits: &super::bitstream::Bitstream,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for route in routing.routes() {
            for id in &route.pips {
                let pip = graph.pip(*id);
                let set = graph.pip_bits(*id);
                if set.is_empty() {
                    continue;
                }
                let Some(ty) = self.arch.tile_index_at(pip.tile.0, pip.tile.1) else {
                    continue;
                };
                let mut key = set.to_vec();
                key.sort_unstable();
                let Some(clear) = self.clears.get(&(ty, key)) else {
                    continue;
                };
                for bit in clear {
                    if bits.get(pip.tile, *bit) == Some(true) {
                        out.push(format!(
                            "X{}Y{} the arc `{}` <- `{}` needs bit {}.{} clear and something else \
                             has set it, so the bits select a different connection",
                            pip.tile.0,
                            pip.tile.1,
                            graph.wire(pip.to).name,
                            graph.wire(pip.from).name,
                            bit.row,
                            bit.col
                        ));
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The arcs a routing takes that cost bits, as the `(name, tile)` pairs
    /// a decoding resolves to.
    ///
    /// Only the arcs with bits: one without leaves no trace in a bitstream,
    /// so nothing can be said about it this way. `Routing::verify` is what
    /// covers those, by walking each sink back to its driver.
    #[must_use]
    pub fn routed_arcs(
        &self,
        graph: &super::arch::RoutingGraph,
        routing: &super::Routing,
    ) -> BTreeSet<(ResolvedWire, ResolvedWire)> {
        routing
            .routes()
            .flat_map(|route| route.pips.iter())
            .filter(|id| !graph.pip_bits(**id).is_empty())
            .map(|id| {
                let pip = graph.pip(*id);
                let to = graph.wire(pip.to);
                let from = graph.wire(pip.from);
                ((to.name.clone(), to.tile), (from.name.clone(), from.tile))
            })
            .collect()
    }

    /// Which global network each placed flip-flop's clock arrived on.
    ///
    /// This exists because [`TrellisFabric::clock_node_costs`] is a
    /// preference and a preference can be lost. A clock that came through
    /// general routing routes, verifies and configures — the failure is a
    /// skew nobody modelled, not a broken bitstream — so nothing else would
    /// notice, and `reticle fpga --bitstream` refuses rather than writing
    /// one quietly.
    #[must_use]
    pub fn clock_network_use(
        &self,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        routing: &super::Routing,
    ) -> ClockUse {
        let mut out = ClockUse::default();
        for (index, instance) in netlist.instances.iter().enumerate() {
            // A distributed RAM is clocked too, and its write clock is the
            // same kind of claim: `ecppack` puts it on a global network in
            // all 111 of the RAMs in this board's own bitstreams, so a
            // write clock off a data wire is the same unmodelled skew a
            // flip-flop's would be. The pin is `wclk` and the wire it
            // reaches is `WCK<n>_SLICE`, fixed to the tile's `CLK1`.
            //
            // A **block** RAM is two clocks rather than one — `CLKA` and
            // `CLKB`, which this flow usually drives from the same net —
            // and each of them arrives on its own tile's `JCLK0`, one
            // column apart. `ecppack` puts both on a global network in all
            // 53 of this board's block RAMs, so both are asked about.
            let roles: &[&str] = match instance.kind.as_str() {
                "ff" => &["clk"],
                "lutram" => &["wclk"],
                "bram" => &["p0_clk", "p1_clk"],
                _ => continue,
            };
            for role in roles {
                self.one_clock_pin(netlist, placement, graph, routing, index, role, &mut out);
            }
        }
        out
    }

    /// One clock pin's own path back through the route, for
    /// [`TrellisFabric::clock_network_use`].
    #[allow(clippy::too_many_arguments)]
    fn one_clock_pin(
        &self,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        routing: &super::Routing,
        index: usize,
        role: &str,
        out: &mut ClockUse,
    ) {
        {
            let Some(site) = placement.site_of(index) else {
                return;
            };
            let site = &graph.sites[site];
            let Some(signal) = netlist
                .pins
                .iter()
                .find(|pin| pin.instance == index && pin.role == role)
                .and_then(|pin| pin.signal)
            else {
                return;
            };
            // Walk this pin's own path back through the route until a
            // branch wire turns up. Asking "does the route touch a branch
            // wire in this tile" would not do: a tile has two clock muxes
            // and four slices, so one flop's clock can be on the network
            // while its neighbour's came off a data wire, and the whole
            // point of the check is to catch exactly that.
            let mut found = None;
            if let (Some(clk), Some(route)) = (site.pin(role), routing.route(signal)) {
                let mut node = clk;
                for _ in 0..8 {
                    let Some(pip) = route
                        .pips
                        .iter()
                        .find(|id| graph.pip(**id).to == node)
                        .map(|id| graph.pip(*id))
                    else {
                        break;
                    };
                    let driver = graph.wire(pip.from);
                    // `CLK0` and `CLK1` are the two clock muxes a tile
                    // shares between its four slices, and what drives the
                    // one this flop's `MUXCLK` selected is the whole
                    // question. It has to be asked of this pin's own path:
                    // one flop of a tile can be on the network while its
                    // neighbour came off a data wire, which is what
                    // happened before `RouteOptions::node_base` existed.
                    // `JCLK0` and `JCLK1` are the same thing in an EBR
                    // tile, and a block RAM's clock pin is joined straight
                    // to one of them with no mux of its own.
                    if matches!(
                        graph.wire(node).name.as_str(),
                        "CLK0" | "CLK1" | "JCLK0" | "JCLK1"
                    ) {
                        found = ClockNetwork::branch_index(&driver.name);
                        break;
                    }
                    node = pip.from;
                }
            }
            match found {
                Some(n) => *out.networks.entry(n).or_default() += 1,
                None => out.off_network.push(format!(
                    "{} on {}",
                    netlist.instances[index].name, site.name
                )),
            }
        }
    }

    /// Sets the bits that configure every placed flip-flop, and returns how
    /// many there were.
    ///
    /// # What this writes, and whose list it is
    ///
    /// nextpnr's `write_ff` is the whole of what Lattice's own flow puts in
    /// a tile for a `TRELLIS_FF`, and this is a line-for-line answer to it:
    ///
    /// | Setting | Where it comes from |
    /// |---|---|
    /// | `SLICE<l>.GSR` | the cell's `GSR` parameter, `ENABLED` by default |
    /// | `SLICE<l>.REG<n>.SD` | always `0`: the data comes from the fabric's `M` wire, because this flow never packs a lookup table and a flop onto one site. A flop whose data is a **constant** has a lookup table built for it by `techcells::drive_constant_data`, because an unrouted slice input reads as a one; a flop with nothing at all on that pin is still **refused**: see the check |
    /// | `SLICE<l>.REG<n>.REGSET` | the cell's `REGSET`, `RESET` by default |
    /// | `SLICE<l>.REG<n>.LSRMODE` | always `LSR`, which is the default and costs nothing |
    /// | `SLICE<l>.CEMUX` | the cell's `CEMUX`; `1` when nothing drives the enable, and the default is `CE`, so **not writing it would leave a flop waiting on an undriven wire** |
    /// | `CLK<c>.CLKMUX` | the cell's `CLKMUX`, and only for the control mux the clock's route actually took |
    /// | `LSR<c>.LSRMUX`, `LSR<c>.SRMODE` | the cell's, and only for the mux the reset's route took |
    ///
    /// The last two rows are why the routing is a parameter. A tile has two
    /// clock muxes and two reset muxes shared between its four slices, so
    /// "which one is this flop's" is a fact about the route and not about
    /// the bel; nextpnr asks the same question the same way, by looking at
    /// which of `CLK0` and `CLK1` carries the net. A flop whose parameter
    /// needs bits in a mux the routing does not identify is **refused**
    /// rather than written into the wrong one.
    ///
    /// `CEMUX = 1` is the entry worth staring at. Its default is `CE` —
    /// take the enable from the fabric — so a bitstream that leaves the
    /// field alone has every flip-flop gated by a wire nothing drives. That
    /// is the same shape as the bank rail and the pull mode: a database
    /// default that is wrong for the design, in a field the design never
    /// mentions.
    ///
    /// # Errors
    ///
    /// [`TrellisError::Bits`] when a bit falls outside the position, and
    /// [`TrellisError::Unsupported`] when a flop asks for a non-default
    /// clock or reset mux whose control index the routing does not settle, or
    /// when nothing at all drives its data input — which a constant no longer
    /// is, since `techcells::drive_constant_data` builds one.
    // One argument more than [`TrellisFabric::configure_logic`], and it is
    // the `routing`: two of the fields a flip-flop needs live in a mux the
    // tile shares between its four slices, so which of them is this flop's
    // is a fact about the route. Bundling the six into a context struct
    // would hide that rather than fix it.
    #[allow(clippy::too_many_arguments)]
    pub fn configure_registers(
        &self,
        design: &crate::ir::Design,
        module: crate::ir::ModuleId,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        routing: &super::Routing,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<usize, TrellisError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut done = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.kind != "ff" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(ff) = self.ffs.get(&(site.tile_type, site.bel.clone())) else {
                continue;
            };
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let value = |name: &str, default: &str| -> String {
                params
                    .and_then(|p| p.get(name))
                    .and_then(crate::ir::AttrValue::as_str)
                    .unwrap_or(default)
                    .to_owned()
            };
            let signal_of = |role: &str| {
                netlist
                    .pins
                    .iter()
                    .find(|pin| pin.instance == index && pin.role == role)
                    .and_then(|pin| pin.signal)
            };

            // **A flip-flop whose data input nothing drives at all is
            // refused. A constant is not that case any more: it is built.**
            //
            // `SD = 0` takes the data from the fabric's `M` wire, and if
            // nothing is routed to that wire, nothing drives it. An unrouted
            // slice input on this family is not a zero: Lattice's own packer
            // ties every unused lookup-table input **high** and this flow does
            // the same, so the flop loads a **one** every clock. A register bit
            // no expression in a design ever assigns anything but zero would
            // therefore come up set.
            //
            // That is not a hypothetical. `ip/usb_device_fs`'s control endpoint
            // had `reg [2:0] stage` for four states, so `stage[2]` was a bit
            // nothing ever set; read back off a real ECP5 through a debug port,
            // `stage` was **5**, `case (stage)` matched none of its four labels,
            // and every IN token a host sent was answered from the `default` arm
            // with a NAK. The device acknowledged the host's SETUP, received
            // every byte of it correctly, transmitted a well-formed handshake —
            // and never enumerated, for eight rounds of looking somewhere else.
            // `docs/fpga-trellis.md` has the whole of it.
            //
            // **What happens to such a flip-flop now is that the constant gets
            // built.** `techcells::drive_constant_data` gives every flip-flop
            // whose data input is a constant a lookup table to take it from —
            // one per constant, shared, `INIT` all zeros or all ones, every
            // input tied high so the value does not depend on what an unrouted
            // input reads as — and the router routes it to the `M` wire like any
            // other signal. That is what nextpnr's `pack_constants` does and,
            // measured rather than assumed, it is what Lattice's own bitstreams
            // for this board contain: `what_lattices_own_packer_writes_for_a_constant`
            // walks `analyzer.bit` and `facedancer.bit` backwards from every one
            // of their 1135 and 3132 fabric-fed flip-flops and finds a driver on
            // every single one, four of them a constant `SLICEA.K0`.
            //
            // So what is left here is the case that genuinely cannot be built,
            // and it is narrower than it was. Three ways to reach it:
            //
            // - the netlist has **no data pin at all** for the flop, or its
            //   constant is an `x` or a `z`, which is a constant nothing can
            //   drive a wire to;
            // - the flow ran with [`super::FpgaOptions::device_cells`] off, or
            //   on a device whose file declares no LUT primitive, so the pass
            //   that makes constants never ran;
            // - something placed a flip-flop whose data signal the router did
            //   not reach, which `Routing::verify` would also report.
            //
            // **A data pin the netlist ties high is still allowed through**, and
            // the reason is the one above read the other way: the untied wire
            // *is* a one, so the flop loads the one the design asked for. That is
            // how `usb_ulpi_link`'s `rst_q` worked before this pass existed — a
            // set-once register that releases a transceiver's reset pin, whose
            // data input is the constant `1'b1` and whose enable is the
            // condition — and it was right by accident rather than by
            // construction. The pass makes it right by construction; this
            // allowance is what keeps a netlist built without the pass from
            // being refused for a case that does work.
            let data = netlist
                .pins
                .iter()
                .find(|pin| pin.instance == index && pin.role == "d");
            let tied_high = data.is_some_and(|pin| {
                pin.signal.is_none() && pin.constant == Some(crate::logic::Bit::One)
            });
            if data.is_none_or(|pin| pin.signal.is_none()) && !tied_high {
                let what = match data.and_then(|pin| pin.constant) {
                    Some(value) if value.is_known() => format!(
                        "has the constant `{value}` on its data input and no lookup table driving \
                         it, so the constant was never built — which means this netlist did not \
                         come through `techcells::drive_constant_data` (the `device_cells` step of \
                         the flow, which a device declaring no LUT primitive also skips)"
                    ),
                    Some(value) => format!(
                        "has `{value}` on its data input, which is not a constant anything can \
                         drive a wire to"
                    ),
                    None => "has nothing at all on its data input".to_owned(),
                };
                return Err(TrellisError::Unsupported {
                    what: format!(
                        "flip-flop `{}` (`{}` at X{}Y{}) {what}. An unrouted slice input on this \
                         family reads as a **one**, so it would come up set rather than at the \
                         constant the design gives it",
                        netlist.instances[index].name, site.bel, site.tile.0, site.tile.1
                    ),
                });
            }
            for bit in &ff.sd_fabric {
                bits.set(site.tile, *bit)?;
            }
            for bit in &ff.lsrmode_lsr {
                bits.set(site.tile, *bit)?;
            }
            let regset = usize::from(value("REGSET", "RESET") == "SET");
            for bit in &ff.regset[regset] {
                bits.set(site.tile, *bit)?;
            }
            let gsr = usize::from(value("GSR", "ENABLED") == "DISABLED");
            for bit in &ff.gsr[gsr] {
                bits.set(site.tile, *bit)?;
            }
            // The enable mux: `1` unless a signal actually reaches the
            // enable pin, whatever the parameter says. A cell asking for
            // `CE` with nothing routed to `CE<c>_SLICE` would be a flop
            // that never clocks.
            let enabled = value("CEMUX", "1") == "CE" && signal_of("en").is_some();
            for bit in &ff.cemux[usize::from(enabled)] {
                bits.set(site.tile, *bit)?;
            }

            // The two shared control muxes. Which of the tile's two a flop
            // uses is a fact about the route, so it is asked of the route.
            let mux_of = |signal: Option<usize>, stem: &str| -> Option<usize> {
                let signal = signal?;
                let route = routing.route(signal)?;
                route.pips.iter().find_map(|id| {
                    let pip = graph.pip(*id);
                    let wire = graph.wire(pip.to);
                    if wire.tile != site.tile {
                        return None;
                    }
                    wire.name
                        .strip_prefix(stem)
                        .and_then(|n| n.parse::<usize>().ok())
                        .filter(|n| *n < 2)
                })
            };
            let refuse = |what: &str| TrellisError::Unsupported {
                what: format!(
                    "flip-flop `{}` at X{}Y{} asks for {what}, and the routing does not say which \
                     of the tile's two control muxes carries its signal, so writing it could \
                     change every other flop of the tile",
                    site.bel, site.tile.0, site.tile.1
                ),
            };
            // The other thing a control mux is shared with, which is not
            // another flip-flop. `CLK1.CLKMUX = INV` and `LSR1.LSRMUX =
            // INV` invert the **wire**, and a distributed RAM's write
            // clock and write enable are joined to `CLK1` and `LSR1` with
            // no mux of their own. A flop sharing those wires with a RAM —
            // which `fpga::place` allows, and which `ecppack` does 158
            // times in this board's own bitstreams for the clock — would
            // therefore invert the RAM's write port as a side effect, and
            // the memory would write on the wrong edge or the wrong level
            // while every structural check passed.
            let ram_here = || {
                netlist.instances.iter().enumerate().any(|(other, cell)| {
                    cell.kind == "lutram"
                        && placement
                            .site_of(other)
                            .is_some_and(|s| graph.sites[s].tile == site.tile)
                })
            };
            let shared_with_a_ram = |what: &str, wire: &str| TrellisError::Unsupported {
                what: format!(
                    "flip-flop `{}` at X{}Y{} asks for {what}, and its tile holds a distributed \
                     RAM whose write port is joined to `{wire}` with no mux of its own — so \
                     inverting that wire would invert the RAM's write port too, and the memory \
                     would write on the wrong edge or the wrong level with nothing to show for \
                     it. Either give the flip-flop the polarity it wants in logic instead, or \
                     keep it out of a RAM's tile with a placement constraint",
                    site.bel, site.tile.0, site.tile.1
                ),
            };
            if value("CLKMUX", "CLK") == "INV" {
                let c = mux_of(signal_of("clk"), "CLK").ok_or_else(|| refuse("CLKMUX=INV"))?;
                if c == 1 && ram_here() {
                    return Err(shared_with_a_ram("CLKMUX=INV", "CLK1"));
                }
                for bit in &ff.clkmux_inv[c] {
                    bits.set(site.tile, *bit)?;
                }
            }
            let reset = signal_of("rst");
            if reset.is_some() {
                let inv = value("LSRMUX", "LSR") == "INV";
                let async_reset = value("SRMODE", "LSR_OVER_CE") == "ASYNC";
                if inv || async_reset {
                    let what = if inv { "LSRMUX=INV" } else { "SRMODE=ASYNC" };
                    let c = mux_of(reset, "LSR").ok_or_else(|| refuse(what))?;
                    if inv {
                        if c == 1 && ram_here() {
                            return Err(shared_with_a_ram("LSRMUX=INV", "LSR1"));
                        }
                        for bit in &ff.lsrmux_inv[c] {
                            bits.set(site.tile, *bit)?;
                        }
                    }
                    if async_reset {
                        // `SRMODE` is a property of the *register*, not of
                        // the wire — it decides whether the reset is taken
                        // on the clock edge — so a RAM in the tile is not
                        // affected by it and it is written as before.
                        for bit in &ff.srmode_async[c] {
                            bits.set(site.tile, *bit)?;
                        }
                    }
                }
            }
            done += 1;
        }
        Ok(done)
    }

    /// The `.bit` stream for a bitmap this fabric's [`Arch`] produced.
    ///
    /// The metadata string is the one `ecppack` writes, because it is what
    /// a reader of the file expects to find and Lattice's own tools put
    /// the part there: `Part: <part>-<speed><package>`.
    ///
    /// # Errors
    ///
    /// [`super::bitstream::BitstreamError::OutOfRange`] when a tile bit
    /// belongs to no window of its position, which would mean the frame
    /// map and the `Arch` disagree about a tile's geometry.
    pub fn stream(
        &self,
        bits: &super::bitstream::Bitstream,
        speed: &str,
    ) -> Result<Ecp5Stream, super::bitstream::BitstreamError> {
        let mut cram = Cram::new(self.format);
        for (format, set) in bits.used_tiles() {
            for bit in set {
                let Some((frame, index)) = self.frames.locate(format.tile, bit) else {
                    return Err(super::bitstream::BitstreamError::OutOfRange {
                        tile: format.tile,
                        bit,
                        size: (format.rows, format.cols),
                    });
                };
                cram.set(frame, index);
            }
        }
        let mut stream = Ecp5Stream::new(self.format, self.idcode);
        stream.metadata = vec![format!("Part: {}-{speed}{}", self.part, self.package)];
        stream.cram = cram;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::memfile::MemoryFiles;

    /// A database small enough to write out, with the shapes the real one
    /// has that matter: a position holding two tiles, a pad whose bits are
    /// one column east of its buffer, and a logic tile with interconnect.
    fn tiny() -> MemoryFiles {
        let mut files = MemoryFiles::new();
        files.insert(
            "devices.json",
            r#"{"families":{"ECP5":{"devices":{"LFE5U-12F":{
                "packages":["caBGA256"],"idcode":"0x21111043",
                "frames":40,"bits_per_frame":16,
                "pad_bits_after_frame":0,"pad_bits_before_frame":0,
                "max_row":1,"max_col":2}}}}}"#,
        );
        // (0,0) is the buffer: a `PIOT0`, which owns the three wires a PIO
        // presents to the fabric and names the `CIB` wire that ties them.
        // (1,0) holds its *bits*, one column east. (1,1) holds a CIB and a
        // PICT1, in that order, because that is how the names sort. (0,1) is
        // a logic tile and (2,0) the bank's reference tile, which no pad
        // owns and every pad needs.
        files.insert(
            "ECP5/LFE5U-12F/tilegrid.json",
            r#"{
              "MIB_R0C0:PIOT0": {"type":"PIOT0","start_frame":0,"start_bit":0,
                                 "cols":20,"rows":1,"sites":[]},
              "R1C0:PLC2":      {"type":"PLC2","start_frame":0,"start_bit":0,
                                 "cols":40,"rows":16,"sites":[]},
              "MIB_R0C1:PIOT1": {"type":"PIOT1","start_frame":0,"start_bit":0,
                                 "cols":20,"rows":1,"sites":[]},
              "CIB_R1C1:CIB":   {"type":"CIB","start_frame":0,"start_bit":1,
                                 "cols":20,"rows":11,"sites":[]},
              "MIB_R1C1:PICT1": {"type":"PICT1","start_frame":0,"start_bit":12,
                                 "cols":20,"rows":1,"sites":[]},
              "MIB_R0C2:BANKREF1": {"type":"BANKREF1","start_frame":20,"start_bit":0,
                                    "cols":20,"rows":1,"sites":[]}
            }"#,
        );
        // `pio_metadata` sits beside `packages` and is the only statement
        // of which bank a PIO is in.
        files.insert(
            "ECP5/LFE5U-12F/iodb.json",
            r#"{"packages":{"CABGA256":{"E13":{"row":0,"col":0,"pio":"B"},
                                        "Z99":{"row":1,"col":0,"pio":"A"}}},
                "pio_metadata":[{"row":0,"col":0,"pio":"B","bank":1},
                                {"row":1,"col":0,"pio":"A","bank":7}]}"#,
        );
        // The clock network's geometry. One quadrant, one tap column and one
        // spine, which is the smallest shape `parse::globals` accepts and
        // enough for `clock_network` to have something to resolve — and the
        // fixture declares no `G_…PCLK…` wire, so the result is a network
        // with no indices and no joins, which is the other case worth
        // having: a die whose database says nothing about a clock must load.
        files.insert(
            "ECP5/LFE5U-12F/globals.json",
            r#"{"quadrants":{"UL":{"x0":0,"y0":0,"x1":2,"y1":1}},
                "taps":{"C1":{"lx0":0,"lx1":0,"rx0":1,"rx1":2}},
                "spines":{"UL1":{"x":0,"y":1}}}"#,
        );
        files.insert(
            "ECP5/tiledata/BANKREF1/bits.db",
            "# Non-Routing Configuration\n\
             .config_enum BANK.VCCIO NONE\n\
             3V3 F18B0\n\
             2V5 F17B0\n\
             NONE -\n",
        );
        // The buffer's tile: no bits of its own in this fixture, only the
        // fixed connections that say which wires it presents and where its
        // constant muxes are. `S1E1_JA0` from (0,0) is the `JA0` of (1,1).
        files.insert(
            "ECP5/tiledata/PIOT0/bits.db",
            "# Fixed Connections\n\
             .fixed_conn PADDOB_PIO JPADDOB\n\
             .fixed_conn PADDTB_PIO JPADDTB\n\
             .fixed_conn JPADDOB S1E1_JA0\n\
             .fixed_conn JPADDTB S1E1_JB0\n\
             .fixed_conn JDIB JPADDIB_PIO\n",
        );
        files.insert(
            "ECP5/tiledata/PIOT1/bits.db",
            "# Non-Routing Configuration\n\
             .config_enum PIOB.BASE_TYPE NONE\n\
             NONE F2B0\n\
             INPUT_LVCMOS33 F2B0 F9B0\n\
             OUTPUT_LVCMOS33 F2B0 F7B0 !F8B0\n\
             BIDIR_LVCMOS33 F2B0 F7B0 F9B0 F10B0\n\
             \n\
             .config_enum PIOB.HYSTERESIS OFF\n\
             OFF !F10B0\n\
             ON F10B0\n\
             \n\
             .config_enum PIOB.PULLMODE DOWN\n\
             DOWN !F11B0 !F12B0\n\
             NONE !F11B0 F12B0\n\
             UP F11B0 F12B0\n",
        );
        files.insert(
            "ECP5/tiledata/PICT1/bits.db",
            "# Non-Routing Configuration\n\
             .config_enum PIOB.BASE_TYPE INPUT_LVCMOS12\n\
             INPUT_LVCMOS12 -\n\
             INPUT_LVCMOS33 -\n\
             OUTPUT_LVCMOS33 F5B0 F6B0\n\
             BIDIR_LVCMOS33 F5B0 F6B0\n",
        );
        files.insert(
            "ECP5/tiledata/CIB/bits.db",
            "# Routing Mux Bits\n\
             .mux JA0\n\
             W1S1_F0 F0B5\n\
             \n\
             # Non-Routing Configuration\n\
             .config_enum CIB.JA0MUX JA0\n\
             0 F9B10\n\
             1 F1B3 F3B2\n\
             JA0 -\n\
             \n\
             .config_enum CIB.JB0MUX JB0\n\
             0 F4B9\n\
             1 F2B4\n\
             JB0 -\n",
        );
        // A logic tile, with just enough to route through: one lookup table
        // whose output mux has a **bitless** source — which is the usual
        // case and the one this loader used to drop — and whose inputs come
        // off the tile's own interconnect.
        files.insert(
            "ECP5/tiledata/PLC2/bits.db",
            "# Routing Mux Bits\n\
             .mux A0\n\
             N1E1_JDIB F4B2\n\
             \n\
             .mux F0\n\
             F0_SLICE -\n\
             \n\
             # Non-Routing Configuration\n\
             .config SLICEA.K0.INIT 1111111111111111\n\
             !F25B1\n\
             !F24B1\n\
             !F23B1\n\
             !F22B1\n\
             !F21B1\n\
             !F20B1\n\
             !F19B1\n\
             !F18B1\n\
             !F17B1\n\
             !F16B1\n\
             !F15B1\n\
             !F14B1\n\
             !F13B1\n\
             !F12B1\n\
             !F11B1\n\
             !F10B1\n\
             \n\
             .config_enum SLICEA.A0MUX A0\n\
             1 F5B1\n\
             A0 -\n\
             \n\
             .config_enum SLICEA.B0MUX B0\n\
             1 F6B1\n\
             B0 -\n\
             \n\
             .config_enum SLICEA.C0MUX C0\n\
             1 F7B1\n\
             C0 -\n\
             \n\
             .config_enum SLICEA.D0MUX D0\n\
             1 F8B1\n\
             D0 -\n\
             \n\
             # Fixed Connections\n\
             .fixed_conn A0_SLICE A0\n\
             .fixed_conn B0_SLICE B0\n\
             .fixed_conn C0_SLICE C0\n\
             .fixed_conn D0_SLICE D0\n",
        );
        files
    }

    /// The read-address permutation, against nextpnr's own table.
    ///
    /// `dram_to_comb` builds the lookup table's contents by walking the
    /// sixteen truth-table indices and asking which RAM address each one
    /// is: `i & 1` (the `A` input) becomes address bit 3, `i & 2` (`B`)
    /// stays bit 1, `i & 4` (`C`) stays bit 2 and `i & 8` (`D`) becomes bit
    /// 0. [`dpram_init_word`] is that map read the other way — address to
    /// truth-table index — and the point of this test is that the two
    /// agree, because the function is used in the direction nextpnr does
    /// not.
    ///
    /// It is its own inverse, which is worth pinning: a permutation that
    /// was wrong but self-inverse would still pass a round-trip test, and a
    /// flow that applied it once too often or once too few would read every
    /// word from the wrong place with nothing to show for it.
    #[test]
    fn a_distributed_rams_contents_are_addressed_the_way_nextpnr_permutes_them() {
        for index in 0..16usize {
            // nextpnr's `dram_to_comb`, transcribed.
            let mut address = 0usize;
            if index & 1 != 0 {
                address |= 8;
            }
            if index & 2 != 0 {
                address |= 2;
            }
            if index & 4 != 0 {
                address |= 4;
            }
            if index & 8 != 0 {
                address |= 1;
            }
            assert_eq!(
                dpram_init_word(address),
                index,
                "address {address} is truth-table bit {index}"
            );
        }
        // Its own inverse, and a permutation: every index once.
        let mut seen = [false; 16];
        for address in 0..16usize {
            let bit = dpram_init_word(address);
            assert_eq!(dpram_init_word(bit), address);
            assert!(!seen[bit], "{bit} twice");
            seen[bit] = true;
        }
        assert!(seen.iter().all(|s| *s));
        // The two ends that are not their own fixed point, spelled out so a
        // change to the function has to change this line too.
        assert_eq!(dpram_init_word(1), 8, "address bit 0 is the `D` input");
        assert_eq!(dpram_init_word(8), 1, "address bit 3 is the `A` input");
        assert_eq!(dpram_init_word(0), 0);
        assert_eq!(dpram_init_word(6), 6, "`B` and `C` stay where they are");
    }

    /// The wire table of a distributed RAM: one role per pin of the
    /// primitive the device file declares, and the read address on all four
    /// of the lookup tables that hold the contents.
    #[test]
    fn a_distributed_rams_pins_cover_the_primitive_and_its_four_lookup_tables() {
        let roles: BTreeSet<&str> = DPRAM_PINS.iter().map(|(role, _)| *role).collect();
        let mut wanted: BTreeSet<&str> = ["wclk", "we"].into_iter().collect();
        for k in 0..4 {
            for base in ["raddr", "waddr", "din", "dout"] {
                wanted.insert(match (base, k) {
                    ("raddr", 0) => "raddr0",
                    ("raddr", 1) => "raddr1",
                    ("raddr", 2) => "raddr2",
                    ("raddr", 3) => "raddr3",
                    ("waddr", 0) => "waddr0",
                    ("waddr", 1) => "waddr1",
                    ("waddr", 2) => "waddr2",
                    ("waddr", 3) => "waddr3",
                    ("din", 0) => "din0",
                    ("din", 1) => "din1",
                    ("din", 2) => "din2",
                    ("din", 3) => "din3",
                    ("dout", 0) => "dout0",
                    ("dout", 1) => "dout1",
                    ("dout", 2) => "dout2",
                    _ => "dout3",
                });
            }
        }
        assert_eq!(
            roles, wanted,
            "every port of `TRELLIS_DPR16X4` and no other"
        );
        // Four wires per read-address bit, one per lookup table holding the
        // contents; two per clock and enable, one per RAM slice; one each
        // for the rest.
        let count = |role: &str| DPRAM_PINS.iter().filter(|(r, _)| *r == role).count();
        for k in 0..4 {
            assert_eq!(count(&format!("raddr{k}")), 4, "raddr{k}");
            assert_eq!(count(&format!("waddr{k}")), 1, "waddr{k}");
            assert_eq!(count(&format!("din{k}")), 1, "din{k}");
            assert_eq!(count(&format!("dout{k}")), 1, "dout{k}");
        }
        assert_eq!(count("wclk"), 2, "slice A's and slice B's write clock");
        assert_eq!(count("we"), 2, "and their write enables");
        // The write port is on the `RAMW` slice, which is slice C: `z` 4
        // and 5 of the tile's eight lookup tables.
        for (role, wire) in DPRAM_PINS {
            if role.starts_with("waddr") {
                assert!(wire.ends_with("4_SLICE"), "{role} -> {wire}");
            }
            if role.starts_with("din") {
                assert!(wire.ends_with("5_SLICE"), "{role} -> {wire}");
            }
        }
        // And the six bels it consumes are the lookup tables of those three
        // slices, never slice D's.
        assert_eq!(DPRAM_BLOCKS.len(), 6);
        assert!(!DPRAM_BLOCKS.iter().any(|b| b.starts_with("SLICED")));
        assert!(!DPRAM_BLOCKS.iter().any(|b| b.contains(".FF")));
    }

    #[test]
    fn a_position_with_several_tiles_addresses_them_end_to_end() {
        let db = open(&tiny(), "", "LFE5U-12F").unwrap();
        assert_eq!(db.device().name, "LFE5U-12F");
        assert_eq!(db.device().idcode, 0x2111_1043);
        assert_eq!(db.size(), (6, 6));
        assert_eq!(db.packages(), vec!["CABGA256"]);

        let fabric = db.load(&TrellisOptions::new()).unwrap();
        // The grid is `max_col + 1` by `max_row + 1`.
        assert_eq!((fabric.arch.width, fabric.arch.height), (3, 2));
        // Five of the six positions hold tiles.
        assert_eq!(fabric.stats.positions, 5);
        assert_eq!(fabric.stats.shared_positions, 1);
        assert_eq!(fabric.stats.most_windows, 2);

        // Position (1, 1) is `CIB+PICT1`: 20 frames each, laid end to end.
        let ty = fabric.arch.tile_at(1, 1).unwrap();
        assert_eq!(ty.name, "CIB+PICT1");
        assert_eq!(ty.bit_rows, 40, "two windows of twenty frames");
        assert_eq!(ty.bit_cols, 11, "the widest window's bit count");
        // And (1, 0) is the pad tile alone.
        assert_eq!(fabric.arch.tile_at(1, 0).unwrap().name, "PIOT1");
        assert_eq!(fabric.arch.tile_at(1, 0).unwrap().bit_rows, 20);
        // (2, 1) holds no tile at all, which `Arch` says with `None` rather
        // than with an empty type.
        assert!(fabric.arch.tile_at(2, 1).is_none());
        // And (2, 0) is the bank reference, which is a tile like any
        // other and holds no bel.
        assert_eq!(fabric.arch.tile_at(2, 0).unwrap().name, "BANKREF1");
        assert!(fabric.arch.tile_at(2, 0).unwrap().bels.is_empty());

        // The frame map resolves a bit of the *second* window back to the
        // right place: row 20 is the `PICT1`'s frame 0, at its own bit
        // offset of 12.
        assert_eq!(
            fabric.frames.locate((1, 1), ConfigBit::new(20, 0)),
            Some((0, 12))
        );
        // And row 0 is the `CIB`'s frame 0, at bit offset 1.
        assert_eq!(
            fabric.frames.locate((1, 1), ConfigBit::new(0, 0)),
            Some((0, 1))
        );
        // A row past the last window belongs to nothing.
        assert_eq!(fabric.frames.locate((1, 1), ConfigBit::new(40, 0)), None);
    }

    #[test]
    fn a_top_edge_pads_bits_are_one_column_east_of_its_buffer() {
        let fabric = open(&tiny(), "", "LFE5U-12F")
            .unwrap()
            .load(&TrellisOptions::new())
            .unwrap();
        // `E13` is at column 0 on side B, so its **bits** are at column 1
        // and its **buffer** is at column 0. The ball on row 1 is on no edge
        // this describes and is left out.
        assert_eq!(fabric.stats.balls, 2);
        assert_eq!(fabric.stats.pads, 1);
        assert_eq!(fabric.stats.pads_skipped, 1);
        let pad = fabric.pad("E13").unwrap();
        assert_eq!((pad.bel, pad.side, pad.edge), ((0, 0), 'B', Edge::Top));
        assert_eq!(pad.pad_at, (1, 0));
        assert_eq!(pad.pic_at, (1, 1));
        // And the `CIB` came from the buffer's own `JPADDOB <- S1E1_JA0`,
        // not from a constant in this file.
        assert_eq!(pad.cib_at, (1, 1));

        // The pad tile's bits are the enum's, with the ones it wants
        // *clear* left out: a bitmap starts zeroed.
        assert_eq!(
            pad.output_pad_bits,
            vec![ConfigBit::new(2, 0), ConfigBit::new(7, 0)],
            "`!F8B0` is not recorded"
        );
        assert_eq!(
            pad.input_pad_bits,
            vec![ConfigBit::new(2, 0), ConfigBit::new(9, 0)]
        );
        // A bidirectional pad is its own pattern and not the union of the
        // other two: it takes the output's drive bit and the input's, and
        // the fixture spells it that way because the real database does.
        assert_eq!(
            pad.bidir_pad_bits,
            vec![
                ConfigBit::new(2, 0),
                ConfigBit::new(7, 0),
                ConfigBit::new(9, 0),
                ConfigBit::new(10, 0)
            ]
        );
        // An input costs two settings an output does not, and `PULLMODE`'s
        // default is a pull-*down*, so `NONE` is a bit that has to be set.
        assert_eq!(pad.hysteresis_bits, vec![ConfigBit::new(10, 0)]);
        assert_eq!(pad.pull_bits(PULL_NONE), [ConfigBit::new(12, 0)]);
        assert_eq!(
            pad.pull_bits(PULL_UP),
            [ConfigBit::new(11, 0), ConfigBit::new(12, 0)]
        );
        // The field's *default* locates to nothing, which is the whole
        // reason leaving it alone is not the same as having no pull.
        assert!(pad.pull_bits("DOWN").is_empty());
        assert!(pad.pull_bits("SIDEWAYS").is_empty());
        // The tile one row south holds two windows and the `PICT1` is the
        // second, so its `F5B0` is row 20 + 5. An input costs nothing there;
        // a bidirectional pad costs what an output costs.
        assert_eq!(
            pad.output_pic_bits,
            vec![ConfigBit::new(25, 0), ConfigBit::new(26, 0)]
        );
        assert_eq!(pad.bidir_pic_bits, pad.output_pic_bits);
        assert!(pad.input_pic_bits.is_empty());
        // The `CIB` is the *first* window of that position, so its bits
        // keep their own frame numbers.
        assert_eq!(pad.low_bits, vec![ConfigBit::new(9, 10)]);
        assert_eq!(
            pad.high_bits,
            vec![ConfigBit::new(1, 3), ConfigBit::new(3, 2)]
        );
        assert_eq!(pad.enable_bits, vec![ConfigBit::new(4, 9)]);
        // Tying high and tying low are different bits, which is the whole
        // point of recording both.
        assert_ne!(pad.low_bits, pad.high_bits);

        // The ball map names the site the placer will fix a constrained
        // cell at, and it is at the **buffer's** position.
        assert_eq!(fabric.arch.site_of_pin("E13"), Some("X0Y0/PIOB"));
        assert_eq!(fabric.arch.site_of_pin("Z99"), None);
        assert_eq!(
            fabric.pad_of_site("X0Y0/PIOB").map(|p| &p.ball[..]),
            Some("E13")
        );
        assert_eq!(fabric.pad_of_site("X1Y0/PIOB"), None);
        assert_eq!(fabric.pad_of_site("X0Y1/PIOB"), None);

        let graph = fabric.arch.build_graph();
        let site = graph.site("X0Y0/PIOB").expect("the bel becomes a site");
        // Three pins, and the data one is what carries a constant.
        assert!(site.pin("dout").is_some());
        assert!(site.pin("oe").is_some());
        assert!(site.pin("din").is_some());
        assert!(site.pin("pad").is_none(), "a ball is not a wire");
    }

    /// The interconnect, at a scale small enough to count by hand.
    ///
    /// Two things are pinned here that the real database cannot pin
    /// cheaply. **A bitless `.mux` source is a connection**: the lookup
    /// table's output reaches `F0` through one, and it is the only way it
    /// reaches the fabric at all. And **a wire belongs to the position its
    /// prefix points at**: `S1E1_JA0` in the buffer's tile is the `JA0` of
    /// (1, 1), so the pad's `dout` pin and the `CIB`'s mux are the same
    /// metal.
    #[test]
    fn the_interconnect_is_pips_from_muxes_and_fixed_connections() {
        let fabric = open(&tiny(), "", "LFE5U-12F")
            .unwrap()
            .load(&TrellisOptions::new())
            .unwrap();
        let stats = fabric.stats;
        // The `PLC2` declares two mux sources and the `CIB` one; the four
        // `.fixed_conn`s of the buffer and the four of the logic tile are
        // the rest.
        assert_eq!(stats.arcs, 3);
        assert_eq!(stats.fixed, 9);
        assert_eq!(stats.luts, 1, "one lookup table has an `INIT` word here");

        let graph = fabric.arch.build_graph();
        // A bitless source is a pip with no bits, which is what the model
        // already means by a connection that is always there.
        let bitless = graph
            .pips
            .iter()
            .enumerate()
            .filter(|(id, _)| graph.pip_bits(u32::try_from(*id).unwrap()).is_empty())
            .count();
        assert!(bitless > 0, "a `.mux` source with no bits is still an arc");

        // `F0_SLICE -> F0` exists and carries no bits.
        let f0_slice = graph
            .nodes
            .iter()
            .position(|w| w.name == "F0_SLICE" && w.tile == (0, 1))
            .expect("the lookup table's output wire");
        let out = graph.outgoing(u32::try_from(f0_slice).unwrap());
        assert_eq!(out.len(), 1);
        assert!(graph.pip_bits(out[0]).is_empty());
        assert_eq!(graph.wire(graph.pip(out[0]).to).name, "F0");

        // And the pad's `dout` pin reaches the same node the `CIB`'s mux
        // drives, one column east and one row south of the buffer.
        let site = graph.site("X0Y0/PIOB").unwrap();
        let dout = site.pin("dout").unwrap();
        // `PADDOB_PIO <- JPADDOB <- S1E1_JA0`: two hops back to (1, 1).
        let first = graph.incoming(dout);
        assert_eq!(first.len(), 1);
        let jpaddob = graph.pip(first[0]).from;
        let second = graph.incoming(jpaddob);
        assert_eq!(second.len(), 1);
        let ja0 = graph.wire(graph.pip(second[0]).from);
        assert_eq!((ja0.name.as_str(), ja0.tile), ("JA0", (1, 1)));
    }

    #[test]
    fn the_identifier_is_checked_because_two_parts_share_a_die() {
        let fabric = open(&tiny(), "", "LFE5U-12F")
            .unwrap()
            .load(&TrellisOptions::new())
            .unwrap();
        assert!(fabric.check_idcode(0x2111_1043).is_ok());
        let err = fabric.check_idcode(0x4111_1043).unwrap_err();
        // The LFE5U-25F: the same die, a different identifier.
        assert!(err.to_string().contains("0x41111043"), "{err}");
    }

    #[test]
    fn a_database_that_is_not_there_says_which_file_is_missing() {
        let mut files = MemoryFiles::new();
        let err = open(&files, "", "LFE5U-12F").unwrap_err();
        assert!(matches!(err, TrellisError::Missing { .. }));
        assert!(err.to_string().contains("devices.json"), "{err}");
        assert!(err.to_string().contains("reticle fetch"), "{err}");

        files.insert(
            "devices.json",
            r#"{"families":{"ECP5":{"devices":{"LFE5U-85F":{
                "packages":[],"idcode":"0x41113043","frames":1,"bits_per_frame":8,
                "pad_bits_after_frame":0,"pad_bits_before_frame":0,
                "max_row":0,"max_col":0}}}}}"#,
        );
        let err = open(&files, "", "LFE5U-12F").unwrap_err();
        assert!(matches!(err, TrellisError::NoSuchDevice { .. }));
        assert!(err.to_string().contains("LFE5U-85F"), "{err}");

        // Every file is named as it is reached, in the order they are
        // read, so what is missing is always the *next* thing needed
        // rather than the first thing looked at.
        files.insert(
            "ECP5/LFE5U-85F/tilegrid.json",
            r#"{"R1C1:PLC2":{"type":"PLC2","start_frame":0,"start_bit":0,
                             "cols":1,"rows":8,"sites":[]}}"#,
        );
        let err = open(&files, "", "LFE5U-85F").unwrap_err();
        assert!(err.to_string().contains("LFE5U-85F/iodb.json"), "{err}");
        files.insert("ECP5/LFE5U-85F/iodb.json", r#"{"packages":{}}"#);
        let err = open(&files, "", "LFE5U-85F").unwrap_err();
        assert!(err.to_string().contains("LFE5U-85F/globals.json"), "{err}");
        files.insert(
            "ECP5/LFE5U-85F/globals.json",
            r#"{"quadrants":{},"taps":{},"spines":{}}"#,
        );
        let err = open(&files, "", "LFE5U-85F").unwrap_err();
        assert!(err.to_string().contains("tiledata/PLC2/bits.db"), "{err}");
    }

    #[test]
    fn a_package_that_is_not_in_the_map_is_refused_by_name() {
        let db = open(&tiny(), "", "LFE5U-12F").unwrap();
        let mut options = TrellisOptions::new();
        options.package = "CABGA381".to_owned();
        let err = db.load(&options).unwrap_err();
        assert!(matches!(err, TrellisError::NoSuchPackage { .. }));
        assert!(err.to_string().contains("CABGA256"), "{err}");

        // And a standard no `BASE_TYPE` offers: every pad is refused, so
        // there is nothing to place, and the message says which standards
        // there are rather than "no pads".
        let mut options = TrellisOptions::new();
        options.io_standard = "LVCMOS99".to_owned();
        let err = db.load(&options).unwrap_err();
        assert!(matches!(err, TrellisError::NoSuchIoStandard { .. }));
        assert!(err.to_string().contains("OUTPUT_LVCMOS33"), "{err}");
    }

    #[test]
    fn a_stream_is_the_bitmap_at_the_absolute_frames_the_grid_gives() {
        let fabric = open(&tiny(), "", "LFE5U-12F")
            .unwrap()
            .load(&TrellisOptions::new())
            .unwrap();
        let mut bits = super::super::bitstream::Bitstream::empty(
            super::super::bitstream::BitstreamFormat::from_arch(&fabric.arch),
        );
        let pad = fabric.pad("E13").unwrap().clone();
        for bit in &pad.output_pad_bits {
            bits.set(pad.pad_at, *bit).unwrap();
        }
        for bit in &pad.output_pic_bits {
            bits.set(pad.pic_at, *bit).unwrap();
        }
        let stream = fabric.stream(&bits, "8").unwrap();
        assert_eq!(stream.idcode, 0x2111_1043);
        assert_eq!(stream.metadata, vec!["Part: LFE5U-12F-8CABGA256"]);
        // Four bits set, at the absolute places the windows put them.
        assert_eq!(stream.cram.count_ones(), 4);
        assert!(stream.cram.get(2, 0), "the pad tile's F2B0");
        assert!(stream.cram.get(7, 0));
        assert!(stream.cram.get(5, 12), "the PICT1's F5B0 at bit offset 12");
        assert!(stream.cram.get(6, 12));
        // And the file round trips, which is the only check that the
        // container and the map agree.
        let bytes = stream.to_bytes(true);
        let back = Ecp5Stream::parse(&bytes, &|id| {
            (id == 0x2111_1043).then(|| FrameFormat::new(40, 16))
        })
        .unwrap();
        assert_eq!(back.cram, stream.cram);
    }

    /// The measurements are what a document quotes, so they are produced
    /// rather than written down.
    #[test]
    fn the_statistics_count_what_was_read() {
        let fabric = open(&tiny(), "", "LFE5U-12F")
            .unwrap()
            .load(&TrellisOptions::new())
            .unwrap();
        let text = fabric.stats.to_text();
        assert!(text.contains("tiles: 6"), "{text}");
        assert!(text.contains("grid positions: 5"), "{text}");
        assert!(text.contains("configuration bits: 640"), "{text}");
        assert!(text.contains("pads declared: 1"), "{text}");
        assert!(text.contains("lookup tables: 1"), "{text}");
        assert!(text.contains("programmable connections: 3"), "{text}");
        assert_eq!(fabric.stats.frames, 40);
        assert_eq!(fabric.stats.bits_per_frame, 16);
    }

    /// Two PIOs of one pair, with **the real patterns** an `LFE5U-12F`'s
    /// `PICR1` has for sides A and B, in a tile ten frames by five bits
    /// because that is all those patterns need.
    ///
    /// Copied out of `ECP5/tiledata/PICR1/bits.db` rather than invented, so
    /// that the test below is about the ECP5 and not about a fixture. The
    /// three things it has to carry are:
    ///
    /// - `PIOA.BASE_TYPE = OUTPUT_LVCMOS33D`, whose ten bits include four
    ///   — `F0B3`, `F1B3`, `F8B3`, `F9B4` — that belong to **side B**;
    /// - `PIOA.BASE_TYPE = BIDIR_LVCMOS33`, whose eight are all side A's,
    ///   two of them (`F5B0`, `F6B0`) wanted by no other feature at all;
    /// - the neighbours that account for those four either way:
    ///   `PIOB.BASE_TYPE`, `PIOB.PULLMODE`, `PIOB.DRIVE` and
    ///   `PIOB.OPENDRAIN`. Side B has no pseudo-differential value of its
    ///   own, and that is not an omission: a differential pair is A over B,
    ///   so only the `A` of a pair (and the `C` of the other) has one.
    fn one_pad_tile() -> MemoryFiles {
        let mut files = MemoryFiles::new();
        files.insert(
            "devices.json",
            r#"{"families":{"ECP5":{"devices":{"LFE5U-12F":{
                "packages":["caBGA256"],"idcode":"0x21111043",
                "frames":10,"bits_per_frame":5,
                "pad_bits_after_frame":0,"pad_bits_before_frame":0,
                "max_row":0,"max_col":0}}}}}"#,
        );
        files.insert(
            "ECP5/LFE5U-12F/tilegrid.json",
            r#"{"MIB_R0C0:PICR1": {"type":"PICR1","start_frame":0,"start_bit":0,
                                   "cols":10,"rows":5,"sites":[]}}"#,
        );
        files.insert(
            "ECP5/LFE5U-12F/iodb.json",
            r#"{"packages":{"CABGA256":{"F16":{"row":0,"col":0,"pio":"A"}}},
                "pio_metadata":[{"row":0,"col":0,"pio":"A","bank":3}]}"#,
        );
        files.insert(
            "ECP5/LFE5U-12F/globals.json",
            r#"{"quadrants":{"UL":{"x0":0,"y0":0,"x1":0,"y1":0}},
                "taps":{},"spines":{}}"#,
        );
        files.insert(
            "ECP5/tiledata/PICR1/bits.db",
            "# Non-Routing Configuration\n\
             .config_enum PIOA.BASE_TYPE NONE\n\
             BIDIR_LVCMOS33 F0B0 F3B1 F4B1 F5B0 F5B1 F6B0 F6B1 F7B0\n\
             INPUT_LVCMOS33 F0B0 F5B0 F6B0 F6B1 F7B0\n\
             NONE F7B0\n\
             OUTPUT_LVCMOS33 F0B0 F2B0 F3B1 F4B1 F5B1 F7B0\n\
             OUTPUT_LVCMOS33D F0B0 F0B3 F1B3 F2B0 F3B1 F4B1 F5B1 F7B0 F8B3 F9B4\n\
             \n\
             .config_enum PIOA.DRIVE\n\
             12 !F1B1 F2B1 !F3B1 !F4B1 !F5B1\n\
             16 !F1B1 F2B1 F3B1 F4B1 F5B1\n\
             4 F1B1 F2B1 F3B1 !F4B1 !F5B1\n\
             8 !F1B1 !F2B1 F3B1 F4B1 F5B1\n\
             \n\
             .config_enum PIOA.HYSTERESIS OFF\n\
             OFF !F6B1\n\
             ON F6B1\n\
             \n\
             .config_enum PIOA.OPENDRAIN\n\
             OFF F3B1 F4B1 !F4B2 F5B1\n\
             ON !F3B1 !F4B1 F4B2 !F5B1\n\
             \n\
             .config_enum PIOA.PULLMODE DOWN\n\
             DOWN !F1B0 !F2B0\n\
             NONE !F1B0 F2B0\n\
             UP F1B0 F2B0\n\
             \n\
             .config_enum PIOB.BASE_TYPE NONE\n\
             BIDIR_LVCMOS33 F0B3 F1B2 F1B3 F2B2 F2B3 F3B2 F6B3 F9B4\n\
             INPUT_LVCMOS33 F1B2 F2B2 F2B3 F3B2 F6B3\n\
             NONE F3B2\n\
             OUTPUT_LVCMOS33 F0B3 F1B3 F3B2 F6B3 F8B3 F9B4\n\
             \n\
             .config_enum PIOB.DRIVE\n\
             12 !F0B3 !F1B3 !F7B4 F8B4 !F9B4\n\
             16 F0B3 F1B3 !F7B4 F8B4 F9B4\n\
             4 !F0B3 !F1B3 F7B4 F8B4 F9B4\n\
             8 F0B3 F1B3 !F7B4 !F8B4 F9B4\n\
             \n\
             .config_enum PIOB.HYSTERESIS OFF\n\
             OFF !F2B3\n\
             ON F2B3\n\
             \n\
             .config_enum PIOB.OPENDRAIN\n\
             OFF F0B3 !F0B4 F1B3 F9B4\n\
             ON !F0B3 F0B4 !F1B3 !F9B4\n\
             \n\
             .config_enum PIOB.PULLMODE DOWN\n\
             DOWN !F7B3 !F8B3\n\
             NONE !F7B3 F8B3\n\
             UP F7B3 F8B3\n",
        );
        files
    }

    /// A configuration memory with exactly the bits named, and nothing
    /// else — the shape `configure_io` builds, which is bits OR-ed into a
    /// zeroed bitmap.
    fn image(bits: &[(u32, u32)]) -> Cram {
        let mut cram = Cram::new(FrameFormat::new(10, 5));
        for (frame, bit) in bits {
            cram.set(*frame, *bit);
        }
        cram
    }

    /// The bits of one `PIO<s>.BASE_TYPE` value or one `PIO<s>.PULLMODE`
    /// value, so a test reads as a list of settings rather than as a list
    /// of coordinates.
    fn pattern(name: &str) -> Vec<(u32, u32)> {
        let spelled: &[(&str, &[(u32, u32)])] = &[
            (
                "A BIDIR",
                &[
                    (0, 0),
                    (3, 1),
                    (4, 1),
                    (5, 0),
                    (5, 1),
                    (6, 0),
                    (6, 1),
                    (7, 0),
                ],
            ),
            (
                "A OUTPUT",
                &[(0, 0), (2, 0), (3, 1), (4, 1), (5, 1), (7, 0)],
            ),
            ("A PULL NONE", &[(2, 0)]),
            ("A PULL UP", &[(1, 0), (2, 0)]),
            (
                "B BIDIR",
                &[
                    (0, 3),
                    (1, 2),
                    (1, 3),
                    (2, 2),
                    (2, 3),
                    (3, 2),
                    (6, 3),
                    (9, 4),
                ],
            ),
            (
                "B OUTPUT",
                &[(0, 3), (1, 3), (3, 2), (6, 3), (8, 3), (9, 4)],
            ),
            ("B PULL NONE", &[(8, 3)]),
            ("B PULL UP", &[(7, 3), (8, 3)]),
        ];
        spelled
            .iter()
            .find(|(what, _)| *what == name)
            .unwrap_or_else(|| panic!("no pattern called `{name}`"))
            .1
            .to_vec()
    }

    /// The rule that decides between two values of one field, and the
    /// reason it is not simply "the longest match".
    ///
    /// Four images of one pad tile, each the whole of what `configure_io`
    /// would write for it. The first is the one that was refused before
    /// this rule existed, and the other three are what had to keep
    /// working: the same pad alone, an ordinary output, and the case the
    /// rule deliberately does **not** touch.
    #[test]
    fn a_field_is_read_as_the_value_that_leaves_fewest_bits_unexplained() {
        let db = open(&one_pad_tile(), "", "LFE5U-12F").unwrap();
        let read = |bits: &[(u32, u32)]| -> (Vec<String>, usize) {
            let decoded = db.decode(&image(bits));
            (
                decoded
                    .enums
                    .iter()
                    .map(|(_, field, value)| format!("{field}={value}"))
                    .collect(),
                decoded.unexplained,
            )
        };
        let all = |names: &[&str]| -> Vec<(u32, u32)> {
            let mut out: Vec<(u32, u32)> = names.iter().flat_map(|n| pattern(n)).collect();
            out.sort_unstable();
            out.dedup();
            out
        };

        // BOTH HALVES BIDIRECTIONAL, which is what an eight-bit bus on this
        // edge comes to and what the longest match could not read: all ten
        // bits of side A's `OUTPUT_LVCMOS33D` are set, because side B's own
        // base type and pull mode set the four that are B's.
        let both = all(&["A BIDIR", "A PULL UP", "B BIDIR", "B PULL UP"]);
        for bit in [(0, 3), (1, 3), (8, 3), (9, 4)] {
            assert!(
                both.contains(&bit),
                "F{}B{} is one of the four side-B bits `OUTPUT_LVCMOS33D` needs; without it \
                 this test proves nothing",
                bit.0,
                bit.1
            );
        }
        let (fields, left) = read(&both);
        assert_eq!(left, 0, "{fields:?}");
        assert!(
            fields.contains(&"PIOA.BASE_TYPE=BIDIR_LVCMOS33".to_owned()),
            "side A read back as something else: {fields:?}"
        );
        assert!(
            fields.contains(&"PIOB.BASE_TYPE=BIDIR_LVCMOS33".to_owned()),
            "side B read back as something else: {fields:?}"
        );
        assert!(
            fields.contains(&"PIOA.PULLMODE=UP".to_owned())
                && fields.contains(&"PIOB.PULLMODE=UP".to_owned()),
            "the pull the released level depends on: {fields:?}"
        );

        // ONE HALF BIDIRECTIONAL, which is the milestone before this one and
        // decoded before it too: `OUTPUT_LVCMOS33D` does not even match, so
        // there is nothing for the rule to do and it must not invent any.
        let (fields, left) = read(&all(&["A BIDIR", "A PULL UP"]));
        assert_eq!(left, 0, "{fields:?}");
        assert!(
            fields.contains(&"PIOA.BASE_TYPE=BIDIR_LVCMOS33".to_owned()),
            "{fields:?}"
        );

        // AN ORDINARY OUTPUT, where rule 2 is the whole answer: `NONE`'s
        // one bit is a subset of `OUTPUT_LVCMOS33`'s six, both match, and
        // the longer one is the reading.
        let (fields, left) = read(&all(&["A OUTPUT", "A PULL NONE"]));
        assert_eq!(left, 0, "{fields:?}");
        assert!(
            fields.contains(&"PIOA.BASE_TYPE=OUTPUT_LVCMOS33".to_owned()),
            "{fields:?}"
        );

        // AND THE CASE THIS RULE DOES NOT FIX, said out loud because it is
        // still there and is a different shape: two ordinary **outputs** on
        // one pair also set all ten bits of side A's `OUTPUT_LVCMOS33D`,
        // and there the longer reading leaves nothing over either — the
        // image is equally consistent with both, so no accounting of bits
        // can tell them apart and the differential one wins on length.
        // `analyzer.bit` has one: (col 72, row 18), whose side C is the aux
        // transceiver's reset pin.
        let (fields, left) = read(&all(&[
            "A OUTPUT",
            "A PULL NONE",
            "B OUTPUT",
            "B PULL NONE",
        ]));
        assert_eq!(left, 0, "{fields:?}");
        assert!(
            fields.contains(&"PIOA.BASE_TYPE=OUTPUT_LVCMOS33D".to_owned()),
            "correct this test if the ambiguity is ever resolved, and say in \
             `docs/fpga-trellis.md` how: {fields:?}"
        );
    }
}
