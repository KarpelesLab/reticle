//! Bitstream generation: a placed and routed design as configuration
//! bits.
//!
//! # What this produces, and what it does not
//!
//! The bits are real bits in a real frame layout, and with the
//! architecture this crate ships they configure nothing. Read
//! [`super::arch::synthetic`] first: the tile grid, the tile types and
//! their bitmap sizes follow the published iCE40 geometry, but *every bit
//! position inside a tile is invented*, because the database that says
//! where the real ones are is Project IceStorm's and is not here.
//!
//! So: the output is structurally a bitstream — the right shape, the
//! right size, a valid `.asc` that reads back to the same bits — and it
//! will not program a part. Swapping in a real database makes it one
//! without a line of code changing here, because nothing in this module
//! knows what a bit *means*: every position comes from the architecture,
//! as a [`Pip`](super::arch::Pip)'s interned bits or a bel's
//! [`ConfigEntry`]. What such a database has to supply is listed in
//! [`super::arch`].
//!
//! # The two files
//!
//! [`Bitstream::write_asc`] writes the IceStorm-style text format: a
//! `.device` line, then one block per tile headed `.<keyword>_tile <x>
//! <y>` and followed by one line of `0`/`1` per bitmap row. It is what
//! `icepack` reads and what `icebox_vlog` explains, it diffs, and
//! [`Bitstream::parse_asc`] reads it back, so a test can prove the writer
//! and the reader agree.
//!
//! [`Bitstream::write_bin`] writes a binary container. It is **Reticle's
//! own** format, documented on that function, not the iCE40
//! configuration image: turning an `.asc` into something an FPGA's
//! configuration engine accepts is what `icepack` does, using the same
//! database this crate does not have. The binary is there so a bitstream
//! can be stored and compared compactly, and it round-trips through
//! [`Bitstream::read_bin`].
//!
//! # What ends up set
//!
//! - every pip a route uses contributes its bits in the tile
//!   that holds it;
//! - every placed cell contributes the bits its bel's
//!   [`ConfigEntry::Cell`] entry gives for that primitive (which is how
//!   twenty `SB_DFF*` variants become one mode field) and, for each
//!   [`ConfigEntry::Param`] entry, the bits of the parameter the cell
//!   actually carries (`LUT_INIT`, `PIN_TYPE`, `READ_MODE`).
//!
//! Nothing else. A bit no pip and no cell claims stays zero.

use std::error::Error;
use std::fmt;

use super::arch::{Arch, ConfigBit, ConfigEntry, RoutingGraph};
use super::place::{Netlist, Placement};
use super::route::Routing;
use crate::ir::{AttrValue, Design, ModuleId};

/// Why a bitstream could not be produced or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BitstreamError {
    /// A tile position that the format does not have.
    NoTile {
        /// The column.
        x: u32,
        /// The row.
        y: u32,
    },
    /// A bit outside a tile's bitmap.
    OutOfRange {
        /// The tile.
        tile: (u32, u32),
        /// The bit that does not fit.
        bit: ConfigBit,
        /// The bitmap's size.
        size: (u32, u32),
    },
    /// The module id does not belong to the design.
    NoSuchModule,
    /// A malformed `.asc` file, with the line number.
    Asc {
        /// The line, counting from 1.
        line: usize,
        /// What is wrong with it.
        message: String,
    },
    /// A malformed binary container.
    Binary(String),
}

impl fmt::Display for BitstreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitstreamError::NoTile { x, y } => write!(f, "there is no tile at ({x}, {y})"),
            BitstreamError::OutOfRange { tile, bit, size } => write!(
                f,
                "bit {}.{} is outside the {} x {} bitmap of the tile at ({}, {})",
                bit.row, bit.col, size.0, size.1, tile.0, tile.1
            ),
            BitstreamError::NoSuchModule => f.write_str("the module is not part of the design"),
            BitstreamError::Asc { line, message } => write!(f, "line {line}: {message}"),
            BitstreamError::Binary(message) => write!(f, "{message}"),
        }
    }
}

