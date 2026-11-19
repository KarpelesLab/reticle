//! An iCE40 configuration image: the `.asc` text and the `.bin` stream.
//!
//! An [`Image`] is every tile's bitmap, the bits that live outside every
//! tile (`.extra_bit`), and the block RAM contents (`.ram_data`). It is
//! written as IceStorm's `.asc` and packed into the binary stream the part
//! loads, the job Project IceStorm's `icepack` does; and it is read back
//! from either, which is what lets a bitstream be decoded and checked.
//!
//! # Where the layout comes from
//!
//! Every number in [`Layout`] and every step of [`Image::to_bin`] and
//! [`Image::from_bin`] is `icepack.cc`'s (`CramIndexConverter`,
//! `BramIndexConverter`, `FpgaConfig::write_bits` and `read_bits`), read
//! from the source and ported, not re-derived. It was then CHECKED: an
//! `.asc` and the `.bin` icepack made of it unpack and pack to each other
//! byte for byte (`tests/fpga_icestorm.rs`, given a directory of such
//! pairs in `RETICLE_ICE40_REF`).
//!
//! The configuration memory is four banks, one per quadrant of the die.
//! Each tile's bitmap lands in its bank at an offset given by the column
//! widths to its side and sixteen rows per tile row, mirrored in the right
//! and top halves, and with a fixed permutation of rows and columns for
//! the top and bottom IO tiles. On an UltraPlus 5k the top half is a third
//! of the die rather than half, and its banks are correspondingly
//! shorter, `height / 2 + 8` rows.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A family member's configuration-memory geometry, as `icepack` has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The device word: `384`, `1k`, `5k`, `8k`, `u4k`, `lm4k`.
    pub device: String,
    /// Logic columns, excluding the two edges (`chip_width`).
    pub chip_width: u32,
    /// Logic rows, excluding the two edges (`chip_height`).
    pub chip_height: u32,
    /// The width of each column of one half, edge first (`chip_cols`).
    pub cols: Vec<u32>,
    /// The width of one bank of configuration memory, in bits.
    pub cram_width: u32,
    /// Its height.
    pub cram_height: u32,
    /// The width of one bank of block RAM contents.
    pub bram_width: u32,
    /// Its height.
    pub bram_height: u32,
}

impl Layout {
    /// The layout of a device `icepack` knows, or `None`.
    #[must_use]
    pub fn for_device(device: &str) -> Option<Layout> {
        let (w, h, cols, cw, ch, bw, bh): (u32, u32, &[u32], u32, u32, u32, u32) = match device {
            "384" => (6, 8, &[18, 54, 54, 54, 54], 182, 80, 0, 0),
            "1k" => (12, 16, &[18, 54, 54, 42, 54, 54, 54], 332, 144, 64, 256),
            "5k" => (
                24,
                30,
                &[54, 54, 54, 54, 54, 54, 42, 54, 54, 54, 54, 54, 54],
                692,
                336,
                160,
                256,
            ),
            "u4k" => (
                24,
                20,
                &[54, 54, 54, 54, 54, 54, 42, 54, 54, 54, 54, 54, 54],
                692,
                176,
                80,
                256,
            ),
            "lm4k" => (
                24,
                20,
                &[18, 54, 54, 54, 54, 54, 42, 54, 54, 54, 54, 54, 54],
                656,
                176,
                80,
                256,
            ),
            "8k" => (
                32,
                32,
                &[
                    18, 54, 54, 54, 54, 54, 54, 54, 42, 54, 54, 54, 54, 54, 54, 54, 54,
                ],
                872,
                272,
                128,
                256,
            ),
            _ => return None,
        };
        Some(Layout {
            device: device.to_owned(),
            chip_width: w,
            chip_height: h,
            cols: cols.to_vec(),
            cram_width: cw,
            cram_height: ch,
            bram_width: bw,
            bram_height: bh,
        })
    }

    fn is_5k(&self) -> bool {
        self.device == "5k"
    }

    /// The height of configuration bank `bank`: on a 5k the top banks are
    /// shorter.
    #[must_use]
    pub fn cram_bank_height(&self, bank: usize) -> u32 {
        if self.is_5k() && bank % 2 == 1 {
            self.cram_height / 2 + 8
        } else {
            self.cram_height
        }
    }

    fn bram_bank_width(&self, bank: usize) -> u32 {
        if self.is_5k() && bank % 2 == 1 {
            self.bram_width / 2
        } else {
            self.bram_width
        }
    }

