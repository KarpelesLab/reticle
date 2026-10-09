//! The Xilinx UltraScale+ configuration frames, read through Project
//! U-Ray's database.
//!
//! # What this is, and how far it reaches
//!
//! The first step of an UltraScale+ backend: given a `.bit` and the
//! database, say which tile every set bit belongs to and which feature
//! explains it. It places nothing, routes nothing and writes no
//! bitstream yet. What it establishes is the ground everything later
//! stands on — the frame layout, the frame address format, where a
//! tile's bits sit inside a frame, and which bits of a frame are not
//! configuration at all.
//!
//! The database is [`mithro/prjuray-db-ultrascaleplus`], CC0, generated
//! by Project U-Ray's die-independent flow from Vivado 2025.2. Only its
//! data is read here; `docs/fpga-uray.md` has the account and the
//! numbers.
//!
//! [`mithro/prjuray-db-ultrascaleplus`]: https://github.com/mithro/prjuray-db-ultrascaleplus
//!
//! # Where every number in this file comes from
//!
//! | Fact | Source |
//! |---|---|
//! | 93 words per frame | AMD UG570, *UltraScale Architecture Configuration*; and measured: Vivado's ZCU104 bitstream is 4 827 258 frame words, 51 906 frames of 93 |
//! | the frame address fields ([`FrameAddress`]) | UG570; and every `base` in `tilegrid.json` decodes to the row and column the same entry states |
//! | which columns a row has, and how many frames each holds | `tilegrid.json`, with the rule in [`FrameLayout::from_grid`] for the rows the processor system covers |
//! | two zero pad frames after each row of each block type | **measured**: the layout plus two per row is exactly the 51 906 frames Vivado wrote, and all 24 pad frames are zero |
//! | the ECC in word 45 and the low half of word 46 ([`is_ecc_bit`]) | **measured** on the same bitstream; see that function |
//! | the packet stream, the registers and the CRC | the 7-series ones, unchanged: [`super::xc7::read_bit`] reads Vivado's ZCU104 bitstream with every CRC matching |

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;

use super::arch::ConfigBit;
use super::xc7;
use crate::json::Json;

/// 32-bit words in one UltraScale+ configuration frame.
pub const WORDS_PER_FRAME: usize = 93;

/// Bits in one frame.
pub const BITS_PER_FRAME: usize = WORDS_PER_FRAME * 32;

/// Zero frames after the last frame of each row of each block type.
///
/// **Measured**, not documented: see the module table.
pub const PAD_FRAMES_PER_ROW: usize = 2;

/// The word of a frame that holds the low 32 bits of its ECC.
pub const ECC_WORD: usize = 45;

/// The word whose low 16 bits hold the rest of the ECC. Its high 16 bits
/// are configuration, owned by the `RCLK` tiles.
pub const ECC_HIGH_WORD: usize = 46;

/// True when bit `bit` of a frame (word `bit / 32`, bit `bit % 32`) is
/// part of the frame's ECC rather than configuration.
///
/// **Measured on Vivado's ZCU104 bitstream, not read from UG570.** Every
/// one of word 45's 32 bits and of word 46's low 16 is set in close to
/// half of the 51 906 frames — 8 400 to 10 100 times each — while every
/// other bit of a frame is set far less often. A frame with no other bit
/// set has all 48 clear. Word 46's high 16 bits behave like
/// configuration (6 to 443 frames each), and the database agrees: they
/// are the middle of the `RCLK` tiles' 96-bit window.
///
/// With these 48 bits left out, every other set bit of that bitstream
/// belongs to a tile. With them left in, 50 983 set bits belong to none.
///
/// What the 48 bits compute is **not** established. The code is
/// deterministic in the frame's data (21 259 distinct frames, no two
/// alike with different ECC) and XORs exactly in every case tried (47 of
/// 47 frames with two bits set match the XOR of the two one-bit frames),
/// yet a GF(2) solve over all the frames is inconsistent.
/// `docs/fpga-uray.md` has the measurements.
pub fn is_ecc_bit(bit: usize) -> bool {
    let word = bit / 32;
    word == ECC_WORD || (word == ECC_HIGH_WORD && bit % 32 < 16)
}