impl Error for BitstreamError {}

/// Where one tile's bits live in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileFormat {
    /// The tile's position.
    pub tile: (u32, u32),
    /// The word an `.asc` file heads it with, without the dot and
    /// without the position (`logic_tile`).
    pub keyword: String,
    /// Rows of its bitmap.
    pub rows: u32,
    /// Columns of its bitmap.
    pub cols: u32,
}

impl TileFormat {
    /// How many bits the tile holds.
    pub fn bits(&self) -> usize {
        self.rows as usize * self.cols as usize
    }
}

/// The frame layout of one part: which tiles there are and how big each
/// one's bitmap is.
///
/// This comes from the architecture, never from Rust. Two architectures
/// with different tile sizes produce different layouts and the rest of
/// the module does not notice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BitstreamFormat {
    /// The word the `.asc` `.device` line carries (`1k`).
    pub device: String,
    /// Every tile, in row-major order (`y` then `x`), which is the order
    /// the files use.
    pub tiles: Vec<TileFormat>,
}

impl BitstreamFormat {
    /// The layout `arch` describes.
    pub fn from_arch(arch: &Arch) -> BitstreamFormat {
        let mut tiles = Vec::new();
        for y in 0..arch.height {
            for x in 0..arch.width {
                if let Some(index) = arch.tile_index_at(x, y) {
                    let tile = &arch.tile_types[index];
                    tiles.push(TileFormat {
                        tile: (x, y),
                        keyword: tile.asc_keyword.clone(),
                        rows: tile.bit_rows,
                        cols: tile.bit_cols,
                    });
                }
            }
        }
        BitstreamFormat {
            device: arch.asc_device.clone(),
            tiles,
        }
    }

    /// The index of the tile at `(x, y)`.
    pub fn index_of(&self, x: u32, y: u32) -> Option<usize> {
        self.tiles.iter().position(|t| t.tile == (x, y))
    }

    /// How many bits the whole part holds.
    pub fn bits(&self) -> usize {
        self.tiles.iter().map(TileFormat::bits).sum()
    }
}

/// One part's configuration bits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bitstream {
    /// The frame layout.
    pub format: BitstreamFormat,
    /// One bitmap per tile of `format`, row-major inside the tile.
    bits: Vec<Vec<bool>>,
}

impl Bitstream {
    /// An all-zero bitstream in the given layout.
    pub fn empty(format: BitstreamFormat) -> Bitstream {
        let bits = format.tiles.iter().map(|t| vec![false; t.bits()]).collect();
        Bitstream { format, bits }
    }

    /// Sets one bit of one tile.
    ///
    /// # Errors
    ///
    /// [`BitstreamError::NoTile`] when the position holds no tile and
    /// [`BitstreamError::OutOfRange`] when the bit is outside its bitmap.
    /// Both mean the architecture and the routing disagree, which is a
    /// broken database rather than a broken design.
    pub fn set(&mut self, tile: (u32, u32), bit: ConfigBit) -> Result<(), BitstreamError> {
        let index = self
            .format
            .index_of(tile.0, tile.1)
            .ok_or(BitstreamError::NoTile {
                x: tile.0,
                y: tile.1,
            })?;
        let format = &self.format.tiles[index];
        if bit.row >= format.rows || bit.col >= format.cols {
            return Err(BitstreamError::OutOfRange {
                tile,
                bit,
                size: (format.rows, format.cols),
            });
        }
        let offset = bit.row as usize * format.cols as usize + bit.col as usize;
        self.bits[index][offset] = true;
        Ok(())
    }

    /// Whether one bit of one tile is set; `None` when there is no such
    /// tile or bit.
    pub fn get(&self, tile: (u32, u32), bit: ConfigBit) -> Option<bool> {
        let index = self.format.index_of(tile.0, tile.1)?;
        let format = &self.format.tiles[index];
        if bit.row >= format.rows || bit.col >= format.cols {
            return None;
        }
        let offset = bit.row as usize * format.cols as usize + bit.col as usize;
        Some(self.bits[index][offset])
    }