    /// The tile kind `icepack` expects at `(x, y)`: `corner`, `io`,
    /// `logic`, `ramb`, `ramt`, `dsp0`..`dsp3` or `ipcon`.
    #[must_use]
    pub fn tile_type(&self, x: u32, y: u32) -> &'static str {
        let (w, h) = (self.chip_width, self.chip_height);
        let edge_x = x == 0 || x == w + 1;
        let edge_y = y == 0 || y == h + 1;
        if edge_x && edge_y {
            return "corner";
        }
        if (self.device == "5k" || self.device == "u4k") && edge_x {
            let bases: &[u32] = if self.device == "5k" {
                &[5, 10, 15, 23]
            } else {
                &[5, 13]
            };
            let dsp = bases
                .iter()
                .find(|&&b| (b..b + 4).contains(&y))
                .map(|b| y - b);
            return match dsp {
                Some(0) => "dsp0",
                Some(1) => "dsp1",
                Some(2) => "dsp2",
                Some(3) => "dsp3",
                _ => "ipcon",
            };
        }
        if edge_x || edge_y {
            return "io";
        }
        let ram_cols: &[u32] = match self.device.as_str() {
            "384" => &[],
            "1k" => &[3, 10],
            "8k" => &[8, 25],
            _ => &[6, 19],
        };
        if ram_cols.contains(&x) {
            return if y % 2 == 1 { "ramb" } else { "ramt" };
        }
        "logic"
    }

    /// The bitmap width of a tile kind.
    #[must_use]
    pub fn tile_width(kind: &str) -> u32 {
        match kind {
            "corner" => 0,
            "io" => 18,
            "ramb" | "ramt" => 42,
            _ => 54,
        }
    }

    /// Where bit `(row, col)` of the tile at `(x, y)` lands: `(bank,
    /// cram_x, cram_y)`. `icepack`'s `CramIndexConverter`.
    #[must_use]
    pub fn cram_index(&self, x: u32, y: u32, row: u32, col: u32) -> (usize, u32, u32) {
        const PERM_X: [u32; 18] = [
            23, 25, 26, 27, 16, 17, 18, 19, 20, 14, 32, 33, 34, 35, 36, 37, 4, 5,
        ];
        const PERM_Y: [u32; 16] = [0, 1, 3, 2, 4, 5, 7, 6, 8, 9, 11, 10, 12, 13, 15, 14];
        let (w, h) = (self.chip_width, self.chip_height);
        let kind = self.tile_type(x, y);
        let left_right_io = x == 0 || x == w + 1;
        let right_half = x > w / 2;
        let top_half = if self.is_5k() {
            y > h * 2 / 3
        } else {
            y > h / 2
        };
        let bank = usize::from(top_half) | usize::from(right_half) << 1;
        let bank_tx = if right_half { w + 1 - x } else { x };
        let bank_ty = if top_half { h + 1 - y } else { y };
        let xoff: u32 = self.cols[..bank_tx as usize].iter().sum();
        let yoff = 16 * bank_ty;
        let column_width = self.cols[bank_tx as usize];
        if kind == "io" {
            if left_right_io {
                let cx = xoff + column_width - 1 - col;
                let cy = if top_half {
                    yoff + 15 - row
                } else {
                    yoff + row
                };
                (bank, cx, cy)
            } else {
                let cy = yoff + 15 - PERM_Y[row as usize];
                let cx = if right_half {
                    xoff + column_width - 1 - PERM_X[col as usize]
                } else {
                    xoff + PERM_X[col as usize]
                };
                (bank, cx, cy)
            }
        } else {
            let cx = if right_half {
                xoff + column_width - 1 - col
            } else {
                xoff + col
            };
            let cy = if top_half {
                yoff + 15 - row
            } else {
                yoff + row
            };
            (bank, cx, cy)
        }
    }

    /// Where bit `bit` of word `word` of the block RAM whose bottom tile is
    /// `(x, y)` lands: `(bank, bram_x, bram_y)`. `icepack`'s
    /// `BramIndexConverter`, with `bit_y` the `.ram_data` line (0..16) and
    /// `bit_x` the bit within it (0..256).
    #[must_use]
    pub fn bram_index(&self, x: u32, y: u32, bit_x: u32, bit_y: u32) -> (usize, u32, u32) {
        let (w, h) = (self.chip_width, self.chip_height);
        let right_half = x > w / 2;
        let mut top_half = y > h / 2;
        if self.is_5k() {
            top_half = y > 2 * h / 3;
        }
        let mut bank = 0usize;
        let mut y_offset = y as i64 - 1;
        if self.is_5k() {
            if top_half {
                bank |= 1;
                y_offset = i64::from(y) - i64::from(2 * h / 3);
            }
        } else if top_half {
            bank |= 1;
            y_offset = i64::from(y) - i64::from(h / 2);
        }
        if right_half {
            bank |= 2;
        }
        let bank_off = 16 * u32::try_from(y_offset.max(0) / 2).unwrap_or(0);
        let index = 256 * bit_y + (16 * (bit_x / 16) + 15 - bit_x % 16);
        (bank, bank_off + index % 16, index / 16)
    }
}

