//! The Gowin `.fs` bitstream container.
//!
//! # What this is, and what has been checked
//!
//! A Gowin configuration file is a **text** file: every byte is written
//! as eight `0`/`1` characters, one line per command and one line per row
//! of the die's configuration bitmap. This module writes that file and
//! reads it back.
//!
//! **Nothing this module produces has been loaded into a part.** What is
//! established is agreement with a reference file, byte for byte:
//! `gowin_pack`, Project Apicula's own packer, was run for `GW2A-18` and
//! its output was taken apart. Against that file this module's reader and
//! writer agree on
//!
//! - the ten header lines and the six footer lines, which come from the
//!   chip database's own `cmd_hdr` and `cmd_ftr` and are written verbatim
//!   except for the two fields named below;
//! - the row count patched into the last header command (`0x3b`), which
//!   for a GW2A-18 is 1342, the height of the die bitmap;
//! - the geometry: 1342 rows of 3376 bits, which is 422 bytes with no
//!   padding, each line 430 bytes and so 3440 characters;
//! - **every one of the 1342 row check words**, computed by
//!   [`crc16_arc`], and the footer's closing check word `0x7334`;
//! - that a design with nothing in it still sets 862 bits, of which 462
//!   are the database's `const` tables.
//!
//! `docs/fpga-gowin.md` gives the command that produces that reference
//! file and says exactly what the comparison does and does not settle.
//!
//! # The layout
//!
//! ```text
//! 20 x 0xff                       preamble
//! 0xff 0xff                       (a two-byte field the vendor programmer shows)
//! 0xa5 0xc3                       the magic
//! 0x06 0x00 0x00 0x00 <idcode:4>  check the part's JTAG IDCODE
//! 0x10 ...                        options: loading rate, compression, program-done bypass
//! 0x51 ...                        the three bytes that stand in for runs of zeros
//! 0x0b 0x00 0x00 0x00             security
//! 0xd2 0x00 0xff 0xff <addr:4>    SPI flash address (excluded from the check word)
//! 0x12 0x00 0x00 0x00
//! 0x3b 0x80 <rows:2>              load configuration: how many rows follow
//! <row bytes> <crc:2> 6 x 0xff    ... one line per row of the die bitmap
//! 18 x 0xff <crc:2>               end of the grid
//! 0x0a 0x00 0x00 0x00 <user:4>    USERCODE
//! 8 x 0xff
//! 0x08 0x00 0x00 0x00             done
//! 8 x 0xff
//! 0xff 0xff
//! ```
//!
//! Every one of those bytes but two fields is **data from the chip
//! database**, not a constant written here: `cmd_hdr` and `cmd_ftr` are
//! arrays of byte strings in the database itself. That is deliberate, and
//! it is the same choice `super::xc7` makes about the frame layout — the
//! database knows the part and this module knows the envelope. The two
//! fields this module fills in are the row count in the `0x3b` command
//! and, when a caller asks for one, the USERCODE in the `0x0a` command.
//!
//! # The check word
//!
//! CRC-16/ARC: the polynomial `0x8005` reflected, initial value zero, no
//! final exclusive-or, and **written low byte first**. Each row's word
//! covers the six `0xff` bytes that ended the *previous* line followed by
//! that row's own bytes, and the first row's covers the header from the
//! `0x06` command onward with the `0xd2` line left out. That last
//! exception is not a guess: with the `0xd2` line included, not one of
//! the 1342 words of the reference file matches, and with it excluded all
//! 1342 do.
//!
//! # The bitmap is a tiled bitmap, which is what [`Arch`] already means
//!
//! The one place this container is *simpler* than the 7-series one: a
//! Gowin die bitmap is the per-tile bitmaps laid side by side. Tile
//! `(row, col)` occupies `height` rows and `width` columns at the offset
//! given by the heights of the rows above it and the widths of the
//! columns left of it, so a tile's `ConfigBit { row, col }` is that tile's
//! bit and nothing has to be translated. [`DieLayout`] is that arithmetic
//! and [`DieBitmap`] is the bitmap; there is no frame address anywhere.
//!
//! A tile's size depends on where it is — an IO or block RAM row is
//! taller than a logic row and the left and right IO columns are wider —
//! so [`DieLayout`] takes the per-row heights and per-column widths
//! rather than one size. On a GW2A-18 the row heights are 24, 26 and 28
//! and the column widths 60 and 68.
//!
//! [`Arch`]: super::arch::Arch
//!
//! # Compression
//!
//! A `.fs` may be compressed, which means only that runs of two, four or
//! eight zero bytes are replaced by three byte values the `0x51` command
//! names, chosen because they appear nowhere in the data. This module
//! **reads** a compressed file and **writes** an uncompressed one: the
//! only thing compression buys is file size (the empty GW2A-18 stream is
//! 4 618 782 bytes uncompressed and 699 990 compressed) and a writer that
//! picks the wrong three bytes produces a file that decompresses into
//! something else. Reading is the direction where refusing would cost a
//! caller a real bitstream.

use std::error::Error;
use std::fmt;