    /// How many bits are set.
    pub fn ones(&self) -> usize {
        self.bits
            .iter()
            .map(|tile| tile.iter().filter(|b| **b).count())
            .sum()
    }

    /// The tiles that hold at least one set bit, with their set bits, in
    /// file order.
    pub fn used_tiles(&self) -> Vec<(&TileFormat, Vec<ConfigBit>)> {
        let mut out = Vec::new();
        for (index, format) in self.format.tiles.iter().enumerate() {
            let mut set = Vec::new();
            for (offset, value) in self.bits[index].iter().enumerate() {
                if *value {
                    let offset = u32::try_from(offset).unwrap_or(0);
                    set.push(ConfigBit::new(offset / format.cols, offset % format.cols));
                }
            }
            if !set.is_empty() {
                out.push((format, set));
            }
        }
        out
    }

    /// A compact, diffable summary: the device, the totals, and the set
    /// bits of each tile that has any.
    ///
    /// A full `.asc` of a part this size is mostly zeroes; this is the
    /// part of it a reviewer reads.
    pub fn to_summary(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "device {}", self.format.device);
        let _ = writeln!(
            out,
            "{} tile(s), {} bit(s), {} set",
            self.format.tiles.len(),
            self.format.bits(),
            self.ones()
        );
        for (format, set) in self.used_tiles() {
            let bits: Vec<String> = set.iter().map(|b| format!("{}.{}", b.row, b.col)).collect();
            let _ = writeln!(
                out,
                "{} {} {}: {}",
                format.keyword,
                format.tile.0,
                format.tile.1,
                bits.join(",")
            );
        }
        out
    }

    /// Writes the IceStorm-style `.asc` text.
    ///
    /// Every tile is written, set or not, which is what a programmer
    /// needs and what `icepack` expects; the comment says where the
    /// bitstream came from and carries no timestamp, so two runs of one
    /// design produce identical bytes.
    pub fn write_asc(&self) -> String {
        let mut out = String::new();
        out.push_str("# generated by reticle\n");
        out.push_str(".comment reticle bitstream; the architecture is synthetic, see fpga::arch\n");
        if !self.format.device.is_empty() {
            out.push_str(&format!(".device {}\n", self.format.device));
        }
        for (index, format) in self.format.tiles.iter().enumerate() {
            out.push_str(&format!(
                ".{} {} {}\n",
                format.keyword, format.tile.0, format.tile.1
            ));
            let cols = format.cols as usize;
            for row in 0..format.rows as usize {
                let start = row * cols;
                let line: String = self.bits[index][start..start + cols]
                    .iter()
                    .map(|b| if *b { '1' } else { '0' })
                    .collect();
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }

    /// Reads back what [`Bitstream::write_asc`] wrote.
    ///
    /// The layout is taken from the file itself, so a bitstream read this
    /// way needs no architecture. `#` comments, blank lines and
    /// `.comment` are skipped.
    ///
    /// # Errors
    ///
    /// [`BitstreamError::Asc`] with the line number, for a header that
    /// does not parse, a bit row that is not `0`s and `1`s, or rows of
    /// unequal length.
    pub fn parse_asc(text: &str) -> Result<Bitstream, BitstreamError> {
        let mut device = String::new();
        let mut tiles: Vec<TileFormat> = Vec::new();
        let mut bits: Vec<Vec<bool>> = Vec::new();
        let mut rows: Vec<Vec<bool>> = Vec::new();
        let mut current: Option<(String, u32, u32)> = None;

        let finish = |current: &mut Option<(String, u32, u32)>,
                      rows: &mut Vec<Vec<bool>>,
                      tiles: &mut Vec<TileFormat>,
                      bits: &mut Vec<Vec<bool>>| {
            if let Some((keyword, x, y)) = current.take() {
                let cols = rows.first().map_or(0, Vec::len);
                tiles.push(TileFormat {
                    tile: (x, y),
                    keyword,
                    rows: u32::try_from(rows.len()).unwrap_or(0),
                    cols: u32::try_from(cols).unwrap_or(0),
                });
                bits.push(rows.drain(..).flatten().collect());
            }
        };

        for (number, raw) in text.lines().enumerate() {
            let line = raw.trim();
            let number = number + 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('.') {
                let mut words = rest.split_whitespace();
                let keyword = words.next().unwrap_or("");
                if keyword == "comment" {
                    continue;
                }
                if keyword == "device" {
                    device = words.next().unwrap_or("").to_owned();
                    continue;
                }
                finish(&mut current, &mut rows, &mut tiles, &mut bits);
                let coords: Vec<&str> = words.collect();
                if coords.len() != 2 {
                    return Err(BitstreamError::Asc {
                        line: number,
                        message: format!("`.{keyword}` wants a column and a row"),
                    });
                }
                let x = coords[0].parse::<u32>().map_err(|_| BitstreamError::Asc {
                    line: number,
                    message: format!("`{}` is not a column", coords[0]),
                })?;
                let y = coords[1].parse::<u32>().map_err(|_| BitstreamError::Asc {
                    line: number,
                    message: format!("`{}` is not a row", coords[1]),
                })?;
                current = Some((keyword.to_owned(), x, y));
                continue;
            }
            if current.is_none() {
                return Err(BitstreamError::Asc {
                    line: number,
                    message: "bits before any tile header".to_owned(),
                });
            }
            let mut row = Vec::with_capacity(line.len());
            for c in line.chars() {
                match c {
                    '0' => row.push(false),
                    '1' => row.push(true),
                    other => {
                        return Err(BitstreamError::Asc {
                            line: number,
                            message: format!("`{other}` is not a bit"),
                        });
                    }
                }
            }
            if let Some(first) = rows.first()
                && first.len() != row.len()
            {
                return Err(BitstreamError::Asc {
                    line: number,
                    message: format!(
                        "this row has {} bits, the first had {}",
                        row.len(),
                        first.len()
                    ),
                });
            }
            rows.push(row);
        }
        finish(&mut current, &mut rows, &mut tiles, &mut bits);
        Ok(Bitstream {
            format: BitstreamFormat { device, tiles },
            bits,
        })
    }

    /// Writes Reticle's own binary container.
    ///
    /// **This is not an iCE40 configuration image.** It is a compact,
    /// self-describing dump of the same bits the `.asc` carries, so a
    /// bitstream can be stored, hashed and compared without reparsing
    /// text. Producing an image a configuration engine accepts is what
    /// `icepack` does, from the `.asc` and from the database this crate
    /// does not ship.
    ///
    /// The layout, little-endian throughout:
    ///
    /// | Bytes | Meaning |
    /// |---|---|
    /// | 4 | the magic `RTBS` |
    /// | 1 | format version, currently 1 |
    /// | 2 + n | the device word's length and bytes |
    /// | 4 | number of tiles |
    /// | | then, per tile: |
    /// | 2 + 2 | column and row |
    /// | 2 + n | the keyword's length and bytes |
    /// | 2 + 2 | bitmap rows and columns |
    /// | ceil(rows * cols / 8) | the bits, row-major, low bit of each byte first |
    pub fn write_bin(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RTBS");
        out.push(1);
        write_string(&mut out, &self.format.device);
        let count = u32::try_from(self.format.tiles.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&count.to_le_bytes());
        for (index, format) in self.format.tiles.iter().enumerate() {
            out.extend_from_slice(&narrow(format.tile.0).to_le_bytes());
            out.extend_from_slice(&narrow(format.tile.1).to_le_bytes());
            write_string(&mut out, &format.keyword);
            out.extend_from_slice(&narrow(format.rows).to_le_bytes());
            out.extend_from_slice(&narrow(format.cols).to_le_bytes());
            let mut byte = 0u8;
            let mut filled = 0u32;
            for bit in &self.bits[index] {
                if *bit {
                    byte |= 1 << filled;
                }
                filled += 1;
                if filled == 8 {
                    out.push(byte);
                    byte = 0;
                    filled = 0;
                }
            }
            if filled > 0 {
                out.push(byte);
            }
        }
        out
    }

    /// Reads back what [`Bitstream::write_bin`] wrote.
    ///
    /// # Errors
    ///
    /// [`BitstreamError::Binary`] for a wrong magic, an unknown version
    /// or a truncated file.
    pub fn read_bin(bytes: &[u8]) -> Result<Bitstream, BitstreamError> {
        let mut r = Reader { bytes, pos: 0 };
        if r.take(4)? != b"RTBS" {
            return Err(BitstreamError::Binary(
                "this is not a reticle bitstream: the magic is wrong".to_owned(),
            ));
        }
        let version = r.take(1)?[0];
        if version != 1 {
            return Err(BitstreamError::Binary(format!(
                "bitstream format version {version} is not one this build knows"
            )));
        }
        let device = r.string()?;
        let count = r.u32()?;
        let mut tiles = Vec::new();
        let mut bits = Vec::new();
        for _ in 0..count {
            let x = u32::from(r.u16()?);
            let y = u32::from(r.u16()?);
            let keyword = r.string()?;
            let rows = u32::from(r.u16()?);
            let cols = u32::from(r.u16()?);
            let total = rows as usize * cols as usize;
            let bytes = total.div_ceil(8);
            let data = r.take(bytes)?;
            let mut tile = Vec::with_capacity(total);
            for index in 0..total {
                tile.push((data[index / 8] >> (index % 8)) & 1 == 1);
            }
            tiles.push(TileFormat {
                tile: (x, y),
                keyword,
                rows,
                cols,
            });
            bits.push(tile);
        }
        Ok(Bitstream {
            format: BitstreamFormat { device, tiles },
            bits,
        })
    }
}

/// Narrows a grid coordinate or bitmap size to the two bytes the binary
/// container gives it.
///
/// Every architecture this crate can hold is far below 65535 tiles on a
/// side and 65535 bits per bitmap row; a database that is not gets its
/// coordinates saturated rather than silently wrapped, and the reader
/// then reports a layout that does not match.
fn narrow(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn write_string(out: &mut Vec<u8>, text: &str) {
    let length = u16::try_from(text.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&text.as_bytes()[..length as usize]);
}

/// A bounds-checked cursor over the binary container.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], BitstreamError> {
        if self.pos + n > self.bytes.len() {
            return Err(BitstreamError::Binary(format!(
                "the bitstream ends after {} bytes, in the middle of a field",
                self.bytes.len()
            )));
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16, BitstreamError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, BitstreamError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn string(&mut self) -> Result<String, BitstreamError> {
        let length = usize::from(self.u16()?);
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| BitstreamError::Binary("a name is not valid UTF-8".to_owned()))
    }
}

/// Turns a placed and routed design into configuration bits.
///
/// # Errors
///
/// [`BitstreamError::NoSuchModule`], and the two database errors of
/// [`Bitstream::set`] when the architecture contradicts itself.
pub fn generate(
    design: &Design,
    module: ModuleId,
    arch: &Arch,
    graph: &RoutingGraph,
    netlist: &Netlist,
    placement: &Placement,
    routing: &Routing,
) -> Result<Bitstream, BitstreamError> {
    let Some(m) = design.modules.get(module) else {
        return Err(BitstreamError::NoSuchModule);
    };
    let mut bitstream = Bitstream::empty(BitstreamFormat::from_arch(arch));

    for (index, instance) in netlist.instances.iter().enumerate() {
        let Some(site) = placement.site_of(index) else {
            continue;
        };
        let site = &graph.sites[site];
        let Some(bel) = arch.tile_types[site.tile_type].bel(&site.bel) else {
            continue;
        };
        let params = m.cells.get(instance.cell).map(|cell| &cell.params);
        for entry in &bel.config {
            match entry {
                ConfigEntry::Cell { primitive, bits } if *primitive == instance.primitive => {
                    for bit in bits {
                        bitstream.set(site.tile, *bit)?;
                    }
                }
                ConfigEntry::Cell { .. } => {}
                ConfigEntry::Param { name, index, at } => {
                    let value = params.and_then(|p| p.get(name));
                    if param_bit(value, *index) {
                        bitstream.set(site.tile, *at)?;
                    }
                }
                ConfigEntry::ParamZero { name, index, at } => {
                    let value = params.and_then(|p| p.get(name));
                    if !param_bit(value, *index) {
                        bitstream.set(site.tile, *at)?;
                    }
                }
            }
        }
    }

    for route in routing.routes() {
        for id in &route.pips {
            let tile = graph.pip(*id).tile;
            for bit in graph.pip_bits(*id) {
                bitstream.set(tile, *bit)?;
            }
        }
    }
    Ok(bitstream)
}

/// Bit `index` of a parameter's value, false when the cell does not carry
/// it.
///
/// A sized constant is read bit by bit, an integer is read as a
/// two's-complement 64-bit word, and a string parameter has no bits: a
/// family that configures something with a keyword has to say so with a
/// [`ConfigEntry::Cell`] entry instead.
fn param_bit(value: Option<&AttrValue>, index: u32) -> bool {
    match value {
        Some(AttrValue::Const(c)) => c
            .get(index)
            .is_some_and(|bit| bit == crate::logic::Bit::One),
        Some(AttrValue::Int(v)) => index < 64 && (v >> index) & 1 == 1,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::arch::{Arch, BelDecl, TileType, WireDecl, WireRef};

    fn format() -> BitstreamFormat {
        BitstreamFormat {
            device: "1k".to_owned(),
            tiles: vec![
                TileFormat {
                    tile: (0, 0),
                    keyword: "io_tile".to_owned(),
                    rows: 2,
                    cols: 3,
                },
                TileFormat {
                    tile: (1, 0),
                    keyword: "logic_tile".to_owned(),
                    rows: 3,
                    cols: 5,
                },
            ],
        }
    }

    #[test]
    fn bits_go_in_and_come_back_out() {
        let mut b = Bitstream::empty(format());
        assert_eq!(b.ones(), 0);
        assert_eq!(b.format.bits(), 6 + 15);
        b.set((0, 0), ConfigBit::new(1, 2)).unwrap();
        b.set((1, 0), ConfigBit::new(0, 0)).unwrap();
        b.set((1, 0), ConfigBit::new(2, 4)).unwrap();
        assert_eq!(b.ones(), 3);
        assert_eq!(b.get((0, 0), ConfigBit::new(1, 2)), Some(true));
        assert_eq!(b.get((0, 0), ConfigBit::new(0, 0)), Some(false));
        assert_eq!(b.get((9, 9), ConfigBit::new(0, 0)), None);
        assert_eq!(b.get((0, 0), ConfigBit::new(9, 9)), None);
        assert_eq!(
            b.set((5, 5), ConfigBit::new(0, 0)).unwrap_err(),
            BitstreamError::NoTile { x: 5, y: 5 }
        );
        assert_eq!(
            b.set((0, 0), ConfigBit::new(7, 0)).unwrap_err(),
            BitstreamError::OutOfRange {
                tile: (0, 0),
                bit: ConfigBit::new(7, 0),
                size: (2, 3),
            }
        );
        assert!(
            b.set((5, 5), ConfigBit::new(0, 0))
                .unwrap_err()
                .to_string()
                .contains("no tile at (5, 5)")
        );
        assert!(
            b.set((0, 0), ConfigBit::new(7, 0))
                .unwrap_err()
                .to_string()
                .contains("outside the 2 x 3 bitmap")
        );
    }

    #[test]
    fn the_asc_round_trips() {
        let mut b = Bitstream::empty(format());
        b.set((0, 0), ConfigBit::new(1, 2)).unwrap();
        b.set((1, 0), ConfigBit::new(2, 4)).unwrap();
        let text = b.write_asc();
        assert!(text.contains(".device 1k\n"), "{text}");
        assert!(text.contains(".io_tile 0 0\n000\n001\n"), "{text}");
        assert!(
            text.contains(".logic_tile 1 0\n00000\n00000\n00001\n"),
            "{text}"
        );
        let again = Bitstream::parse_asc(&text).unwrap();
        assert_eq!(again, b);
        assert_eq!(again.write_asc(), text);
    }

    #[test]
    fn the_binary_round_trips() {
        let mut b = Bitstream::empty(format());
        b.set((0, 0), ConfigBit::new(0, 1)).unwrap();
        b.set((1, 0), ConfigBit::new(1, 1)).unwrap();
        let bytes = b.write_bin();
        assert_eq!(&bytes[..4], b"RTBS");
        let again = Bitstream::read_bin(&bytes).unwrap();
        assert_eq!(again, b);
        assert_eq!(again.write_bin(), bytes);
    }

    #[test]
    fn a_broken_file_is_reported_with_its_line() {
        let err = Bitstream::parse_asc("0101\n").unwrap_err();
        assert_eq!(
            err,
            BitstreamError::Asc {
                line: 1,
                message: "bits before any tile header".to_owned(),
            }
        );
        assert!(err.to_string().starts_with("line 1: "));
        let err = Bitstream::parse_asc(".logic_tile 1\n").unwrap_err();
        assert!(err.to_string().contains("wants a column and a row"));
        let err = Bitstream::parse_asc(".logic_tile a 1\n").unwrap_err();
        assert!(err.to_string().contains("`a` is not a column"));
        let err = Bitstream::parse_asc(".logic_tile 1 b\n").unwrap_err();
        assert!(err.to_string().contains("`b` is not a row"));
        let err = Bitstream::parse_asc(".logic_tile 1 1\n012\n").unwrap_err();
        assert!(err.to_string().contains("`2` is not a bit"));
        let err = Bitstream::parse_asc(".logic_tile 1 1\n01\n011\n").unwrap_err();
        assert!(err.to_string().contains("this row has 3 bits"));

        assert!(
            Bitstream::read_bin(b"nope")
                .unwrap_err()
                .to_string()
                .contains("the magic is wrong")
        );
        assert!(
            Bitstream::read_bin(b"RTBS\x09")
                .unwrap_err()
                .to_string()
                .contains("version 9")
        );
        assert!(
            Bitstream::read_bin(b"RTBS\x01\xff")
                .unwrap_err()
                .to_string()
                .contains("ends after")
        );
    }

    #[test]
    fn a_comment_and_blank_lines_are_skipped() {
        let text = "# a comment\n\n.comment whatever\n.device 8k\n.logic_tile 0 0\n11\n";
        let b = Bitstream::parse_asc(text).unwrap();
        assert_eq!(b.format.device, "8k");
        assert_eq!(b.ones(), 2);
        assert_eq!(b.to_summary().lines().next(), Some("device 8k"));
        assert!(b.to_summary().contains("logic_tile 0 0: 0.0,0.1"));
    }

    /// The bits come from the architecture and from the cell's own
    /// parameters, and nothing else.
    #[test]
    fn generation_reads_the_architecture_and_the_parameters() {
        use crate::fpga::arch::ConfigEntry;
        use crate::fpga::{Netlist, PlaceOptions, place};
        use crate::ir::builder::ModuleBuilder;
        use crate::ir::{CellKind, Id, Name, Type};
        use crate::source::{SourceMap, Span};

        // One tile, one LUT bel with a two-bit LUT_INIT and a mode bit.
        let mut arch = Arch::new("t", "ice40", 1, 1);
        let mut tile = TileType::new("logic", "logic_tile", 2, 4);
        tile.wires.push(WireDecl {
            name: "i".to_owned(),
            dx: 0,
            dy: 0,
        });
        tile.wires.push(WireDecl {
            name: "o".to_owned(),
            dx: 0,
            dy: 0,
        });
        let mut bel = BelDecl::new("lut", "lut");
        bel.pins.push(("i0".to_owned(), WireRef::local("i")));
        bel.pins.push(("o".to_owned(), WireRef::local("o")));
        bel.config.push(ConfigEntry::Cell {
            primitive: "SB_LUT4".to_owned(),
            bits: vec![ConfigBit::new(0, 0)],
        });
        for index in 0..2 {
            bel.config.push(ConfigEntry::Param {
                name: "LUT_INIT".to_owned(),
                index,
                at: ConfigBit::new(1, index),
            });
        }
        tile.bels.push(bel);
        arch.tile_types.push(tile);
        arch.set_tile(0, 0, 0);
        let graph = arch.build_graph();

        let mut sources = SourceMap::new();
        let file = sources.add("t.v", "").unwrap();
        let span = Span::new(file, 0, 0);
        let mut b = ModuleBuilder::new("t", span);
        let a = b.input("a", Type::bit());
        let y = b.output("y", Type::bit());
        let a_e = b.net(a);
        let cell = b.cell(
            "l",
            CellKind::Blackbox(Name::new("SB_LUT4")),
            vec![(Name::new("I0"), a_e)],
            vec![(Name::new("O"), y)],
        );
        let mut design = Design::new();
        {
            let module = b.finish();
            let top = design.add_module(module);
            design.top = Some(top);
            // `LUT_INIT = 2` sets bit 1 and clears bit 0.
            design.modules[top].cells[cell]
                .params
                .set("LUT_INIT", AttrValue::Int(2));
            let device = crate::fpga::target("ice40-hx1k-tq144").unwrap();
            let netlist = Netlist::build(&design, top, device, &graph).unwrap();
            let (placement, _) = place(
                &netlist,
                &arch,
                &graph,
                &crate::fpga::Constraints::new(),
                &PlaceOptions::default(),
            )
            .unwrap();
            let bitstream = generate(
                &design,
                top,
                &arch,
                &graph,
                &netlist,
                &placement,
                &Routing::new(netlist.signals.len()),
            )
            .unwrap();
            // The mode bit and LUT_INIT bit 1, and nothing else.
            assert_eq!(bitstream.ones(), 2);
            assert_eq!(bitstream.get((0, 0), ConfigBit::new(0, 0)), Some(true));
            assert_eq!(bitstream.get((0, 0), ConfigBit::new(1, 0)), Some(false));
            assert_eq!(bitstream.get((0, 0), ConfigBit::new(1, 1)), Some(true));

            let missing = crate::ir::ModuleId::from_index(42);
            assert_eq!(
                generate(
                    &design,
                    missing,
                    &arch,
                    &graph,
                    &netlist,
                    &placement,
                    &Routing::new(netlist.signals.len())
                )
                .unwrap_err(),
                BitstreamError::NoSuchModule
            );
        }
    }

    #[test]
    fn parameter_bits_read_every_value_kind() {
        use crate::ir::Const;
        assert!(param_bit(Some(&AttrValue::Int(0b100)), 2));
        assert!(!param_bit(Some(&AttrValue::Int(0b100)), 1));
        assert!(!param_bit(Some(&AttrValue::Int(1)), 200));
        let c = AttrValue::Const(Const::from_u64(0b1010, 4));
        assert!(param_bit(Some(&c), 1));
        assert!(!param_bit(Some(&c), 0));
        assert!(!param_bit(Some(&c), 9));
        assert!(!param_bit(Some(&AttrValue::String("x".to_owned())), 0));
        assert!(!param_bit(None, 0));
    }
}
