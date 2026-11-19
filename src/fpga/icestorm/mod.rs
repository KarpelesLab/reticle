//! Project IceStorm's Lattice iCE40 database, turned into an [`Arch`].
//!
//! # What this builds
//!
//! An icebox chip database (`chipdb-5k.txt` for the iCE40 UltraPlus 5k) is
//! already a flat routing graph: every piece of metal is a numbered *net*
//! listing the name it has in each tile it passes through, and every
//! programmable connection is a `.buffer` or `.routing` multiplexer in one
//! tile, with the pattern of that tile's bits that selects each source.
//! [`IceFabric::build`] turns that into the flow's [`Arch`]:
//!
//! - **one tile type per grid position**, because icebox's tiles of one
//!   kind are not alike at the edges and the database states each tile's
//!   multiplexers separately anyway;
//! - **one wire per net**, declared in the tile at the corner of the box its
//!   segments cover and named after its segment there when it has one, so
//!   that the placer's distance estimates see a span-12 wire as long;
//! - the eight **global networks** (`glb_netwk_0`..`7`) as the
//!   architecture's globals;
//! - **one pip per multiplexer option**, whose bits are the ones of its
//!   pattern;
//! - one **`lc` bel per logic cell**: the lookup table, the flip-flop and the
//!   carry of an iCE40 logic cell are one bel here, as they are in the
//!   silicon, so a design reaches it as an `ICESTORM_LC` that [`pack`] has
//!   built out of `SB_LUT4`, `SB_DFF*` and `SB_CARRY` cells;
//! - an **`io` bel** per IO block, and bels for the RGB LED driver and the
//!   two oscillators of an UltraPlus part;
//! - a **`bram` bel** per block RAM, anchored on its `ramb` tile with pins
//!   on the `ram/*` wires of both of its tiles, and a **`spram` bel** at each
//!   of the four places IceStorm's `.extra_cell SPRAM` records give, with
//!   pins on the IP-connect tiles' wires those records name;
//! - a **`mac16` bel** for each of the eight DSP blocks, the same way from
//!   the `.extra_cell MAC16` records, its parameters read back from the
//!   bits by [`IceFabric::decode`] (see [`DspConfig::as_bits`] for the two
//!   that IceStorm names after each other);
//! - and the package's **pin map**.
//!
//! The logic cell's flip-flop takes its data from its own lookup table and
//! from nowhere else, and its clock, enable and set/reset are the *tile's*,
//! shared by all eight cells; both are what the bel's pins say, so the
//! placer keeps a tile's cells on one clock without a line of code here
//! naming a clock. The carry of cell *k* comes from cell *k-1*'s carry out
//! and the first cell's from the tile below, over wires nothing else drives,
//! which the placer turns into rigid chains on its own (`place::build_clusters`).
//!
//! # What is not in an icebox database and is in every bitstream
//!
//! Three things, all read off bitstreams nextpnr wrote for an UltraPlus
//! part (`docs/fpga-icestorm.md` has the account) and written by
//! [`IceFabric::configure`]:
//!
//! - an IO block's input enable and pull-up live in *another* block's bits
//!   (`.ieren`), and a used block also sets one more bit there;
//! - every column buffer of every global network is enabled;
//! - every IP-connect and DSP tile's logic cells are set to pass their third
//!   input through, with the cascade bits beside them set.
//!
//! # Obtaining the database
//!
//! Nothing here reads a file: [`db::ChipDb::parse`] takes the text, and
//! `reticle fetch icestorm` is how the command line gets it.
//!
//! `docs/fpga-icestorm.md` is the account of what was checked and how.

pub mod db;
pub mod image;
pub mod pack;
pub mod timing;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use super::arch::{Arch, BelDecl, ConfigBit, ConfigEntry, PipDecl, TileType, WireDecl, WireRef};
use db::{ChipDb, TileKind};

/// The order icebox reads a lookup table's sixteen bits out of a logic
/// cell's twenty: bit `i` of `LUT_INIT` is bit `LUT_PERM[i]` of `LC_<k>`.
/// `icebox.get_lutff_lut_bits`, and the same table in nextpnr.
pub const LUT_PERM: [usize; 16] = [4, 14, 15, 5, 6, 16, 17, 7, 3, 13, 12, 2, 1, 11, 10, 0];
/// The logic cell bit that enables its carry (`icebox.get_lutff_seq_bits`).
pub const LC_CARRY_ENABLE: usize = 8;
/// The bit that puts the flip-flop on the cell's output.
pub const LC_DFF_ENABLE: usize = 9;
/// The bit that makes the set/reset a set.
pub const LC_SET_NORESET: usize = 18;
/// The bit that makes the set/reset asynchronous.
pub const LC_ASYNC_SR: usize = 19;

/// The pattern nextpnr writes into every logic cell of every IP-connect
/// and DSP tile of an UltraPlus part, as the twenty `LC_<k>` bits in the
/// database's order: a lookup table passing its third input through.
pub const IPCON_LC_PATTERN: &str = "11000011001100001100";

/// The one IP-connect tile of the 5k whose cells nextpnr does not set to
/// pass through, unless a DSP block's carry out leaves through it.
pub const SKIPPED_IPCON: (u32, u32) = (25, 14);

/// The parameters of an `SB_MAC16` that its configuration bits hold, with
/// their widths in bits: the list nextpnr's `bitstream.cc` writes, and
/// the `.extra_cell MAC16` records name a `CBIT` for each bit of each.
/// `NEG_TRIGGER` is not among them: no bit inverts the block's clock, and
/// a cell asking for it is refused.
pub const MAC16_PARAMS: [(&str, u32); 19] = [
    ("C_REG", 1),
    ("A_REG", 1),
    ("B_REG", 1),
    ("D_REG", 1),
    ("TOP_8x8_MULT_REG", 1),
    ("BOT_8x8_MULT_REG", 1),
    ("PIPELINE_16x16_MULT_REG1", 1),
    ("PIPELINE_16x16_MULT_REG2", 1),
    ("TOPOUTPUT_SELECT", 2),
    ("TOPADDSUB_LOWERINPUT", 2),
    ("TOPADDSUB_UPPERINPUT", 1),
    ("TOPADDSUB_CARRYSELECT", 2),
    ("BOTOUTPUT_SELECT", 2),
    ("BOTADDSUB_LOWERINPUT", 2),
    ("BOTADDSUB_UPPERINPUT", 1),
    ("BOTADDSUB_CARRYSELECT", 2),
    ("MODE_8x8", 1),
    ("A_SIGNED", 1),
    ("B_SIGNED", 1),
];

/// One iCE40 part the backend places on: a device file and where its
/// fabric is in Project IceStorm's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    /// The device file's name (`ice40-hx8k-ct256`).
    pub device: &'static str,
    /// The die, the `<die>` of `chipdb-<die>.txt`: `384`, `1k`, `8k`, `u4k`
    /// or `5k`.
    pub die: &'static str,
    /// The chip database's name for the package (`ct256`, `tq144:4k`).
    pub package: &'static str,
    /// The timing file nextpnr builds the part's figures from.
    pub timing: &'static str,
}

/// Every iCE40 part the backend places on.
///
/// The die is the chip database a part is on: an LP and an HX of one size
/// share a die and differ in their timing, and an HX4K is the 8k die in
/// packages IceStorm names `<package>:4k`. The timing file is the one
/// nextpnr builds that part's delays from (its `CMakeLists.txt`):
/// `timings_hx<n>` for an HX, its `--fast`; `timings_lp<n>` for an LP, its
/// `--slow`; the 8k's for an HX4K. The lm4k die, which IceStorm also ships,
/// has no timing file and no nextpnr support, and no part here.
pub const PARTS: [Part; 9] = [
    Part {
        device: "ice40-lp384-qn32",
        die: "384",
        package: "qn32",
        timing: "timings_lp384.txt",
    },
    Part {
        device: "ice40-lp1k-tq144",
        die: "1k",
        package: "tq144",
        timing: "timings_lp1k.txt",
    },
    Part {
        device: "ice40-hx1k-tq144",
        die: "1k",
        package: "tq144",
        timing: "timings_hx1k.txt",
    },
    Part {
        device: "ice40-hx1k-vq100",
        die: "1k",
        package: "vq100",
        timing: "timings_hx1k.txt",
    },
    Part {
        device: "ice40-hx4k-tq144",
        die: "8k",
        package: "tq144:4k",
        timing: "timings_hx8k.txt",
    },
    Part {
        device: "ice40-hx8k-ct256",
        die: "8k",
        package: "ct256",
        timing: "timings_hx8k.txt",
    },
    Part {
        device: "ice40-lp8k-cm81",
        die: "8k",
        package: "cm81",
        timing: "timings_lp8k.txt",
    },
    Part {
        device: "ice40-u4k-sg48",
        die: "u4k",
        package: "sg48",
        timing: "timings_u4k.txt",
    },
    Part {
        device: "ice40-up5k-sg48",
        die: "5k",
        package: "sg48",
        timing: "timings_up5k.txt",
    },
];

/// The part a device file describes, if the backend places on it.
#[must_use]
pub fn part(device: &str) -> Option<&'static Part> {
    PARTS.iter().find(|p| p.device == device)
}

/// Why a fabric could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FabricError(pub String);

impl fmt::Display for FabricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FabricError {}

/// Where a net is in the [`Arch`]: a global, or a wire declared in a tile.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Home {
    Global(String),
    Tile { x: u32, y: u32, name: String },
}

/// An icebox database expanded into the flow's architecture.
pub struct IceFabric {
    /// The architecture.
    pub arch: Arch,
    /// The database it came from.
    pub db: ChipDb,
    /// The package.
    pub package: String,
    homes: Vec<Home>,
    /// `(x, y, name)` of every net segment to the net.
    segments: HashMap<(u32, u32, String), u32>,
    /// The part's timing figures, when the caller read them: with them,
    /// [`implement`] reports each clock's longest path ([`timing::analyse`]).
    pub timing: Option<timing::TimingDb>,
    /// Whether [`implement`] drives placement and routing by the timing
    /// figures too ([`timing::place_timing`]); only with
    /// [`IceFabric::timing`].
    pub timing_driven: bool,
}

