//! Reading an icebox chip database (`chipdb-<device>.txt`).
//!
//! The format is icebox's own, documented at the head of every file it
//! writes, and it is flat: a list of dot-directives each followed by data
//! lines. What this keeps of it:
//!
//! | Directive | Kept as |
//! |---|---|
//! | `.device NAME W H NETS` | [`ChipDb::device`], the grid size |
//! | `.io_tile` / `.logic_tile` / `.ramb_tile` / `.ramt_tile` / `.dsp<n>_tile` / `.ipcon_tile X Y` | [`ChipDb::tiles`] |
//! | `.<kind>_tile_bits COLS ROWS` and its `FUNCTION B<r>[<c>] ...` lines | [`ChipDb::tile_bits`] |
//! | `.net N` and its `X Y NAME` lines | [`ChipDb::nets`]: every name a piece of metal has, in every tile it passes |
//! | `.buffer X Y DST BITS...` / `.routing ...` and their `VALUE SRC` lines | [`ChipDb::muxes`] |
//! | `.pins PACKAGE` | [`ChipDb::packages`] |
//! | `.gbufin`, `.gbufpin`, `.iolatch`, `.ieren`, `.colbuf` | the fields of the same names |
//! | `.extra_cell`, `.extra_bits` | [`ChipDb::extra_cells`], [`ChipDb::extra_bits`] |
//!
//! Nothing here interprets a single bit; that is [`super`]'s job. This is
//! only the parse, and it refuses a line it does not understand rather
//! than skipping it, because a skipped `.buffer` is a pip that silently
//! does not exist.

use std::collections::BTreeMap;

use crate::fpga::arch::ConfigBit;

/// What kind of tile a grid position is, as icebox names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TileKind {
    /// A corner, or a position with no tile.
    Empty,
    /// An IO tile: two IO blocks on the edge of the die.
    Io,
    /// A logic tile: eight logic cells.
    Logic,
    /// The bottom half of a block RAM.
    Ramb,
    /// The top half of a block RAM.
    Ramt,
    /// One of the four tiles of a UltraPlus DSP block.
    Dsp(u8),
    /// An UltraPlus "IP connect" tile, through which the hard IP's ports
    /// reach the fabric.
    Ipcon,
}

impl TileKind {
    /// The keyword the `.asc` format and the chip database use: `io`,
    /// `logic`, `ramb`, `ramt`, `dsp0`..`dsp3`, `ipcon`.
    #[must_use]
    pub fn keyword(self) -> String {
        match self {
            TileKind::Empty => "corner".to_owned(),
            TileKind::Io => "io".to_owned(),
            TileKind::Logic => "logic".to_owned(),
            TileKind::Ramb => "ramb".to_owned(),
            TileKind::Ramt => "ramt".to_owned(),
            TileKind::Dsp(n) => format!("dsp{n}"),
            TileKind::Ipcon => "ipcon".to_owned(),
        }
    }

    fn from_keyword(word: &str) -> Option<TileKind> {
        Some(match word {
            "io" => TileKind::Io,
            "logic" => TileKind::Logic,
            "ramb" => TileKind::Ramb,
            "ramt" => TileKind::Ramt,
            "ipcon" => TileKind::Ipcon,
            "dsp0" => TileKind::Dsp(0),
            "dsp1" => TileKind::Dsp(1),
            "dsp2" => TileKind::Dsp(2),
            "dsp3" => TileKind::Dsp(3),
            _ => return None,
        })
    }

    /// The width of the tile's bitmap in columns, which is fixed per kind
    /// (icepack's `tile_width`); every kind is sixteen rows tall.
    #[must_use]
    pub fn bit_cols(self) -> u32 {
        match self {
            TileKind::Empty => 0,
            TileKind::Io => 18,
            TileKind::Ramb | TileKind::Ramt => 42,
            TileKind::Logic | TileKind::Dsp(_) | TileKind::Ipcon => 54,
        }
    }
}

/// An IO block's position: `(x, y, z)`, `z` its index in the tile.
pub type IoAt = (u32, u32, u32);

/// One multiplexer: a destination net, the bits that select its source,
/// and every source with the bit pattern that selects it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mux {
    /// The tile it lives in.
    pub tile: (u32, u32),
    /// The net it drives.
    pub dst: u32,
    /// The bits of the selection field, in the order the patterns give
    /// them.
    pub bits: Vec<ConfigBit>,
    /// Each source: the pattern (bit `i` of the field is bit
    /// `bits.len() - 1 - i` of the number, as the text writes it left to
    /// right) and the source net.
    pub options: Vec<(u32, u32)>,
    /// Whether icebox calls it a `.routing` switch (a pass transistor
    /// between two span wires) rather than a `.buffer`.
    pub routing: bool,
}

