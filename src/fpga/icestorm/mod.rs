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
        };
        fabric.add_pips();
        fabric.add_logic_bels()?;
        fabric.add_io_bels();
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

    /// The RGB LED driver and the oscillators: bels anchored where the
    /// database puts the cell, with pins on the wires its entries name.
    fn add_extra_cells(&mut self) {
        let cells: Vec<db::ExtraCell> = self.db.extra_cells.clone();
        for cell in cells {
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
            }
            self.tile_mut(ax, ay).bels.push(bel);
        }
    }

    /// The `(x, y, function bits)` of a function of the tile at `(x, y)`.
    fn function(&self, x: u32, y: u32, name: &str) -> Option<&[ConfigBit]> {
        self.db.function_bits(self.db.tile(x, y), name)
    }

    /// The bits a placed design needs that no bel's configuration entry
    /// and no pip states; see the module documentation. Returns what was
    /// configured, for the report.
    ///
    /// `ios` is every used IO block as `(x, y, z, pin_type, input_used,
    /// pullup)`; `extra` every extra cell's parameters by kind.
    ///
    /// # Errors
    ///
    /// [`FabricError`] when the database lacks a bit this needs.
    pub fn configure(
        &self,
        image: &mut image::Image,
        ios: &[(u32, u32, u32, u32, bool, bool)],
        extra: &[(String, BTreeMap<String, u64>)],
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
            if input {
                set(image, ix, iy, &need(ix, iy, &format!("IoCtrl.IE_{iz}"))?);
            }
            if !pullup {
                set(image, ix, iy, &need(ix, iy, &format!("IoCtrl.REN_{iz}"))?);
            }
            if self.db.device == "5k" {
                let bit = if iz == 0 {
                    "IoCtrl.cf_bit_39"
                } else {
                    "IoCtrl.cf_bit_35"
                };
                set(image, ix, iy, &need(ix, iy, bit)?);
            }
        }
        if self.db.device == "5k" {
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
            // Every IP-connect and DSP tile's pass-through cells.
            for y in 0..self.db.height {
                for x in 0..self.db.width {
                    let kind = self.db.tile(x, y);
                    if !matches!(kind, TileKind::Ipcon | TileKind::Dsp(_)) {
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
        }
        for (kind, params) in extra {
            let Some(cell) = self.db.extra_cells.iter().find(|c| &c.kind == kind) else {
                return Err(FabricError(format!("the database has no `{kind}` cell")));
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

    /// Reads an image back through the database: every multiplexer whose
    /// field is not all zero becomes an arc, every named function bit is
    /// accounted for, and every set bit that is neither is reported.
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
    /// Returns `(io blocks, extra cells)` configured.
    ///
    /// # Errors
    ///
    /// [`FabricError`] when the design mixes clock edges in one logic tile,
    /// which the tile's single `NegClk` bit cannot hold, or when a bit the
    /// database should have is missing.
    pub fn configure_placed(
        &self,
        design: &crate::ir::Design,
        module: crate::ir::ModuleId,
        netlist: &super::place::Netlist,
        placement: &super::place::Placement,
        graph: &super::arch::RoutingGraph,
        image: &mut image::Image,
    ) -> Result<(usize, usize), FabricError> {
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
        let mut extra = Vec::new();
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
                    extra.push(("RGBA_DRV".to_owned(), p));
                }
                "hfosc" => {
                    let mut p = BTreeMap::new();
                    p.insert(
                        "CLKHF_DIV".to_owned(),
                        param(cell, "CLKHF_DIV").unwrap_or(0),
                    );
                    extra.push(("HFOSC".to_owned(), p));
                }
                "lfosc" => extra.push(("LFOSC".to_owned(), BTreeMap::new())),
                _ => {}
            }
        }
        if let Some((tile, _)) = edges.iter().find(|(_, e)| e.len() > 1) {
            return Err(FabricError(format!(
                "the logic tile at ({}, {}) holds flip-flops on both clock edges; an iCE40 logic \
                 tile has one clock-polarity bit for all eight cells",
                tile.0, tile.1
            )));
        }
        let counts = (ios.len(), extra.len());
        self.configure(image, &ios, &extra)?;
        Ok(counts)
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
    let driverless = netlist.driverless();
    if !driverless.is_empty() {
        return Err(format!(
            "nothing drives {} signal(s) that something reads, so nothing was written (an \
             unrouted input of an iCE40 reads as zero, not as the value the netlist meant): {}",
            driverless.len(),
            driverless.join("; ")
        ));
    }
    for instance in &netlist.instances {
        if !matches!(
            instance.kind.as_str(),
            "lc" | "lcc" | "io" | "gb" | "rgba_drv" | "hfosc" | "lfosc"
        ) {
            return Err(format!(
                "`{}` is a `{}`, which this flow does not place on an iCE40 yet; nothing was \
                 written",
                instance.name, instance.primitive
            ));
        }
    }
    let (placement, place_report) =
        place::place(&netlist, &fabric.arch, graph, constraints, place_options)
            .map_err(|e| e.to_string())?;
    let (routing, route_report) =
        route::route(&netlist, graph, &placement, &route::RouteOptions::default())
            .map_err(|e| e.to_string())?;
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
    let (pads, hard_cells) = fabric
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
    Ok(Implemented {
        image,
        pack: pack_report,
        pads,
        hard_cells,
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
}