/// Why the database or a bitstream could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrayError {
    /// A file is there but does not hold what it should.
    Malformed {
        /// What was being read.
        path: String,
        /// What is wrong with it.
        message: String,
    },
    /// The bitstream's frame data does not fit the layout.
    WrongSize {
        /// Frame words the layout wants.
        expected: usize,
        /// Frame words the bitstream carries.
        found: usize,
    },
    /// The bitstream itself does not read.
    Bit(xc7::Xc7Error),
}

impl fmt::Display for UrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrayError::Malformed { path, message } => write!(f, "`{path}`: {message}"),
            UrayError::WrongSize { expected, found } => write!(
                f,
                "the bitstream carries {found} frame word(s) and the part has {expected}"
            ),
            UrayError::Bit(e) => write!(f, "{e}"),
        }
    }
}

impl Error for UrayError {}

impl From<xc7::Xc7Error> for UrayError {
    fn from(e: xc7::Xc7Error) -> Self {
        UrayError::Bit(e)
    }
}

/// One UltraScale+ frame address, as the frame address register holds
/// it: block type in bits 26:24, row in 23:18, column in 17:8 and minor
/// (the frame within the column) in 7:0.
///
/// There is no top/bottom bit: unlike a 7-series part, rows count from
/// the bottom of the die.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameAddress {
    /// The block type: 0 is `CLB_IO_CLK`, 1 is `BLOCK_RAM` (contents).
    pub block: u8,
    /// The clock region row.
    pub row: u8,
    /// The configuration column.
    pub column: u16,
    /// The frame within the column.
    pub minor: u8,
}

impl FrameAddress {
    /// The address as the frame address register word.
    pub fn to_u32(self) -> u32 {
        (u32::from(self.block & 0x7) << 24)
            | (u32::from(self.row & 0x3F) << 18)
            | (u32::from(self.column & 0x3FF) << 8)
            | u32::from(self.minor)
    }

    /// The address a frame address register word names.
    pub fn from_u32(word: u32) -> FrameAddress {
        FrameAddress {
            block: u8::try_from((word >> 24) & 0x7).unwrap_or(0),
            row: u8::try_from((word >> 18) & 0x3F).unwrap_or(0),
            column: u16::try_from((word >> 8) & 0x3FF).unwrap_or(0),
            minor: u8::try_from(word & 0xFF).unwrap_or(0),
        }
    }
}

impl fmt::Display for FrameAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:#010x} (block {}, row {}, column {}, minor {})",
            self.to_u32(),
            self.block,
            self.row,
            self.column,
            self.minor
        )
    }
}

/// Where one tile's bits sit: `frames` frames from `base`, and within
/// each the `bits` bits from bit `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitWindow {
    /// The first frame.
    pub base: FrameAddress,
    /// How many consecutive minors.
    pub frames: u32,
    /// The first bit within each frame.
    pub offset: u32,
    /// How many bits.
    pub bits: u32,
}

impl BitWindow {
    /// The tile bit that frame `address`, bit `bit` is, when the window
    /// covers it. A [`ConfigBit`]'s row is the frame counted from
    /// `base`, its column the bit counted from `offset` — the
    /// `<frame>_<bit>` of a `segbits` line.
    pub fn tile_bit(&self, address: FrameAddress, bit: u32) -> Option<ConfigBit> {
        let same_column = address.block == self.base.block
            && address.row == self.base.row
            && address.column == self.base.column;
        let frame = u32::from(address.minor).checked_sub(u32::from(self.base.minor))?;
        let col = bit.checked_sub(self.offset)?;
        (same_column && frame < self.frames && col < self.bits).then(|| ConfigBit::new(frame, col))
    }
}

/// One tile of the grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    /// The tile's name, `CLEL_R_X0Y268`.
    pub name: String,
    /// Its type, which names its `segbits_<type>.db`.
    pub kind: String,
    /// Its position in the database's grid.
    pub grid: (u32, u32),
    /// Where its bits are. Most tiles have none, most of the rest one; a
    /// block RAM has a second, in the `BLOCK_RAM` block, for its
    /// contents.
    pub windows: Vec<BitWindow>,
}

/// Every tile of one die: `<die>/tilegrid.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileGrid {
    tiles: Vec<Tile>,
}