impl Mux {
    /// The bits an option sets, which are the ones of its pattern.
    #[must_use]
    pub fn set_bits(&self, pattern: u32) -> Vec<ConfigBit> {
        let n = self.bits.len();
        self.bits
            .iter()
            .enumerate()
            .filter(|(i, _)| pattern >> (n - 1 - i) & 1 != 0)
            .map(|(_, b)| *b)
            .collect()
    }
}

/// A special-purpose cell: the PLL, an oscillator, the RGB driver, a DSP
/// or SPRAM block. Each entry is a port or parameter name and the tile
/// position and name of what it is wired to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraCell {
    /// The anchor tile.
    pub tile: (u32, u32),
    /// The index among cells of that tile, where the database gives one.
    pub z: u32,
    /// The cell type: `PLL`, `RGBA_DRV`, `HFOSC`, `SPRAM`, `MAC16`, ...
    pub kind: String,
    /// `(key, x, y, name)`: a port is wired to wire `name` of tile
    /// `(x, y)`; a parameter bit is the function bit `name` of that tile.
    pub entries: Vec<(String, u32, u32, String)>,
}

/// A parsed icebox chip database.
#[derive(Clone, Debug, Default)]
pub struct ChipDb {
    /// The device word: `1k`, `5k`, `8k`, ...
    pub device: String,
    /// Grid width in tiles.
    pub width: u32,
    /// Grid height in tiles.
    pub height: u32,
    /// The kind of every position, row-major from `(0, 0)`.
    pub tiles: Vec<TileKind>,
    /// Per tile kind, every named function and its bits.
    pub tile_bits: BTreeMap<TileKind, Vec<(String, Vec<ConfigBit>)>>,
    /// Every net: each `(x, y, name)` it is known by.
    pub nets: Vec<Vec<(u32, u32, String)>>,
    /// Every multiplexer and switch.
    pub muxes: Vec<Mux>,
    /// Package name to `(pin, (x, y, z))`.
    pub packages: BTreeMap<String, Vec<(String, IoAt)>>,
    /// `(x, y, network)`: the IO tile whose `fabout` can drive a global
    /// network.
    pub gbufin: Vec<(u32, u32, u32)>,
    /// `(x, y, z, network)`: the pad that drives a global network
    /// directly.
    pub gbufpin: Vec<(u32, u32, u32, u32)>,
    /// The IO tiles that drive a bank's latch signal.
    pub iolatch: Vec<(u32, u32)>,
    /// `((x, y, z), (x, y, z))`: the IO block whose input-enable and
    /// pull-up bits sit in another block.
    pub ieren: Vec<(IoAt, IoAt)>,
    /// `((x, y), (x, y))`: a column buffer at the first tile feeding the
    /// second.
    pub colbuf: Vec<((u32, u32), (u32, u32))>,
    /// Every special-purpose cell.
    pub extra_cells: Vec<ExtraCell>,
    /// `(function, bank, x, y)`: a bit outside every tile.
    pub extra_bits: Vec<(String, u32, u32, u32)>,
}

/// Why a database could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbError {
    /// The line number, from one.
    pub line: usize,
    /// What was wrong.
    pub message: String,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "chip database line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for DbError {}

/// `B12[3]` as a bit.
pub(crate) fn parse_bit(word: &str) -> Option<ConfigBit> {
    let rest = word.strip_prefix('B')?;
    let (row, col) = rest.split_once('[')?;
    let col = col.strip_suffix(']')?;
    Some(ConfigBit::new(row.parse().ok()?, col.parse().ok()?))
}

enum Section {
    None,
    Pins(String),
    GbufIn,
    GbufPin,
    IoLatch,
    IeRen,
    ColBuf,
    TileBits(TileKind),
    Net(usize),
    Mux(usize),
    ExtraCell(usize),
    ExtraBits,
}