/// A whole configuration image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// The geometry.
    pub layout: Layout,
    /// Each tile's set bits, as `(row, col)`.
    pub tiles: BTreeMap<(u32, u32), BTreeSet<(u32, u32)>>,
    /// Bits outside every tile: `(bank, x, y)`.
    pub extra_bits: BTreeSet<(usize, u32, u32)>,
    /// Block RAM contents by the RAM's bottom tile: sixteen lines of 256
    /// bits, bit `i` of line `j` at `[j][i]`.
    pub ram_data: BTreeMap<(u32, u32), Vec<[bool; 256]>>,
    /// Whether the warm-boot command is enabled, which `icepack` defaults
    /// to on for an `.asc`.
    pub warmboot: bool,
}

/// Why an image could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageError(pub String);

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImageError {}

fn crc16(crc: &mut u16, byte: u8) {
    for i in (0..8).rev() {
        let feedback = ((*crc >> 15) ^ u16::from(byte >> i & 1)) != 0;
        *crc <<= 1;
        if feedback {
            *crc ^= 0x1021;
        }
    }
}

struct Writer {
    out: Vec<u8>,
    crc: u16,
}

impl Writer {
    fn byte(&mut self, b: u8) {
        self.out.push(b);
        crc16(&mut self.crc, b);
    }
    fn bytes(&mut self, bs: &[u8]) {
        for &b in bs {
            self.byte(b);
        }
    }
    fn bits(&mut self, bits: &[bool]) {
        for chunk in bits.chunks(8) {
            let mut b = 0u8;
            for (j, &bit) in chunk.iter().enumerate() {
                if bit {
                    b |= 0x80 >> j;
                }
            }
            self.byte(b);
        }
    }
}

impl Image {
    /// An empty image for `device`.
    #[must_use]
    pub fn new(layout: Layout) -> Image {
        Image {
            layout,
            tiles: BTreeMap::new(),
            extra_bits: BTreeSet::new(),
            ram_data: BTreeMap::new(),
            warmboot: true,
        }
    }

    /// Sets bit `(row, col)` of the tile at `(x, y)`.
    pub fn set(&mut self, x: u32, y: u32, row: u32, col: u32) {
        self.tiles.entry((x, y)).or_default().insert((row, col));
    }

    /// Whether bit `(row, col)` of the tile at `(x, y)` is set.
    #[must_use]
    pub fn get(&self, x: u32, y: u32, row: u32, col: u32) -> bool {
        self.tiles
            .get(&(x, y))
            .is_some_and(|t| t.contains(&(row, col)))
    }

    /// How many bits are set, extra bits included.
    #[must_use]
    pub fn ones(&self) -> usize {
        self.tiles.values().map(BTreeSet::len).sum::<usize>() + self.extra_bits.len()
    }

    /// The `.asc` text, in the form `icepack` reads and nextpnr writes.
    #[must_use]
    pub fn to_asc(&self) -> String {
        let l = &self.layout;
        let mut out = String::new();
        out.push_str(".comment reticle\n");
        let _ = writeln!(out, ".device {}", l.device);
        for y in 0..=l.chip_height + 1 {
            for x in 0..=l.chip_width + 1 {
                let kind = l.tile_type(x, y);
                if kind == "corner" {
                    continue;
                }
                let cols = Layout::tile_width(kind);
                let _ = writeln!(out, ".{kind}_tile {x} {y}");
                let set = self.tiles.get(&(x, y));
                for row in 0..16 {
                    for col in 0..cols {
                        let on = set.is_some_and(|s| s.contains(&(row, col)));
                        out.push(if on { '1' } else { '0' });
                    }
                    out.push('\n');
                }
                out.push('\n');
            }
        }
        for (&(x, y), lines) in &self.ram_data {
            let _ = writeln!(out, ".ram_data {x} {y}");
            for line in lines {
                for nibble in (0..64).rev() {
                    let mut v = 0u8;
                    for i in 0..4 {
                        if line[nibble * 4 + i] {
                            v |= 1 << i;
                        }
                    }
                    out.push(char::from_digit(u32::from(v), 16).unwrap_or('0'));
                }
                out.push('\n');
            }
            out.push('\n');
        }
        for &(bank, x, y) in &self.extra_bits {
            let _ = writeln!(out, ".extra_bit {bank} {x} {y}");
        }
        out
    }