impl TileGrid {
    /// Reads a `tilegrid.json`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for text that is not JSON, or an entry
    /// without the fields every entry has.
    pub fn parse(text: &str, path: &str) -> Result<TileGrid, UrayError> {
        let bad = |message: String| UrayError::Malformed {
            path: path.to_owned(),
            message,
        };
        let json = Json::parse(text).map_err(|e| bad(e.to_string()))?;
        let Json::Object(entries) = json else {
            return Err(bad("the top level is not an object".to_owned()));
        };
        let mut tiles = Vec::with_capacity(entries.len());
        for (name, entry) in entries {
            let field = |key: &str| {
                entry
                    .get(key)
                    .and_then(Json::as_u32)
                    .ok_or_else(|| bad(format!("tile `{name}` has no `{key}`")))
            };
            let kind = entry
                .get("type")
                .and_then(Json::as_str)
                .ok_or_else(|| bad(format!("tile `{name}` has no `type`")))?
                .to_owned();
            let grid = (field("gx")?, field("gy")?);
            let mut windows = Vec::new();
            if let Some(Json::Array(bits)) = entry.get("bits") {
                for window in bits {
                    let get = |key: &str| {
                        window
                            .get(key)
                            .and_then(Json::as_u32)
                            .ok_or_else(|| bad(format!("a bit window of `{name}` has no `{key}`")))
                    };
                    let base = window
                        .get("base")
                        .and_then(Json::as_str)
                        .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                        .ok_or_else(|| bad(format!("a bit window of `{name}` has no `base`")))?;
                    let base = FrameAddress::from_u32(base);
                    // The entry states its row, column and block apart
                    // from `base`; they must be the fields `base` decodes
                    // to, or the address format above is wrong.
                    let stated = (get("block")?, get("row")?, get("col")?);
                    let decoded = (
                        u32::from(base.block),
                        u32::from(base.row),
                        u32::from(base.column),
                    );
                    if stated != decoded {
                        return Err(bad(format!(
                            "`{name}`: base {base} decodes to block/row/column {decoded:?} \
                             but the entry says {stated:?}"
                        )));
                    }
                    windows.push(BitWindow {
                        base,
                        frames: get("frames")?,
                        offset: get("offset")?,
                        bits: get("nbits")?,
                    });
                }
            }
            tiles.push(Tile {
                name,
                kind,
                grid,
                windows,
            });
        }
        Ok(TileGrid { tiles })
    }

    /// Builds a grid from tiles, for a test.
    pub fn from_tiles(tiles: Vec<Tile>) -> TileGrid {
        TileGrid { tiles }
    }

    /// Every tile, in file order.
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }
}

/// Every frame of one die, in the order the configuration engine
/// consumes them when the frame address register starts at zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameLayout {
    /// Frames per column, by (block, row).
    rows: BTreeMap<(u8, u8), Vec<u32>>,
    /// One entry per frame in stream order; `None` is a pad frame.
    order: Vec<Option<FrameAddress>>,
    index: HashMap<u32, usize>,
}

