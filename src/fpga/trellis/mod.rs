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
//! two rows south for C and D, and the `CIB` is one column west. That is
//! nextpnr's own rule (`get_pio_tile`, `get_pic_tile`), and both edges were
//! checked against bitstreams Lattice's own packer wrote for this board
//! rather than taken on trust — see `docs/fpga-trellis.md`.
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
        for tile in &self.tiles {
            let at = (tile.col, tile.row);
            frames.push(at, tile.window);
            members.entry(at).or_default().push(tile.ty.clone());
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
            let mut windows: Vec<(String, u32)> = Vec::with_capacity(list.len());
            let mut before = 0u32;
            for tile in self.tiles.iter().filter(|t| t.col == at.0 && t.row == at.1) {
                windows.push((tile.ty.clone(), before));
                before += tile.window.frames;
            }
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
        let mut owned: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut globals: BTreeSet<&str> = BTreeSet::new();
        let mut off_grid = 0usize;
        for (at, list) in &members {
            let Some(comp) = composition_key(&type_of, list) else {
                continue;
            };
            for ty in list {
                let Some(db) = self.types.get(ty) else {
                    continue;
                };
                for name in db.wire_names() {
                    match parse::globalise_ref(name, prefix) {
                        None => {}
                        Some(parse::WireTargetRef::Global { name }) => {
                            globals.insert(name);
                        }
                        Some(parse::WireTargetRef::Tile { dx: 0, dy: 0, name }) => {
                            owned.entry(comp).or_default().insert(name);
                        }
                        Some(parse::WireTargetRef::Tile { dx, dy, name }) => {
                            let x = i64::from(at.0) + i64::from(dx);
                            let y = i64::from(at.1) + i64::from(dy);
                            let Ok(x) = u32::try_from(x) else {
                                off_grid += 1;
                                continue;
                            };
                            let Ok(y) = u32::try_from(y) else {
                                off_grid += 1;
                                continue;
                            };
                            match members
                                .get(&(x, y))
                                .and_then(|list| composition_key(&type_of, list))
                            {
                                Some(comp) => {
                                    owned.entry(comp).or_default().insert(name);
                                }
                                None => off_grid += 1,
                            }
                        }
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
            let Some(edge) = Edge::of(*col, *row, width, side) else {
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
    /// Balls left out, because they are on an edge this does not describe
    /// or their tiles are not where the rule says.
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
/// Only two of the four are here, and the reason is the same one that kept
/// the other three out before: the rule is nextpnr's `get_pio_tile` /
/// `get_pic_tile`, and a rule that has not been checked against a part
/// produces a bitstream that loads, asserts `DONE` and drives the wrong
/// ball. Both of these have been checked against bitstreams Lattice's own
/// packer wrote for this very board — the top edge against all six of its
/// LEDs, the right edge against its USER button — and the left and bottom
/// edges have not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Row 0. Two PIOs per position, `PIOT0` / `PIOT1`, and side B's tiles
    /// are one column east of its ball's.
    Top,
    /// The last column. Four PIOs per position; the pad tile is one row
    /// south and the second `BASE_TYPE` is at the ball's own row for sides
    /// A and B and two rows south for C and D.
    Right,
}

impl Edge {
    /// The edge a `(col, row, side)` belongs to, or `None` for one this
    /// backend does not describe.
    #[must_use]
    pub fn of(col: u32, row: u32, width: u32, side: char) -> Option<Edge> {
        if row == 0 && matches!(side, 'A' | 'B') {
            return Some(Edge::Top);
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
/// both edges this describes. Before there was interconnect only the bits
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
    /// `PIO<side>.HYSTERESIS = ON`, which Lattice's own packer writes for
    /// every single-ended input and for no output.
    pub hysteresis_bits: Vec<ConfigBit>,
    /// `PIO<side>.PULLMODE = NONE`. **This one is not cosmetic.** The
    /// database's default for the field is `DOWN`, so a bitstream that
    /// leaves it alone leaves an internal pull-down on the pin; see
    /// [`PULL_NONE`].
    pub pull_none_bits: Vec<ConfigBit>,
    /// The bits, in the `CIB` tile, that tie the output data wire to a
    /// fixed **one**.
    pub high_bits: Vec<ConfigBit>,
    /// The bits that tie it to a fixed **zero**.
    pub low_bits: Vec<ConfigBit>,
    /// The bits that tie the output-enable wire to a fixed zero, which is
    /// the enabled state.
    pub enable_bits: Vec<ConfigBit>,
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
            Edge::Right => {
                let pic_row = if matches!(side, 'A' | 'B') {
                    row
                } else {
                    row + 2
                };
                ((col, row + 1), (col, pic_row))
            }
        };
        let field = format!("PIO{side}.BASE_TYPE");
        let output = format!("OUTPUT_{standard}");
        let input = format!("INPUT_{standard}");

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
            hysteresis_bits: db.locate_field(
                pad_at,
                &format!("PIO{side}.HYSTERESIS"),
                HYSTERESIS_ON,
            )?,
            pull_none_bits: db.locate_field(pad_at, &format!("PIO{side}.PULLMODE"), PULL_NONE)?,
            high_bits: db.locate_field(cib_at, &data_field, TIE_HIGH)?,
            low_bits: db.locate_field(cib_at, &data_field, TIE_LOW)?,
            enable_bits: db.locate_field(cib_at, &enable_field, TIE_LOW)?,
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
    /// Network index to how many flip-flop clock pins it reached.
    pub networks: BTreeMap<u32, usize>,
    /// The flip-flops whose clock arrived through general interconnect
    /// instead, as `<instance> on <site>`.
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

/// The letter Project Trellis names slice `index` with.
fn slice_letter(index: usize) -> char {
    ['A', 'B', 'C', 'D'][index & 3]
}

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
pub const PULL_NONE: &str = "NONE";

/// `PIO<side>.HYSTERESIS = ON`, which Lattice's own packer writes for every
/// single-ended input (`write_io` in nextpnr's `ecp5/bitstream.cc`, where
/// the default of the attribute is `ON`) and for no output.
pub const HYSTERESIS_ON: &str = "ON";

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
    /// it wants clear is clear, and where several values of one field match
    /// the one with the most bits wins — which is what picks
    /// `OUTPUT_LVCMOS33` over the `NONE` whose single bit it contains.
    #[must_use]
    pub fn decode(&self, cram: &Cram) -> Decoded {
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
            // The tile's own set bits, in its own coordinates, and which of
            // them a feature has accounted for.
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
            let mut covered: BTreeSet<(u32, u32)> = BTreeSet::new();
            let matches = |bits: &[parse::DbBit]| -> bool {
                bits.iter()
                    .all(|b| ones.contains(&(b.frame, b.bit)) != b.inverted)
            };
            let cover = |bits: &[parse::DbBit], covered: &mut BTreeSet<(u32, u32)>| {
                for b in bits.iter().filter(|b| !b.inverted) {
                    covered.insert((b.frame, b.bit));
                }
            };

            // The muxes, one sink at a time.
            let mut sink_seen: BTreeSet<&str> = BTreeSet::new();
            for (sink, _, _) in &db.muxes {
                if !sink_seen.insert(sink.as_str()) {
                    continue;
                }
                let mut best: Option<(&str, &[parse::DbBit])> = None;
                for (other, source, bits) in &db.muxes {
                    if other != sink || !matches(bits) {
                        continue;
                    }
                    if best.is_none_or(|(_, chosen)| bits.len() > chosen.len()) {
                        best = Some((source.as_str(), bits.as_slice()));
                    }
                }
                // A source with no bit it wants **set** is the state an
                // untouched bitstream is in everywhere, so reporting it
                // would say nothing — and that is true of a source with no
                // bits at all *and* of one whose whole pattern is inverted.
                // The centre muxes of this die have the second kind:
                // `G_DCS0CLK1 <- G_VPFN0000` is six bits all wanted clear,
                // so every centre mux of the part would otherwise appear to
                // be carrying an arc as soon as anything else in its tile
                // is written.
                if let Some((source, bits)) = best
                    && bits.iter().any(|bit| !bit.inverted)
                {
                    cover(bits, &mut covered);
                    out.arcs.push((at, sink.clone(), source.to_owned()));
                }
            }

            // The enumerated fields.
            for (field, default, values) in &db.enums {
                let mut best: Option<(&str, &[parse::DbBit])> = None;
                for (value, bits) in values {
                    if !matches(bits) {
                        continue;
                    }
                    if best.is_none_or(|(_, chosen)| bits.len() > chosen.len()) {
                        best = Some((value.as_str(), bits.as_slice()));
                    }
                }
                if let Some((value, bits)) = best {
                    cover(bits, &mut covered);
                    if default.as_deref() != Some(value) {
                        out.enums.push((at, field.clone(), value.to_owned()));
                    }
                }
            }

            // The multi-bit fields, bit 0 first.
            for (field, default, groups) in &db.words {
                let mut value = String::with_capacity(groups.len());
                for group in groups {
                    let one = matches(group);
                    value.push(if one { '1' } else { '0' });
                    for b in group {
                        if b.inverted != one {
                            covered.insert((b.frame, b.bit));
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

            for bit in &ones {
                if !covered.contains(bit) {
                    out.unexplained += 1;
                }
            }
        }
        out.tiles = positions.len();
        out.arcs.sort();
        out.enums.sort();
        out.words.sort();
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
    /// database: a `TRELLIS_IO` whose `O` port carries a signal is an
    /// input, one whose `I` port does is an output. That is read off the
    /// pins rather than off the cell's `DIR` parameter so this needs no
    /// access to the design.
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
    /// 3. `PIO<side>.PULLMODE = NONE` — see [`PULL_NONE`], which is the
    ///    one of these that changes what a person sees;
    /// 4. no tristate tie and no data tie, neither of which nextpnr writes
    ///    for an input either.
    ///
    /// And, once per bank any of them is in, [`BANK_VCCIO`] in that bank's
    /// reference tile — which is none of the pad's own positions and is the
    /// one thing this pass was missing when it first put a bitstream in a
    /// part.
    ///
    /// # Errors
    ///
    /// [`super::bitstream::BitstreamError`] when a bit falls outside the
    /// position it was located in, which would mean the frame map and the
    /// `Arch` disagree.
    pub fn configure_io(
        &self,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        bits: &mut super::bitstream::Bitstream,
    ) -> Result<usize, super::bitstream::BitstreamError> {
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
            // An input's data wire leaves the buffer, so its pin is an
            // output of the cell; an output's arrives, so its pin is an
            // input. A cell with both would be bidirectional and this does
            // not build one: it would need the tristate routed rather than
            // tied, and nothing has been on a part that way.
            let is_input = din.is_some_and(|pin| pin.signal.is_some());
            if is_input {
                for bit in &pad.input_pad_bits {
                    bits.set(pad.pad_at, *bit)?;
                }
                for bit in &pad.input_pic_bits {
                    bits.set(pad.pic_at, *bit)?;
                }
                for bit in pad.hysteresis_bits.iter().chain(&pad.pull_none_bits) {
                    bits.set(pad.pad_at, *bit)?;
                }
            } else {
                for bit in &pad.output_pad_bits {
                    bits.set(pad.pad_at, *bit)?;
                }
                for bit in &pad.output_pic_bits {
                    bits.set(pad.pic_at, *bit)?;
                }
                for bit in &pad.enable_bits {
                    bits.set(pad.cib_at, *bit)?;
                }
                // Which constant the pin carries, if it carries one. A pin
                // a signal drives is left alone: the route into it is what
                // drives the wire, and a tie would be a second driver.
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
            if instance.kind != "ff" {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(signal) = netlist
                .pins
                .iter()
                .find(|pin| pin.instance == index && pin.role == "clk")
                .and_then(|pin| pin.signal)
            else {
                continue;
            };
            // Walk this pin's own path back through the route until a
            // branch wire turns up. Asking "does the route touch a branch
            // wire in this tile" would not do: a tile has two clock muxes
            // and four slices, so one flop's clock can be on the network
            // while its neighbour's came off a data wire, and the whole
            // point of the check is to catch exactly that.
            let mut found = None;
            if let (Some(clk), Some(route)) = (site.pin("clk"), routing.route(signal)) {
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
                    if matches!(graph.wire(node).name.as_str(), "CLK0" | "CLK1") {
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
        out
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
    /// | `SLICE<l>.REG<n>.SD` | always `0`: the data comes from the fabric's `M` wire, because this flow never packs a lookup table and a flop onto one site |
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
    /// clock or reset mux whose control index the routing does not settle.
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
            if value("CLKMUX", "CLK") == "INV" {
                let c = mux_of(signal_of("clk"), "CLK").ok_or_else(|| refuse("CLKMUX=INV"))?;
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
                        for bit in &ff.lsrmux_inv[c] {
                            bits.set(site.tile, *bit)?;
                        }
                    }
                    if async_reset {
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
             OUTPUT_LVCMOS33 F5B0 F6B0\n",
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
        // An input costs two settings an output does not, and `PULLMODE`'s
        // default is a pull-*down*, so `NONE` is a bit that has to be set.
        assert_eq!(pad.hysteresis_bits, vec![ConfigBit::new(10, 0)]);
        assert_eq!(pad.pull_none_bits, vec![ConfigBit::new(12, 0)]);
        // The tile one row south holds two windows and the `PICT1` is the
        // second, so its `F5B0` is row 20 + 5. An input costs nothing there.
        assert_eq!(
            pad.output_pic_bits,
            vec![ConfigBit::new(25, 0), ConfigBit::new(26, 0)]
        );
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
}