    /// Reads an `.asc`.
    ///
    /// # Errors
    ///
    /// [`ImageError`] for a device `icepack` does not know, a tile header
    /// that names the wrong kind for its position, or a malformed line.
    pub fn from_asc(text: &str) -> Result<Image, ImageError> {
        let mut image: Option<Image> = None;
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            let words: Vec<&str> = line.split_whitespace().collect();
            let Some(&command) = words.first() else {
                continue;
            };
            let need = |image: &mut Option<Image>| -> Result<(), ImageError> {
                if image.is_none() {
                    return Err(ImageError(format!("`{command}` before `.device`")));
                }
                Ok(())
            };
            let num = |i: usize| -> Result<u32, ImageError> {
                words
                    .get(i)
                    .and_then(|w| w.parse().ok())
                    .ok_or_else(|| ImageError(format!("a bad number in `{line}`")))
            };
            match command {
                ".comment" => {
                    while lines.peek().is_some_and(|l| !l.starts_with('.')) {
                        lines.next();
                    }
                }
                ".device" => {
                    let device = words.get(1).copied().unwrap_or("");
                    let layout = Layout::for_device(device)
                        .ok_or_else(|| ImageError(format!("unknown device `{device}`")))?;
                    image = Some(Image::new(layout));
                }
                ".warmboot" => {
                    need(&mut image)?;
                    if let Some(img) = image.as_mut() {
                        img.warmboot = words.get(1) != Some(&"disabled");
                    }
                }
                ".sym" => {}
                ".extra_bit" => {
                    need(&mut image)?;
                    let (bank, x, y) = (num(1)? as usize, num(2)?, num(3)?);
                    if let Some(img) = image.as_mut() {
                        img.extra_bits.insert((bank, x, y));
                    }
                }
                ".ram_data" => {
                    need(&mut image)?;
                    let (x, y) = (num(1)?, num(2)?);
                    let mut data = Vec::new();
                    for _ in 0..16 {
                        let Some(row) = lines.next() else { break };
                        let mut bits = [false; 256];
                        for (ch_idx, ch) in row.trim().chars().enumerate().take(64) {
                            let v = ch.to_digit(16).ok_or_else(|| {
                                ImageError(format!("not a hex digit `{ch}` in `.ram_data`"))
                            })?;
                            let base = 256 - 4 - 4 * ch_idx;
                            for (i, slot) in bits[base..base + 4].iter_mut().enumerate() {
                                *slot = v >> i & 1 != 0;
                            }
                        }
                        data.push(bits);
                    }
                    if let Some(img) = image.as_mut() {
                        img.ram_data.insert((x, y), data);
                    }
                }
                tile if tile.ends_with("_tile") => {
                    need(&mut image)?;
                    let (x, y) = (num(1)?, num(2)?);
                    let Some(img) = image.as_mut() else { continue };
                    let kind = img.layout.tile_type(x, y);
                    if format!(".{kind}_tile") != tile {
                        return Err(ImageError(format!(
                            "`{line}` names the wrong kind: icepack has a {kind} tile there"
                        )));
                    }
                    let cols = Layout::tile_width(kind);
                    for row in 0..16 {
                        let Some(bits) = lines.next() else { break };
                        for (col, ch) in bits.trim().chars().enumerate().take(cols as usize) {
                            if ch == '1' {
                                img.set(x, y, row, u32::try_from(col).unwrap_or(0));
                            }
                        }
                    }
                }
                other => {
                    return Err(ImageError(format!("unknown `.asc` line `{other}`")));
                }
            }
        }
        image.ok_or_else(|| ImageError("no `.device` line".into()))
    }

    /// The four banks of configuration memory, `[bank][y][x]`.
    fn cram(&self) -> Vec<Vec<Vec<bool>>> {
        let l = &self.layout;
        let mut cram: Vec<Vec<Vec<bool>>> = (0..4)
            .map(|b| vec![vec![false; l.cram_width as usize]; l.cram_bank_height(b) as usize])
            .collect();
        for (&(x, y), set) in &self.tiles {
            for &(row, col) in set {
                let (bank, cx, cy) = l.cram_index(x, y, row, col);
                if let Some(slot) = cram[bank]
                    .get_mut(cy as usize)
                    .and_then(|r| r.get_mut(cx as usize))
                {
                    *slot = true;
                }
            }
        }
        for &(bank, x, y) in &self.extra_bits {
            if let Some(slot) = cram
                .get_mut(bank)
                .and_then(|b| b.get_mut(y as usize))
                .and_then(|r| r.get_mut(x as usize))
            {
                *slot = true;
            }
        }
        cram
    }

    /// The block RAM contents' four banks, `[bank][y][x]`.
    fn bram(&self) -> Vec<Vec<Vec<bool>>> {
        let l = &self.layout;
        let mut bram: Vec<Vec<Vec<bool>>> = (0..4)
            .map(|b| vec![vec![false; l.bram_bank_width(b) as usize]; l.bram_height as usize])
            .collect();
        for (&(x, y), lines) in &self.ram_data {
            for (bit_y, line) in lines.iter().enumerate() {
                for (bit_x, &on) in line.iter().enumerate() {
                    if on {
                        let (bank, bx, by) = l.bram_index(
                            x,
                            y,
                            u32::try_from(bit_x).unwrap_or(0),
                            u32::try_from(bit_y).unwrap_or(0),
                        );
                        if let Some(slot) = bram[bank]
                            .get_mut(by as usize)
                            .and_then(|r| r.get_mut(bx as usize))
                        {
                            *slot = true;
                        }
                    }
                }
            }
        }
        bram
    }

    /// Packs the image into the stream the part loads, as `icepack` does:
    /// the `FF 00 ... 00 FF` comment, the synchronisation word, frequency
    /// range low, a CRC reset, the warm-boot byte, each configuration bank,
    /// each block RAM bank in 128-row chunks, the CRC, and wake-up.
    #[must_use]
    pub fn to_bin(&self) -> Vec<u8> {
        let l = &self.layout;
        let mut w = Writer {
            out: Vec::new(),
            crc: 0,
        };
        // The comment block icepack writes for a `.comment` line with no
        // text under it, which is what `to_asc` writes.
        w.out.extend_from_slice(&[0xFF, 0x00, 0x00, 0xFF]);
        w.crc = 0;
        w.bytes(&[0x7E, 0xAA, 0x99, 0x7E]);
        w.bytes(&[0x51, 0x00]);
        w.bytes(&[0x01, 0x05]);
        w.crc = 0xFFFF;
        w.bytes(&[0x92, 0x00, if self.warmboot { 0x20 } else { 0x00 }]);
        let [wh, wl] = u16::try_from(l.cram_width - 1).unwrap_or(0).to_be_bytes();
        w.bytes(&[0x62, wh, wl]);
        if !l.is_5k() {
            let [hh, hl] = u16::try_from(l.cram_height).unwrap_or(0).to_be_bytes();
            w.bytes(&[0x72, hh, hl]);
        }
        w.bytes(&[0x82, 0x00, 0x00]);
        let cram = self.cram();
        for (bank, rows) in cram.iter().enumerate() {
            if l.is_5k() {
                let [hh, hl] = u16::try_from(l.cram_bank_height(bank))
                    .unwrap_or(0)
                    .to_be_bytes();
                w.bytes(&[0x72, hh, hl]);
            }
            w.bytes(&[0x11, u8::try_from(bank).unwrap_or(0)]);
            w.bytes(&[0x01, 0x01]);
            let flat: Vec<bool> = rows.iter().flatten().copied().collect();
            w.bits(&flat);
            w.bytes(&[0x00, 0x00]);
        }
        if l.bram_width > 0 && l.bram_height > 0 {
            const CHUNK: u32 = 128;
            if !l.is_5k() {
                let [wh, wl] = u16::try_from(l.bram_width - 1).unwrap_or(0).to_be_bytes();
                w.bytes(&[0x62, wh, wl]);
            }
            w.bytes(&[0x72, 0x00, 0x80]);
            let bram = self.bram();
            for (bank, rows) in bram.iter().enumerate() {
                w.bytes(&[0x11, u8::try_from(bank).unwrap_or(0)]);
                let mut offset = 0;
                while offset < l.bram_height {
                    let [oh, ol] = u16::try_from(offset).unwrap_or(0).to_be_bytes();
                    w.bytes(&[0x82, oh, ol]);
                    if l.is_5k() {
                        let width = l.bram_bank_width(bank);
                        let [wh, wl] = u16::try_from(width - 1).unwrap_or(0).to_be_bytes();
                        w.bytes(&[0x62, wh, wl]);
                    }
                    w.bytes(&[0x01, 0x03]);
                    let flat: Vec<bool> = rows[offset as usize..(offset + CHUNK) as usize]
                        .iter()
                        .flatten()
                        .copied()
                        .collect();
                    w.bits(&flat);
                    w.bytes(&[0x00, 0x00]);
                    offset += CHUNK;
                }
            }
        }
        // The command byte goes into the CRC before the CRC is taken, as
        // icepack has it, so the check over command and value comes out zero.
        w.byte(0x22);
        let [ch, cl] = w.crc.to_be_bytes();
        w.bytes(&[ch, cl]);
        w.bytes(&[0x01, 0x06]);
        w.byte(0x00);
        w.out
    }

    /// Unpacks a stream into an image, as `iceunpack` does, checking its
    /// CRC; bits that land outside every tile are kept as extra bits.
    ///
    /// # Errors
    ///
    /// [`ImageError`] for a stream with no synchronisation word, a command
    /// it does not know, a failed CRC, or a geometry no device has.
    pub fn from_bin(bytes: &[u8]) -> Result<Image, ImageError> {
        let start = bytes
            .windows(4)
            .position(|w| w == [0x7E, 0xAA, 0x99, 0x7E])
            .ok_or_else(|| ImageError("no synchronisation word".into()))?;
        let mut pos = start + 4;
        let mut crc: u16 = 0;
        for &b in &bytes[start..pos] {
            crc16(&mut crc, b);
        }
        let next = |pos: &mut usize, crc: &mut u16| -> Result<u8, ImageError> {
            let b = *bytes
                .get(*pos)
                .ok_or_else(|| ImageError("the stream ends early".into()))?;
            *pos += 1;
            crc16(crc, b);
            Ok(b)
        };
        let (mut bank, mut width, mut height, mut offset) = (0usize, 0u32, 0u32, 0u32);
        let mut warmboot = false;
        let mut cram: Vec<BTreeMap<(u32, u32), ()>> = vec![BTreeMap::new(); 4];
        let mut bram: Vec<BTreeMap<(u32, u32), ()>> = vec![BTreeMap::new(); 4];
        let (mut cram_w, mut cram_h) = (0u32, 0u32);
        loop {
            let command = next(&mut pos, &mut crc)?;
            let mut payload: u32 = 0;
            for _ in 0..(command & 0x0F) {
                payload = payload << 8 | u32::from(next(&mut pos, &mut crc)?);
            }
            match command & 0xF0 {
                0x00 => match payload {
                    0x01 | 0x03 => {
                        let into = if payload == 1 { &mut cram } else { &mut bram };
                        if payload == 1 {
                            cram_w = cram_w.max(width);
                            cram_h = cram_h.max(offset + height);
                        }
                        let count = (width * height / 8) as usize;
                        for i in 0..count {
                            let b = next(&mut pos, &mut crc)?;
                            for j in 0..8u32 {
                                if b << j & 0x80 != 0 {
                                    let n = u32::try_from(i).unwrap_or(0) * 8 + j;
                                    into[bank].insert((n % width, n / width + offset), ());
                                }
                            }
                        }
                        let end = u16::from(next(&mut pos, &mut crc)?) << 8
                            | u16::from(next(&mut pos, &mut crc)?);
                        if end != 0 {
                            return Err(ImageError(format!(
                                "expected 0000 after a data block, got {end:04x}"
                            )));
                        }
                    }
                    0x05 => crc = 0xFFFF,
                    0x06 => break,
                    other => return Err(ImageError(format!("unknown command 00 {other:02x}"))),
                },
                0x10 => bank = payload as usize & 3,
                0x20 => {
                    if crc != 0 {
                        return Err(ImageError("the CRC check failed".into()));
                    }
                }
                0x50 => {}
                0x60 => width = payload + 1,
                0x70 => height = payload,
                0x80 => offset = payload,
                0x90 => warmboot = payload & 0x20 != 0,
                other => return Err(ImageError(format!("unknown command {other:02x}"))),
            }
        }
        let device = match (cram_w, cram_h) {
            (182, 80) => "384",
            (332, 144) => "1k",
            (872, 272) => "8k",
            (692, 336) => "5k",
            (692, 176) => "u4k",
            (656, 176) => "lm4k",
            _ => {
                return Err(ImageError(format!(
                    "a {cram_w} x {cram_h} configuration memory is no device icepack knows"
                )));
            }
        };
        let layout = Layout::for_device(device).ok_or_else(|| ImageError(device.into()))?;
        let mut image = Image::new(layout.clone());
        image.warmboot = warmboot;
        // Invert the tile mapping: every tile bit's position, once.
        let mut claimed: BTreeSet<(usize, u32, u32)> = BTreeSet::new();
        for y in 0..=layout.chip_height + 1 {
            for x in 0..=layout.chip_width + 1 {
                let kind = layout.tile_type(x, y);
                for row in 0..16 {
                    for col in 0..Layout::tile_width(kind) {
                        let at = layout.cram_index(x, y, row, col);
                        claimed.insert(at);
                        if cram[at.0].contains_key(&(at.1, at.2)) {
                            image.set(x, y, row, col);
                        }
                    }
                }
            }
        }
        for (b, bits) in cram.iter().enumerate() {
            for &(cx, cy) in bits.keys() {
                if !claimed.contains(&(b, cx, cy)) {
                    image.extra_bits.insert((b, cx, cy));
                }
            }
        }
        // The block RAM contents, by inverting `bram_index` over every RAM.
        if !bram.iter().all(BTreeMap::is_empty) {
            for x in 0..=layout.chip_width + 1 {
                for y in 0..=layout.chip_height + 1 {
                    if layout.tile_type(x, y) != "ramb" {
                        continue;
                    }
                    let mut lines = vec![[false; 256]; 16];
                    let mut any = false;
                    for (bit_y, line) in lines.iter_mut().enumerate() {
                        for (bit_x, slot) in line.iter_mut().enumerate() {
                            let (b, bx, by) = layout.bram_index(
                                x,
                                y,
                                u32::try_from(bit_x).unwrap_or(0),
                                u32::try_from(bit_y).unwrap_or(0),
                            );
                            if bram[b].contains_key(&(bx, by)) {
                                *slot = true;
                                any = true;
                            }
                        }
                    }
                    if any {
                        image.ram_data.insert((x, y), lines);
                    }
                }
            }
        }
        Ok(image)
    }
}