impl FrameLayout {
    /// The layout the tile grid implies.
    ///
    /// A column's width is the furthest frame any tile's window reaches
    /// in it. Where the processor system sits — the left of the bottom
    /// rows on a Zynq UltraScale+ — no tile has bits, yet the
    /// configuration engine still streams those columns: a column absent
    /// from one row takes the width the same column has in the rows of
    /// the same block that do have it.
    ///
    /// That rule is **measured**, not documented. On the ZU7EV die,
    /// columns 0 to 84 of rows 0 to 3 have no tile; filled in this way
    /// the layout is 51 882 frames, plus two pad frames per row, 51 906 —
    /// exactly what Vivado wrote — and the ownership check of [`decode`]
    /// finds every set bit of that bitstream inside a tile, which a
    /// misaligned column would not allow.
    pub fn from_grid(grid: &TileGrid) -> FrameLayout {
        let mut widths: BTreeMap<(u8, u8, u16), u32> = BTreeMap::new();
        for window in grid.tiles.iter().flat_map(|t| &t.windows) {
            let key = (window.base.block, window.base.row, window.base.column);
            let end = u32::from(window.base.minor) + window.frames;
            let width = widths.entry(key).or_insert(0);
            *width = (*width).max(end);
        }
        let mut by_column: BTreeMap<(u8, u16), u32> = BTreeMap::new();
        let mut row_set: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
        for (&(block, row, column), &width) in &widths {
            let w = by_column.entry((block, column)).or_insert(0);
            *w = (*w).max(width);
            let rows = row_set.entry(block).or_default();
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
        let mut rows = BTreeMap::new();
        for (&block, block_rows) in &row_set {
            let columns: Vec<(u16, u32)> = by_column
                .iter()
                .filter(|((b, _), _)| *b == block)
                .map(|(&(_, c), &w)| (c, w))
                .collect();
            let count = columns.last().map_or(0, |(c, _)| usize::from(*c) + 1);
            for &row in block_rows {
                let mut frames = vec![0u32; count];
                for &(column, width) in &columns {
                    frames[usize::from(column)] =
                        widths.get(&(block, row, column)).copied().unwrap_or(width);
                }
                rows.insert((block, row), frames);
            }
        }
        FrameLayout::from_rows(rows)
    }

    /// The layout of the given rows: frames per column, by
    /// (block, row).
    pub fn from_rows(rows: BTreeMap<(u8, u8), Vec<u32>>) -> FrameLayout {
        let mut order = Vec::new();
        let mut index = HashMap::new();
        for (&(block, row), columns) in &rows {
            for (column, &frames) in columns.iter().enumerate() {
                for minor in 0..frames {
                    let address = FrameAddress {
                        block,
                        row,
                        column: u16::try_from(column).unwrap_or(u16::MAX),
                        minor: u8::try_from(minor).unwrap_or(u8::MAX),
                    };
                    index.insert(address.to_u32(), order.len());
                    order.push(Some(address));
                }
            }
            order.extend(std::iter::repeat_n(None, PAD_FRAMES_PER_ROW));
        }
        FrameLayout { rows, order, index }
    }

    /// Frames per column, by (block, row).
    pub fn rows(&self) -> &BTreeMap<(u8, u8), Vec<u32>> {
        &self.rows
    }

    /// Every frame in stream order, `None` for a pad frame.
    pub fn order(&self) -> &[Option<FrameAddress>] {
        &self.order
    }

    /// Frames in the stream, pad frames included.
    pub fn frames(&self) -> usize {
        self.order.len()
    }

    /// Frames that carry data.
    pub fn data_frames(&self) -> usize {
        self.order.iter().filter(|f| f.is_some()).count()
    }

    /// Where in the stream a frame sits.
    pub fn position(&self, address: FrameAddress) -> Option<usize> {
        self.index.get(&address.to_u32()).copied()
    }
}

/// What a `.bit` holds, read for an UltraScale+ part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UltraScaleBit {
    /// The wrapper's part field (`xczu7ev-ffvc1156-2-e`).
    pub part: String,
    /// The IDCODE the stream writes.
    pub idcode: Option<u32>,
    /// Every word written to `FDRI`, in stream order.
    pub frames: Vec<u32>,
}

/// Reads a `.bit` for an UltraScale+ part.
///
/// The packet stream, its registers and its CRC are the 7-series ones,
/// so this is [`xc7::read_bit`], which checks every CRC; only what the
/// words mean differs, which is why its 7-series start address is not
/// passed on.
///
/// # Errors
///
/// [`UrayError::Bit`] for a file [`xc7::read_bit`] refuses.
pub fn read_bit(bytes: &[u8]) -> Result<UltraScaleBit, UrayError> {
    let bit = xc7::read_bit(bytes)?;
    Ok(UltraScaleBit {
        part: bit.header.part,
        idcode: bit.idcode,
        frames: bit.frames,
    })
}

/// Every set bit of a frame stream, given to the tiles that own it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// Set bits, ECC excluded.
    pub set_bits: usize,
    /// Set ECC bits ([`is_ecc_bit`]).
    pub ecc_bits: usize,
    /// Set bits in pad frames, which should be none.
    pub pad_bits: usize,
    /// Set bits no tile owns, with the frame and bit.
    pub unowned: Vec<(FrameAddress, u32)>,
    /// Each owning tile's set bits, by index into [`TileGrid::tiles`].
    /// A bit two tiles' windows share is given to both.
    pub tiles: BTreeMap<usize, Vec<ConfigBit>>,
    /// For each tile bit, the frame bit it came from, so a bit owned
    /// twice can be counted once.
    origin: HashMap<(usize, ConfigBit), (u32, u32)>,
}

