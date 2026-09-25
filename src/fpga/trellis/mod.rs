//! Project Trellis' Lattice ECP5 database, turned into an [`Arch`].
//!
//! # What this builds, and what it does not
//!
//! Read this part first, because the gap is the point.
//!
//! What it builds is **the part's geometry and its pads**: the tile grid
//! of an ECP5, every position's rectangle of configuration memory, the
//! package's ball map, and an `io` bel for every PIO of the top edge, with
//! the bits that make one an output and the bits that tie its data to a
//! constant. That is enough to compile a design whose ports are driven by
//! constants, all the way to a `.bit` a part accepts, and it is what
//! `docs/fpga-trellis.md` records having done on a real board.
//!
//! What it does **not** build is the interconnect. [`Arch`] here declares
//! **no pips at all**, and the only wires it declares are the three a pad
//! bel's pins name, so:
//!
//! - nothing that needs a signal carried from one site to another can be
//!   compiled, and
//! - [`TrellisFabric::unroutable`] says so *before* a bitstream is
//!   written, by name, rather than a bitstream being emitted with nets
//!   that go nowhere.
//!
//! That is a deliberate choice between two ways of being incomplete.
//! Project Trellis has the routing: `bits.db`'s `.mux` records are the
//! programmable connections and [`parse::TileDatabase::arcs`] reads them.
//! What is not here is the *rest* of what a routing graph needs to be
//! right — which wire a neighbour's spelling means, where a global begins
//! and ends, which of six tiles at a position owns a name — and a graph
//! that is half right does not fail, it routes a net through a connection
//! that is not there. `src/fpga/arch/synthetic.rs`'s opening says the same
//! thing about inventing a database: a file that looks authoritative and
//! programs nothing is worse than having none. So the interconnect is
//! absent and named as absent.
//!
//! # The two structural surprises
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
//! **A pad's configuration is spread over three tiles at two positions,
//! and not all of them are the bel's own** — and there is a fourth tile,
//! at the far end of the edge, holding one bit the pad cannot do without.
//! For a PIO on the top edge:
//!
//! | What | Tile | Position |
//! |---|---|---|
//! | the pad: standard, drive, pull | `PIOT0` (side A) or `PIOT1` (side B) | row 0 |
//! | a second copy of the standard, and the output data mux | `PICT0` / `PICT1` | row 1 |
//! | the constant that ties the data wire | `CIB` | row 1 |
//! | the bank's VCCIO rail | `BANKREF<bank>` | wherever the bank's is |
//!
//! The last of those is the one that was left out when the first `.bit`
//! this backend built went into a part, and it is why the LEDs that
//! bitstream should have lit were dark with `DONE` high. See
//! [`BANK_VCCIO`].
//!
//! and the *column* differs by side: side A's three tiles are at the
//! column `iodb.json` gives the ball, side B's are at one column east of
//! it. That is nextpnr's own rule (`get_pio_tile`, `get_pic_tile`), and it
//! was checked against this board's own gateware rather than taken on
//! trust — see `docs/fpga-trellis.md`.
//!
//! Two of those three tiles are therefore **not** the tile a bel could
//! carry [`ConfigEntry`](super::arch::ConfigEntry)s in, whichever of the
//! two positions the bel is put at. So an IO's bits are not a bel's here;
//! they are [`TrellisFabric::configure_io`]'s, computed from the
//! placement. That is the same division `super::apicula`'s `Periphery`
//! makes for the same reason, and it is why `src/fpga/bitstream.rs` needs
//! no edit for this vendor either.
//!
//! # Obtaining the database
//!
//! Nothing here reads a file: a [`crate::ir::memfile::FileProvider`] is handed in,
//! rooted at the directory holding `devices.json`. `reticle fetch
//! prjtrellis-db` puts one in the per-user cache; `docs/fpga-trellis.md`
//! has the command and what is downloaded.

pub mod parse;
pub mod sites;

use std::collections::BTreeMap;
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
        }
    }
}