/// Check the part's JTAG IDCODE. Followed by three option bytes and the
/// IDCODE, most significant byte first.
pub const CMD_IDCODE: u8 = 0x06;
/// Stream options: the loading rate, the compression-enabled bit (13) and
/// the program-done-bypass bit (12).
pub const CMD_OPTIONS: u8 = 0x10;
/// The three byte values that stand in for runs of eight, four and two
/// zero bytes in a compressed stream, in bytes 5, 6 and 7.
pub const CMD_COMPRESS: u8 = 0x51;
/// Security.
pub const CMD_SECURITY: u8 = 0x0b;
/// The SPI flash address, and **the one header line the check word does
/// not cover**.
pub const CMD_SPI_ADDRESS: u8 = 0xd2;
/// Load the configuration: `0x3b 0x80 <rows:2>`, the last header line.
pub const CMD_LOAD_CONFIG: u8 = 0x3b;
/// Load the configuration with the row check words switched off.
pub const CMD_LOAD_CONFIG_NO_CRC: u8 = 0xbb;
/// Set the USERCODE. Followed by three option bytes and four bytes of
/// user code.
pub const CMD_USERCODE: u8 = 0x0a;
/// The last command of the stream.
pub const CMD_DONE: u8 = 0x08;

/// How many `0xff` bytes follow every row's check word, and therefore how
/// many bytes of the next row's check word are already decided before its
/// data is seen.
pub const ROW_TAIL_BYTES: usize = 6;

/// The bytes at the end of a body line that are not bitmap data: the
/// two-byte check word and [`ROW_TAIL_BYTES`] of `0xff`.
pub const ROW_SUFFIX_BYTES: usize = 2 + ROW_TAIL_BYTES;

/// Why a `.fs` file could not be written or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GowinError {
    /// A line held a character other than `0`, `1` or whitespace.
    NotBinary {
        /// The line, counting from one.
        line: usize,
        /// The character that was not a bit.
        found: char,
    },
    /// A line's length was not a whole number of bytes.
    NotWholeBytes {
        /// The line, counting from one.
        line: usize,
        /// How many bit characters it held.
        bits: usize,
    },
    /// The header ended without a `0x3b` load-configuration command, so
    /// nothing says how many rows follow.
    NoLoadConfig,
    /// A row's check word disagreed with the bytes it covers.
    BadCrc {
        /// The row, counting from zero.
        row: usize,
        /// What the file says.
        found: u16,
        /// What the bytes give.
        wanted: u16,
    },
    /// The `0x3b` command promised one number of rows and the file holds
    /// another.
    RowCount {
        /// What the command says.
        declared: usize,
        /// How many body lines there are.
        found: usize,
    },
    /// A body line is not as wide as the die is.
    RowWidth {
        /// The row, counting from zero.
        row: usize,
        /// The bits the line holds before its check word.
        found: usize,
        /// The bits a row of this die needs, padding included.
        wanted: usize,
    },
    /// The bits that pad a row out to a whole number of bytes were not
    /// all ones, which is what a Gowin stream pads with.
    BadPadding {
        /// The row, counting from zero.
        row: usize,
    },
    /// The stream's IDCODE is not the part the caller asked for.
    IdcodeMismatch {
        /// The IDCODE that was wanted.
        wanted: u32,
        /// The IDCODE the stream carries.
        found: u32,
    },
    /// A header or footer line the database did not supply, or supplied
    /// too short to hold the field that has to be patched into it.
    ShortCommand {
        /// The command byte.
        command: u8,
        /// How many bytes the line holds.
        length: usize,
    },
}

impl fmt::Display for GowinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GowinError::NotBinary { line, found } => write!(
                f,
                "line {line}: `{found}` is not a bit; a `.fs` file holds only `0` and `1`"
            ),
            GowinError::NotWholeBytes { line, bits } => write!(
                f,
                "line {line}: {bits} bit(s) is not a whole number of bytes"
            ),
            GowinError::NoLoadConfig => write!(
                f,
                "the header has no `{CMD_LOAD_CONFIG:#04x}` load-configuration command, \
                 so nothing says how many rows the bitmap has"
            ),
            GowinError::BadCrc { row, found, wanted } => write!(
                f,
                "row {row}: the check word is {found:#06x} and the bytes give {wanted:#06x}"
            ),
            GowinError::RowCount { declared, found } => write!(
                f,
                "the load-configuration command promises {declared} row(s) and the file holds {found}"
            ),
            GowinError::RowWidth { row, found, wanted } => write!(
                f,
                "row {row} is {found} bit(s) wide and the die is {wanted}"
            ),
            GowinError::BadPadding { row } => write!(
                f,
                "row {row}: the bits padding it out to a whole byte are not all ones"
            ),
            GowinError::IdcodeMismatch { wanted, found } => write!(
                f,
                "this stream is for IDCODE {found:#010x} and the part is {wanted:#010x}"
            ),
            GowinError::ShortCommand { command, length } => write!(
                f,
                "the `{command:#04x}` command is {length} byte(s) long, too short for the \
                 field that has to be written into it"
            ),
        }
    }
}

impl Error for GowinError {}