/// Gives every set bit of `words` to the tiles whose windows hold it.
///
/// # Errors
///
/// [`UrayError::WrongSize`] when `words` is not the layout's length.
pub fn decode(grid: &TileGrid, layout: &FrameLayout, words: &[u32]) -> Result<Decoded, UrayError> {
    let expected = layout.frames() * WORDS_PER_FRAME;
    if words.len() != expected {
        return Err(UrayError::WrongSize {
            expected,
            found: words.len(),
        });
    }
    // Which windows touch each data frame.
    let mut owners: HashMap<u32, Vec<(usize, BitWindow)>> = HashMap::new();
    for (index, tile) in grid.tiles.iter().enumerate() {
        for window in &tile.windows {
            for frame in 0..window.frames {
                let mut address = window.base;
                address.minor = u8::try_from(u32::from(address.minor) + frame).unwrap_or(u8::MAX);
                owners
                    .entry(address.to_u32())
                    .or_default()
                    .push((index, *window));
            }
        }
    }
    let mut out = Decoded::default();
    for (position, slot) in layout.order().iter().enumerate() {
        let frame = &words[position * WORDS_PER_FRAME..(position + 1) * WORDS_PER_FRAME];
        for (word_index, &word) in frame.iter().enumerate() {
            let mut rest = word;
            while rest != 0 {
                let low = rest.trailing_zeros();
                rest &= rest - 1;
                let bit = word_index * 32 + usize::try_from(low).unwrap_or(0);
                let Some(address) = slot else {
                    out.pad_bits += 1;
                    continue;
                };
                if is_ecc_bit(bit) {
                    out.ecc_bits += 1;
                    continue;
                }
                out.set_bits += 1;
                let bit = u32::try_from(bit).unwrap_or(u32::MAX);
                let mut owned = false;
                for (tile, window) in owners.get(&address.to_u32()).into_iter().flatten() {
                    if let Some(tile_bit) = window.tile_bit(*address, bit) {
                        owned = true;
                        out.tiles.entry(*tile).or_default().push(tile_bit);
                        out.origin
                            .insert((*tile, tile_bit), (address.to_u32(), bit));
                    }
                }
                if !owned {
                    out.unowned.push((*address, bit));
                }
            }
        }
    }
    Ok(out)
}

/// What one tile type's files say: its features, and the bits that are
/// set when a feature is *not* used (`defaults_<type>.db`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileTypeBits {
    /// The features of `segbits_<type>.db`.
    pub features: Vec<super::xray::Feature>,
    /// The bits of `defaults_<type>.db`.
    pub defaults: Vec<ConfigBit>,
}

impl TileTypeBits {
    /// Reads a `segbits_<type>.db` and, when there is one, its
    /// `defaults_<type>.db`. A `defaults` line is one `<frame>_<bit>`.
    ///
    /// # Errors
    ///
    /// [`UrayError::Malformed`] for a line that does not parse.
    pub fn parse(
        segbits: &str,
        segbits_path: &str,
        defaults: Option<(&str, &str)>,
    ) -> Result<TileTypeBits, UrayError> {
        let features = super::xray::parse_segbits(segbits, segbits_path)
            .map_err(|e| UrayError::Malformed {
                path: segbits_path.to_owned(),
                message: e.to_string(),
            })?
            .into_features();
        let mut bits = Vec::new();
        if let Some((text, path)) = defaults {
            for (number, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parsed = line
                    .split_once('_')
                    .and_then(|(f, b)| Some(ConfigBit::new(f.parse().ok()?, b.parse().ok()?)));
                bits.push(parsed.ok_or_else(|| UrayError::Malformed {
                    path: path.to_owned(),
                    message: format!("line {}: `{line}` is not a <frame>_<bit>", number + 1),
                })?);
            }
        }
        Ok(TileTypeBits {
            features,
            defaults: bits,
        })
    }

    /// The file a tile type's bits are in, without the `.db`:
    /// `CLEL_R` is `segbits_clel_r`. A `<type>@TOP` variant has its own
    /// lines in the base type's file, so only the part before `@` names
    /// it.
    pub fn file_stem(kind: &str) -> String {
        let base = kind.split('@').next().unwrap_or(kind);
        format!("segbits_{}", base.to_ascii_lowercase())
    }
}