impl IceFabric {
    /// Expands `db` for one package.
    ///
    /// # Errors
    ///
    /// [`FabricError`] for a package the database has no pins for.
    pub fn build(db: ChipDb, package: &str) -> Result<IceFabric, FabricError> {
        if db.package(package).is_none() {
            return Err(FabricError(format!(
                "the {} database has no package `{package}`; it has {}",
                db.device,
                db.packages.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        let mut segments = HashMap::new();
        for (n, segs) in db.nets.iter().enumerate() {
            for (x, y, name) in segs {
                segments.insert((*x, *y, name.clone()), u32::try_from(n).unwrap_or(0));
            }
        }
        let mut arch = Arch::new(format!("ice40-{}", db.device), "ice40", db.width, db.height);
        arch.asc_device = db.device.clone();
        // A tile type per position, corners included: a net's home is the
        // corner of its box, which may be a corner of the die.
        for y in 0..db.height {
            for x in 0..db.width {
                let kind = db.tile(x, y);
                let keyword = match kind {
                    TileKind::Empty => "corner_tile".to_owned(),
                    k => format!("{}_tile", k.keyword()),
                };
                let index = arch.tile_types.len();
                arch.tile_types.push(TileType::new(
                    format!("X{x}Y{y}"),
                    keyword,
                    16,
                    kind.bit_cols(),
                ));
                arch.set_tile(x, y, index);
            }
        }
        let mut homes = Vec::with_capacity(db.nets.len());
        for segs in &db.nets {
            if let Some((_, _, name)) = segs.iter().find(|(_, _, n)| n.starts_with("glb_netwk_")) {
                if !arch.globals.contains(name) {
                    arch.globals.push(name.clone());
                }
                homes.push(Home::Global(name.clone()));
                continue;
            }
            let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
                segs.iter().map(|s| s.0).min(),
                segs.iter().map(|s| s.1).min(),
                segs.iter().map(|s| s.0).max(),
                segs.iter().map(|s| s.1).max(),
            ) else {
                homes.push(Home::Global(String::new()));
                continue;
            };
            let index = homes.len();
            let name = segs
                .iter()
                .find(|(x, y, _)| (*x, *y) == (x0, y0))
                .map_or_else(|| format!("net{index}"), |(_, _, n)| n.clone());
            let ti = (y0 * db.width + x0) as usize;
            arch.tile_types[ti].wires.push(WireDecl {
                name: name.clone(),
                dx: i32::try_from(x1 - x0).unwrap_or(0),
                dy: i32::try_from(y1 - y0).unwrap_or(0),
            });
            homes.push(Home::Tile { x: x0, y: y0, name });
        }
        let mut fabric = IceFabric {
            arch,
            db,
            package: package.to_owned(),
            homes,
            segments,
            timing: None,
            timing_driven: false,
        };
        fabric.add_pips();
        fabric.add_logic_bels()?;
        fabric.add_io_bels();
        fabric.add_ram_bels();
        fabric.add_global_buffers();
        fabric.add_extra_cells();
        let pins: Vec<(String, (u32, u32, u32))> = fabric
            .db
            .package(package)
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        for (pin, (x, y, z)) in pins {
            if fabric.db.tile(x, y) == TileKind::Io {
                fabric.arch.pinmap.push((pin, format!("X{x}Y{y}/io{z}")));
            }
        }
        Ok(fabric)
    }

    /// A reference to net `n` from the tile at `(x, y)`.
    fn wref(&self, n: u32, x: u32, y: u32) -> Option<WireRef> {
        match self.homes.get(n as usize)? {
            Home::Global(name) if name.is_empty() => None,
            Home::Global(name) => Some(WireRef::global(name.clone())),
            Home::Tile { x: hx, y: hy, name } => Some(WireRef::at(
                name.clone(),
                i32::try_from(i64::from(*hx) - i64::from(x)).unwrap_or(0),
                i32::try_from(i64::from(*hy) - i64::from(y)).unwrap_or(0),
            )),
        }
    }

    /// A reference to the wire named `name` in the tile at `(x, y)`, from
    /// the tile at `(fx, fy)`.
    fn named(&self, x: u32, y: u32, name: &str, fx: u32, fy: u32) -> Option<WireRef> {
        let n = *self.segments.get(&(x, y, name.to_owned()))?;
        self.wref(n, fx, fy)
    }

    /// The net a tile's wire name belongs to.
    #[must_use]
    pub fn net_of(&self, x: u32, y: u32, name: &str) -> Option<u32> {
        self.segments.get(&(x, y, name.to_owned())).copied()
    }

    fn tile_mut(&mut self, x: u32, y: u32) -> &mut TileType {
        let w = self.arch.width;
        &mut self.arch.tile_types[(y * w + x) as usize]
    }

    fn add_pips(&mut self) {
        let mut per_tile: BTreeMap<(u32, u32), Vec<PipDecl>> = BTreeMap::new();
        for m in &self.db.muxes {
            let (x, y) = m.tile;
            let Some(to) = self.wref(m.dst, x, y) else {
                continue;
            };
            for &(pattern, src) in &m.options {
                let Some(from) = self.wref(src, x, y) else {
                    continue;
                };
                per_tile.entry((x, y)).or_default().push(PipDecl {
                    from,
                    to: to.clone(),
                    bits: m.set_bits(pattern),
                });
            }
        }
        // A global network is driven by its `.gbufin` tile's `fabout` with
        // no bit of its own: the network is that wire's, unless a pad drives
        // it instead, which this flow never asks for.
        for &(x, y, network) in &self.db.gbufin.clone() {
            if let Some(from) = self.named(x, y, "fabout", x, y) {
                per_tile.entry((x, y)).or_default().push(PipDecl {
                    from,
                    to: WireRef::global(format!("glb_netwk_{network}")),
                    bits: Vec::new(),
                });
            }
        }
        for ((x, y), pips) in per_tile {
            self.tile_mut(x, y).pips = pips;
        }
    }

    fn add_logic_bels(&mut self) -> Result<(), FabricError> {
        let lc_bits: Vec<Vec<ConfigBit>> = (0..8)
            .map(|k| {
                self.db
                    .function_bits(TileKind::Logic, &format!("LC_{k}"))
                    .map(<[ConfigBit]>::to_vec)
                    .unwrap_or_default()
            })
            .collect();
        if lc_bits.iter().any(|b| b.len() != 20) {
            return Err(FabricError(
                "the database's logic tile does not have eight twenty-bit logic cells".into(),
            ));
        }
        let neg = self
            .db
            .function_bits(TileKind::Logic, "NegClk")
            .and_then(|b| b.first().copied());
        let cin_set = self
            .db
            .function_bits(TileKind::Logic, "CarryInSet")
            .and_then(|b| b.first().copied());
        for y in 0..self.db.height {
            for x in 0..self.db.width {
                if self.db.tile(x, y) != TileKind::Logic {
                    continue;
                }
                let mut bels = Vec::new();
                for (k, bits) in lc_bits.iter().enumerate() {
                    let mut bel = BelDecl::new(format!("lc{k}"), "lc");
                    let w = |name: String| self.named(x, y, &name, x, y);
                    let mut pins: Vec<(&str, Option<WireRef>)> = vec![
                        ("i0", w(format!("lutff_{k}/in_0"))),
                        ("i1", w(format!("lutff_{k}/in_1"))),
                        ("i2", w(format!("lutff_{k}/in_2"))),
                        ("i3", w(format!("lutff_{k}/in_3"))),
                        ("o", w(format!("lutff_{k}/out"))),
                        ("cout", w(format!("lutff_{k}/cout"))),
                        ("clk", w("lutff_global/clk".to_owned())),
                        ("en", w("lutff_global/cen".to_owned())),
                        ("rst", w("lutff_global/s_r".to_owned())),
                    ];
                    pins.push((
                        "cin",
                        if k == 0 {
                            w("carry_in_mux".to_owned())
                        } else {
                            w(format!("lutff_{}/cout", k - 1))
                        },
                    ));
                    for (role, wire) in pins {
                        if let Some(wire) = wire {
                            bel.pins.push((role.to_owned(), wire));
                        }
                    }
                    for (i, &p) in LUT_PERM.iter().enumerate() {
                        bel.config.push(ConfigEntry::Param {
                            name: "LUT_INIT".into(),
                            index: u32::try_from(i).unwrap_or(0),
                            at: bits[p],
                        });
                    }
                    for (name, at) in [
                        ("CARRY_ENABLE", LC_CARRY_ENABLE),
                        ("DFF_ENABLE", LC_DFF_ENABLE),
                        ("SET_NORESET", LC_SET_NORESET),
                        ("ASYNC_SR", LC_ASYNC_SR),
                    ] {
                        bel.config.push(ConfigEntry::Param {
                            name: name.into(),
                            index: 0,
                            at: bits[at],
                        });
                    }
                    if let Some(at) = neg {
                        bel.config.push(ConfigEntry::Param {
                            name: "NEG_CLK".into(),
                            index: 0,
                            at,
                        });
                    }
                    if k == 0
                        && let Some(at) = cin_set
                    {
                        bel.config.push(ConfigEntry::Param {
                            name: "CIN_SET".into(),
                            index: 0,
                            at,
                        });
                    }
                    // The same cell with its flip-flop unused: no clock,
                    // enable or set/reset pin, so that it shares a tile with
                    // registered cells on any clock. The two exclude each
                    // other; they are one piece of silicon.
                    let mut comb = bel.clone();
                    comb.name = format!("lcc{k}");
                    comb.kind = "lcc".into();
                    comb.pins
                        .retain(|(role, _)| !matches!(role.as_str(), "clk" | "en" | "rst"));
                    comb.blocks.push(bel.name.clone());
                    bel.blocks.push(comb.name.clone());
                    bels.push(bel);
                    bels.push(comb);
                }
                self.tile_mut(x, y).bels = bels;
            }
        }
        Ok(())
    }

    fn add_io_bels(&mut self) {
        for y in 0..self.db.height {
            for x in 0..self.db.width {
                if self.db.tile(x, y) != TileKind::Io {
                    continue;
                }
                let mut bels = Vec::new();
                for z in 0..2 {
                    let mut bel = BelDecl::new(format!("io{z}"), "io");
                    for (role, name) in [
                        ("din", format!("io_{z}/D_IN_0")),
                        ("din1", format!("io_{z}/D_IN_1")),
                        ("dout", format!("io_{z}/D_OUT_0")),
                        ("dout1", format!("io_{z}/D_OUT_1")),
                        ("oe", format!("io_{z}/OUT_ENB")),
                        ("iclk", "io_global/inclk".to_owned()),
                        ("oclk", "io_global/outclk".to_owned()),
                        ("ce", "io_global/cen".to_owned()),
                    ] {
                        if let Some(wire) = self.named(x, y, &name, x, y) {
                            bel.pins.push((role.to_owned(), wire));
                        }
                    }
                    bels.push(bel);
                }
                self.tile_mut(x, y).bels = bels;
            }
        }
    }

    /// A `bram` bel on every `ramb` tile, for an `SB_RAM40_4K`.
    ///
    /// A block RAM is two tiles, its `ramb` and the `ramt` above it, and
    /// which of its wires is in which differs between members of the
    /// family: on a 1k the write side is in the bottom tile, on the 5k and
    /// the 8k the read side is (nextpnr's `chipdb.py`, "8K-style
    /// memories"). Rather than say which, each wire is looked for in both,
    /// as `icebox_vlog` does; each is in exactly one.
    ///
    /// The pin roles are the ones the device file's `port` lines give the
    /// primitive's ports — the read port first, so `p0_*`, the write port
    /// `p1_*` — and `MASK0`..`MASK15` for the bit mask, which no `port` line
    /// names (`place::describe`).
    fn add_ram_bels(&mut self) {
        let mut pins: Vec<(String, String)> = vec![
            ("p0_clk".into(), "RCLK".into()),
            ("p0_en".into(), "RE".into()),
            ("p0_ce".into(), "RCLKE".into()),
            ("p1_clk".into(), "WCLK".into()),
            ("p1_en".into(), "WE".into()),
            ("p1_ce".into(), "WCLKE".into()),
        ];
        for i in 0..11 {
            pins.push((format!("p0_addr{i}"), format!("RADDR_{i}")));
            pins.push((format!("p1_addr{i}"), format!("WADDR_{i}")));
        }
        for i in 0..16 {
            pins.push((format!("p0_dout{i}"), format!("RDATA_{i}")));
            pins.push((format!("p1_din{i}"), format!("WDATA_{i}")));
            pins.push((format!("MASK{i}"), format!("MASK_{i}")));
        }
        for y in 0..self.db.height {
            for x in 0..self.db.width {
                if self.db.tile(x, y) != TileKind::Ramb {
                    continue;
                }
                let mut bel = BelDecl::new("ram", "bram");
                for (role, name) in &pins {
                    let wire = format!("ram/{name}");
                    let found = self
                        .named(x, y, &wire, x, y)
                        .or_else(|| self.named(x, y + 1, &wire, x, y));
                    if let Some(found) = found {
                        bel.pins.push((role.clone(), found));
                    }
                }
                self.tile_mut(x, y).bels.push(bel);
            }
        }
    }

    /// The RGB LED driver, the oscillators and the single-port RAMs: bels
    /// anchored where the database puts the cell, with pins on the wires
    /// its entries name.
    ///
    /// An `SB_SPRAM256KA`'s entries are its ports bit by bit (`ADDRESS_3`,
    /// `WREN`) and one configuration bit (`SPRAM_EN`, a `CBIT` of the
    /// IP-connect tile, which is not a wire and so gives no pin). The device
    /// file names none of its ports, so a pin's role is the port's name
    /// and bit, `ADDRESS3` and `WREN0` (`place::describe`); and there are
    /// two per corner, so each bel is named after its index there,
    /// `spram1`..`spram4`.
    ///
    /// An `SB_MAC16` is the same shape: a `mac16` bel on each of the eight
    /// DSP blocks' anchor tiles, its pins its ports bit by bit (`A3`,
    /// `CLK0`, `O31`) on the five tiles of the block, and its parameters
    /// `CBIT`s of those tiles, which [`IceFabric::configure_dsps`] writes.
    fn add_extra_cells(&mut self) {
        let cells: Vec<db::ExtraCell> = self.db.extra_cells.clone();
        for cell in cells {
            // A cell the package does not bond, such as the 1k's PLL in its
            // smaller packages, is not there at all.
            if !cell.available_in(&self.package) {
                continue;
            }
            let (kind, ports): (&str, &[(&str, &str)]) = match cell.kind.as_str() {
                "RGBA_DRV" => (
                    "rgba_drv",
                    &[
                        ("curren", "CURREN"),
                        ("rgbleden", "RGBLEDEN"),
                        ("pwm0", "RGB0PWM"),
                        ("pwm1", "RGB1PWM"),
                        ("pwm2", "RGB2PWM"),
                    ],
                ),
                "HFOSC" => (
                    "hfosc",
                    &[("en", "CLKHFEN"), ("pu", "CLKHFPU"), ("o", "CLKHF")],
                ),
                "LFOSC" => (
                    "lfosc",
                    &[("en", "CLKLFEN"), ("pu", "CLKLFPU"), ("o", "CLKLF")],
                ),
                "PLL" => {
                    let bel = self.pll_bel(&cell);
                    let (ax, ay) = cell.tile;
                    self.tile_mut(ax, ay).bels.push(bel);
                    continue;
                }
                "SPRAM" | "MAC16" => {
                    let (ax, ay) = cell.tile;
                    let mut bel = if cell.kind == "SPRAM" {
                        BelDecl::new(format!("spram{}", cell.z), "spram")
                    } else {
                        BelDecl::new("mac16", "mac16")
                    };
                    for (key, x, y, name) in &cell.entries {
                        let role = match key.rsplit_once('_') {
                            Some((port, bit)) if bit.parse::<u32>().is_ok() => {
                                format!("{port}{bit}")
                            }
                            _ => format!("{key}0"),
                        };
                        if let Some(wire) = self.named(*x, *y, name, ax, ay) {
                            bel.pins.push((role, wire));
                        }
                    }
                    self.tile_mut(ax, ay).bels.push(bel);
                    continue;
                }
                // The UltraPlus's other hard IP, and the warm-boot block
                // every die has. Their ports are one bit each in the
                // library (`SBADRI3` is a port, not bit 3 of one), so a pin's
                // role is the port's name and bit 0, `SBADRI30`, as
                // `place::describe` gives a port no `port` line names; an
                // entry that names a configuration bit rather than a wire
                // (`I2C_ENABLE_0`) gives no pin.
                "I2C" | "SPI" | "LEDDA_IP" | "WARMBOOT" => {
                    let (ax, ay) = cell.tile;
                    let kind = cell.kind.to_ascii_lowercase();
                    let mut bel = BelDecl::new(kind.clone(), kind);
                    for (key, x, y, name) in &cell.entries {
                        if let Some(wire) = self.named(*x, *y, name, ax, ay) {
                            bel.pins.push((format!("{key}0"), wire));
                        }
                    }
                    self.tile_mut(ax, ay).bels.push(bel);
                    continue;
                }
                // An IO with switchable pull-ups: the IO block its
                // `PACKAGE_PIN` names gets the two pull-up enables as pins
                // of its own, which an `SB_IO_I3C` placed there reaches and
                // an `SB_IO` leaves alone.
                "IO_I3C" => {
                    let Some((_, px, py, pz)) =
                        cell.entries.iter().find(|(k, ..)| k == "PACKAGE_PIN")
                    else {
                        continue;
                    };
                    let (px, py) = (*px, *py);
                    let bel_name = format!("io{pz}");
                    let pins: Vec<(String, WireRef)> =
                        [("PU_ENB", "pu_enb"), ("WEAK_PU_ENB", "weak_pu_enb")]
                            .iter()
                            .filter_map(|(key, role)| {
                                let (_, x, y, name) =
                                    cell.entries.iter().find(|(k, ..)| k == key)?;
                                Some(((*role).to_owned(), self.named(*x, *y, name, px, py)?))
                            })
                            .collect();
                    if let Some(bel) = self
                        .tile_mut(px, py)
                        .bels
                        .iter_mut()
                        .find(|b| b.name == bel_name)
                    {
                        bel.pins.extend(pins);
                    }
                    continue;
                }
                _ => continue,
            };
            let (ax, ay) = cell.tile;
            let mut bel = BelDecl::new(kind, kind);
            for (role, key) in ports {
                let Some((_, x, y, name)) = cell.entries.iter().find(|(k, ..)| k == key) else {
                    continue;
                };
                if let Some(wire) = self.named(*x, *y, name, ax, ay) {
                    bel.pins.push(((*role).to_owned(), wire));
                }
                // An oscillator's clock is a global network itself, which
                // the global buffer at that network's `.gbufin` tile
                // drives too: the two exclude each other, as a PLL's do.
                if let Some(network) = name
                    .strip_prefix("glb_netwk_")
                    .and_then(|n| n.parse::<u32>().ok())
                    && let Some(&(gx, gy, _)) = self.db.gbufin.iter().find(|g| g.2 == network)
                {
                    bel.blocks.push(format!("X{gx}Y{gy}/gb{network}"));
                }
            }
            self.tile_mut(ax, ay).bels.push(bel);
        }
    }

    /// The PLL's bel, from its `.extra_cell PLL` record.
    ///
    /// Its inputs are the record's wires, with the roles the device file's
    /// `pll` lines give them (`ref` for `REFERENCECLK`; `resetb` and
    /// `bypass` for the two it ties) or else the port's name and bit
    /// (`DYNAMICDELAY3`). Its outputs are not wires in the record but IO
    /// blocks, `PLLOUT_A 12 31 1`: the PLL's clock comes out of that
    /// block's `D_IN_0` into the fabric (`out`, or `outb` for the second),
    /// and onto the global network the block's pad would drive
    /// (`.gbufpin`), as `outglobal` and `outglobalb` — nextpnr's
    /// `chipdb.py`, `add_pll_clock_output`, reads them the same way.
    fn pll_bel(&self, cell: &db::ExtraCell) -> BelDecl {
        let (ax, ay) = cell.tile;
        let mut bel = BelDecl::new("pll", "pll");
        for (key, x, y, name) in &cell.entries {
            let role = match key.as_str() {
                "PLLOUT_A" | "PLLOUT_B" => continue,
                "REFERENCECLK" => "ref".to_owned(),
                "LOCK" => "lock".to_owned(),
                "RESETB" => "resetb".to_owned(),
                "BYPASS" => "bypass".to_owned(),
                other => match other.rsplit_once('_') {
                    Some((port, bit)) if bit.parse::<u32>().is_ok() => format!("{port}{bit}"),
                    _ => format!("{other}0"),
                },
            };
            if let Some(wire) = self.named(*x, *y, name, ax, ay) {
                bel.pins.push((role, wire));
            }
        }
        for (key, fabric, global) in [
            ("PLLOUT_A", "out", "outglobal"),
            ("PLLOUT_B", "outb", "outglobalb"),
        ] {
            let Some((x, y, z)) = pll_output_io(cell, key) else {
                continue;
            };
            if let Some(wire) = self.named(x, y, &format!("io_{z}/D_IN_0"), ax, ay) {
                bel.pins.push((fabric.to_owned(), wire));
            }
            if let Some(&(.., network)) = self
                .db
                .gbufpin
                .iter()
                .find(|g| (g.0, g.1, g.2) == (x, y, z))
            {
                bel.pins.push((
                    global.to_owned(),
                    WireRef::global(format!("glb_netwk_{network}")),
                ));
                // The network has another driver, the global buffer fed
                // from the fabric at its `.gbufin` tile; a placed PLL takes
                // that buffer's network whether or not its global output is
                // used, which costs a design one of eight buffers and keeps
                // the two from ever being placed together.
                if let Some(&(gx, gy, _)) = self.db.gbufin.iter().find(|g| g.2 == network) {
                    bel.blocks.push(format!("X{gx}Y{gy}/gb{network}"));
                }
            }
        }
        bel
    }

    /// The `(x, y, function bits)` of a function of the tile at `(x, y)`.
    fn function(&self, x: u32, y: u32, name: &str) -> Option<&[ConfigBit]> {
        self.db.function_bits(self.db.tile(x, y), name)
    }

    /// Whether an IO block's input enable is active low on this die: on
    /// the 1k it is, so a set `IE` bit is an input *disabled* (nextpnr's
    /// `bitstream.cc`, `set_ie_bit_logical`; `icebox_vlog` reads it the same
    /// way). On every other die a set bit enables the input.
    #[must_use]
    pub fn ie_active_low(&self) -> bool {
        self.db.device == "1k"
    }

    /// Whether a block RAM's `RamConfig.PowerUp` bit is set on the RAMs
    /// *not* in use rather than on those in use: on the 1k it is (nextpnr's
    /// `bitstream.cc` sets it on every unused RAM of an `LP1K` or `HX1K`
    /// and on every used RAM of any other part).
    #[must_use]
    pub fn ram_powerup_when_unused(&self) -> bool {
        self.db.device == "1k"
    }

    /// The bits a placed design needs that no bel's configuration entry
    /// and no pip states; see the module documentation. Returns what was
    /// configured, for the report.
    ///
    /// `ios` is every used IO block as `(x, y, z, pin_type, input_used,
    /// pullup)`; `reserved` the IO blocks a placed PLL's outputs pass
    /// through, which are neither a design's pins nor unused; `extra`
    /// every extra cell's parameters, by kind and anchor tile.
    ///
    /// Per die, as nextpnr's `bitstream.cc` writes them:
    ///
    /// - an IO block's input enable and pull-up are in the block `.ieren`
    ///   names; on the 1k the input enable is active low, and every IO
    ///   block nothing uses has its input disabled, which on the 1k is a
    ///   set bit and on the others no bit at all;
    /// - on the UltraPlus 5k a used block without a pull-up also sets
    ///   `IoCtrl.cf_bit_39` (block 0) or `cf_bit_35` (block 1);
    /// - every column buffer of every global network is enabled, on every
    ///   die but the 384, where nextpnr enables none: the tiles holding
    ///   them are the `.colbuf` sources, which on each other die are
    ///   exactly the rows nextpnr names (`colbuf_rows_are_nextpnrs`);
    /// - every IP-connect and DSP tile's logic cells pass their third input
    ///   through, but for the 5k's tile at (25, 14).
    ///
    /// # Errors
    ///
    /// [`FabricError`] when the database lacks a bit this needs.
    pub fn configure(
        &self,
        image: &mut image::Image,
        ios: &[(u32, u32, u32, u32, bool, bool)],
        reserved: &BTreeSet<db::IoAt>,
        extra: &[ExtraConfig],
    ) -> Result<(), FabricError> {
        let set = |image: &mut image::Image, x: u32, y: u32, bits: &[ConfigBit]| {
            for b in bits {
                image.set(x, y, b.row, b.col);
            }
        };
        let need = |x: u32, y: u32, name: &str| -> Result<Vec<ConfigBit>, FabricError> {
            self.function(x, y, name).map(<[_]>::to_vec).ok_or_else(|| {
                FabricError(format!(
                    "the database has no `{name}` in the tile at ({x}, {y})"
                ))
            })
        };
        let active_low = self.ie_active_low();
        for &(x, y, z, pin_type, input, pullup) in ios {
            for i in 0..6 {
                if pin_type >> i & 1 != 0 {
                    set(image, x, y, &need(x, y, &format!("IOB_{z}.PINTYPE_{i}"))?);
                }
            }
            let (ix, iy, iz) = self
                .db
                .ieren
                .iter()
                .find(|(pio, _)| *pio == (x, y, z))
                .map_or((x, y, z), |(_, at)| *at);
            if input != active_low {
                set(image, ix, iy, &need(ix, iy, &format!("IoCtrl.IE_{iz}"))?);
            }
            if !pullup {
                set(image, ix, iy, &need(ix, iy, &format!("IoCtrl.REN_{iz}"))?);
            }
            if self.db.device == "5k" && !pullup {
                let bit = if iz == 0 {
                    "IoCtrl.cf_bit_39"
                } else {
                    "IoCtrl.cf_bit_35"
                };
                set(image, ix, iy, &need(ix, iy, bit)?);
            }
        }
        if active_low {
            let used: BTreeSet<db::IoAt> = ios.iter().map(|io| (io.0, io.1, io.2)).collect();
            for &(pio, (ix, iy, iz)) in &self.db.ieren {
                if used.contains(&pio) || reserved.contains(&pio) {
                    continue;
                }
                set(image, ix, iy, &need(ix, iy, &format!("IoCtrl.IE_{iz}"))?);
            }
        }
        if self.db.device != "384" {
            // Every column buffer of every network.
            let sources: BTreeSet<(u32, u32)> = self.db.colbuf.iter().map(|(s, _)| *s).collect();
            for (x, y) in sources {
                for n in 0..8 {
                    set(
                        image,
                        x,
                        y,
                        &need(x, y, &format!("ColBufCtrl.glb_netwk_{n}"))?,
                    );
                }
            }
        }
        // Every IP-connect and DSP tile's pass-through cells, but one:
        // nextpnr leaves the 5k's IP-connect tile at (25, 14) alone
        // ("mystery bits not set in this one tile", `bitstream.cc`), and
        // sets only its first cell when the DSP block below uses it for its
        // carry out, which `configure_dsps` does.
        for y in 0..self.db.height {
            for x in 0..self.db.width {
                let kind = self.db.tile(x, y);
                if !matches!(kind, TileKind::Ipcon | TileKind::Dsp(_))
                    || (self.db.device == "5k" && (x, y) == SKIPPED_IPCON)
                {
                    continue;
                }
                for k in 0..8 {
                    let bits = need(x, y, &format!("LC_{k}"))?;
                    for (b, c) in bits.iter().zip(IPCON_LC_PATTERN.chars()) {
                        if c == '1' {
                            image.set(x, y, b.row, b.col);
                        }
                    }
                }
                let cascades: Vec<Vec<ConfigBit>> = self
                    .db
                    .tile_bits
                    .get(&kind)
                    .map(|fs| {
                        fs.iter()
                            .filter(|(n, _)| n.starts_with("Cascade."))
                            .map(|(_, b)| b.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for bits in cascades {
                    set(image, x, y, &bits);
                }
            }
        }
        for (kind, tile, params) in extra {
            let Some(cell) = self
                .db
                .extra_cells
                .iter()
                .find(|c| &c.kind == kind && c.tile == *tile)
            else {
                return Err(FabricError(format!(
                    "the database has no `{kind}` cell at ({}, {})",
                    tile.0, tile.1
                )));
            };
            for (key, x, y, name) in &cell.entries {
                // A key `NAME_<i>` is bit `i` of parameter `NAME`; a plain
                // `NAME` is bit 0.
                let (param, bit) = match key.rsplit_once('_') {
                    Some((p, i)) if i.parse::<u32>().is_ok() && params.contains_key(p) => {
                        (p, i.parse::<u32>().unwrap_or(0))
                    }
                    _ => (key.as_str(), 0),
                };
                let Some(value) = params.get(param) else {
                    continue;
                };
                if value >> bit & 1 == 0 {
                    continue;
                }
                let bits = self
                    .db
                    .function_by_suffix(self.db.tile(*x, *y), name)
                    .ok_or_else(|| {
                        FabricError(format!(
                            "`{kind}`'s `{key}` names `{name}` at ({x}, {y}), which that tile \
                             does not have"
                        ))
                    })?
                    .to_vec();
                set(image, *x, *y, &bits);
            }
        }
        Ok(())
    }

    /// Writes the block RAMs' and single-port RAMs' configuration: for a
    /// block RAM, `RamConfig.PowerUp` in its `ramb` tile, `WRITE_MODE` and
    /// `READ_MODE` in `RamConfig.CBIT_0`..`3` of its `ramt` tile, and its
    /// contents as the image's `.ram_data`; for a single-port RAM, the
    /// `SPRAM_EN` bit its `.extra_cell` record names.
    ///
    /// Which bit is which is nextpnr's `bitstream.cc`, and `icebox_vlog`
    /// reads them back the same way. A set `PowerUp` is a RAM in use on
    /// every die but the 1k, where it is the other way round: there the bit
    /// is set on every block RAM *not* in use
    /// ([`IceFabric::ram_powerup_when_unused`]).
    ///
    /// # Errors
    ///
    /// [`FabricError`] when the database lacks a bit, or for a single-port
    /// RAM on a die that has none.
    pub fn configure_rams(
        &self,
        image: &mut image::Image,
        rams: &BTreeMap<(u32, u32), RamConfig>,
        sprams: &BTreeSet<((u32, u32), u32)>,
    ) -> Result<(), FabricError> {
        let need = |x: u32, y: u32, name: &str| -> Result<Vec<ConfigBit>, FabricError> {
            self.function(x, y, name).map(<[_]>::to_vec).ok_or_else(|| {
                FabricError(format!(
                    "the database has no `{name}` in the tile at ({x}, {y})"
                ))
            })
        };
        let set = |image: &mut image::Image, x: u32, y: u32, bits: &[ConfigBit]| {
            for b in bits {
                image.set(x, y, b.row, b.col);
            }
        };
        let inverted = self.ram_powerup_when_unused();
        for y in 0..self.db.height {
            for x in 0..self.db.width {
                if self.db.tile(x, y) == TileKind::Ramb && rams.contains_key(&(x, y)) != inverted {
                    set(image, x, y, &need(x, y, "RamConfig.PowerUp")?);
                }
            }
        }
        for (&(x, y), ram) in rams {
            for (bit, on) in [
                (0, ram.write_mode & 1 != 0),
                (1, ram.write_mode & 2 != 0),
                (2, ram.read_mode & 1 != 0),
                (3, ram.read_mode & 2 != 0),
            ] {
                if on {
                    set(
                        image,
                        x,
                        y + 1,
                        &need(x, y + 1, &format!("RamConfig.CBIT_{bit}"))?,
                    );
                }
            }
            if ram.contents.iter().any(|line| line.iter().any(|b| *b)) {
                image.ram_data.insert((x, y), ram.contents.clone());
            }
        }
        for &(tile, z) in sprams {
            let (x, y, name) = self.spram_enable(tile, z).ok_or_else(|| {
                FabricError(format!(
                    "the database has no SPRAM {z} at ({}, {}) with an `SPRAM_EN`",
                    tile.0, tile.1
                ))
            })?;
            let bits = self
                .db
                .function_by_suffix(self.db.tile(x, y), &name)
                .ok_or_else(|| FabricError(format!("no `{name}` in the tile at ({x}, {y})")))?
                .to_vec();
            set(image, x, y, &bits);
        }
        Ok(())
    }

    /// Where the `SPRAM_EN` bit of the single-port RAM `z` anchored at
    /// `tile` is: `(x, y, function)`.
    fn spram_enable(&self, tile: (u32, u32), z: u32) -> Option<(u32, u32, String)> {
        let cell = self
            .db
            .extra_cells
            .iter()
            .find(|c| c.kind == "SPRAM" && c.tile == tile && c.z == z)?;
        cell.entries
            .iter()
            .find(|(key, ..)| key == "SPRAM_EN")
            .map(|(_, x, y, name)| (*x, *y, name.clone()))
    }

    /// Writes the PLL's configuration: its parameters into the
    /// `PLLCONFIG_<i>` bits the `.extra_cell PLL` record names; each
    /// output's IO block set to pass the clock to the fabric, `PINTYPE_0`
    /// and, with its ICEGATE enabled, `PINTYPE_1` (nextpnr's
    /// `bitstream.cc`, "Configure the SB_IO that the clock outputs are
    /// going through"); and, for each output that drives its global
    /// network, that network's `padin_glb_netwk` extra bit, the one a pad
    /// driving the network sets. Returns the networks so driven.
    ///
    /// `tile` is the PLL's anchor: the 8k has two, at the middle of its
    /// bottom and top edges.
    ///
    /// # Errors
    ///
    /// [`FabricError`] when the database lacks the PLL or a bit.
    pub fn configure_pll(
        &self,
        image: &mut image::Image,
        tile: (u32, u32),
        config: &PllConfig,
        globals: [bool; 2],
    ) -> Result<BTreeSet<u32>, FabricError> {
        let cell = self.pll_at(tile).ok_or_else(|| {
            FabricError(format!(
                "the {} database has no PLL at ({}, {})",
                self.db.device, tile.0, tile.1
            ))
        })?;
        for (key, x, y, name) in &cell.entries {
            let Some((param, bit)) = pll_param_bit(key) else {
                continue;
            };
            if config.params.get(param).copied().unwrap_or(0) >> bit & 1 == 0 {
                continue;
            }
            let bits = self
                .db
                .function_by_suffix(self.db.tile(*x, *y), name)
                .ok_or_else(|| {
                    FabricError(format!(
                        "the PLL's `{key}` names `{name}` at ({x}, {y}), which that tile does not \
                         have"
                    ))
                })?;
            for b in bits {
                image.set(*x, *y, b.row, b.col);
            }
        }
        let mut networks = BTreeSet::new();
        for (i, (key, icegate)) in [
            ("PLLOUT_A", "ENABLE_ICEGATE_PORTA"),
            ("PLLOUT_B", "ENABLE_ICEGATE_PORTB"),
        ]
        .into_iter()
        .enumerate()
        {
            if i == 1 && !config.dual() {
                continue;
            }
            let (x, y, z) = pll_output_io(cell, key)
                .ok_or_else(|| FabricError(format!("the PLL record has no `{key}`")))?;
            let mut names = vec![format!("IOB_{z}.PINTYPE_0")];
            if config.params.get(icegate).copied().unwrap_or(0) != 0 {
                names.push(format!("IOB_{z}.PINTYPE_1"));
            }
            for name in names {
                let bits = self
                    .function(x, y, &name)
                    .ok_or_else(|| FabricError(format!("no `{name}` in the tile at ({x}, {y})")))?;
                for b in bits {
                    image.set(x, y, b.row, b.col);
                }
            }
            if globals[i] {
                let network = self
                    .db
                    .gbufpin
                    .iter()
                    .find(|g| (g.0, g.1, g.2) == (x, y, z))
                    .map(|g| g.3)
                    .ok_or_else(|| {
                        FabricError(format!("the IO block at ({x}, {y}, {z}) drives no global"))
                    })?;
                self.set_padin(image, network)?;
                networks.insert(network);
            }
        }
        Ok(networks)
    }

    /// The `.extra_cell PLL` record anchored at `tile`, if the package has
    /// that PLL.
    #[must_use]
    pub fn pll_at(&self, tile: (u32, u32)) -> Option<&db::ExtraCell> {
        self.plls().find(|c| c.tile == tile)
    }

    /// Every PLL the package has: none on the 384 or in the smaller
    /// packages of the 1k, two on the 8k in most of its packages.
    pub fn plls(&self) -> impl Iterator<Item = &db::ExtraCell> {
        self.db
            .extra_cells
            .iter()
            .filter(|c| c.kind == "PLL" && c.available_in(&self.package))
    }

    /// Sets the extra bit that has global network `network` driven from its
    /// pad-input path, `padin_glb_netwk.<n>`.
    fn set_padin(&self, image: &mut image::Image, network: u32) -> Result<(), FabricError> {
        let (_, bank, x, y) = self
            .db
            .extra_bits
            .iter()
            .find(|(n, ..)| *n == format!("padin_glb_netwk.{network}"))
            .ok_or_else(|| FabricError(format!("no `padin_glb_netwk.{network}` extra bit")))?;
        image
            .extra_bits
            .insert((usize::try_from(*bank).unwrap_or(0), *x, *y));
        Ok(())
    }

    /// Refuses an IO buffer that only some pads have on a pad without it:
    /// an `SB_IO_I3C` anywhere but an IO block an `.extra_cell IO_I3C`
    /// record names (pins 23 and 25 of the UltraPlus's SG48), whose pull-up
    /// enables reach the fabric there and nowhere else; and an `SB_IO_OD`
    /// anywhere but one of the three pads the RGB LED driver can drive, the
    /// UltraPlus's open-drain pads (IceStorm's `ultraplus.rst`: with the
    /// driver off, "the pin can be used as an open-drain IO using the
    /// standard IO cell").
    fn check_special_io(
        &self,
        primitive: &str,
        cell: &crate::ir::Cell,
        at: db::IoAt,
    ) -> Result<(), FabricError> {
        let pads = |kind: &str, keys: &[&str]| -> Vec<db::IoAt> {
            self.db
                .extra_cells
                .iter()
                .filter(|c| c.kind == kind)
                .flat_map(|c| {
                    c.entries
                        .iter()
                        .filter(|(k, ..)| keys.contains(&k.as_str()))
                        .filter_map(|(_, x, y, z)| Some((*x, *y, z.parse().ok()?)))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        let (allowed, what) = match primitive {
            "SB_IO_I3C" => (pads("IO_I3C", &["PACKAGE_PIN"]), "an I3C pad"),
            "SB_IO_OD" => (
                pads("RGBA_DRV", &["RGB0", "RGB1", "RGB2"]),
                "one of the open-drain pads the RGB LED driver drives",
            ),
            _ => return Ok(()),
        };
        if allowed.contains(&at) {
            return Ok(());
        }
        Err(FabricError(format!(
            "`{}` is an `{primitive}`, which sits only on {what}; the IO block at ({}, {}, {}) is \
             not one{}",
            cell.name.as_str(),
            at.0,
            at.1,
            at.2,
            if allowed.is_empty() {
                format!(" and the {} die has none", self.db.device)
            } else {
                String::new()
            }
        )))
    }

    /// The `.extra_cell MAC16` record of the DSP block anchored at `tile`.
    fn mac16_at(&self, tile: (u32, u32)) -> Option<&db::ExtraCell> {
        self.db
            .extra_cells
            .iter()
            .find(|c| c.kind == "MAC16" && c.tile == tile)
    }

    /// Writes each placed DSP block's parameters into the bits its
    /// `.extra_cell MAC16` record names — each `CBIT_<i>` an `IpConfig`
    /// bit of one of the block's tiles, as nextpnr's `bitstream.cc` writes
    /// them — and sets the logic cell its carry out leaves through to pass
    /// it on, as nextpnr does for the one tile whose cells are otherwise
    /// left alone ([`SKIPPED_IPCON`]).
    ///
    /// # Errors
    ///
    /// [`FabricError`] for a tile with no DSP block, or when the database
    /// lacks a bit.
    pub fn configure_dsps(
        &self,
        image: &mut image::Image,
        dsps: &BTreeMap<(u32, u32), DspConfig>,
    ) -> Result<(), FabricError> {
        for (&tile, config) in dsps {
            let cell = self.mac16_at(tile).ok_or_else(|| {
                FabricError(format!(
                    "the database has no DSP block at ({}, {})",
                    tile.0, tile.1
                ))
            })?;
            for (key, x, y, name) in &cell.entries {
                let Some((param, bit)) = mac16_param_bit(key) else {
                    continue;
                };
                if config.params.get(param).copied().unwrap_or(0) >> bit & 1 == 0 {
                    continue;
                }
                let bits = self
                    .db
                    .function_by_suffix(self.db.tile(*x, *y), name)
                    .ok_or_else(|| {
                        FabricError(format!(
                            "the DSP block's `{key}` names `{name}` at ({x}, {y}), which that \
                             tile does not have"
                        ))
                    })?;
                for b in bits {
                    image.set(*x, *y, b.row, b.col);
                }
            }
            // The carry out is the first logic cell's output of the tile
            // above the block (`slf_op_0`), passed through like every
            // IP-connect tile's cells.
            if let Some((_, x, y, name)) = cell.entries.iter().find(|(k, ..)| k == "CO")
                && name == "slf_op_0"
            {
                let kind = self.db.tile(*x, *y);
                let lc = self
                    .db
                    .function_bits(kind, "LC_0")
                    .ok_or_else(|| FabricError(format!("no `LC_0` in the tile at ({x}, {y})")))?;
                for (b, c) in lc.iter().zip(IPCON_LC_PATTERN.chars()) {
                    if c == '1' {
                        image.set(*x, *y, b.row, b.col);
                    }
                }
                let cascade = self.db.tile_bits.get(&kind).and_then(|fs| {
                    fs.iter()
                        .find(|(n, _)| n.starts_with("Cascade.") && n.contains("_LC00_"))
                });
                if let Some((_, bits)) = cascade {
                    for b in bits {
                        image.set(*x, *y, b.row, b.col);
                    }
                }
            }
        }
        Ok(())
    }

    /// Reads an image back through the database: every multiplexer whose
    /// field is not all zero becomes an arc, every named function bit is
    /// accounted for, and every set bit that is neither is reported. Block
    /// RAMs and single-port RAMs are read back too, configuration and
    /// contents, and contents for a block RAM that is not in use are
    /// reported as unexplained.
    #[must_use]
    pub fn decode(&self, image: &image::Image) -> Decoded {
        let mut by_tile: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
        for (i, m) in self.db.muxes.iter().enumerate() {
            by_tile.entry(m.tile).or_default().push(i);
        }
        let mut out = Decoded::default();
        for (&(x, y), set) in &image.tiles {
            out.bits += set.len();
            let mut explained: BTreeSet<(u32, u32)> = BTreeSet::new();
            for &i in by_tile.get(&(x, y)).map_or(&[][..], Vec::as_slice) {
                let m = &self.db.muxes[i];
                let n = m.bits.len();
                let mut value = 0u32;
                for (j, b) in m.bits.iter().enumerate() {
                    if set.contains(&(b.row, b.col)) {
                        value |= 1 << (n - 1 - j);
                    }
                }
                if value == 0 {
                    continue;
                }
                match m.options.iter().find(|(p, _)| *p == value) {
                    Some(&(_, src)) => {
                        out.arcs.insert(((x, y), m.dst, src));
                        for b in &m.bits {
                            explained.insert((b.row, b.col));
                        }
                    }
                    None => out.bad_patterns.push(format!(
                        "X{x}Y{y} net {}: pattern {value:0n$b} selects nothing",
                        m.dst
                    )),
                }
            }
            if let Some(functions) = self.db.tile_bits.get(&self.db.tile(x, y)) {
                for (name, bits) in functions {
                    let hit: Vec<ConfigBit> = bits
                        .iter()
                        .filter(|b| set.contains(&(b.row, b.col)))
                        .copied()
                        .collect();
                    if hit.is_empty() {
                        continue;
                    }
                    for b in &hit {
                        explained.insert((b.row, b.col));
                    }
                    out.functions.insert(((x, y), name.clone()), hit);
                }
            }
            for &(r, c) in set {
                if !explained.contains(&(r, c)) {
                    out.unexplained.push(format!("X{x}Y{y} B{r}[{c}]"));
                }
            }
        }
        for &(bank, x, y) in &image.extra_bits {
            out.bits += 1;
            if !self
                .db
                .extra_bits
                .iter()
                .any(|(_, b, bx, by)| (*b as usize, *bx, *by) == (bank, x, y))
            {
                out.unexplained.push(format!("extra bit {bank} {x} {y}"));
            }
        }
        // The block RAMs: in use when `PowerUp` is set (clear, on the 1k),
        // their modes from the top tile, their contents from `.ram_data`.
        let on = |x: u32, y: u32, name: &str| {
            self.function(x, y, name)
                .is_some_and(|bits| bits.iter().all(|b| image.get(x, y, b.row, b.col)))
        };
        for y in 0..self.db.height {
            for x in 0..self.db.width {
                if self.db.tile(x, y) != TileKind::Ramb
                    || on(x, y, "RamConfig.PowerUp") == self.ram_powerup_when_unused()
                {
                    continue;
                }
                let cbit = |i: u32| u32::from(on(x, y + 1, &format!("RamConfig.CBIT_{i}")));
                let contents = image
                    .ram_data
                    .get(&(x, y))
                    .cloned()
                    .unwrap_or_else(|| vec![[false; 256]; 16]);
                out.ram_bits += contents
                    .iter()
                    .map(|l| l.iter().filter(|b| **b).count())
                    .sum::<usize>();
                out.rams.insert(
                    (x, y),
                    RamConfig {
                        write_mode: cbit(0) | cbit(1) << 1,
                        read_mode: cbit(2) | cbit(3) << 1,
                        contents,
                    },
                );
            }
        }
        for (&(x, y), lines) in &image.ram_data {
            if !out.rams.contains_key(&(x, y)) && lines.iter().any(|l| l.iter().any(|b| *b)) {
                out.unexplained.push(format!(
                    "contents for a block RAM at X{x}Y{y}, which is not one in use"
                ));
            }
        }
        for cell in self.db.extra_cells.iter().filter(|c| c.kind == "SPRAM") {
            if let Some((x, y, name)) = self.spram_enable(cell.tile, cell.z)
                && self
                    .db
                    .function_by_suffix(self.db.tile(x, y), &name)
                    .is_some_and(|bits| bits.iter().all(|b| image.get(x, y, b.row, b.col)))
            {
                out.sprams.insert((cell.tile, cell.z));
            }
        }
        // Every DSP block's parameters, used or not: a block whose bits are
        // all clear reads back as every parameter zero.
        for cell in self.db.extra_cells.iter().filter(|c| c.kind == "MAC16") {
            let mut config = DspConfig::zero();
            for (key, x, y, name) in &cell.entries {
                let Some((param, bit)) = mac16_param_bit(key) else {
                    continue;
                };
                let set = self
                    .db
                    .function_by_suffix(self.db.tile(*x, *y), name)
                    .is_some_and(|bits| bits.iter().all(|b| image.get(*x, *y, b.row, b.col)));
                if set && let Some(v) = config.params.get_mut(param) {
                    *v |= 1 << bit;
                }
            }
            out.dsps.insert(cell.tile, config);
        }
        // Each PLL's parameters, and which global networks a pad (or a PLL
        // through one) drives.
        for cell in self.plls() {
            let mut config = PllConfig::zero();
            for (key, x, y, name) in &cell.entries {
                let Some((param, bit)) = pll_param_bit(key) else {
                    continue;
                };
                let set = self
                    .db
                    .function_by_suffix(self.db.tile(*x, *y), name)
                    .is_some_and(|bits| bits.iter().all(|b| image.get(*x, *y, b.row, b.col)));
                if set && let Some(v) = config.params.get_mut(param) {
                    *v |= 1 << bit;
                }
            }
            out.plls.insert(cell.tile, config);
        }
        // Every hard cell's configuration bits: the RGB driver's, the high
        // frequency oscillator's divider, the I2C and SPI blocks' enables
        // and delays.
        for cell in &self.db.extra_cells {
            if !HARD_IP_KINDS.contains(&cell.kind.as_str()) || !cell.available_in(&self.package) {
                continue;
            }
            let mut params: BTreeMap<String, u64> = BTreeMap::new();
            for (key, x, y, name) in &cell.entries {
                let Some(bits) = self.db.function_by_suffix(self.db.tile(*x, *y), name) else {
                    continue;
                };
                if !bits.iter().all(|b| image.get(*x, *y, b.row, b.col)) {
                    continue;
                }
                let (param, bit) = param_and_bit(cell, key);
                *params.entry(param).or_default() |= 1 << bit;
            }
            if !params.is_empty() {
                out.hard_ip.insert((cell.kind.clone(), cell.tile), params);
            }
        }
        for (name, bank, x, y) in &self.db.extra_bits {
            if let Some(n) = name
                .strip_prefix("padin_glb_netwk.")
                .and_then(|n| n.parse::<u32>().ok())
                && image
                    .extra_bits
                    .contains(&(usize::try_from(*bank).unwrap_or(0), *x, *y))
            {
                out.padin.insert(n);
            }
        }
        out
    }

    /// The arcs of the database a routed pip of the graph stands for, as
    /// `((x, y), dst, src)`: every pip with bits, mapped back to the nets
    /// its two wires are. The two sets this and [`IceFabric::decode`]
    /// produce are what must be equal.
    #[must_use]
    pub fn net_of_node(&self, wire: &super::arch::Wire) -> Option<u32> {
        if wire.global {
            return self
                .homes
                .iter()
                .position(|h| matches!(h, Home::Global(n) if *n == wire.name))
                .and_then(|i| u32::try_from(i).ok());
        }
        self.net_of(wire.tile.0, wire.tile.1, &wire.name)
            .or_else(|| {
                wire.name
                    .strip_prefix("net")
                    .and_then(|n| n.parse::<u32>().ok())
            })
    }
}

impl IceFabric {
    /// The global network a design's `SB_GB` reaches from each `.gbufin`
    /// tile: a `gb` bel per entry, its input the tile's `fabout` and its
    /// output the network.
    fn add_global_buffers(&mut self) {
        for &(x, y, network) in &self.db.gbufin.clone() {
            let Some(input) = self.named(x, y, "fabout", x, y) else {
                continue;
            };
            let mut bel = BelDecl::new(format!("gb{network}"), "gb");
            bel.pins.push(("i".into(), input));
            bel.pins
                .push(("o".into(), WireRef::global(format!("glb_netwk_{network}"))));
            self.tile_mut(x, y).bels.push(bel);
        }
    }

    /// Everything [`IceFabric::configure`] needs from a placed and routed
    /// design, then that configuration written into `image`, which already
    /// holds what the pips and the bels' entries set.
    ///
    /// Returns what was configured, which for the RAMs is also what
    /// [`IceFabric::decode`] must read back.
    ///
    /// # Errors
    ///
    /// [`FabricError`] when the design mixes clock edges in one logic tile,
    /// which the tile's single `NegClk` bit cannot hold, when a RAM's
    /// parameters are not ones it can hold, or when a bit the database
    /// should have is missing.
    pub fn configure_placed(
        &self,
        design: &crate::ir::Design,
        module: crate::ir::ModuleId,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        image: &mut image::Image,
    ) -> Result<Configured, FabricError> {
        let m = design
            .modules
            .get(module)
            .ok_or_else(|| FabricError("no such module".into()))?;
        let param = |cell: &crate::ir::Cell, key: &str| -> Option<u64> {
            match cell.params.get(key)? {
                crate::ir::AttrValue::String(s) => {
                    let digits = s.strip_prefix("0b").unwrap_or(s);
                    u64::from_str_radix(digits, 2).ok()
                }
                other => other.as_int().and_then(|v| u64::try_from(v).ok()),
            }
        };
        let mut ios = Vec::new();
        let mut extra: Vec<ExtraConfig> = Vec::new();
        let mut rams: BTreeMap<(u32, u32), RamConfig> = BTreeMap::new();
        let mut sprams: BTreeSet<((u32, u32), u32)> = BTreeSet::new();
        let mut dsps: BTreeMap<(u32, u32), DspConfig> = BTreeMap::new();
        let mut plls: BTreeMap<(u32, u32), (PllConfig, [bool; 2])> = BTreeMap::new();
        let mut osc_networks: BTreeSet<u32> = BTreeSet::new();
        let mut edges: BTreeMap<(u32, u32), BTreeSet<bool>> = BTreeMap::new();
        for (index, instance) in netlist.instances.iter().enumerate() {
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let cell = &m.cells[instance.cell];
            match instance.kind.as_str() {
                "io" => {
                    let z = u32::from(site.bel.ends_with('1'));
                    self.check_special_io(
                        &instance.primitive,
                        cell,
                        (site.tile.0, site.tile.1, z),
                    )?;
                    let input = netlist.pins.iter().any(|p| {
                        p.instance == index
                            && (p.role == "din" || p.role == "din1")
                            && p.signal.is_some()
                    });
                    let pin_type = param(cell, "PIN_TYPE").unwrap_or(1);
                    let pullup = param(cell, "PULLUP").unwrap_or(0) != 0;
                    ios.push((
                        site.tile.0,
                        site.tile.1,
                        z,
                        u32::try_from(pin_type & 0x3F).unwrap_or(1),
                        input,
                        pullup,
                    ));
                }
                "lc" | "lcc" => {
                    if param(cell, "DFF_ENABLE").unwrap_or(0) != 0 {
                        edges
                            .entry(site.tile)
                            .or_default()
                            .insert(param(cell, "NEG_CLK").unwrap_or(0) != 0);
                    }
                }
                "rgba_drv" => {
                    let mut p = BTreeMap::new();
                    p.insert("RGBA_DRV_EN".to_owned(), 1);
                    for key in [
                        "CURRENT_MODE",
                        "RGB0_CURRENT",
                        "RGB1_CURRENT",
                        "RGB2_CURRENT",
                    ] {
                        p.insert(key.to_owned(), param(cell, key).unwrap_or(0));
                    }
                    extra.push(("RGBA_DRV".to_owned(), site.tile, p));
                }
                "hfosc" | "lfosc" => {
                    let mut p = BTreeMap::new();
                    if instance.kind == "hfosc" {
                        p.insert(
                            "CLKHF_DIV".to_owned(),
                            param(cell, "CLKHF_DIV").unwrap_or(0),
                        );
                        extra.push(("HFOSC".to_owned(), site.tile, p));
                    } else {
                        extra.push(("LFOSC".to_owned(), site.tile, p));
                    }
                    // Its clock reaches its global network through that
                    // network's pad-input path, as nextpnr's packer puts
                    // a pad-input global buffer behind it
                    // (`create_padin_gbuf`).
                    let drives = netlist
                        .pins
                        .iter()
                        .any(|p| p.instance == index && p.role == "o" && p.signal.is_some());
                    if drives
                        && let Some(network) = site
                            .pin("o")
                            .map(|n| graph.wire(n).name.as_str())
                            .and_then(|n| n.strip_prefix("glb_netwk_"))
                            .and_then(|n| n.parse::<u32>().ok())
                    {
                        osc_networks.insert(network);
                    }
                }
                "bram" => {
                    rams.insert(site.tile, RamConfig::of_cell(cell)?);
                }
                // The I2C block's input and output delays on SDA are on
                // unless an attribute says otherwise, and both of its enable
                // bits and all four of the SPI block's are set whenever the
                // block is used: nextpnr's `bitstream.cc`. The LED dimmer and
                // the warm-boot block have no bit at all.
                "i2c" => {
                    let delay = |key: &str| match cell.attrs.get(key) {
                        None => 1,
                        Some(v) => u64::from(attr_true(v)),
                    };
                    let mut p = BTreeMap::new();
                    p.insert("I2C_ENABLE".to_owned(), 3);
                    p.insert("SDA_INPUT_DELAYED".to_owned(), delay("SDA_INPUT_DELAYED"));
                    p.insert("SDA_OUTPUT_DELAYED".to_owned(), delay("SDA_OUTPUT_DELAYED"));
                    extra.push(("I2C".to_owned(), site.tile, p));
                }
                "spi" => {
                    let mut p = BTreeMap::new();
                    p.insert("SPI_ENABLE".to_owned(), 15);
                    extra.push(("SPI".to_owned(), site.tile, p));
                }
                "ledda_ip" => extra.push(("LEDDA_IP".to_owned(), site.tile, BTreeMap::new())),
                "warmboot" => extra.push(("WARMBOOT".to_owned(), site.tile, BTreeMap::new())),
                "mac16" => {
                    dsps.insert(site.tile, DspConfig::of_cell(cell)?.as_bits(cell)?);
                }
                "pll" => {
                    let used = |role: &str| {
                        netlist
                            .pins
                            .iter()
                            .any(|p| p.instance == index && p.role == role && p.signal.is_some())
                    };
                    plls.insert(
                        site.tile,
                        (
                            PllConfig::of_cell(cell, &instance.primitive)?,
                            [used("outglobal"), used("outglobalb")],
                        ),
                    );
                }
                "spram" => {
                    let z = site
                        .bel
                        .rsplit('/')
                        .next()
                        .and_then(|b| b.strip_prefix("spram"))
                        .and_then(|z| z.parse::<u32>().ok())
                        .ok_or_else(|| {
                            FabricError(format!("`{}` is not a single-port RAM site", site.bel))
                        })?;
                    sprams.insert((site.tile, z));
                }
                _ => {}
            }
        }
        // The placer keeps the two edges apart (`place::slice_mode`); this
        // is the check that it did.
        if let Some((tile, _)) = edges.iter().find(|(_, e)| e.len() > 1) {
            return Err(FabricError(format!(
                "the logic tile at ({}, {}) holds flip-flops on both clock edges; an iCE40 logic \
                 tile has one clock-polarity bit for all eight cells",
                tile.0, tile.1
            )));
        }
        // A placed PLL's outputs leave through IO blocks, which a design's
        // own IO cannot use then, and which are not unused either.
        let mut reserved: BTreeSet<db::IoAt> = BTreeSet::new();
        for (&tile, (config, _)) in &plls {
            let Some(cell) = self.pll_at(tile) else {
                continue;
            };
            for (i, key) in ["PLLOUT_A", "PLLOUT_B"].into_iter().enumerate() {
                if i == 1 && !config.dual() {
                    continue;
                }
                // A pad form's first output leaves through its own pad,
                // whose IO buffer is the design's: its input is the PLL's
                // reference, so it is enabled whatever the fabric reads of
                // it (nextpnr's `bitstream.cc`, `sb_io_used_by_pll_pad`).
                if i == 0
                    && config.pad()
                    && let Some(at) = pll_output_io(cell, key)
                {
                    if ios.iter().any(|io| (io.0, io.1, io.2) == at) {
                        return Err(FabricError(format!(
                            "the pad of the PLL at ({}, {}) is also used as a pin of its own",
                            tile.0, tile.1
                        )));
                    }
                    // Its `PINTYPE` bits are the PLL's, set below.
                    ios.push((at.0, at.1, at.2, 0, true, false));
                    continue;
                }
                if let Some((x, y, z)) = pll_output_io(cell, key) {
                    if ios.iter().any(|io| (io.0, io.1, io.2) == (x, y, z)) {
                        return Err(FabricError(format!(
                            "the PLL's {key} leaves through the IO block at ({x}, {y}, {z}), \
                             which this design also uses as a pin"
                        )));
                    }
                    reserved.insert((x, y, z));
                }
            }
        }
        self.configure(image, &ios, &reserved, &extra)?;
        self.configure_rams(image, &rams, &sprams)?;
        self.configure_dsps(image, &dsps)?;
        let mut padin = BTreeSet::new();
        for (&tile, (config, globals)) in &plls {
            padin.extend(self.configure_pll(image, tile, config, *globals)?);
        }
        for network in osc_networks {
            self.set_padin(image, network)?;
            padin.insert(network);
        }
        Ok(Configured {
            pads: ios.len(),
            hard_cells: extra.len(),
            hard_ip: hard_ip_wanted(self, &extra),
            rams,
            sprams,
            dsps,
            plls: plls.into_iter().map(|(t, (c, _))| (t, c)).collect(),
            padin,
        })
    }

    /// The arcs the router chose that cost bits, as the database's
    /// `((x, y), destination net, source net)`: what [`IceFabric::decode`]
    /// must read back.
    #[must_use]
    pub fn routed_arcs(
        &self,
        graph: &super::arch::RoutingGraph,
        routing: &super::route::Routing,
        signals: usize,
    ) -> BTreeSet<((u32, u32), u32, u32)> {
        let mut out = BTreeSet::new();
        for s in 0..signals {
            for &pip in routing.pips(s) {
                if graph.pip_bits(pip).is_empty() {
                    continue;
                }
                let p = graph.pip(pip);
                if let (Some(to), Some(from)) = (
                    self.net_of_node(graph.wire(p.to)),
                    self.net_of_node(graph.wire(p.from)),
                ) {
                    out.insert((p.tile, to, from));
                }
            }
        }
        out
    }
}

/// One hard cell's configuration as [`IceFabric::configure`] writes it:
/// the cell's kind as the chip database names it (`RGBA_DRV`, `I2C`), its
/// anchor tile, and each parameter's value.
pub type ExtraConfig = (String, (u32, u32), BTreeMap<String, u64>);

/// The hard cells whose configuration bits [`IceFabric::decode`] reads
/// back: every one with a bit in its record. The low-frequency oscillator,
/// the LED dimmer and the warm-boot block have none.
pub const HARD_IP_KINDS: [&str; 4] = ["RGBA_DRV", "HFOSC", "I2C", "SPI"];

/// The parameter and bit an entry `key` of `cell`'s record is: `NAME_<i>`
/// is bit `i` of `NAME` when the record has a `NAME_0`, and any other key
/// is bit 0 of itself (`SDA_INPUT_DELAYED`, `RGBA_DRV_EN`).
fn param_and_bit(cell: &db::ExtraCell, key: &str) -> (String, u32) {
    if let Some((name, bit)) = key.rsplit_once('_')
        && let Ok(bit) = bit.parse::<u32>()
        && cell.entries.iter().any(|(k, ..)| *k == format!("{name}_0"))
    {
        return (name.to_owned(), bit);
    }
    (key.to_owned(), 0)
}

/// What [`IceFabric::decode`] should read back of `extra`: for each cell of
/// a [`HARD_IP_KINDS`] kind, every parameter not zero, masked to the bits its
/// record has. A cell with none is absent, as an unused one is.
fn hard_ip_wanted(
    fabric: &IceFabric,
    extra: &[ExtraConfig],
) -> BTreeMap<(String, (u32, u32)), BTreeMap<String, u64>> {
    let mut out = BTreeMap::new();
    for (kind, tile, params) in extra {
        if !HARD_IP_KINDS.contains(&kind.as_str()) {
            continue;
        }
        let Some(cell) = fabric
            .db
            .extra_cells
            .iter()
            .find(|c| &c.kind == kind && c.tile == *tile)
        else {
            continue;
        };
        let mut held: BTreeMap<String, u64> = BTreeMap::new();
        for (key, x, y, name) in &cell.entries {
            if fabric
                .db
                .function_by_suffix(fabric.db.tile(*x, *y), name)
                .is_none()
            {
                continue;
            }
            let (param, bit) = param_and_bit(cell, key);
            if params.get(&param).is_some_and(|v| v >> bit & 1 != 0) {
                *held.entry(param).or_default() |= 1 << bit;
            }
        }
        if !held.is_empty() {
            out.insert((kind.clone(), *tile), held);
        }
    }
    out
}

/// Whether an attribute's value is true: a number not zero, or a string
/// that is not `0`, `false` or empty.
fn attr_true(value: &crate::ir::AttrValue) -> bool {
    match value {
        crate::ir::AttrValue::String(s) => {
            let s = s.trim_matches('"');
            !(s.is_empty() || s == "0" || s.eq_ignore_ascii_case("false"))
        }
        other => other.as_int().is_some_and(|v| v != 0),
    }
}

/// What [`IceFabric::configure_placed`] configured.
#[derive(Clone, Debug, Default)]
pub struct Configured {
    /// IO blocks.
    pub pads: usize,
    /// Hard cells: the RGB driver, the oscillators, the I2C and SPI blocks,
    /// the LED dimmer, the warm-boot block.
    pub hard_cells: usize,
    /// The configuration bits of every hard cell that has any, by kind and
    /// anchor tile, as written: what [`Decoded::hard_ip`] must read back.
    pub hard_ip: BTreeMap<(String, (u32, u32)), BTreeMap<String, u64>>,
    /// Block RAMs, by their `ramb` tile, as their cells ask for them.
    pub rams: BTreeMap<(u32, u32), RamConfig>,
    /// Single-port RAMs, as `(anchor tile, index)`.
    pub sprams: BTreeSet<((u32, u32), u32)>,
    /// DSP blocks, by their anchor tile, as their cells ask for them.
    pub dsps: BTreeMap<(u32, u32), DspConfig>,
    /// Each placed PLL, by its anchor tile, as its cell asks for it.
    pub plls: BTreeMap<(u32, u32), PllConfig>,
    /// The global networks the PLLs and oscillators drive through a pad's
    /// input path.
    pub padin: BTreeSet<u32>,
}

/// What [`implement`] built, and the numbers its report prints.
pub struct Implemented {
    /// The configuration image, checked.
    pub image: image::Image,
    /// What packing did.
    pub pack: pack::PackReport,
    /// IO blocks configured.
    pub pads: usize,
    /// Hard cells (RGB driver, oscillators) configured.
    pub hard_cells: usize,
    /// Block RAMs configured, each read back with its modes and contents.
    pub rams: usize,
    /// Single-port RAMs configured, each read back.
    pub sprams: usize,
    /// DSP blocks configured, each read back with its parameters.
    pub dsps: usize,
    /// Each placed PLL's configuration, read back, by its anchor tile.
    pub plls: BTreeMap<(u32, u32), PllConfig>,
    /// Each clock's longest path, when the fabric has timing figures.
    pub timing: Option<timing::TimingReport>,
    /// Sites used per kind, as the placer reports them.
    pub usage: Vec<(String, usize, usize)>,
    /// Signals routed, signals in all, pips and wires used.
    pub routed: (usize, usize, usize, usize),
    /// What decoding the image back read.
    pub decoded: Decoded,
    /// The arcs the router chose that cost bits.
    pub chosen: usize,
}

/// Packs, places, routes and configures `top` on `fabric`, and holds the
/// result to the two checks this project never weakens: every set bit
/// decodes back through the database, and the connections those bits
/// select are exactly the ones the router chose. Nothing is returned that
/// failed either.
///
/// # Errors
///
/// A message naming what failed: packing, a cell kind this flow does not
/// place, placement, routing, the routing's own verification, or either
/// check.
pub fn implement(
    design: &crate::ir::Design,
    top: crate::ir::ModuleId,
    device: &super::Device,
    constraints: &super::Constraints,
    fabric: &IceFabric,
    graph: &super::arch::RoutingGraph,
    place_options: &super::place::PlaceOptions,
) -> Result<Implemented, String> {
    use super::{bitstream, place, route};
    let mut packed = design.clone();
    let module = packed
        .modules
        .get_mut(top)
        .ok_or_else(|| "no such module".to_owned())?;
    let pack_report = pack::pack(module).map_err(|e| e.to_string())?;
    let design = &packed;
    let netlist = place::Netlist::build(design, top, device, graph).map_err(|e| e.to_string())?;
    for instance in &netlist.instances {
        if !matches!(
            instance.kind.as_str(),
            "lc" | "lcc"
                | "io"
                | "gb"
                | "rgba_drv"
                | "hfosc"
                | "lfosc"
                | "bram"
                | "spram"
                | "mac16"
                | "pll"
                | "i2c"
                | "spi"
                | "ledda_ip"
                | "warmboot"
        ) {
            return Err(format!(
                "`{}` is a `{}`, which this flow does not place on an iCE40 yet; nothing was \
                 written",
                instance.name, instance.primitive
            ));
        }
    }
    // An IO buffer only some pads have, on a pin that is not one of them,
    // is refused by name before the placer finds the pins it lacks.
    for instance in netlist.instances.iter().filter(|i| i.kind == "io") {
        let Some(at) = instance
            .pin
            .as_deref()
            .and_then(|pin| fabric.arch.pinmap.iter().find(|(p, _)| p == pin))
            .and_then(|(_, site)| io_site(site))
        else {
            continue;
        };
        let cell = &design.modules[top].cells[instance.cell];
        fabric
            .check_special_io(&instance.primitive, cell, at)
            .map_err(|e| e.to_string())?;
    }
    // A cell of a kind this die (or this package of it) has no site for is
    // refused by name, before anything else is said about it: its pins
    // reach no wire, so the check below would only report its signals as
    // undriven.
    for instance in &netlist.instances {
        if instance.kind != "pll" && !graph.sites.iter().any(|s| s.kind == instance.kind) {
            return Err(format!(
                "`{}` is a `{}`, and the iCE40 {} die has no `{}` site in the `{}` package; \
                 nothing was written",
                instance.name, instance.primitive, fabric.db.device, instance.kind, fabric.package
            ));
        }
    }
    // Each PLL gets its site before placement. A die may have two (the
    // 8k's, at the middle of its bottom and top edges); the cell packing
    // made to pass a PLL's `LOCK` on must sit in the one logic tile that
    // wire reaches (packing's rule 10); and the global networks a PLL drives
    // must be known before the design's clock buffers are seated.
    let mut pll_sites = assign_pll_sites(fabric, &netlist, constraints, &pack_report.pad_plls)?;
    // An I2C or SPI block's site is named by its `BUS_ADDR74`, the address
    // its system bus answers on; they are confined the same way.
    pll_sites.extend(assign_bus_sites(fabric, &netlist, design, top)?);
    let driverless = netlist.driverless();
    if !driverless.is_empty() {
        return Err(format!(
            "nothing drives {} signal(s) that something reads, so nothing was written (an \
             unrouted input of an iCE40 reads as zero, not as the value the netlist meant): {}",
            driverless.len(),
            driverless.join("; ")
        ));
    }
    let mut confined;
    let constraints = if pll_sites.is_empty() {
        constraints
    } else {
        confined = constraints.clone();
        let m = &design.modules[top];
        let span_of = |name: &str| {
            m.cells
                .iter()
                .find(|(_, c)| c.name.as_str() == name)
                .map_or(m.span, |(_, c)| c.span)
        };
        let mut confine = |name: &str, region: String, (x, y): (u32, u32)| {
            let span = span_of(name);
            if !confined.regions.iter().any(|r| r.name == region) {
                confined.regions.push(super::constraints::Region {
                    name: region.clone(),
                    x0: x,
                    y0: y,
                    x1: x,
                    y1: y,
                    span,
                });
            }
            confined
                .region_assignments
                .push(super::constraints::RegionAssignment {
                    pattern: name.to_owned(),
                    region,
                    span,
                    origin: super::constraints::Origin::File,
                });
        };
        for (name, tile) in &pll_sites {
            confine(name, format!("$ice40$site_X{}Y{}", tile.0, tile.1), *tile);
        }
        for (pll, lock) in &pack_report.lock_cells {
            let Some((_, tile)) = pll_sites.iter().find(|(n, _)| n == pll) else {
                continue;
            };
            let (x, y) = fabric
                .pll_at(*tile)
                .and_then(|c| c.entries.iter().find(|(k, ..)| k == "LOCK"))
                .map(|(_, x, y, _)| (*x, *y))
                .ok_or_else(|| "the database's PLL has no LOCK wire".to_owned())?;
            confine(lock, format!("$ice40$pll_lock_X{x}Y{y}"), (x, y));
        }
        &confined
    };
    // A placed PLL or oscillator takes the global networks it drives (its
    // bel blocks the buffers that drive them from the fabric), and the
    // placer, which seats a design's clock buffers before a cell with one
    // site, could give a clock one of those buffers and leave the PLL
    // nowhere. So the clock buffers are confined to the rectangle of the
    // die holding the most of them and none of the blocked ones.
    let mut kept;
    let blockers: Vec<usize> = graph
        .sites
        .iter()
        .enumerate()
        .filter(|(_, s)| match s.kind.as_str() {
            "hfosc" | "lfosc" => netlist.instances.iter().any(|i| i.kind == s.kind),
            "pll" => pll_sites.iter().any(|(_, t)| *t == s.tile),
            _ => false,
        })
        .map(|(i, _)| i)
        .collect();
    let constraints = match gb_region_avoiding(graph, &blockers) {
        None => constraints,
        Some((x0, y0, x1, y1)) => {
            kept = constraints.clone();
            let m = &design.modules[top];
            let name = "$ice40$beside_pll".to_owned();
            for instance in netlist.instances.iter().filter(|i| i.kind == "gb") {
                kept.region_assignments
                    .push(super::constraints::RegionAssignment {
                        pattern: instance.name.clone(),
                        region: name.clone(),
                        span: m.span,
                        origin: super::constraints::Origin::File,
                    });
            }
            kept.regions.push(super::constraints::Region {
                name,
                x0,
                y0,
                x1,
                y1,
                span: m.span,
            });
            &kept
        }
    };
    // Timing-driven: the placement weighs each signal by how critical its
    // estimated delay is in its clock's domain, and the route each
    // connection by its routed delay's criticality. See `timing`.
    let mut route_options = route::RouteOptions::default();
    let timed_options;
    let place_options = match (&fabric.timing, fabric.timing_driven) {
        (Some(db), true) => {
            let (paths, _) = timing::place_timing(design, top, &netlist, constraints, db);
            route_options.timing = Some(timing::route_timing(
                paths.clone(),
                timing::node_delays(graph, db),
            ));
            let mut options = place_options.clone();
            options.timing = Some(paths);
            timed_options = options;
            &timed_options
        }
        _ => place_options,
    };
    let (placement, place_report) =
        place::place(&netlist, &fabric.arch, graph, constraints, place_options)
            .map_err(|e| e.to_string())?;
    let (routing, route_report) =
        route::route(&netlist, graph, &placement, &route_options).map_err(|e| e.to_string())?;
    let problems = routing.verify(&netlist, graph, &placement);
    if !problems.is_empty() {
        return Err(format!(
            "the routing does not implement the netlist, so nothing was written: {}",
            problems.join("; ")
        ));
    }
    let tiles = bitstream::generate(
        design,
        top,
        &fabric.arch,
        graph,
        &netlist,
        &placement,
        &routing,
    )
    .map_err(|e| e.to_string())?;
    let layout = image::Layout::for_device(&fabric.db.device)
        .ok_or_else(|| format!("no configuration layout for `{}`", fabric.db.device))?;
    let mut image = image::Image::new(layout);
    for (format, set) in tiles.used_tiles() {
        for bit in set {
            image.set(format.tile.0, format.tile.1, bit.row, bit.col);
        }
    }
    let configured = fabric
        .configure_placed(design, top, &netlist, &placement, graph, &mut image)
        .map_err(|e| e.to_string())?;

    let decoded = fabric.decode(&image);
    if !decoded.unexplained.is_empty() || !decoded.bad_patterns.is_empty() {
        return Err(format!(
            "{} of this bitstream's {} set bit(s) belong to nothing the database names, so \
             nothing was written:\n  {}",
            decoded.unexplained.len() + decoded.bad_patterns.len(),
            decoded.bits,
            decoded
                .unexplained
                .iter()
                .chain(&decoded.bad_patterns)
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        ));
    }
    let chosen = fabric.routed_arcs(graph, &routing, netlist.signals.len());
    if decoded.arcs != chosen {
        let extra = decoded
            .arcs
            .difference(&chosen)
            .map(|((x, y), to, from)| format!("X{x}Y{y} net {to} <- net {from} (not chosen)"));
        let lost = chosen
            .difference(&decoded.arcs)
            .map(|((x, y), to, from)| format!("X{x}Y{y} net {to} <- net {from} (not selected)"));
        return Err(format!(
            "the bits of this bitstream do not select the connections the router chose, so \
             nothing was written; the router took {} arc(s) that cost bits and the bitstream \
             says {}:\n  {}",
            chosen.len(),
            decoded.arcs.len(),
            extra.chain(lost).take(40).collect::<Vec<_>>().join("\n  ")
        ));
    }
    // The third check, for the RAMs: the bits and the contents read back
    // as exactly the RAMs placed, each with the modes and contents its
    // cell asked for, and no other.
    if decoded.rams != configured.rams {
        let placed: BTreeSet<&(u32, u32)> = configured.rams.keys().collect();
        let read: BTreeSet<&(u32, u32)> = decoded.rams.keys().collect();
        let differ: Vec<String> = placed
            .intersection(&read)
            .filter(|t| decoded.rams.get(t) != configured.rams.get(t))
            .map(|(x, y)| format!("X{x}Y{y} reads back other modes or contents"))
            .chain(
                placed
                    .difference(&read)
                    .map(|(x, y)| format!("X{x}Y{y} is placed and reads back unused")),
            )
            .chain(
                read.difference(&placed)
                    .map(|(x, y)| format!("X{x}Y{y} reads back in use and nothing is placed")),
            )
            .collect();
        return Err(format!(
            "the block RAMs this bitstream describes are not the ones placed, so nothing was \
             written:\n  {}",
            differ.join("\n  ")
        ));
    }
    if decoded.sprams != configured.sprams {
        return Err(format!(
            "the single-port RAMs this bitstream enables ({:?}) are not the ones placed ({:?}), \
             so nothing was written",
            decoded.sprams, configured.sprams
        ));
    }
    // And for the PLLs: each one's parameters read back as its cell asked
    // for them (all zero with none placed), and the pad-input bits they and
    // the oscillators set are the only ones set.
    let pll_wanted: BTreeMap<(u32, u32), PllConfig> = decoded
        .plls
        .keys()
        .map(|t| {
            (
                *t,
                configured
                    .plls
                    .get(t)
                    .cloned()
                    .unwrap_or_else(PllConfig::zero),
            )
        })
        .collect();
    if decoded.plls != pll_wanted
        || configured
            .plls
            .keys()
            .any(|t| !decoded.plls.contains_key(t))
        || decoded.padin != configured.padin
    {
        let differ: Vec<String> = pll_wanted
            .iter()
            .flat_map(|(t, want)| {
                let got = decoded.plls.get(t);
                want.params
                    .iter()
                    .filter(move |(k, v)| got.and_then(|g| g.params.get(*k)) != Some(v))
                    .map(move |(k, v)| {
                        format!(
                            "X{}Y{} {k} reads {} for {v}",
                            t.0,
                            t.1,
                            got.and_then(|g| g.params.get(k)).copied().unwrap_or(0)
                        )
                    })
            })
            .collect();
        return Err(format!(
            "the PLL this bitstream describes is not the one placed, so nothing was written: \
             {}; pad-input networks {:?} for {:?}",
            differ.join(", "),
            decoded.padin,
            configured.padin
        ));
    }
    // And for the DSP blocks: every block's parameters read back as the
    // placed cell asked for, and every other block's as all zero.
    let wanted: BTreeMap<(u32, u32), DspConfig> = decoded
        .dsps
        .keys()
        .map(|t| {
            (
                *t,
                configured
                    .dsps
                    .get(t)
                    .cloned()
                    .unwrap_or_else(DspConfig::zero),
            )
        })
        .collect();
    if decoded.dsps != wanted
        || configured
            .dsps
            .keys()
            .any(|t| !decoded.dsps.contains_key(t))
    {
        let differ: Vec<String> = wanted
            .iter()
            .filter(|(t, w)| decoded.dsps.get(t) != Some(w))
            .map(|((x, y), w)| {
                let got = decoded.dsps.get(&(*x, *y));
                let params: Vec<String> = w
                    .params
                    .iter()
                    .filter(|(k, v)| got.and_then(|g| g.params.get(*k)) != Some(v))
                    .map(|(k, v)| {
                        format!(
                            "{k} reads {} for {v}",
                            got.and_then(|g| g.params.get(k)).copied().unwrap_or(0)
                        )
                    })
                    .collect();
                format!("X{x}Y{y}: {}", params.join(", "))
            })
            .collect();
        return Err(format!(
            "the DSP blocks this bitstream describes are not the ones placed, so nothing was \
             written:\n  {}",
            differ.join("\n  ")
        ));
    }
    // And for every other hard cell: the bits read back as the parameters
    // each placed one asked for, and no other cell's bit is set.
    if decoded.hard_ip != configured.hard_ip {
        let keys: BTreeSet<&(String, (u32, u32))> = decoded
            .hard_ip
            .keys()
            .chain(configured.hard_ip.keys())
            .collect();
        let differ: Vec<String> = keys
            .into_iter()
            .filter(|k| decoded.hard_ip.get(*k) != configured.hard_ip.get(*k))
            .map(|(kind, (x, y))| {
                format!(
                    "{kind} at X{x}Y{y} reads back {:?} for {:?}",
                    decoded.hard_ip.get(&(kind.clone(), (*x, *y))),
                    configured.hard_ip.get(&(kind.clone(), (*x, *y)))
                )
            })
            .collect();
        return Err(format!(
            "the hard cells this bitstream configures are not the ones placed, so nothing was \
             written:\n  {}",
            differ.join("\n  ")
        ));
    }
    let timing = fabric
        .timing
        .as_ref()
        .map(|db| timing::analyse(fabric, &image, db));
    Ok(Implemented {
        image,
        pack: pack_report,
        pads: configured.pads,
        hard_cells: configured.hard_cells,
        rams: configured.rams.len(),
        sprams: configured.sprams.len(),
        dsps: configured.dsps.len(),
        plls: configured.plls.clone(),
        timing,
        usage: place_report.usage,
        routed: (
            route_report.signals,
            netlist.signals.len(),
            route_report.pips,
            route_report.nodes,
        ),
        chosen: chosen.len(),
        decoded,
    })
}

/// Cells given a site before placement: `(instance name, anchor tile)`.
type SiteAssignments = Vec<(String, (u32, u32))>;

/// The PLL site each of the netlist's PLLs is given, as `(instance name,
/// anchor tile)`. A pad form (`pads`, packing's rule 11) first: the site
/// whose first output leaves through the IO block of the pin its pad's IO
/// buffer is on, which is the PLL's own pad. Then each other PLL, in the
/// order the netlist holds them: the first site left whose output IO
/// blocks — `PLLOUT_A`, and `PLLOUT_B` for a two-output form — are none of
/// the design's pins.
///
/// # Errors
///
/// A message naming the PLL when no site is left for it: the die or the
/// package has no PLL at all (the 384, the 1k in its smaller packages), or
/// fewer than the design asks for, or every one left would take a pin the
/// design uses; or, for a pad form, when its pad's pin is no PLL's pad.
fn assign_pll_sites(
    fabric: &IceFabric,
    netlist: &super::place::Netlist,
    constraints: &super::Constraints,
    pads: &[(String, String)],
) -> Result<SiteAssignments, String> {
    let pin_block = |pin: &str| {
        fabric
            .arch
            .pinmap
            .iter()
            .find(|(p, _)| p == pin)
            .and_then(|(_, site)| io_site(site))
    };
    let pins: BTreeSet<db::IoAt> = constraints
        .pins
        .iter()
        .filter_map(|p| pin_block(&p.pin))
        .collect();
    let mut free: Vec<&db::ExtraCell> = fabric.plls().collect();
    let total = free.len();
    let mut out = Vec::new();
    for (pll, port) in pads {
        let Some(instance) = netlist.instances.iter().find(|i| &i.name == pll) else {
            continue;
        };
        let pin = constraints
            .pins
            .iter()
            .find(|p| &p.port == port)
            .map(|p| p.pin.clone());
        let Some(pin) = pin else {
            return Err(format!(
                "`{pll}` is a `{}` whose pad, port `{port}`, has no pin constraint; a pad form \
                 of the PLL needs its pad on the PLL's own pin; nothing was written",
                instance.primitive
            ));
        };
        let block = pin_block(&pin);
        let at = free
            .iter()
            .position(|c| block.is_some() && pll_output_io(c, "PLLOUT_A") == block);
        let Some(at) = at else {
            let own: Vec<String> = fabric
                .plls()
                .filter_map(|c| pll_output_io(c, "PLLOUT_A"))
                .filter_map(|b| {
                    fabric
                        .arch
                        .pinmap
                        .iter()
                        .find(|(_, s)| io_site(s) == Some(b))
                        .map(|(p, _)| p.clone())
                })
                .collect();
            return Err(format!(
                "`{pll}` is a `{}` whose pad is on pin {pin}, which is no free PLL's own pad: \
                 in the `{}` package {}; nothing was written",
                instance.primitive,
                fabric.package,
                if own.is_empty() {
                    "no PLL has one".to_owned()
                } else {
                    format!("the PLL pads are pin(s) {}", own.join(", "))
                }
            ));
        };
        out.push((instance.name.clone(), free.remove(at).tile));
    }
    for instance in netlist
        .instances
        .iter()
        .filter(|i| i.kind == "pll" && !pads.iter().any(|(p, _)| *p == i.name))
    {
        let outputs = if PllConfig::is_dual_primitive(&instance.primitive) {
            2
        } else {
            1
        };
        let fits = |cell: &&db::ExtraCell| {
            ["PLLOUT_A", "PLLOUT_B"]
                .iter()
                .take(outputs)
                .all(|k| pll_output_io(cell, k).is_none_or(|io| !pins.contains(&io)))
        };
        let Some(at) = free.iter().position(fits) else {
            let why = if total == 0 {
                format!(
                    "the iCE40 {} die has no PLL in the `{}` package",
                    fabric.db.device, fabric.package
                )
            } else if free.is_empty() {
                format!(
                    "the iCE40 {} die has {total} PLL(s) in the `{}` package, and the design \
                     asks for more",
                    fabric.db.device, fabric.package
                )
            } else {
                "every PLL left would send its output through an IO block the design uses as \
                 a pin"
                    .to_owned()
            };
            return Err(format!(
                "`{}` is a `{}`, and {why}; nothing was written",
                instance.name, instance.primitive
            ));
        };
        out.push((instance.name.clone(), free.remove(at).tile));
    }
    Ok(out)
}

/// The site of each I2C and SPI block, as `(instance name, anchor tile)`,
/// from its `BUS_ADDR74`: on the UltraPlus, `0b0000` is the SPI block at
/// (0, 0), `0b0001` the I2C block at (0, 31), `0b0010` the SPI block at
/// (25, 0) and `0b0011` the I2C block at (25, 31) — nextpnr's `pack.cc`
/// (`map_ba74`); the default is `0b0001` for an I2C block and `0b0000` for
/// an SPI one, the library's.
///
/// # Errors
///
/// A message naming the cell for another value, or on a die without those
/// blocks there.
fn assign_bus_sites(
    fabric: &IceFabric,
    netlist: &super::place::Netlist,
    design: &crate::ir::Design,
    top: crate::ir::ModuleId,
) -> Result<SiteAssignments, String> {
    let m = &design.modules[top];
    let mut out = Vec::new();
    for instance in netlist
        .instances
        .iter()
        .filter(|i| i.kind == "i2c" || i.kind == "spi")
    {
        let cell = &m.cells[instance.cell];
        let default = if instance.kind == "i2c" { 1 } else { 0 };
        let value = match cell.params.get("BUS_ADDR74") {
            None => Some(default),
            Some(crate::ir::AttrValue::String(s)) => {
                let s = s.trim_matches('"');
                u64::from_str_radix(s.strip_prefix("0b").unwrap_or(s), 2).ok()
            }
            Some(other) => other.as_int().and_then(|v| u64::try_from(v).ok()),
        };
        let tile = match (instance.kind.as_str(), value) {
            ("spi", Some(0)) => (0, 0),
            ("i2c", Some(1)) => (0, 31),
            ("spi", Some(2)) => (25, 0),
            ("i2c", Some(3)) => (25, 31),
            _ => {
                return Err(format!(
                    "`{}` is a `{}` with BUS_ADDR74 {:?}, which names none of the die's blocks: \
                     an SPI block answers on 0b0000 or 0b0010, an I2C block on 0b0001 or \
                     0b0011; nothing was written",
                    instance.name,
                    instance.primitive,
                    value.map_or_else(|| "unreadable".to_owned(), |v| format!("{v:#06b}"))
                ));
            }
        };
        let kind = instance.kind.to_ascii_uppercase();
        if !fabric
            .db
            .extra_cells
            .iter()
            .any(|c| c.kind == kind && c.tile == tile)
        {
            return Err(format!(
                "`{}` is a `{}` for the block at ({}, {}), which the iCE40 {} die does not have \
                 there; nothing was written",
                instance.name, instance.primitive, tile.0, tile.1, fabric.db.device
            ));
        }
        out.push((instance.name.clone(), tile));
    }
    Ok(out)
}

/// `X12Y31/io1` as an IO block's position.
fn io_site(site: &str) -> Option<db::IoAt> {
    let (tile, bel) = site.split_once('/')?;
    let (x, y) = tile.strip_prefix('X')?.split_once('Y')?;
    Some((
        x.parse().ok()?,
        y.parse().ok()?,
        bel.strip_prefix("io")?.parse().ok()?,
    ))
}

/// A rectangle of the die, `(x0, y0, x1, y1)`, holding as many global
/// buffer sites as can be had and none of those that the sites `blockers`
/// block — a placed PLL's or oscillator's, which drive those buffers'
/// networks themselves. Its corners are global buffer sites' columns and
/// rows, so every candidate is tried. `None` when nothing is blocked.
fn gb_region_avoiding(
    graph: &super::arch::RoutingGraph,
    blockers: &[usize],
) -> Option<(u32, u32, u32, u32)> {
    let blocked: BTreeSet<(u32, u32)> = blockers
        .iter()
        .flat_map(|&s| graph.sites[s].blocks.iter().map(|b| graph.sites[*b].tile))
        .collect();
    if blocked.is_empty() {
        return None;
    }
    let gbs: Vec<(u32, u32)> = graph
        .sites
        .iter()
        .filter(|s| s.kind == "gb")
        .map(|s| s.tile)
        .collect();
    let xs: BTreeSet<u32> = gbs.iter().map(|t| t.0).collect();
    let ys: BTreeSet<u32> = gbs.iter().map(|t| t.1).collect();
    let mut best: Option<((u32, u32, u32, u32), usize)> = None;
    for &x0 in &xs {
        for &x1 in xs.range(x0..) {
            for &y0 in &ys {
                for &y1 in ys.range(y0..) {
                    let inside =
                        |t: &(u32, u32)| (x0..=x1).contains(&t.0) && (y0..=y1).contains(&t.1);
                    if blocked.iter().any(inside) {
                        continue;
                    }
                    let n = gbs.iter().filter(|t| inside(t)).count();
                    if best.is_none_or(|(_, m)| n > m) {
                        best = Some(((x0, y0, x1, y1), n));
                    }
                }
            }
        }
    }
    best.map(|(r, _)| r)
}

/// One block RAM's configuration: what [`IceFabric::configure_rams`]
/// writes and [`IceFabric::decode`] reads back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RamConfig {
    /// `READ_MODE`: 0 for 256 x 16 through 3 for 2048 x 2.
    pub read_mode: u32,
    /// `WRITE_MODE`, likewise.
    pub write_mode: u32,
    /// The contents, sixteen lines of 256 bits: bit `i` of line `k` is
    /// bit `i` of the primitive's `INIT_<k>`.
    pub contents: Vec<[bool; 256]>,
}

impl RamConfig {
    /// The configuration an `SB_RAM40_4K` cell asks for, from its
    /// `READ_MODE`, `WRITE_MODE` and `INIT_0`..`INIT_F` parameters.
    ///
    /// # Errors
    ///
    /// [`FabricError`] for a mode out of range or contents that are not
    /// a number.
    pub fn of_cell(cell: &crate::ir::Cell) -> Result<RamConfig, FabricError> {
        use crate::ir::AttrValue;
        let mode = |key: &str| -> Result<u32, FabricError> {
            let value = match cell.params.get(key) {
                None => 0,
                Some(AttrValue::Const(c)) => c.to_u64().unwrap_or(u64::MAX),
                Some(other) => other
                    .as_int()
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(u64::MAX),
            };
            u32::try_from(value).ok().filter(|v| *v < 4).ok_or_else(|| {
                FabricError(format!(
                    "`{}`'s {key} is not one of 0..3",
                    cell.name.as_str()
                ))
            })
        };
        let mut contents = vec![[false; 256]; 16];
        for (k, line) in contents.iter_mut().enumerate() {
            let key = format!("INIT_{k:X}");
            match cell.params.get(&key) {
                None => {}
                Some(AttrValue::Const(c)) => {
                    for (i, slot) in line.iter_mut().enumerate() {
                        let i = u32::try_from(i).unwrap_or(u32::MAX);
                        *slot = i < c.width() && c.bit(i) == crate::logic::Bit::One;
                    }
                }
                Some(other) => {
                    let Some(v) = other.as_int() else {
                        return Err(FabricError(format!(
                            "`{}`'s {key} is not a number",
                            cell.name.as_str()
                        )));
                    };
                    for (i, slot) in line.iter_mut().enumerate().take(64) {
                        *slot = v >> i & 1 != 0;
                    }
                }
            }
        }
        Ok(RamConfig {
            read_mode: mode("READ_MODE")?,
            write_mode: mode("WRITE_MODE")?,
            contents,
        })
    }
}

/// What [`IceFabric::decode`] read out of an image.
#[derive(Clone, Debug, Default)]
pub struct Decoded {
    /// Set bits read.
    pub bits: usize,
    /// Every connection selected: `((x, y), destination net, source net)`.
    pub arcs: BTreeSet<((u32, u32), u32, u32)>,
    /// Every named function with a bit set, and which.
    pub functions: BTreeMap<((u32, u32), String), Vec<ConfigBit>>,
    /// Set bits that belong to nothing the database names.
    pub unexplained: Vec<String>,
    /// Multiplexer fields whose pattern is no option.
    pub bad_patterns: Vec<String>,
    /// Every block RAM in use, by its `ramb` tile.
    pub rams: BTreeMap<(u32, u32), RamConfig>,
    /// Set bits of block RAM contents, all of them belonging to a RAM in
    /// [`Decoded::rams`] (any other is in `unexplained`).
    pub ram_bits: usize,
    /// Every single-port RAM enabled, as `(anchor tile, index)`.
    pub sprams: BTreeSet<((u32, u32), u32)>,
    /// Every DSP block's parameters as its bits hold them, by its anchor
    /// tile; an unused block reads back as all zero.
    pub dsps: BTreeMap<(u32, u32), DspConfig>,
    /// Every PLL's parameters as its bits hold them, by its anchor tile,
    /// all zero when unused.
    pub plls: BTreeMap<(u32, u32), PllConfig>,
    /// The global networks whose pad-input bit is set.
    pub padin: BTreeSet<u32>,
    /// Every hard cell with a configuration bit set, by kind and anchor
    /// tile: its parameters as the bits hold them, only those not zero.
    pub hard_ip: BTreeMap<(String, (u32, u32)), BTreeMap<String, u64>>,
}

/// The IO block a PLL output's entry names, `PLLOUT_A 12 31 1`.
fn pll_output_io(cell: &db::ExtraCell, key: &str) -> Option<(u32, u32, u32)> {
    let (_, x, y, z) = cell.entries.iter().find(|(k, ..)| k == key)?;
    Some((*x, *y, z.parse().ok()?))
}

/// The parameters of the PLL that its configuration bits hold, with their
/// widths: the list nextpnr's `bitstream.cc` writes, each bit a
/// `PLLCONFIG_<i>` of one of the IO tiles beside it, as the `.extra_cell
/// PLL` record names.
pub const PLL_PARAMS: [(&str, u32); 16] = [
    ("DELAY_ADJMODE_FB", 1),
    ("DELAY_ADJMODE_REL", 1),
    ("DIVF", 7),
    ("DIVQ", 3),
    ("DIVR", 4),
    ("ENABLE_ICEGATE_PORTA", 1),
    ("ENABLE_ICEGATE_PORTB", 1),
    ("FDA_FEEDBACK", 4),
    ("FDA_RELATIVE", 4),
    ("FEEDBACK_PATH", 3),
    ("FILTER_RANGE", 3),
    ("PLLOUT_SELECT_A", 2),
    ("PLLOUT_SELECT_B", 2),
    ("PLLTYPE", 3),
    ("SHIFTREG_DIV_MODE", 2),
    ("TEST_MODE", 1),
];

/// The `(parameter, bit)` an entry of the `.extra_cell PLL` record sets,
/// when it is a parameter bit.
#[must_use]
pub fn pll_param_bit(key: &str) -> Option<(&'static str, u32)> {
    for (name, width) in PLL_PARAMS {
        if key == name && width == 1 {
            return Some((name, 0));
        }
        if let Some(bit) = key
            .strip_prefix(name)
            .and_then(|r| r.strip_prefix('_'))
            .and_then(|r| r.parse::<u32>().ok())
            && bit < width
        {
            return Some((name, bit));
        }
    }
    None
}

/// The PLL's configuration: every parameter of [`PLL_PARAMS`], as
/// [`IceFabric::configure_pll`] writes them and [`IceFabric::decode`]
/// reads them back. All zero is a PLL not in use: a placed one always has
/// a `PLLTYPE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PllConfig {
    /// Each parameter's value, all of [`PLL_PARAMS`] present.
    pub params: BTreeMap<String, u64>,
}

impl PllConfig {
    /// Every parameter zero.
    #[must_use]
    pub fn zero() -> PllConfig {
        PllConfig {
            params: PLL_PARAMS
                .iter()
                .map(|(n, _)| ((*n).to_owned(), 0))
                .collect(),
        }
    }

    /// The configuration an `SB_PLL40_CORE` or `SB_PLL40_2F_CORE` cell
    /// asks for, translated as nextpnr's packer translates the library's
    /// parameters (`pack.cc`, `pack_plls`): `PLLTYPE` from the primitive
    /// (3 and 7), `FEEDBACK_PATH` from its name (`DELAY` 0, `SIMPLE` 1,
    /// `PHASE_AND_DELAY` 2, `EXTERNAL` 6), each output's `PLLOUT_SELECT`
    /// (`GENCLK` 0, `GENCLK_HALF` 1, `SHIFTREG_90deg` 2, `SHIFTREG_0deg`
    /// 3), and a `DYNAMIC` delay adjustment as its mode bit with the delay
    /// field all ones.
    ///
    /// # Errors
    ///
    /// [`FabricError`] for the pad forms, which this flow does not place,
    /// a value that is not one of those names or does not fit its field.
    pub fn of_cell(cell: &crate::ir::Cell, primitive: &str) -> Result<PllConfig, FabricError> {
        use crate::ir::AttrValue;
        let who = cell.name.as_str();
        let plltype = match primitive {
            "SB_PLL40_CORE" => 3,
            "SB_PLL40_2F_CORE" => 7,
            "SB_PLL40_PAD" => 2,
            "SB_PLL40_2_PAD" => 4,
            "SB_PLL40_2F_PAD" => 6,
            other => return Err(FabricError(format!("`{who}`: `{other}` is not a PLL"))),
        };
        let text = |key: &str| -> Option<String> {
            match cell.params.get(key)? {
                AttrValue::String(s) => Some(s.trim_matches('"').to_owned()),
                _ => None,
            }
        };
        let number = |key: &str| -> Result<u64, FabricError> {
            let v = match cell.params.get(key) {
                None => Some(0),
                Some(AttrValue::Const(c)) => c.to_u64(),
                Some(AttrValue::String(s)) => {
                    let digits = s.strip_prefix("0b").unwrap_or(s);
                    u64::from_str_radix(digits, 2).ok()
                }
                Some(other) => other.as_int().and_then(|v| u64::try_from(v).ok()),
            };
            v.ok_or_else(|| FabricError(format!("`{who}`'s {key} is not a number")))
        };
        let named = |key: &str, names: &[(&str, u64)], default: u64| -> Result<u64, FabricError> {
            match text(key) {
                Some(s) if s.bytes().all(|b| b.is_ascii_digit()) && !s.is_empty() => s
                    .parse()
                    .map_err(|_| FabricError(format!("`{who}`'s {key}"))),
                Some(s) => names
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&s))
                    .map(|(_, v)| *v)
                    .ok_or_else(|| {
                        FabricError(format!(
                            "`{who}`'s {key} is \"{s}\", which is none of {}",
                            names.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                        ))
                    }),
                None if cell.params.get(key).is_some() => number(key),
                None => Ok(default),
            }
        };
        let select = [
            ("GENCLK", 0),
            ("GENCLK_HALF", 1),
            ("SHIFTREG_90deg", 2),
            ("SHIFTREG_0deg", 3),
        ];
        let mut out = PllConfig::zero();
        let mut set = |name: &str, v: u64| out.params.insert(name.to_owned(), v);
        set("PLLTYPE", plltype);
        set(
            "FEEDBACK_PATH",
            named(
                "FEEDBACK_PATH",
                &[
                    ("DELAY", 0),
                    ("SIMPLE", 1),
                    ("PHASE_AND_DELAY", 2),
                    ("EXTERNAL", 6),
                ],
                1,
            )?,
        );
        // The one-output forms name their output's selection
        // `PLLOUT_SELECT`; `SB_PLL40_2_PAD`'s first output is its pad's
        // signal passed through, so only its second has one; the `2F` forms
        // name both.
        match plltype {
            2 | 3 => {
                set("PLLOUT_SELECT_A", named("PLLOUT_SELECT", &select, 0)?);
            }
            4 => {
                set("PLLOUT_SELECT_B", named("PLLOUT_SELECT_PORTB", &select, 0)?);
            }
            _ => {
                set("PLLOUT_SELECT_A", named("PLLOUT_SELECT_PORTA", &select, 0)?);
                set("PLLOUT_SELECT_B", named("PLLOUT_SELECT_PORTB", &select, 0)?);
            }
        }
        for (key, mode, field) in [
            (
                "DELAY_ADJUSTMENT_MODE_FEEDBACK",
                "DELAY_ADJMODE_FB",
                "FDA_FEEDBACK",
            ),
            (
                "DELAY_ADJUSTMENT_MODE_RELATIVE",
                "DELAY_ADJMODE_REL",
                "FDA_RELATIVE",
            ),
        ] {
            let dynamic = named(key, &[("FIXED", 0), ("DYNAMIC", 1)], 0)?;
            set(mode, dynamic);
            set(field, if dynamic == 1 { 15 } else { number(field)? });
        }
        for key in [
            "DIVR",
            "DIVF",
            "DIVQ",
            "FILTER_RANGE",
            "SHIFTREG_DIV_MODE",
            "ENABLE_ICEGATE_PORTA",
            "ENABLE_ICEGATE_PORTB",
            "TEST_MODE",
        ] {
            set(key, number(key)?);
        }
        for (name, width) in PLL_PARAMS {
            let v = out.params.get(name).copied().unwrap_or(0);
            if v >> width != 0 {
                return Err(FabricError(format!(
                    "`{who}`'s {name} is {v}, which does not fit its {width} bit(s)"
                )));
            }
        }
        Ok(out)
    }

    /// Whether `primitive` is a form of the PLL with two outputs:
    /// `SB_PLL40_2F_CORE`, `SB_PLL40_2_PAD` or `SB_PLL40_2F_PAD`.
    #[must_use]
    pub fn is_dual_primitive(primitive: &str) -> bool {
        matches!(
            primitive,
            "SB_PLL40_2F_CORE" | "SB_PLL40_2_PAD" | "SB_PLL40_2F_PAD"
        )
    }

    /// Whether this is a pad form, whose reference is its own pad
    /// (`PLLTYPE` 2, 4 and 6), as nextpnr's `is_sb_pll40_pad`.
    #[must_use]
    pub fn pad(&self) -> bool {
        matches!(self.params.get("PLLTYPE"), Some(2 | 4 | 6))
    }

    /// Whether the second output exists in this configuration: the `2F`
    /// and `2` forms (`PLLTYPE` 4, 6 and 7), as nextpnr's
    /// `is_sb_pll40_dual`.
    #[must_use]
    pub fn dual(&self) -> bool {
        matches!(self.params.get("PLLTYPE"), Some(4 | 6 | 7))
    }
}

/// The `(parameter, bit)` an entry of an `.extra_cell MAC16` record sets,
/// when it is a parameter bit and not a port: `A_REG` is bit 0 of
/// `A_REG`, `TOPOUTPUT_SELECT_1` bit 1 of `TOPOUTPUT_SELECT`.
#[must_use]
pub fn mac16_param_bit(key: &str) -> Option<(&'static str, u32)> {
    for (name, width) in MAC16_PARAMS {
        if key == name && width == 1 {
            return Some((name, 0));
        }
        if let Some(bit) = key
            .strip_prefix(name)
            .and_then(|r| r.strip_prefix('_'))
            .and_then(|r| r.parse::<u32>().ok())
            && bit < width
        {
            return Some((name, bit));
        }
    }
    None
}

/// One DSP block's configuration: every parameter of [`MAC16_PARAMS`] and
/// its value, what [`IceFabric::configure_dsps`] writes and
/// [`IceFabric::decode`] reads back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DspConfig {
    /// Each parameter's value, all of [`MAC16_PARAMS`] present.
    pub params: BTreeMap<String, u64>,
}

impl DspConfig {
    /// Every parameter zero: an unused block, or a 16 x 16 unsigned
    /// multiply-and-add with no register.
    #[must_use]
    pub fn zero() -> DspConfig {
        DspConfig {
            params: MAC16_PARAMS
                .iter()
                .map(|(n, _)| ((*n).to_owned(), 0))
                .collect(),
        }
    }

    /// The configuration an `SB_MAC16` cell asks for.
    ///
    /// # Errors
    ///
    /// [`FabricError`] for a value wider than its parameter, a value that
    /// is not a number, or `NEG_TRIGGER` set, which no bit can hold.
    pub fn of_cell(cell: &crate::ir::Cell) -> Result<DspConfig, FabricError> {
        use crate::ir::AttrValue;
        let value = |key: &str| -> Result<u64, FabricError> {
            let v = match cell.params.get(key) {
                None => Some(0),
                Some(AttrValue::Const(c)) => c.to_u64(),
                Some(AttrValue::String(s)) => {
                    let digits = s.strip_prefix("0b").unwrap_or(s);
                    u64::from_str_radix(digits, 2).ok()
                }
                Some(other) => other.as_int().and_then(|v| u64::try_from(v).ok()),
            };
            v.ok_or_else(|| {
                FabricError(format!("`{}`'s {key} is not a number", cell.name.as_str()))
            })
        };
        if value("NEG_TRIGGER")? != 0 {
            return Err(FabricError(format!(
                "`{}` asks for NEG_TRIGGER, a DSP block clocked on the falling edge, which no \
                 configuration bit of the iCE40UP5K holds; invert the clock in logic instead",
                cell.name.as_str()
            )));
        }
        let mut out = DspConfig::zero();
        for (name, width) in MAC16_PARAMS {
            let v = value(name)?;
            if v >> width != 0 {
                return Err(FabricError(format!(
                    "`{}`'s {name} is {v}, which does not fit its {width} bit(s)",
                    cell.name.as_str()
                )));
            }
            out.params.insert(name.to_owned(), v);
        }
        Ok(out)
    }

    /// The configuration as the database names its bits, from the one the
    /// library's parameters describe (`self`, from [`DspConfig::of_cell`]).
    ///
    /// They differ in two names. On the silicon, measured
    /// (`docs/fpga-icestorm.md`, "Two DSP registers by other names"), the
    /// bit IceStorm calls `TOP_8x8_MULT_REG` registers the partial product
    /// `A[15:8] x B[7:0]` and nothing else, and the one it calls
    /// `PIPELINE_16x16_MULT_REG1` registers both the top product `A[15:8] x
    /// B[15:8]` — the 8 x 8 split's top output — and `A[7:0] x B[15:8]`. The
    /// library's `TOP_8x8_MULT_REG` is the top product's register, and its
    /// `PIPELINE_16x16_MULT_REG1` the two middle products'. So the first is
    /// written into the bit IceStorm names after the second and the other
    /// way round; and since one bit holds the top product's register and
    /// one middle product's, a cell asking for the two differently is
    /// refused when the 16 x 16 sum `H` reaches an output or an adder, the
    /// only place the middle products go.
    ///
    /// # Errors
    ///
    /// [`FabricError`] for that one configuration, naming `cell`.
    pub fn as_bits(&self, cell: &crate::ir::Cell) -> Result<DspConfig, FabricError> {
        let get = |k: &str| self.params.get(k).copied().unwrap_or(0);
        let (top, pipe) = (get("TOP_8x8_MULT_REG"), get("PIPELINE_16x16_MULT_REG1"));
        let sum_used = get("TOPOUTPUT_SELECT") == 3
            || get("BOTOUTPUT_SELECT") == 3
            || get("TOPADDSUB_LOWERINPUT") == 2
            || get("BOTADDSUB_LOWERINPUT") == 2;
        if top != pipe && sum_used {
            return Err(FabricError(format!(
                "`{}` asks for TOP_8x8_MULT_REG = {top} and PIPELINE_16x16_MULT_REG1 = {pipe} \
                 and uses the 16 x 16 product, which an iCE40UP5K cannot hold: one \
                 configuration bit registers both the top partial product and one of the middle \
                 two; give the two parameters the same value",
                cell.name.as_str()
            )));
        }
        let mut out = self.clone();
        out.params.insert("TOP_8x8_MULT_REG".to_owned(), pipe);
        out.params
            .insert("PIPELINE_16x16_MULT_REG1".to_owned(), top);
        Ok(out)
    }
}