/// CRC-16/ARC over `data`, continuing from `seed`.
///
/// Width 16, polynomial `0x8005`, initial value zero, input and output
/// reflected, no final exclusive-or — which reflected is the familiar
/// `0xa001` shift. `seed` is what makes a row's word continue from the
/// six `0xff` bytes that ended the previous line, which is how a Gowin
/// stream chains them.
///
/// ```
/// use reticle::fpga::gowin::crc16_arc;
///
/// // The word the footer of every GW2A-18 stream ends with: twenty-four
/// // 0xff bytes, six from the last row's tail and eighteen of its own.
/// assert_eq!(crc16_arc(&[0xff; 24], 0), 0x7334);
/// ```
pub fn crc16_arc(data: &[u8], seed: u16) -> u16 {
    let mut crc = seed;
    for byte in data {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// Where each tile's bitmap sits in the die's.
///
/// A Gowin die bitmap is the tiles' own bitmaps laid out in a grid, so
/// this is the cumulative sums of the row heights and the column widths
/// and nothing more. Every tile in one grid row has the same height and
/// every tile in one grid column the same width, which is what makes the
/// layout a pair of one-dimensional sums rather than a packing problem;
/// [`DieLayout::new`] takes the heights and widths and
/// [`DieLayout::origin`] gives a tile's top-left bit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DieLayout {
    row_offsets: Vec<u32>,
    col_offsets: Vec<u32>,
    rows: u32,
    cols: u32,
}

impl DieLayout {
    /// The layout of a grid whose rows are `heights` tall and whose
    /// columns are `widths` wide.
    pub fn new(heights: &[u32], widths: &[u32]) -> DieLayout {
        let mut row_offsets = Vec::with_capacity(heights.len());
        let mut rows = 0u32;
        for h in heights {
            row_offsets.push(rows);
            rows = rows.saturating_add(*h);
        }
        let mut col_offsets = Vec::with_capacity(widths.len());
        let mut cols = 0u32;
        for w in widths {
            col_offsets.push(cols);
            cols = cols.saturating_add(*w);
        }
        DieLayout {
            row_offsets,
            col_offsets,
            rows,
            cols,
        }
    }

    /// Rows of the die bitmap.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Columns of the die bitmap.
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// Grid rows.
    pub fn grid_rows(&self) -> u32 {
        u32::try_from(self.row_offsets.len()).unwrap_or(u32::MAX)
    }

    /// Grid columns.
    pub fn grid_cols(&self) -> u32 {
        u32::try_from(self.col_offsets.len()).unwrap_or(u32::MAX)
    }

    /// The die bitmap position of tile `(grid_row, grid_col)`'s bit
    /// `(0, 0)`, or `None` when the grid has no such tile.
    pub fn origin(&self, grid_row: u32, grid_col: u32) -> Option<(u32, u32)> {
        let r = *self.row_offsets.get(usize::try_from(grid_row).ok()?)?;
        let c = *self.col_offsets.get(usize::try_from(grid_col).ok()?)?;
        Some((r, c))
    }

    /// An empty bitmap of this die's size.
    pub fn bitmap(&self) -> DieBitmap {
        DieBitmap::new(self.rows, self.cols)
    }
}

/// The die's configuration bitmap: `rows` by `cols` bits.
///
/// Bit packed, most significant bit first within a byte, which is the
/// order the file writes them in — though not the order it writes them
/// *along a row*; see [`DieBitmap::to_text_rows`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DieBitmap {
    rows: u32,
    cols: u32,
    row_bytes: usize,
    bits: Vec<u8>,
}

impl DieBitmap {
    /// An all-zero bitmap.
    pub fn new(rows: u32, cols: u32) -> DieBitmap {
        let row_bytes = (cols as usize).div_ceil(8);
        DieBitmap {
            rows,
            cols,
            row_bytes,
            bits: vec![0; row_bytes * rows as usize],
        }
    }

    /// Rows.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Columns.
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// The bits that pad a row out to a whole number of bytes, which the
    /// file writes as **ones**.
    pub fn pad_bits(&self) -> usize {
        self.row_bytes * 8 - self.cols as usize
    }

    /// Sets the bit at `(row, col)`, and says whether it was inside the
    /// bitmap.
    ///
    /// A bit outside is reported rather than ignored: the databases this
    /// is filled from state a tile's bits in the tile's own coordinates,
    /// and one that lands outside means the tile's declared size and its
    /// bit list disagree, which is worth counting.
    pub fn set(&mut self, row: u32, col: u32) -> bool {
        let Some(index) = self.index(row, col) else {
            return false;
        };
        self.bits[index] |= 0x80 >> (col % 8);
        true
    }

    /// The bit at `(row, col)`; false outside the bitmap.
    pub fn get(&self, row: u32, col: u32) -> bool {
        match self.index(row, col) {
            Some(index) => self.bits[index] & (0x80 >> (col % 8)) != 0,
            None => false,
        }
    }