/// How much of a decoded bitstream the database accounts for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Explained {
    /// Owned set bits that some feature or default of some owning tile
    /// accounts for.
    pub explained: usize,
    /// Owned set bits none does, counted once each however many tiles
    /// own them, with one of the owning tiles' types.
    pub unexplained: BTreeMap<String, usize>,
}

impl Explained {
    /// Every unexplained bit.
    pub fn unexplained_total(&self) -> usize {
        self.unexplained.values().sum()
    }
}

/// Which of a decoded bitstream's bits the database accounts for.
///
/// A feature is taken to be on in a tile when every bit it needs set is
/// set there and every bit it needs clear is clear; its set bits are
/// then explained. A tile's `defaults` bits are explained when set. A
/// frame bit two tiles share is explained when either tile explains it.
/// A tile type missing from `types` explains nothing.
pub fn explain(
    grid: &TileGrid,
    decoded: &Decoded,
    types: &HashMap<String, TileTypeBits>,
) -> Explained {
    let mut explained_at: std::collections::HashSet<(u32, u32)> = Default::default();
    for (&tile, bits) in &decoded.tiles {
        let Some(info) = types.get(&grid.tiles[tile].kind) else {
            continue;
        };
        let set: std::collections::HashSet<ConfigBit> = bits.iter().copied().collect();
        let mut mark = |bit: &ConfigBit| {
            if let Some(origin) = decoded.origin.get(&(tile, *bit)) {
                explained_at.insert(*origin);
            }
        };
        for feature in &info.features {
            let on = !feature.ones.is_empty()
                && feature.ones.iter().all(|b| set.contains(b))
                && feature.zeros.iter().all(|b| !set.contains(b));
            if on {
                feature.ones.iter().for_each(&mut mark);
            }
        }
        info.defaults
            .iter()
            .filter(|b| set.contains(b))
            .for_each(&mut mark);
    }
    let mut out = Explained::default();
    let mut counted: std::collections::HashSet<(u32, u32)> = Default::default();
    for (&tile, bits) in &decoded.tiles {
        for bit in bits {
            let Some(&origin) = decoded.origin.get(&(tile, *bit)) else {
                continue;
            };
            if !counted.insert(origin) {
                continue;
            }
            if explained_at.contains(&origin) {
                out.explained += 1;
            } else {
                *out.unexplained
                    .entry(grid.tiles[tile].kind.clone())
                    .or_default() += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_addresses_round_trip_through_the_register_word() {
        let far = FrameAddress {
            block: 1,
            row: 5,
            column: 217,
            minor: 255,
        };
        assert_eq!(FrameAddress::from_u32(far.to_u32()), far);
        // Two `base` values of the ZU7EV `tilegrid.json`, with the fields
        // the same entries state beside them.
        assert_eq!(
            FrameAddress::from_u32(0x0004_8400),
            FrameAddress {
                block: 0,
                row: 1,
                column: 132,
                minor: 0
            }
        );
        assert_eq!(
            FrameAddress::from_u32(0x0104_0200),
            FrameAddress {
                block: 1,
                row: 1,
                column: 2,
                minor: 0
            }
        );
    }

    #[test]
    fn the_ecc_is_word_45_and_the_low_half_of_word_46() {
        let ecc: Vec<usize> = (0..BITS_PER_FRAME).filter(|&b| is_ecc_bit(b)).collect();
        assert_eq!(ecc.len(), 48);
        assert_eq!(ecc.first(), Some(&(45 * 32)));
        assert_eq!(ecc.last(), Some(&(46 * 32 + 15)));
        assert!(!is_ecc_bit(46 * 32 + 16));
    }

    fn window(block: u8, row: u8, column: u16, frames: u32, offset: u32, bits: u32) -> BitWindow {
        BitWindow {
            base: FrameAddress {
                block,
                row,
                column,
                minor: 0,
            },
            frames,
            offset,
            bits,
        }
    }

    fn tile(name: &str, kind: &str, windows: Vec<BitWindow>) -> Tile {
        Tile {
            name: name.to_owned(),
            kind: kind.to_owned(),
            grid: (0, 0),
            windows,
        }
    }

    /// Row 0 lacks column 0, as the rows under a processor system do; it
    /// takes row 1's width, and each row ends in two pad frames.
    #[test]
    fn a_column_missing_from_one_row_takes_its_width_from_another() {
        let grid = TileGrid::from_tiles(vec![
            tile("A", "T", vec![window(0, 1, 0, 3, 0, 48)]),
            tile("B", "T", vec![window(0, 0, 1, 2, 0, 48)]),
            tile("C", "T", vec![window(0, 1, 1, 2, 0, 48)]),
        ]);
        let layout = FrameLayout::from_grid(&grid);
        assert_eq!(layout.rows()[&(0, 0)], vec![3, 2]);
        assert_eq!(layout.rows()[&(0, 1)], vec![3, 2]);
        assert_eq!(layout.frames(), 2 * (5 + PAD_FRAMES_PER_ROW));
        assert_eq!(layout.order()[5], None);
        assert_eq!(
            layout.position(FrameAddress {
                block: 0,
                row: 1,
                column: 0,
                minor: 0
            }),
            Some(7)
        );
    }

    #[test]
    fn a_set_bit_goes_to_its_tile_and_ecc_and_strays_are_counted_apart() {
        let grid = TileGrid::from_tiles(vec![tile("A", "T", vec![window(0, 0, 0, 2, 64, 32)])]);
        let layout = FrameLayout::from_grid(&grid);
        let mut words = vec![0u32; layout.frames() * WORDS_PER_FRAME];
        words[WORDS_PER_FRAME + 2] = 1 << 3; // frame 1, bit 67: tile bit (1, 3)
        words[ECC_WORD] = 0xFFFF_FFFF; // 32 ECC bits
        words[ECC_HIGH_WORD] = 0x0001_0000; // configuration, but no tile has it
        let decoded = decode(&grid, &layout, &words).unwrap();
        assert_eq!(decoded.ecc_bits, 32);
        assert_eq!(decoded.set_bits, 2);
        assert_eq!(decoded.tiles[&0], vec![ConfigBit::new(1, 3)]);
        assert_eq!(decoded.unowned.len(), 1);
        assert_eq!(decoded.unowned[0].1, 46 * 32 + 16);

        let mut types = HashMap::new();
        types.insert(
            "T".to_owned(),
            TileTypeBits::parse("T.F 01_003\n", "segbits_t.db", None).unwrap(),
        );
        let explained = explain(&grid, &decoded, &types);
        assert_eq!(explained.explained, 1);
        assert_eq!(explained.unexplained_total(), 0);
    }

    #[test]
    fn a_feature_whose_clear_bit_is_set_is_not_on() {
        let grid = TileGrid::from_tiles(vec![tile("A", "T", vec![window(0, 0, 0, 1, 0, 32)])]);
        let layout = FrameLayout::from_grid(&grid);
        let mut words = vec![0u32; layout.frames() * WORDS_PER_FRAME];
        words[0] = 0b11;
        let decoded = decode(&grid, &layout, &words).unwrap();
        let mut types = HashMap::new();
        types.insert(
            "T".to_owned(),
            TileTypeBits::parse("T.F 00_000 !00_001\n", "segbits_t.db", None).unwrap(),
        );
        let explained = explain(&grid, &decoded, &types);
        assert_eq!(explained.explained, 0);
        assert_eq!(explained.unexplained["T"], 2);
    }

    #[test]
    fn the_base_must_agree_with_the_fields_beside_it() {
        let good = r#"{"T_X0Y0": {"type": "T", "gx": 1, "gy": 2, "bits": [
            {"base": "0x00048400", "block": 0, "row": 1, "col": 132,
             "frames": 6, "half": 0, "nbits": 240, "offset": 2256}]}}"#;
        let grid = TileGrid::parse(good, "tilegrid.json").unwrap();
        assert_eq!(grid.tiles()[0].windows[0].base.column, 132);
        let bad = good.replace("\"col\": 132", "\"col\": 133");
        assert!(TileGrid::parse(&bad, "tilegrid.json").is_err());
    }

    #[test]
    fn a_top_variant_reads_its_base_types_file() {
        assert_eq!(TileTypeBits::file_stem("CLEL_R"), "segbits_clel_r");
        assert_eq!(
            TileTypeBits::file_stem("RIOB18_SING@TOP"),
            "segbits_riob18_sing"
        );
    }
}