/// The size of one header of a multi-image flash layout, in bytes.
pub const MULTIBOOT_HEADER: usize = 32;

/// How many images a multi-image layout's headers can point at: what a
/// warm boot's `S1`/`S0` select.
pub const MULTIBOOT_IMAGES: usize = 4;

/// Several bitstreams laid out for a flash the part boots from and warm
/// boots between, as IceStorm's `icemulti` writes them (ported from
/// `icemulti.cc`, read from IceStorm's source).
///
/// The flash starts with five 32-byte headers: the first is the one the
/// part reads at power-on and points at image `power_on`; the next four
/// are what an `SB_WARMBOOT` with `S1 S0` = 0..3 boots, each pointing at
/// the image of that number. Each header is a short command stream —
/// synchronisation word, `92 00 00` (no cold-boot selection), `44 03` and
/// the image's 24-bit address, `82 00 00`, `01 08` (reboot) — padded with
/// zeros. The images follow at once, each starting on a multiple of
/// `2^align_bits` bytes, padded with `0xFF`. Fewer than four images are
/// repeated: a slot with none boots the power-on image, as `icemulti`
/// does.
///
/// Returns `None` for no image, more than four, or `power_on` out of range.
#[must_use]
pub fn multiboot(images: &[&[u8]], power_on: usize, align_bits: u32) -> Option<Vec<u8>> {
    if images.is_empty() || images.len() > MULTIBOOT_IMAGES || power_on >= images.len() {
        return None;
    }
    let align = |offset: usize| {
        let mask = (1usize << align_bits) - 1;
        if offset & mask == 0 {
            offset
        } else {
            (offset | mask) + 1
        }
    };
    let mut offsets = Vec::with_capacity(images.len());
    let mut offset = (MULTIBOOT_IMAGES + 1) * MULTIBOOT_HEADER;
    for image in images {
        offsets.push(offset);
        offset = align(offset + image.len());
    }
    let header = |at: usize| -> Vec<u8> {
        let a = u32::try_from(at).unwrap_or(0);
        let [_, hi, mid, lo] = a.to_be_bytes();
        let mut h = vec![
            0x7E, 0xAA, 0x99, 0x7E, 0x92, 0x00, 0x00, 0x44, 0x03, hi, mid, lo, 0x82, 0x00, 0x00,
            0x01, 0x08,
        ];
        h.resize(MULTIBOOT_HEADER, 0x00);
        h
    };
    let mut out = header(offsets[power_on]);
    for slot in 0..MULTIBOOT_IMAGES {
        let index = if slot < images.len() { slot } else { power_on };
        out.extend(header(offsets[index]));
    }
    for (image, at) in images.iter().zip(&offsets) {
        out.resize(*at, 0xFF);
        out.extend_from_slice(image);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout `icemulti -p0 a.bin b.bin` writes, byte for byte where it
    /// matters: the five headers, each image's address in them, and the
    /// images where those addresses say. The bytes were read from
    /// `icemulti.cc`'s `write_header`; what would catch a misreading of it
    /// is the board (`docs/fpga-icestorm.md`, "Warm boot"), which this test
    /// is not.
    #[test]
    fn a_multiboot_layout_is_icemultis() {
        let a = [0x11u8; 100];
        let b = [0x22u8; 50];
        let flash = multiboot(&[&a, &b], 0, 0).unwrap();
        // Five headers, then a at 160 and b straight after it.
        assert_eq!(flash.len(), 160 + 100 + 50);
        let address = |flash: &[u8], h: usize| {
            let at = h * MULTIBOOT_HEADER;
            assert_eq!(
                &flash[at..at + 9],
                &[0x7E, 0xAA, 0x99, 0x7E, 0x92, 0x00, 0x00, 0x44, 0x03]
            );
            assert_eq!(&flash[at + 12..at + 17], &[0x82, 0x00, 0x00, 0x01, 0x08]);
            assert!(flash[at + 17..at + 32].iter().all(|b| *b == 0));
            usize::from(flash[at + 9]) << 16
                | usize::from(flash[at + 10]) << 8
                | usize::from(flash[at + 11])
        };
        assert_eq!(address(&flash, 0), 160, "power-on boots image 0");
        assert_eq!(address(&flash, 1), 160, "warm boot 0 is image 0");
        assert_eq!(address(&flash, 2), 260, "warm boot 1 is image 1");
        assert_eq!(
            address(&flash, 3),
            160,
            "a slot with no image boots the power-on one"
        );
        assert_eq!(address(&flash, 4), 160);
        assert!(flash[160..260].iter().all(|b| *b == 0x11));
        assert!(flash[260..].iter().all(|b| *b == 0x22));
        // Aligned, image 1 starts on the boundary and the gap is 0xFF.
        let flash = multiboot(&[&a, &b], 1, 16).unwrap();
        assert_eq!(address(&flash, 0), 0x1_0000, "power-on boots image 1");
        assert!(flash[260..0x1_0000].iter().all(|b| *b == 0xFF));
        let five: [&[u8]; 5] = [&a; 5];
        assert!(multiboot(&five, 0, 0).is_none());
        assert!(multiboot(&[&a], 1, 0).is_none());
    }

    /// The CRC is CCITT with an initial value of all ones, and a stream
    /// carries its own CRC so that the check over it comes out zero.
    #[test]
    fn the_crc_is_ccitt() {
        let mut crc = 0xFFFF;
        for b in b"123456789" {
            crc16(&mut crc, *b);
        }
        assert_eq!(
            crc, 0x29B1,
            "CRC-16/CCITT-FALSE of the standard check string"
        );
    }

    /// Every tile bit of a 5k lands somewhere distinct, inside its bank:
    /// a mapping that sent two bits to one place would lose one of them
    /// silently.
    #[test]
    fn every_tile_bit_has_its_own_place() {
        let layout = Layout::for_device("5k").unwrap();
        let mut seen = BTreeSet::new();
        for y in 0..=31 {
            for x in 0..=25 {
                let kind = layout.tile_type(x, y);
                for row in 0..16 {
                    for col in 0..Layout::tile_width(kind) {
                        let (b, cx, cy) = layout.cram_index(x, y, row, col);
                        assert!(cx < layout.cram_width, "{x} {y} {row} {col}");
                        assert!(cy < layout.cram_bank_height(b), "{x} {y} {row} {col}");
                        assert!(seen.insert((b, cx, cy)), "{x} {y} {row} {col} collides");
                    }
                }
            }
        }
    }

    /// The grid kinds icepack assumes are the ones the chip database
    /// lists, spot-checked: corners, the DSP and IP-connect columns, the
    /// RAM columns.
    #[test]
    fn the_5k_tile_kinds() {
        let l = Layout::for_device("5k").unwrap();
        assert_eq!(l.tile_type(0, 0), "corner");
        assert_eq!(l.tile_type(0, 5), "dsp0");
        assert_eq!(l.tile_type(25, 26), "dsp3");
        assert_eq!(l.tile_type(0, 27), "ipcon");
        assert_eq!(l.tile_type(6, 1), "ramb");
        assert_eq!(l.tile_type(19, 2), "ramt");
        assert_eq!(l.tile_type(10, 0), "io");
        assert_eq!(l.tile_type(10, 10), "logic");
    }

    /// An image packs and unpacks to itself, tiles, extra bits and RAM
    /// contents together.
    #[test]
    fn an_image_survives_the_stream() {
        let layout = Layout::for_device("5k").unwrap();
        let mut image = Image::new(layout);
        image.set(1, 1, 0, 36);
        image.set(10, 0, 3, 17);
        image.set(25, 20, 15, 53);
        image.set(19, 30, 7, 41);
        image.extra_bits.insert((1, 690, 175));
        let mut line = [false; 256];
        line[0] = true;
        line[255] = true;
        let mut lines = vec![[false; 256]; 16];
        lines[3] = line;
        image.ram_data.insert((6, 1), lines);
        let bin = image.to_bin();
        let back = Image::from_bin(&bin).unwrap();
        assert_eq!(back, image);
        // And the text form.
        let asc = Image::from_asc(&image.to_asc()).unwrap();
        assert_eq!(asc, image);
    }
}