    fn index(&self, row: u32, col: u32) -> Option<usize> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        Some(row as usize * self.row_bytes + (col as usize) / 8)
    }

    /// How many bits are set.
    pub fn count_ones(&self) -> usize {
        self.bits.iter().map(|b| b.count_ones() as usize).sum()
    }

    /// How many rows have a bit set.
    pub fn rows_used(&self) -> usize {
        (0..self.rows as usize)
            .filter(|r| {
                self.bits[r * self.row_bytes..(r + 1) * self.row_bytes]
                    .iter()
                    .any(|b| *b != 0)
            })
            .count()
    }

    /// The bytes of one row **as the file writes them**: the row's bits
    /// reversed, with the padding ones in front, packed most significant
    /// bit first.
    ///
    /// The reversal is the container's, not this crate's: a Gowin `.fs`
    /// writes a row from its highest column to its lowest, so the first
    /// byte of a line holds columns `cols-1 ..= cols-8`. Getting it
    /// backwards mirrors the whole die, which every check but a real part
    /// would pass.
    pub fn row_bytes(&self, row: u32) -> Vec<u8> {
        let pad = self.pad_bits();
        let mut out = vec![0u8; self.row_bytes];
        // The padding ones occupy the first `pad` bit positions.
        for i in 0..pad {
            out[i / 8] |= 0x80 >> (i % 8);
        }
        for i in 0..self.cols as usize {
            // Written bit `pad + i` is column `cols - 1 - i`.
            if self.get(row, self.cols - 1 - u32::try_from(i).unwrap_or(0)) {
                let at = pad + i;
                out[at / 8] |= 0x80 >> (at % 8);
            }
        }
        out
    }

    /// Every row's bytes, in order.
    pub fn to_text_rows(&self) -> Vec<Vec<u8>> {
        (0..self.rows).map(|r| self.row_bytes(r)).collect()
    }

    /// A bitmap from rows of bytes as the file writes them, which is
    /// [`DieBitmap::row_bytes`] undone.
    ///
    /// # Errors
    ///
    /// [`GowinError::RowWidth`] when a row does not hold `cols` bits plus
    /// its padding, and [`GowinError::BadPadding`] when the padding bits
    /// are not ones.
    pub fn from_text_rows(rows: &[Vec<u8>], cols: u32) -> Result<DieBitmap, GowinError> {
        let row_bytes = (cols as usize).div_ceil(8);
        let pad = row_bytes * 8 - cols as usize;
        let mut map = DieBitmap::new(u32::try_from(rows.len()).unwrap_or(u32::MAX), cols);
        for (r, bytes) in rows.iter().enumerate() {
            if bytes.len() != row_bytes {
                return Err(GowinError::RowWidth {
                    row: r,
                    found: bytes.len() * 8,
                    wanted: row_bytes * 8,
                });
            }
            let bit = |i: usize| bytes[i / 8] & (0x80 >> (i % 8)) != 0;
            for i in 0..pad {
                if !bit(i) {
                    return Err(GowinError::BadPadding { row: r });
                }
            }
            let row = u32::try_from(r).unwrap_or(0);
            for i in 0..cols as usize {
                if bit(pad + i) {
                    map.set(row, cols - 1 - u32::try_from(i).unwrap_or(0));
                }
            }
        }
        Ok(map)
    }
}

/// A whole `.fs` file: the header commands, the die bitmap, the footer.
///
/// The header and footer are byte strings the chip database supplies
/// (`cmd_hdr` and `cmd_ftr`); this module patches the row count into the
/// load-configuration command and, on request, the USERCODE, and computes
/// the check words. Nothing else in them is invented here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsStream {
    /// The header commands, in order, ending with the `0x3b` line.
    pub header: Vec<Vec<u8>>,
    /// The die's configuration bitmap.
    pub bitmap: DieBitmap,
    /// The footer lines, in order, starting with the end-of-grid line.
    pub footer: Vec<Vec<u8>>,
}

impl FsStream {
    /// A stream with the given header and footer over `bitmap`, with the
    /// row count written into the load-configuration command.
    ///
    /// # Errors
    ///
    /// [`GowinError::NoLoadConfig`] when the header has no `0x3b` (or
    /// `0xbb`) command, and [`GowinError::ShortCommand`] when that
    /// command is too short to hold a row count.
    pub fn new(
        header: Vec<Vec<u8>>,
        bitmap: DieBitmap,
        footer: Vec<Vec<u8>>,
    ) -> Result<FsStream, GowinError> {
        let mut stream = FsStream {
            header,
            bitmap,
            footer,
        };
        stream.set_row_count()?;
        Ok(stream)
    }

    /// Writes the bitmap's row count into the load-configuration command,
    /// which is what tells the part how many lines follow.
    fn set_row_count(&mut self) -> Result<(), GowinError> {
        let rows = self.bitmap.rows();
        let line = self
            .header
            .iter_mut()
            .find(|l| {
                matches!(
                    l.first(),
                    Some(&CMD_LOAD_CONFIG) | Some(&CMD_LOAD_CONFIG_NO_CRC)
                )
            })
            .ok_or(GowinError::NoLoadConfig)?;
        if line.len() < 4 {
            return Err(GowinError::ShortCommand {
                command: line[0],
                length: line.len(),
            });
        }
        let be = u16::try_from(rows).unwrap_or(u16::MAX).to_be_bytes();
        line[2] = be[0];
        line[3] = be[1];
        Ok(())
    }

    /// Sets the USERCODE in the footer's `0x0a` command.
    ///
    /// # Errors
    ///
    /// [`GowinError::ShortCommand`] when the footer has no `0x0a` line or
    /// it is shorter than eight bytes.
    pub fn set_usercode(&mut self, code: u32) -> Result<(), GowinError> {
        let line = self
            .footer
            .iter_mut()
            .find(|l| l.first() == Some(&CMD_USERCODE))
            .ok_or(GowinError::ShortCommand {
                command: CMD_USERCODE,
                length: 0,
            })?;
        if line.len() < 8 {
            return Err(GowinError::ShortCommand {
                command: CMD_USERCODE,
                length: line.len(),
            });
        }
        line[4..8].copy_from_slice(&code.to_be_bytes());
        Ok(())
    }

    /// The IDCODE the header's `0x06` command checks, when it has one.
    pub fn idcode(&self) -> Option<u32> {
        let line = self
            .header
            .iter()
            .find(|l| l.first() == Some(&CMD_IDCODE))?;
        let bytes: [u8; 4] = line.get(4..8)?.try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    }

    /// Checks that this stream is for the part the caller means.
    ///
    /// # Errors
    ///
    /// [`GowinError::IdcodeMismatch`] when the header's IDCODE differs,
    /// which is what stops a stream built from one die's database being
    /// sent to another. A header with no `0x06` command is accepted,
    /// since then the stream makes no claim.
    pub fn check_idcode(&self, wanted: u32) -> Result<(), GowinError> {
        match self.idcode() {
            Some(found) if found != wanted => Err(GowinError::IdcodeMismatch { wanted, found }),
            _ => Ok(()),
        }
    }

    /// True when the `0x10` command's bit 13 says the body is compressed.
    pub fn is_compressed(&self) -> bool {
        compressed_flag(&self.header)
    }