impl ChipDb {
    /// Parses a chip database's text.
    ///
    /// # Errors
    ///
    /// [`DbError`] for a line that does not parse, naming it.
    pub fn parse(text: &str) -> Result<ChipDb, DbError> {
        let mut db = ChipDb::default();
        let mut section = Section::None;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let err = |message: String| DbError {
                line: index + 1,
                message,
            };
            let words: Vec<&str> = line.split_whitespace().collect();
            let num = |i: usize| -> Result<u32, DbError> {
                words
                    .get(i)
                    .and_then(|w| w.parse::<u32>().ok())
                    .ok_or_else(|| err(format!("expected a number in `{line}`")))
            };
            if let Some(directive) = words[0].strip_prefix('.') {
                section = match directive {
                    "device" => {
                        db.device = words
                            .get(1)
                            .ok_or_else(|| err("`.device` without a name".into()))?
                            .to_string();
                        db.width = num(2)?;
                        db.height = num(3)?;
                        let nets = num(4)? as usize;
                        db.tiles = vec![TileKind::Empty; (db.width * db.height) as usize];
                        db.nets = vec![Vec::new(); nets];
                        Section::None
                    }
                    "pins" => Section::Pins(
                        words
                            .get(1)
                            .ok_or_else(|| err("`.pins` without a package".into()))?
                            .to_string(),
                    ),
                    "gbufin" => Section::GbufIn,
                    "gbufpin" => Section::GbufPin,
                    "iolatch" => Section::IoLatch,
                    "ieren" => Section::IeRen,
                    "colbuf" => Section::ColBuf,
                    "extra_bits" => Section::ExtraBits,
                    "net" => {
                        let n = num(1)? as usize;
                        if n >= db.nets.len() {
                            return Err(err(format!("net {n} is past the count `.device` gave")));
                        }
                        Section::Net(n)
                    }
                    "buffer" | "routing" => {
                        let x = num(1)?;
                        let y = num(2)?;
                        let dst = num(3)?;
                        let bits = words[4..]
                            .iter()
                            .map(|w| parse_bit(w))
                            .collect::<Option<Vec<_>>>()
                            .ok_or_else(|| err(format!("a bad bit name in `{line}`")))?;
                        db.muxes.push(Mux {
                            tile: (x, y),
                            dst,
                            bits,
                            options: Vec::new(),
                            routing: directive == "routing",
                        });
                        Section::Mux(db.muxes.len() - 1)
                    }
                    "extra_cell" => {
                        let x = num(1)?;
                        let y = num(2)?;
                        let (z, kind) = if words.len() >= 5 {
                            (num(3)?, words[4])
                        } else {
                            (
                                0,
                                *words
                                    .get(3)
                                    .ok_or_else(|| err("`.extra_cell` without a type".into()))?,
                            )
                        };
                        db.extra_cells.push(ExtraCell {
                            tile: (x, y),
                            z,
                            kind: kind.to_owned(),
                            entries: Vec::new(),
                        });
                        Section::ExtraCell(db.extra_cells.len() - 1)
                    }
                    other => {
                        if let Some(kind) = other.strip_suffix("_tile_bits") {
                            let kind = TileKind::from_keyword(kind)
                                .ok_or_else(|| err(format!("unknown tile kind in `{line}`")))?;
                            db.tile_bits.entry(kind).or_default();
                            Section::TileBits(kind)
                        } else if let Some(kind) = other.strip_suffix("_tile") {
                            let kind = TileKind::from_keyword(kind)
                                .ok_or_else(|| err(format!("unknown tile kind in `{line}`")))?;
                            let (x, y) = (num(1)?, num(2)?);
                            if x >= db.width || y >= db.height {
                                return Err(err(format!("tile {x} {y} is outside the grid")));
                            }
                            db.tiles[(y * db.width + x) as usize] = kind;
                            Section::None
                        } else {
                            return Err(err(format!("unknown directive `.{other}`")));
                        }
                    }
                };
                continue;
            }
            match &section {
                Section::None => {
                    return Err(err(format!("a data line outside a section: `{line}`")));
                }
                Section::Pins(package) => {
                    let pin = words[0].to_owned();
                    let at = (num(1)?, num(2)?, num(3)?);
                    db.packages
                        .entry(package.clone())
                        .or_default()
                        .push((pin, at));
                }
                Section::GbufIn => db.gbufin.push((num(0)?, num(1)?, num(2)?)),
                Section::GbufPin => db.gbufpin.push((num(0)?, num(1)?, num(2)?, num(3)?)),
                Section::IoLatch => db.iolatch.push((num(0)?, num(1)?)),
                Section::IeRen => db
                    .ieren
                    .push(((num(0)?, num(1)?, num(2)?), (num(3)?, num(4)?, num(5)?))),
                Section::ColBuf => db.colbuf.push(((num(0)?, num(1)?), (num(2)?, num(3)?))),
                Section::TileBits(kind) => {
                    let bits = words[1..]
                        .iter()
                        .map(|w| parse_bit(w))
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| err(format!("a bad bit name in `{line}`")))?;
                    db.tile_bits
                        .entry(*kind)
                        .or_default()
                        .push((words[0].to_owned(), bits));
                }
                Section::Net(n) => {
                    let name = words
                        .get(2)
                        .ok_or_else(|| err(format!("a net segment without a name: `{line}`")))?;
                    db.nets[*n].push((num(0)?, num(1)?, (*name).to_owned()));
                }
                Section::Mux(m) => {
                    let pattern = words[0];
                    if pattern.len() != db.muxes[*m].bits.len()
                        || !pattern.bytes().all(|b| b == b'0' || b == b'1')
                    {
                        return Err(err(format!("a bad pattern in `{line}`")));
                    }
                    let value = u32::from_str_radix(pattern, 2)
                        .map_err(|_| err(format!("a bad pattern in `{line}`")))?;
                    let src = num(1)?;
                    db.muxes[*m].options.push((value, src));
                }
                Section::ExtraCell(c) => {
                    let key = words[0].to_owned();
                    let (x, y) = (num(1)?, num(2)?);
                    let name = words.get(3).ok_or_else(|| {
                        err(format!("an extra cell entry without a name: `{line}`"))
                    })?;
                    db.extra_cells[*c]
                        .entries
                        .push((key, x, y, (*name).to_owned()));
                }
                Section::ExtraBits => {
                    db.extra_bits
                        .push((words[0].to_owned(), num(1)?, num(2)?, num(3)?));
                }
            }
        }
        if db.device.is_empty() {
            return Err(DbError {
                line: 0,
                message: "no `.device` line: this is not an icebox chip database".into(),
            });
        }
        Ok(db)
    }

    /// The kind of the tile at `(x, y)`.
    #[must_use]
    pub fn tile(&self, x: u32, y: u32) -> TileKind {
        if x >= self.width || y >= self.height {
            return TileKind::Empty;
        }
        self.tiles[(y * self.width + x) as usize]
    }

    /// The bits of the named function of a tile kind (`LC_3`, `NegClk`,
    /// `IOB_0.PINTYPE_4`).
    #[must_use]
    pub fn function_bits(&self, kind: TileKind, function: &str) -> Option<&[ConfigBit]> {
        self.tile_bits
            .get(&kind)?
            .iter()
            .find(|(name, _)| name == function)
            .map(|(_, bits)| bits.as_slice())
    }

    /// The function of a tile kind whose name ends in `.<suffix>` or is
    /// `suffix`: an extra cell names `CBIT_3` where the tile says
    /// `IpConfig.CBIT_3`.
    #[must_use]
    pub fn function_by_suffix(&self, kind: TileKind, suffix: &str) -> Option<&[ConfigBit]> {
        self.tile_bits.get(&kind)?.iter().find_map(|(name, bits)| {
            (name == suffix
                || name
                    .rsplit_once('.')
                    .is_some_and(|(_, tail)| tail == suffix))
            .then_some(bits.as_slice())
        })
    }

    /// The package pins of `package`.
    #[must_use]
    pub fn package(&self, package: &str) -> Option<&[(String, IoAt)]> {
        self.packages.get(package).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# a comment
.device 5k 3 3 3

.pins sg48
20 1 0 1

.gbufpin
1 0 1 0

.io_tile 1 0
.logic_tile 1 1

.logic_tile_bits 54 16
NegClk B0[0]
LC_0 B0[36] B1[36]

.net 0
1 1 lutff_0/in_0
.net 1
1 1 local_g0_0
1 0 io_0/D_IN_0
.net 2
1 1 lutff_0/out

.buffer 1 1 0 B2[3] B2[4]
01 1
10 2

.extra_cell 1 1 0 HFOSC
CLKHF 1 1 glb_netwk_4
";

    #[test]
    fn a_small_database_reads_back() {
        let db = ChipDb::parse(SAMPLE).unwrap();
        assert_eq!((db.device.as_str(), db.width, db.height), ("5k", 3, 3));
        assert_eq!(db.tile(1, 1), TileKind::Logic);
        assert_eq!(db.tile(1, 0), TileKind::Io);
        assert_eq!(db.tile(0, 0), TileKind::Empty);
        assert_eq!(db.package("sg48").unwrap()[0], ("20".into(), (1, 0, 1)));
        assert_eq!(db.gbufpin, vec![(1, 0, 1, 0)]);
        assert_eq!(db.nets[1].len(), 2);
        let mux = &db.muxes[0];
        assert_eq!(mux.options, vec![(0b01, 1), (0b10, 2)]);
        // `01` sets the second bit named, not the first.
        assert_eq!(mux.set_bits(0b01), vec![ConfigBit::new(2, 4)]);
        assert_eq!(
            db.function_bits(TileKind::Logic, "LC_0").unwrap(),
            &[ConfigBit::new(0, 36), ConfigBit::new(1, 36)]
        );
        assert_eq!(db.extra_cells[0].kind, "HFOSC");
    }

    /// A line the parser does not understand is an error with its line
    /// number, never skipped: a skipped `.buffer` is a pip that is not
    /// there.
    #[test]
    fn an_unknown_directive_is_refused() {
        let err = ChipDb::parse(".device 1k 2 2 1\n.wire 0 0 x\n").unwrap_err();
        assert_eq!(err.line, 2);
        let err = ChipDb::parse(".device 1k 2 2 1\n.buffer 0 0 0 B0[1]\n11 0\n").unwrap_err();
        assert_eq!(err.line, 3, "{err}");
    }
}