impl Error for TrellisError {}

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
/// `globals.json` is **not** read. It describes the clock quadrants,
/// spines and taps, and nothing here routes a clock.
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
        for (at, list) in &members {
            let name = list.join("+");
            let rows = frames.bit_rows(*at);
            let cols = frames.bit_cols(*at);
            let index = match type_of.get(&name) {
                Some(index) => {
                    let existing = &arch.tile_types[*index];
                    if existing.bit_rows != rows || existing.bit_cols != cols {
                        return Err(TrellisError::Malformed {
                            path: format!(
                                "{}/{}/tilegrid.json",
                                self.device.family, self.device.name
                            ),
                            what: format!(
                                "two positions are both `{name}` but their bit regions are \
                                 {}x{} and {rows}x{cols}",
                                existing.bit_rows, existing.bit_cols
                            ),
                        });
                    }
                    *index
                }
                None => {
                    let index = arch.tile_types.len();
                    // No wires and no pips: see the module header. The
                    // keyword is the type's name in lower case, which is
                    // only ever used by the `.asc` debugging form.
                    arch.tile_types.push(TileType::new(
                        name.clone(),
                        name.to_lowercase(),
                        rows,
                        cols,
                    ));
                    type_of.insert(name, index);
                    index
                }
            };
            arch.set_tile(at.0, at.1, index);
        }

        // ---- the pads ----
        let Some(pinout) = self.pinouts.iter().find(|p| p.package == options.package) else {
            return Err(TrellisError::NoSuchPackage {
                wanted: options.package.clone(),
                known: self.pinouts.iter().map(|p| p.package.clone()).collect(),
            });
        };

        let output = format!("OUTPUT_{}", options.io_standard);
        let mut io = Vec::new();
        let mut skipped = 0usize;
        for (ball, (row, col, side)) in &pinout.balls {
            let Some(side) = side.chars().next() else {
                continue;
            };
            // Only the top edge. The other three edges' pads are laid out
            // differently — `PICL*`/`PICR*` put four PIOs on a position and
            // `PICB*` two — and none of it has been checked against this
            // board, which has its LEDs and its oscillator on the top.
            // Leaving them out is what stops a pin constraint naming one
            // from being placed somewhere and configured nowhere.
            if *row != 0 {
                skipped += 1;
                continue;
            }
            // The bank comes from `pio_metadata` and nothing infers it. A
            // pad whose bank the database does not state is left out for
            // the same reason as one whose tiles are missing: it could be
            // placed and then configured incompletely, which is a dark
            // pin with nothing to explain it.
            let Some(bank) = self.banks.get(&(0, *col, side.to_string())).copied() else {
                skipped += 1;
                continue;
            };
            match IoSite::top(ball, *col, side, bank, self, &output) {
                Ok(site) => io.push(site),
                Err(TrellisError::NoSuchField { .. }) => {
                    // A pad whose tiles this grid does not have at that
                    // column — the last column of the edge has no eastern
                    // neighbour — is left out rather than guessed at.
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
        // constraint through. A bel belongs to a tile *type*, and the
        // top-edge positions of one PIO side all have the same type, so
        // the declarations are per type and the sites come out per
        // position from `Arch::build_graph`.
        let mut sides: BTreeMap<usize, Vec<char>> = BTreeMap::new();
        for site in &io {
            let Some(index) = arch.tile_index_at(site.x, 0) else {
                continue;
            };
            let letters = sides.entry(index).or_default();
            if !letters.contains(&site.side) {
                letters.push(site.side);
            }
        }
        for (index, letters) in sides {
            for letter in letters {
                let mut bel = BelDecl::new(format!("PIO{letter}"), "io");
                // Three pins, and the wires they name are declared on the
                // same tile type just below, because **a pin with no wire
                // is a pin the netlist does not record**: `Netlist::build`
                // puts it in `off_fabric` and builds no `NetPin` for it, so
                // the constant behind an output pad would never reach
                // `configure_io`. That is how this was found — two designs
                // driving different constants produced the same bitstream.
                //
                // They are real wires, from the fixed connections in the
                // pad tile's own `bits.db`: `PADDO<L>_PIO` is what the
                // output buffer takes its data from, `PADDT<L>_PIO` its
                // output enable, and `JPADDI<L>_PIO` is what an input
                // presents. There is no `pad` pin, because a package ball
                // is not a wire a router can reach — `super::xray` and
                // `super::apicula` both say the same about their IO.
                bel.pins = vec![
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
                ];
                bel.config = Vec::new();
                let ty = &mut arch.tile_types[index];
                for (_, wire) in &bel.pins {
                    if !ty.has_wire(&wire.name) {
                        ty.wires.push(super::arch::WireDecl {
                            name: wire.name.clone(),
                            dx: 0,
                            dy: 0,
                        });
                    }
                }
                ty.bels.push(bel);
            }
        }
        for site in &io {
            arch.pinmap
                .push((site.ball.clone(), format!("X{}Y0/PIO{}", site.x, site.side)));
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
            let ty = format!("BANKREF{}", site.bank);
            let Some(tile) = self.tiles.iter().find(|t| t.ty == ty) else {
                return Err(TrellisError::NoSuchField {
                    tile: ty,
                    field: BANK_VCCIO.to_owned(),
                    value: voltage.to_owned(),
                });
            };
            let at = (tile.col, tile.row);
            let bits = self.locate_enum(at.0, at.1, &ty, BANK_VCCIO, voltage)?;
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
        };

        Ok(TrellisFabric {
            arch,
            format: self.device.format,
            frames,
            idcode: self.device.idcode,
            part: self.device.name.clone(),
            package: options.package.clone(),
            io,
            bank_bits,
            voltage: voltage.to_owned(),
            stats,
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
    /// Balls left out, because they are not on the top edge or their
    /// tiles are not there.
    pub pads_skipped: usize,
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
        out
    }
}

/// One pad, with every bit that configures it already located.
///
/// The three tiles and two positions the module header describes are
/// resolved here, once, at load time — not when a bitstream is written —
/// so a database that cannot describe a pad is an error from
/// [`TrellisDatabase::load`] rather than a surprise much later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoSite {
    /// The package ball.
    pub ball: String,
    /// `A` or `B`.
    pub side: char,
    /// The grid column its tiles are at, which is the column
    /// `iodb.json` gives the ball for side A and one east of it for side
    /// B.
    pub x: u32,
    /// The bits, in the pad tile at row 0, that make this an output in
    /// the chosen standard.
    pub pad_bits: Vec<ConfigBit>,
    /// The bits, in the tile at row 1, that say the same thing again.
    pub pic_bits: Vec<ConfigBit>,
    /// The bits, in the `CIB` tile at row 1, that tie the output data
    /// wire to a fixed **one**.
    pub high_bits: Vec<ConfigBit>,
    /// The bits that tie it to a fixed **zero**.
    pub low_bits: Vec<ConfigBit>,
    /// The bits that tie the output-enable wire to a fixed zero, which
    /// is the enabled state.
    pub enable_bits: Vec<ConfigBit>,
    /// The IO bank the pad is wired to, from `iodb.json`'s
    /// `pio_metadata`.
    ///
    /// A bank has one setting of its own that none of the three tiles
    /// above holds — [`BANK_VCCIO`], located per bank in
    /// [`TrellisFabric::bank_bits`] — so a pad that only configures its
    /// own tiles is configured incompletely.
    pub bank: u32,
}

/// The `CIB` field that ties a PIO's output **data** wire.
///
/// `JA0` is the wire, whichever side the PIO is: `PIOT0`'s fixed
/// connections are `JPADDOA <- S1_JA0` and `JPADDOB <- S1E1_JA0`, so each
/// side reads the `JA0` of the tile one row south of its own pad tile —
/// the same position this module puts the side's other bits at.
pub const DATA_MUX: &str = "CIB.JA0MUX";

/// The `CIB` field that ties a PIO's output **enable** wire.
///
/// `JB0`, by the same two fixed connections one letter along: `JPADDTA <-
/// S1_JB0` and `JPADDTB <- S1E1_JB0`.
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

impl IoSite {
    /// Locates every bit of one top-edge pad.
    fn top(
        ball: &str,
        col: u32,
        side: char,
        bank: u32,
        db: &TrellisDatabase,
        output: &str,
    ) -> Result<IoSite, TrellisError> {
        // nextpnr's rule, checked against this board's own gateware: side
        // A's tiles are at the ball's column and side B's one east.
        let x = match side {
            'A' => col,
            'B' => col + 1,
            _ => {
                return Err(TrellisError::NoSuchField {
                    tile: "PIOT?".to_owned(),
                    field: format!("PIO{side}.BASE_TYPE"),
                    value: output.to_owned(),
                });
            }
        };
        let pad_type = if side == 'A' { "PIOT0" } else { "PIOT1" };
        let pic_type = if side == 'A' { "PICT0" } else { "PICT1" };
        let field = format!("PIO{side}.BASE_TYPE");

        let pad_bits = db.locate_enum(x, 0, pad_type, &field, output)?;
        let pic_bits = db.locate_enum(x, 1, pic_type, &field, output)?;
        let high_bits = db.locate_enum(x, 1, "CIB", DATA_MUX, TIE_HIGH)?;
        let low_bits = db.locate_enum(x, 1, "CIB", DATA_MUX, TIE_LOW)?;
        let enable_bits = db.locate_enum(x, 1, "CIB", ENABLE_MUX, TIE_LOW)?;

        Ok(IoSite {
            ball: ball.to_owned(),
            side,
            x,
            pad_bits,
            pic_bits,
            high_bits,
            low_bits,
            enable_bits,
            bank,
        })
    }
}

impl TrellisDatabase {
    /// The [`ConfigBit`]s of one enumerated field of one Lattice tile at
    /// one position, in the position's combined numbering.
    ///
    /// The two halves of this are the whole reason
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
    /// express. This is only true because nothing else writes to the same
    /// tiles; a flow that composed two features into one tile would have
    /// to keep them.
    ///
    /// # Errors
    ///
    /// [`TrellisError::NoSuchField`] when the position holds no tile of
    /// that type, or its `bits.db` has no such field or value.
    fn locate_enum(
        &self,
        x: u32,
        y: u32,
        ty: &str,
        field: &str,
        value: &str,
    ) -> Result<Vec<ConfigBit>, TrellisError> {
        let miss = || TrellisError::NoSuchField {
            tile: ty.to_owned(),
            field: field.to_owned(),
            value: value.to_owned(),
        };
        // The position's windows, in the order `load` pushes them, which
        // is the order the tile names sort in.
        let mut before = 0u32;
        let mut found = false;
        for tile in self.tiles.iter().filter(|t| t.col == x && t.row == y) {
            if tile.ty == ty {
                found = true;
                break;
            }
            before += tile.window.frames;
        }
        if !found {
            return Err(miss());
        }
        let db = self.types.get(ty).ok_or_else(miss)?;
        let bits = db.enum_bits(field, value).ok_or_else(miss)?;
        Ok(bits
            .iter()
            .filter(|bit| !bit.inverted)
            .map(|bit| ConfigBit::new(before + bit.frame, bit.bit))
            .collect())
    }
}

/// An ECP5 fabric: an [`Arch`], where its bits live, and where its pads
/// are.
#[derive(Clone, Debug)]
pub struct TrellisFabric {
    /// The grid, its tile types' bit geometry, the `io` bels of the top
    /// edge and the package's ball map. **No pips**, and no wires but the
    /// three each pad bel's pins name; see the module header.
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
        let x: u32 = position
            .strip_prefix('X')?
            .split_once('Y')
            .and_then(|(x, y)| (y == "0").then(|| x.parse().ok())?)?;
        self.io
            .iter()
            .find(|candidate| candidate.x == x && candidate.side == letter)
    }

    /// Every signal a design needs carried from one site to another, by
    /// name.
    ///
    /// **This is the guard that keeps the gap in the module header from
    /// becoming a lie.** There is no interconnect here, so a design with
    /// anything to route cannot be built; a caller asks this first and
    /// refuses, rather than writing a bitstream whose nets go nowhere.
    /// Only a design whose ports are driven by constants comes back empty.
    #[must_use]
    pub fn unroutable(&self, netlist: &super::place::Netlist) -> Vec<String> {
        netlist
            .signals
            .iter()
            .filter(|signal| signal.is_routable())
            .map(|signal| signal.name.clone())
            .collect()
    }

    /// Sets the bits that configure every placed IO, and returns how many
    /// pads were configured.
    ///
    /// This is the pass the module header describes: a pad's bits live in
    /// three tiles at two positions and two of them are never the bel's
    /// own, so they cannot be a bel's
    /// [`ConfigEntry`](super::arch::ConfigEntry)s and are computed from
    /// the placement instead. [`super::bitstream::generate`] has already
    /// done everything that *is* a bel's or a pip's; this adds what is
    /// neither, exactly as `super::xray`'s `enable_global_clocks` does for
    /// the 7 series.
    ///
    /// What each pad gets:
    ///
    /// 1. `PIO<side>.BASE_TYPE = OUTPUT_<standard>` in the pad tile;
    /// 2. the same field again in the tile one row south, which is what
    ///    nextpnr writes and what this board's own gateware has;
    /// 3. the output-enable wire tied low, so the buffer drives;
    /// 4. the data wire tied to the constant the netlist gives the pin;
    /// 5. and, once per bank that any of them is in, [`BANK_VCCIO`] in
    ///    that bank's reference tile — which is neither of the pad's own
    ///    two positions and is the one thing this pass was missing when
    ///    it first put a bitstream in a part.
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
            let at = (pad.x, 0);
            let south = (pad.x, 1);
            for bit in &pad.pad_bits {
                bits.set(at, *bit)?;
            }
            for bit in &pad.pic_bits {
                bits.set(south, *bit)?;
            }
            for bit in &pad.enable_bits {
                bits.set(south, *bit)?;
            }
            // Which constant the pin carries. A pin with a signal on it
            // cannot be here: `unroutable` refused the design before this
            // was reached.
            let driven = netlist
                .pins
                .iter()
                .filter(|pin| pin.instance == index && !pin.output)
                .find_map(|pin| pin.constant);
            let tie = match driven {
                Some(crate::logic::Bit::One) => &pad.high_bits,
                _ => &pad.low_bits,
            };
            for bit in tie {
                bits.set(south, *bit)?;
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

    /// A database small enough to write out, with the shape the real one
    /// has: two positions, one of which holds two tiles.
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
        // Position (1,0) holds a PIOT1; position (1,1) a CIB and a PICT1,
        // in that order, because that is how the names sort. Position
        // (2,0) is the bank's reference tile, which no pad owns and every
        // pad needs.
        files.insert(
            "ECP5/LFE5U-12F/tilegrid.json",
            r#"{
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
        files.insert(
            "ECP5/tiledata/BANKREF1/bits.db",
            "# Non-Routing Configuration\n\
             .config_enum BANK.VCCIO NONE\n\
             3V3 F18B0\n\
             2V5 F17B0\n\
             NONE -\n",
        );
        files.insert(
            "ECP5/tiledata/PIOT1/bits.db",
            "# Non-Routing Configuration\n\
             .config_enum PIOB.BASE_TYPE NONE\n\
             NONE F2B0\n\
             OUTPUT_LVCMOS33 F2B0 F7B0 !F8B0\n",
        );
        files.insert(
            "ECP5/tiledata/PICT1/bits.db",
            "# Non-Routing Configuration\n\
             .config_enum PIOB.BASE_TYPE INPUT_LVCMOS12\n\
             INPUT_LVCMOS12 -\n\
             OUTPUT_LVCMOS33 F5B0 F6B0\n",
        );
        files.insert(
            "ECP5/tiledata/CIB/bits.db",
            "# Non-Routing Configuration\n\
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
        files
    }

    #[test]
    fn a_position_with_several_tiles_addresses_them_end_to_end() {
        let db = open(&tiny(), "", "LFE5U-12F").unwrap();
        assert_eq!(db.device().name, "LFE5U-12F");
        assert_eq!(db.device().idcode, 0x2111_1043);
        assert_eq!(db.size(), (4, 4));
        assert_eq!(db.packages(), vec!["CABGA256"]);

        let fabric = db.load(&TrellisOptions::new()).unwrap();
        // The grid is `max_col + 1` by `max_row + 1`.
        assert_eq!((fabric.arch.width, fabric.arch.height), (3, 2));
        // Three positions hold tiles: the pad's, the one south of it, and
        // the bank reference at the end of the edge.
        assert_eq!(fabric.stats.positions, 3);
        assert_eq!(fabric.stats.shared_positions, 1);
        assert_eq!(fabric.stats.most_windows, 2);

        // Position (0, 1) is `CIB+PICT1`: 20 frames each, laid end to end.
        let ty = fabric.arch.tile_at(1, 1).unwrap();
        assert_eq!(ty.name, "CIB+PICT1");
        assert_eq!(ty.bit_rows, 40, "two windows of twenty frames");
        assert_eq!(ty.bit_cols, 11, "the widest window's bit count");
        // And (0, 0) is the pad tile alone.
        assert_eq!(fabric.arch.tile_at(1, 0).unwrap().name, "PIOT1");
        assert_eq!(fabric.arch.tile_at(1, 0).unwrap().bit_rows, 20);
        // Column 0 and column 2 hold no tile at all, which `Arch` says
        // with `None` rather than with an empty type.
        assert!(fabric.arch.tile_at(0, 0).is_none());
        assert!(fabric.arch.tile_at(2, 1).is_none());
        // And (2, 0) is the bank reference, which is a tile like any
        // other and holds no bel.
        assert_eq!(fabric.arch.tile_at(2, 0).unwrap().name, "BANKREF1");

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
    fn a_top_edge_pad_of_side_b_sits_one_column_east_of_its_ball() {
        let fabric = open(&tiny(), "", "LFE5U-12F")
            .unwrap()
            .load(&TrellisOptions::new())
            .unwrap();
        // `E13` is at column 0 on side B, so its tiles are at column 1.
        // The ball on row 1 is not on the top edge and is left out.
        assert_eq!(fabric.stats.balls, 2);
        assert_eq!(fabric.stats.pads, 1);
        assert_eq!(fabric.stats.pads_skipped, 1);
        let pad = fabric.pad("E13").unwrap();
        assert_eq!((pad.x, pad.side), (1, 'B'));

        // The pad tile's bits are the enum's, with the ones it wants
        // *clear* left out: a bitmap starts zeroed.
        assert_eq!(
            pad.pad_bits,
            vec![ConfigBit::new(2, 0), ConfigBit::new(7, 0)],
            "`!F8B0` is not recorded"
        );
        // The tile one row south holds two windows and the `PICT1` is the
        // second, so its `F5B0` is row 20 + 5.
        assert_eq!(
            pad.pic_bits,
            vec![ConfigBit::new(25, 0), ConfigBit::new(26, 0)]
        );
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
        // cell at, and it is at the pad tile's position.
        assert_eq!(fabric.arch.site_of_pin("E13"), Some("X1Y0/PIOB"));
        assert_eq!(fabric.arch.site_of_pin("Z99"), None);
        assert_eq!(
            fabric.pad_of_site("X1Y0/PIOB").map(|p| &p.ball[..]),
            Some("E13")
        );
        assert_eq!(fabric.pad_of_site("X0Y0/PIOB"), None);
        assert_eq!(fabric.pad_of_site("X1Y1/PIOB"), None);

        // The bel is declared on the pad tile's *type*, so building the
        // graph produces one site per position of that type.
        let graph = fabric.arch.build_graph();
        let site = graph.site("X1Y0/PIOB").expect("the bel becomes a site");
        // Three pins, and the data one is what carries a constant.
        assert!(site.pin("dout").is_some());
        assert!(site.pin("oe").is_some());
        assert!(site.pin("din").is_some());
        assert!(site.pin("pad").is_none(), "a ball is not a wire");
        // Three wires, and **no pips**: the fabric can hold a constant at
        // a pad and cannot carry anything anywhere.
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.pips.len(), 0, "no interconnect is declared");
        assert_eq!(graph.dangling, 0, "and so nothing dangles");
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
        for bit in &pad.pad_bits {
            bits.set((1, 0), *bit).unwrap();
        }
        for bit in &pad.pic_bits {
            bits.set((1, 1), *bit).unwrap();
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
        assert!(text.contains("tiles: 4"), "{text}");
        assert!(text.contains("grid positions: 3"), "{text}");
        assert!(text.contains("configuration bits: 640"), "{text}");
        assert!(text.contains("pads declared: 1"), "{text}");
        assert_eq!(fabric.stats.frames, 40);
        assert_eq!(fabric.stats.bits_per_frame, 16);
    }
}