    /// The whole file as the text a `.fs` is.
    ///
    /// Uncompressed, whatever the `0x10` command says: see the module
    /// documentation for why this direction does not compress. The flag
    /// is cleared so the file describes itself correctly.
    pub fn to_text(&self) -> String {
        let rows = self.bitmap.to_text_rows();
        let mut out = String::with_capacity(
            (self.header.len() + self.footer.len()) * 80
                + rows.len() * (rows.first().map_or(0, Vec::len) + ROW_SUFFIX_BYTES) * 8
                + rows.len(),
        );

        // The check word covers the header from the third line on, less
        // the SPI address line.
        let mut covered: Vec<u8> = Vec::new();
        for (index, line) in self.header.iter().enumerate() {
            let mut line = line.clone();
            if line.first() == Some(&CMD_OPTIONS) && line.len() >= 8 {
                // Bit 13 of the eight-byte field: not compressed.
                line[6] &= !(1 << 5);
            }
            if index >= PREAMBLE_LINES && line.first() != Some(&CMD_SPI_ADDRESS) {
                covered.extend_from_slice(&line);
            }
            push_bits(&mut out, &line);
            out.push('\n');
        }

        for row in &rows {
            covered.extend_from_slice(row);
            let crc = crc16_arc(&covered, 0);
            covered = vec![0xff; ROW_TAIL_BYTES];
            push_bits(&mut out, row);
            push_bits(&mut out, &[(crc & 0xff) as u8, (crc >> 8) as u8]);
            push_bits(&mut out, &[0xff; ROW_TAIL_BYTES]);
            out.push('\n');
        }

        for line in &self.footer {
            push_bits(&mut out, line);
            out.push('\n');
        }
        out
    }

    /// Reads a `.fs` file whose die is `cols` bits wide.
    ///
    /// The width has to be supplied because the file does not carry it:
    /// a line holds the row's bits plus however many padding ones make a
    /// whole number of bytes (or, in a compressed file, a whole number of
    /// eight-byte groups), and only the chip database says which of those
    /// bits are the die's. [`written_width`] reports the width a file's
    /// lines do carry, for a caller with no database to hand.
    ///
    /// Every row's check word is verified. A compressed body is expanded
    /// using the three bytes the `0x51` command names.
    ///
    /// # Errors
    ///
    /// [`GowinError`]: a character that is not a bit, a line that is not
    /// a whole number of bytes, a missing load-configuration command, a
    /// row count that disagrees with the file, a row of the wrong width,
    /// padding that is not ones, or a check word that does not match.
    pub fn parse(text: &str, cols: u32) -> Result<FsStream, GowinError> {
        let lines = byte_lines(text)?;
        let mut header: Vec<Vec<u8>> = Vec::new();
        let mut declared: Option<usize> = None;
        let mut index = 0;
        while index < lines.len() {
            let line = &lines[index];
            header.push(line.clone());
            index += 1;
            if matches!(
                line.first(),
                Some(&CMD_LOAD_CONFIG) | Some(&CMD_LOAD_CONFIG_NO_CRC)
            ) && line.len() >= 4
            {
                declared = Some(usize::from(u16::from_be_bytes([line[2], line[3]])));
                break;
            }
        }
        let declared = declared.ok_or(GowinError::NoLoadConfig)?;
        if lines.len() < index + declared {
            return Err(GowinError::RowCount {
                declared,
                found: lines.len() - index,
            });
        }
        let body = &lines[index..index + declared];
        let footer = lines[index + declared..].to_vec();

        let compressed = compressed_flag(&header);
        let keys = compress_keys(&header);
        let check_crc = header.iter().any(|l| l.first() == Some(&CMD_LOAD_CONFIG));

        let mut covered: Vec<u8> = Vec::new();
        for (i, line) in header.iter().enumerate() {
            if i >= PREAMBLE_LINES && line.first() != Some(&CMD_SPI_ADDRESS) {
                covered.extend_from_slice(line);
            }
        }

        let row_bytes = (cols as usize).div_ceil(8);
        let mut rows: Vec<Vec<u8>> = Vec::with_capacity(body.len());
        for (row, line) in body.iter().enumerate() {
            if line.len() < ROW_SUFFIX_BYTES {
                return Err(GowinError::RowWidth {
                    row,
                    found: line.len() * 8,
                    wanted: row_bytes * 8,
                });
            }
            let split = line.len() - ROW_SUFFIX_BYTES;
            let data = &line[..split];
            let found = u16::from(line[split]) | (u16::from(line[split + 1]) << 8);
            covered.extend_from_slice(data);
            let wanted = crc16_arc(&covered, 0);
            if check_crc && wanted != found {
                return Err(GowinError::BadCrc { row, found, wanted });
            }
            covered = vec![0xff; ROW_TAIL_BYTES];

            let mut expanded = if compressed {
                expand(data, keys)
            } else {
                data.to_vec()
            };
            // A compressed row is padded out to a whole eight-byte group
            // rather than a whole byte, so it may carry whole extra bytes
            // of padding ones in front. They are ones, so dropping them
            // here and letting `from_text_rows` check the rest is safe.
            if expanded.len() > row_bytes {
                let extra = expanded.len() - row_bytes;
                if expanded[..extra].iter().any(|b| *b != 0xff) {
                    return Err(GowinError::BadPadding { row });
                }
                expanded.drain(..extra);
            }
            rows.push(expanded);
        }

        let bitmap = DieBitmap::from_text_rows(&rows, cols)?;
        Ok(FsStream {
            header,
            bitmap,
            footer,
        })
    }
}

/// The lines of the preamble the check word does not cover: the twenty
/// `0xff` bytes, the two-byte field after them and the `0xa5 0xc3` magic.
///
/// Three, and it is a measurement rather than a choice: the reference
/// file's 1342 check words all match with three skipped and none match
/// with two or four.
const PREAMBLE_LINES: usize = 3;

/// The bits a `.fs` file's body lines carry before their check word, for
/// a caller who has no chip database and wants to know the die's width.
///
/// A file whose body lines are `n` bytes long has `(n - 8) * 8` such
/// bits, of which the die's width is all but the padding.
///
/// # Errors
///
/// [`GowinError::NoLoadConfig`] when the header has no `0x3b` command,
/// and the parse errors of [`FsStream::parse`] for a malformed file.
pub fn written_width(text: &str) -> Result<usize, GowinError> {
    let lines = byte_lines(text)?;
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        index += 1;
        if matches!(
            line.first(),
            Some(&CMD_LOAD_CONFIG) | Some(&CMD_LOAD_CONFIG_NO_CRC)
        ) {
            let first = lines.get(index).ok_or(GowinError::NoLoadConfig)?;
            return Ok(first.len().saturating_sub(ROW_SUFFIX_BYTES) * 8);
        }
    }
    Err(GowinError::NoLoadConfig)
}

/// True when the `0x10` command's bit 13 is set.
fn compressed_flag(header: &[Vec<u8>]) -> bool {
    header
        .iter()
        .filter(|l| l.first() == Some(&CMD_OPTIONS) && l.len() >= 8)
        .any(|l| l[6] & (1 << 5) != 0)
}

/// The bytes that stand in for runs of eight, four and two zero bytes.
/// `0xff` in a slot means the run length it names is not substituted.
fn compress_keys(header: &[Vec<u8>]) -> [u8; 3] {
    for line in header {
        if line.first() == Some(&CMD_COMPRESS) && line.len() >= 8 {
            return [line[5], line[6], line[7]];
        }
    }
    [0xff, 0xff, 0xff]
}

/// Expands a compressed row.
///
/// A key of `0xff` means "this run length was not substituted", which is
/// what the format says a packer writes when it could find no unused
/// byte; such a stream is legal and simply expands to itself, so there is
/// nothing here to fail on.
fn expand(data: &[u8], keys: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for byte in data {
        // A real key is a byte that appears nowhere in the data, so a
        // data byte never collides with one; `0xff` is the marker for an
        // unused slot and is checked for first.
        let run = match *byte {
            0xff => 0,
            b if b == keys[0] => 8,
            b if b == keys[1] => 4,
            b if b == keys[2] => 2,
            _ => 0,
        };
        if run == 0 {
            out.push(*byte);
        } else {
            out.extend(std::iter::repeat_n(0u8, run));
        }
    }
    out
}

/// Appends `bytes` as eight `0`/`1` characters each.
fn push_bits(out: &mut String, bytes: &[u8]) {
    for byte in bytes {
        for shift in (0..8).rev() {
            out.push(if byte & (1 << shift) != 0 { '1' } else { '0' });
        }
    }
}

/// Every non-comment, non-blank line of a `.fs` file as bytes.
fn byte_lines(text: &str) -> Result<Vec<Vec<u8>>, GowinError> {
    let mut out = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.len() % 8 != 0 {
            return Err(GowinError::NotWholeBytes {
                line: number + 1,
                bits: line.len(),
            });
        }
        let mut bytes = Vec::with_capacity(line.len() / 8);
        let mut byte = 0u8;
        for (i, ch) in line.chars().enumerate() {
            let bit = match ch {
                '0' => 0,
                '1' => 1,
                other => {
                    return Err(GowinError::NotBinary {
                        line: number + 1,
                        found: other,
                    });
                }
            };
            byte = (byte << 1) | bit;
            if i % 8 == 7 {
                bytes.push(byte);
                byte = 0;
            }
        }
        out.push(bytes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GW2A-18's ten header lines, as the chip database carries them,
    /// with the row count still blank.
    fn header() -> Vec<Vec<u8>> {
        vec![
            vec![0xff; 20],
            vec![0xff, 0xff],
            vec![0xa5, 0xc3],
            vec![0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x1b],
            vec![0x10, 0x00, 0x00, 0x00, 0x00, 0xae, 0x00, 0x00],
            vec![0x51, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            vec![0x0b, 0x00, 0x00, 0x00],
            vec![0xd2, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00],
            vec![0x12, 0x00, 0x00, 0x00],
            vec![0x3b, 0x80, 0x00, 0x00],
        ]
    }

    /// And its six footer lines.
    fn footer() -> Vec<Vec<u8>> {
        vec![
            {
                let mut l = vec![0xff; 18];
                l.extend_from_slice(&[0x34, 0x73]);
                l
            },
            vec![0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            vec![0xff; 8],
            vec![0x08, 0x00, 0x00, 0x00],
            vec![0xff; 8],
            vec![0xff, 0xff],
        ]
    }

    #[test]
    fn the_check_word_is_crc16_arc() {
        // The three vectors any CRC-16/ARC implementation is checked
        // against, plus the one this container ends every file with.
        assert_eq!(crc16_arc(b"", 0), 0x0000);
        assert_eq!(crc16_arc(b"123456789", 0), 0xbb3d);
        assert_eq!(crc16_arc(&[0xff; 24], 0), 0x7334);
        // Seeding is the same as prepending, which is what makes a row's
        // word continue from the previous line's tail.
        let split = crc16_arc(b"456789", crc16_arc(b"123", 0));
        assert_eq!(split, 0xbb3d);
    }

    #[test]
    fn the_layout_is_the_cumulative_sums_of_the_rows_and_columns() {
        // A GW2A-18's first three rows are 28, 24, 24 tall and its first
        // three columns 60, 68, 68 wide.
        let layout = DieLayout::new(&[28, 24, 24], &[60, 68, 68]);
        assert_eq!(layout.rows(), 76);
        assert_eq!(layout.cols(), 196);
        assert_eq!(layout.origin(0, 0), Some((0, 0)));
        assert_eq!(layout.origin(1, 1), Some((28, 60)));
        assert_eq!(layout.origin(2, 2), Some((52, 128)));
        assert_eq!(layout.origin(3, 0), None);
        assert_eq!(layout.origin(0, 3), None);
        assert_eq!(layout.grid_rows(), 3);
        assert_eq!(layout.grid_cols(), 3);
    }

    #[test]
    fn a_row_is_written_from_its_highest_column_down() {
        // Sixteen columns, so two bytes and no padding. Setting column 0
        // must light the *last* bit of the line, not the first.
        let mut map = DieBitmap::new(1, 16);
        assert!(map.set(0, 0));
        assert_eq!(map.row_bytes(0), vec![0x00, 0x01]);
        let mut map = DieBitmap::new(1, 16);
        assert!(map.set(0, 15));
        assert_eq!(map.row_bytes(0), vec![0x80, 0x00]);
    }

    #[test]
    fn the_padding_bits_are_ones_and_come_first() {
        // Twelve columns is one and a half bytes, so four padding bits.
        let mut map = DieBitmap::new(1, 12);
        assert_eq!(map.pad_bits(), 4);
        assert_eq!(map.row_bytes(0), vec![0xf0, 0x00]);
        map.set(0, 11);
        assert_eq!(map.row_bytes(0), vec![0xf8, 0x00]);
        map.set(0, 0);
        assert_eq!(map.row_bytes(0), vec![0xf8, 0x01]);
    }

    #[test]
    fn a_bitmap_round_trips_through_the_written_rows() {
        let mut map = DieBitmap::new(5, 13);
        for (row, col) in [(0u32, 0u32), (0, 12), (2, 7), (4, 1), (4, 11)] {
            assert!(map.set(row, col));
        }
        assert_eq!(map.count_ones(), 5);
        assert_eq!(map.rows_used(), 3);
        let rows = map.to_text_rows();
        assert_eq!(DieBitmap::from_text_rows(&rows, 13).unwrap(), map);
    }

    #[test]
    fn a_bit_outside_the_bitmap_is_reported_and_not_written() {
        let mut map = DieBitmap::new(2, 8);
        assert!(!map.set(2, 0));
        assert!(!map.set(0, 8));
        assert_eq!(map.count_ones(), 0);
        assert!(!map.get(2, 0));
    }

    #[test]
    fn padding_that_is_not_ones_is_refused_on_the_way_in() {
        // Twelve columns, four padding bits, and one of them zero.
        let err = DieBitmap::from_text_rows(&[vec![0xe0, 0x00]], 12).unwrap_err();
        assert_eq!(err, GowinError::BadPadding { row: 0 });
        let err = DieBitmap::from_text_rows(&[vec![0xf0]], 12).unwrap_err();
        assert_eq!(
            err,
            GowinError::RowWidth {
                row: 0,
                found: 8,
                wanted: 16
            }
        );
    }

    #[test]
    fn a_stream_round_trips_and_every_check_word_verifies() {
        let mut map = DieBitmap::new(6, 3376 / 8 * 8);
        map.set(0, 0);
        map.set(3, 1000);
        map.set(5, 3375);
        let stream = FsStream::new(header(), map.clone(), footer()).unwrap();
        // The row count reached the load-configuration command.
        assert_eq!(stream.header[9], vec![0x3b, 0x80, 0x00, 0x06]);
        assert_eq!(stream.idcode(), Some(0x0000_081b));
        let text = stream.to_text();
        assert_eq!(text.lines().count(), 10 + 6 + 6);
        assert_eq!(written_width(&text).unwrap(), 3376);
        let again = FsStream::parse(&text, 3376).unwrap();
        assert_eq!(again.bitmap, map);
        assert_eq!(again.header, stream.header);
        assert_eq!(again.footer, stream.footer);
    }

    #[test]
    fn a_corrupted_row_is_caught_by_its_check_word() {
        let stream = FsStream::new(header(), DieBitmap::new(2, 16), footer()).unwrap();
        let text = stream.to_text();
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        // Flip one bit of the first body row's data.
        let body = &mut lines[10];
        let flipped = body
            .char_indices()
            .map(|(i, c)| if i == 0 { '1' } else { c })
            .collect::<String>();
        *body = flipped;
        let broken = lines.join("\n");
        let err = FsStream::parse(&broken, 16).unwrap_err();
        assert!(matches!(err, GowinError::BadCrc { row: 0, .. }), "{err}");
    }

    #[test]
    fn the_idcode_check_is_what_stops_the_wrong_part() {
        let stream = FsStream::new(header(), DieBitmap::new(1, 8), footer()).unwrap();
        assert!(stream.check_idcode(0x0000_081b).is_ok());
        assert_eq!(
            stream.check_idcode(0x0900_281b).unwrap_err(),
            GowinError::IdcodeMismatch {
                wanted: 0x0900_281b,
                found: 0x0000_081b
            }
        );
    }

    #[test]
    fn a_usercode_reaches_the_footer() {
        let mut stream = FsStream::new(header(), DieBitmap::new(1, 8), footer()).unwrap();
        stream.set_usercode(0x0000_bf45).unwrap();
        assert_eq!(
            stream.footer[1],
            vec![0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0xbf, 0x45]
        );
    }

    #[test]
    fn the_substitute_bytes_expand_to_runs_of_zeros() {
        assert_eq!(
            expand(&[0x01, 0xaa, 0x02, 0x03], [0x01, 0x02, 0x03]),
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0xaa, 0, 0, 0, 0, 0, 0]
        );
        // A slot set to `0xff` means that run length was not substituted,
        // so a data byte of `0xff` stays a data byte.
        assert_eq!(expand(&[0xff, 0x01], [0xff, 0xff, 0x01]), vec![0xff, 0, 0]);
        // And a stream that could find no unused byte at all expands to
        // itself, which the format allows.
        assert_eq!(
            expand(&[0x00, 0x11, 0xff], [0xff, 0xff, 0xff]),
            vec![0x00, 0x11, 0xff]
        );
    }

    /// Replaces runs of zero bytes in `row` the way the format says,
    /// eight bytes at a time, so a test can build a compressed file.
    fn compress_row(row: &[u8], keys: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        for group in row.chunks(8) {
            let mut rest = group;
            while !rest.is_empty() {
                let zeros = rest.iter().take_while(|b| **b == 0).count();
                let run = if zeros >= 8 {
                    8
                } else if zeros >= 4 {
                    4
                } else if zeros >= 2 {
                    2
                } else {
                    0
                };
                match run {
                    8 => out.push(keys[0]),
                    4 => out.push(keys[1]),
                    2 => out.push(keys[2]),
                    _ => out.push(rest[0]),
                }
                rest = &rest[run.max(1)..];
            }
        }
        out
    }

    #[test]
    fn a_compressed_body_reads_back_as_the_same_bitmap() {
        // 120 columns, so a plain row is 15 bytes and a compressed one is
        // padded out to 128 bits — a whole eight-byte group — which is
        // the extra byte of padding ones the reader has to drop.
        const COLS: u32 = 120;
        // Three bytes that appear nowhere in this bitmap's rows, which is
        // the rule the format states and what keeps a key from being
        // mistaken for data.
        const KEYS: [u8; 3] = [0xaa, 0xbb, 0xcc];
        let mut map = DieBitmap::new(3, COLS);
        map.set(0, 0);
        map.set(1, 64);
        map.set(2, 119);

        // The header says compressed and names the three bytes.
        let mut head = header();
        head[4][6] |= 1 << 5;
        head[5][5] = KEYS[0];
        head[5][6] = KEYS[1];
        head[5][7] = KEYS[2];
        head[9][2] = 0x00;
        head[9][3] = 0x03;

        let mut covered: Vec<u8> = Vec::new();
        let mut text = String::new();
        for (i, line) in head.iter().enumerate() {
            if i >= PREAMBLE_LINES && line.first() != Some(&CMD_SPI_ADDRESS) {
                covered.extend_from_slice(line);
            }
            push_bits(&mut text, line);
            text.push('\n');
        }
        for row in 0..map.rows() {
            // One byte of padding ones in front, then the row as written.
            let mut padded = vec![0xffu8];
            padded.extend_from_slice(&map.row_bytes(row));
            let packed = compress_row(&padded, KEYS);
            covered.extend_from_slice(&packed);
            let crc = crc16_arc(&covered, 0);
            covered = vec![0xff; ROW_TAIL_BYTES];
            push_bits(&mut text, &packed);
            push_bits(&mut text, &[(crc & 0xff) as u8, (crc >> 8) as u8]);
            push_bits(&mut text, &[0xff; ROW_TAIL_BYTES]);
            text.push('\n');
        }
        for line in footer() {
            push_bits(&mut text, &line);
            text.push('\n');
        }

        let read = FsStream::parse(&text, COLS).unwrap();
        assert!(read.is_compressed());
        assert_eq!(read.bitmap, map);
    }

    #[test]
    fn a_file_that_is_not_bits_is_refused_with_the_line_and_the_character() {
        let err = FsStream::parse("0000000x\n", 8).unwrap_err();
        assert_eq!(
            err,
            GowinError::NotBinary {
                line: 1,
                found: 'x'
            }
        );
        let err = FsStream::parse("0000000\n", 8).unwrap_err();
        assert_eq!(err, GowinError::NotWholeBytes { line: 1, bits: 7 });
        // Comments and blank lines are skipped, which is what the vendor
        // tool's `//Tool Version:` line is.
        let err = FsStream::parse("// a comment\n\n", 8).unwrap_err();
        assert_eq!(err, GowinError::NoLoadConfig);
    }

    #[test]
    fn a_short_file_says_how_many_rows_are_missing() {
        let stream = FsStream::new(header(), DieBitmap::new(4, 8), footer()).unwrap();
        let text = stream.to_text();
        // Drop everything after the header and two body rows.
        let kept: Vec<&str> = text.lines().take(12).collect();
        let err = FsStream::parse(&kept.join("\n"), 8).unwrap_err();
        assert_eq!(
            err,
            GowinError::RowCount {
                declared: 4,
                found: 2
            }
        );
    }

    #[test]
    fn a_header_without_a_load_configuration_command_is_refused() {
        let mut short = header();
        short.pop();
        let err = FsStream::new(short, DieBitmap::new(1, 8), footer()).unwrap_err();
        assert_eq!(err, GowinError::NoLoadConfig);
    }
}
