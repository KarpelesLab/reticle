//! The Lattice ECP5 `.bit` bitstream container.
//!
//! # What this is, and what has been checked
//!
//! An ECP5 configuration file is a short metadata header followed by a
//! stream of **commands**, one of which carries the whole configuration
//! memory as a sequence of *frames*. This module writes that file and
//! reads it back.
//!
//! **What is established is byte-for-byte agreement with `ecppack`**,
//! Project Trellis' own packer, on three real bitstreams built for the
//! very part this module was written for — a Great Scott Gadgets Cynthion
//! r1.4's LFE5U-12F. These are the measurements, taken by
//! `tests/fpga_trellis.rs::the_reference_bitstreams_round_trip_byte_for_byte`,
//! which asserts every number in the table so none of them can drift:
//!
//! | File | Bytes | Frames | Set bits | Block RAM blocks |
//! |---|---|---|---|---|
//! | `analyzer.bit` | 238 282 | 7562 | 250 001 | 9 |
//! | `selftest.bit` | 112 303 | 7562 | 27 006 | 0 |
//! | `facedancer.bit` | 399 402 | 7562 | 424 160 | 44 |
//!
//! Each is read by [`Ecp5Stream::parse`] and written back by
//! [`Ecp5Stream::to_bytes`] **identical to the original, byte for byte**.
//! That covers the metadata header, the preamble, every command and its
//! operands, the control register, the compression dictionary *and the
//! rule that chooses it*, the compression encoding, all 7562 per-frame
//! check words, the reversed frame order, the bit packing inside a frame,
//! the security and SED padding, the USERCODE, the block-RAM
//! initialisation blocks with their nine-bytes-per-eight-words packing,
//! and the closing `ISC_PROGRAM_DONE`. `docs/fpga-trellis.md` says where
//! those three files come from and what the comparison does and does not
//! settle.
//!
//! It settles the **envelope**. It says nothing about whether the frames
//! inside carry a configuration that makes a part do anything; that is
//! [`super::trellis`]'s business.
//!
//! # The layout
//!
//! ```text
//! 0xff 0x00 <string> 0x00 ... 0xff   metadata, one NUL-terminated string each
//! 0xff 0xff 0xbd 0xb3                the preamble
//! 4 x 0xff                           padding
//! 0x3b 0x00 0x00 0x00                LSC_RESET_CRC   (and the check word starts here)
//! 0xe2 0x00 0x00 0x00 <idcode:4>     VERIFY_ID
//! 0x22 0x00 0x00 0x00 <ctrl0:4>      LSC_PROG_CNTRL0
//! 0x46 0x00 0x00 0x00                LSC_INIT_ADDRESS
//! 0x02 0x00 0x00 0x00 <dict:8>       LSC_WRITE_COMP_DIC     (compressed only)
//! 0xb8 0x91 <frames:2>               LSC_PROG_INCR_CMP      (compressed)
//! 0x82 0x91 <frames:2>               LSC_PROG_INCR_RTI      (uncompressed)
//!   <frame bytes> <crc:2> 0xff       ... one per frame, highest frame first
//! 12 x 0xff                          space for SECURITY and SED
//! 0xc2 0x80 0x00 0x00 <user:4> <crc:2>   ISC_PROGRAM_USERCODE
//! 0xf6 0x00 0x00 0x00 <addr:4>       LSC_EBR_ADDRESS        (per block RAM)
//! 0xb2 0xd0 <rows:2> <9 bytes>...<crc:2>  LSC_EBR_WRITE
//! 0x5e 0x00 0x00 0x00                ISC_PROGRAM_DONE
//! 4 x 0xff                           trailing padding
//! ```
//!
//! Three details are not guessable and each was measured against the
//! reference files.
//!
//! - **Frames are written from the last to the first.** Frame *i* of the
//!   stream is frame `num_frames - 1 - i` of the configuration memory.
//! - **A frame's bits are packed from its last byte upwards.** Bit *j* of
//!   a frame lands in byte `bytes_per_frame - 1 - (j + pad_after) / 8` at
//!   bit `(j + pad_after) % 8`, so a frame is a big-endian integer whose
//!   least significant bit is bit 0. Getting this backwards mirrors every
//!   frame and no structural check would notice.
//! - **The `0x91` operand byte is a bitfield**, not a magic number: bit 7
//!   asks for check words, bit 6 *clear* asks for one after every frame
//!   rather than one at the end, and the low nibble is how many dummy
//!   bytes follow each frame. `0x91` is "check words, per frame, one
//!   dummy byte", which is what an ECP5 wants.
//!
//! # The check word
//!
//! CRC-16/BUYPASS: width 16, polynomial `0x8005`, initial value zero, no
//! reflection and no final exclusive-or, **written high byte first**.
//! [`Crc16`] is the running accumulator, in the shape the stream needs it:
//! it covers the command stream from `LSC_RESET_CRC` onwards, is emitted
//! and **reset** after each frame, again after the USERCODE and again
//! after each block-RAM block. A command opcode of `0xff` is a dummy and
//! does not enter the CRC; every other byte does, including the frames'
//! trailing `0xff`.
//!
//! # Where a tile's bits live
//!
//! There is no per-tile bitmap and no frame *address*: a tile owns a
//! rectangle of the configuration memory given by a start frame and a
//! start bit, which is what [`TileWindow`] holds and [`Ecp5FrameMap`]
//! keys by grid position. That is the same division of labour
//! [`super::xc7::FrameMap`] has for the 7 series — the database knows the
//! part, this module knows the envelope.
//!
//! **A grid position may own several windows**, because Project Trellis
//! splits one position of the fabric into up to six tiles with separate
//! bit regions (the pad configuration of an IO in one, its IO logic in
//! another). The windows of a position are kept in a fixed order and a
//! [`ConfigBit`]'s `row` selects between them: window *k* claims the rows
//! from the frames of windows 0..*k* up to that plus its own. So a
//! combined tile type's bitmap is its members' frame counts laid end to
//! end, and nothing about the ECP5's several-tiles-per-position layout
//! reaches [`Arch`].
//!
//! [`Arch`]: super::arch::Arch
//!
//! # Compression
//!
//! A frame may be compressed with a prefix-free code over bytes: `0` is a
//! zero byte, `100xxx` a byte with only bit *xxx* set, `101xxx` entry
//! *xxx* of an eight-byte dictionary, and `11` followed by eight bits a
//! literal. The dictionary is **the eight most frequent bytes that are
//! neither zero nor a single set bit**, ordered by frequency and, for
//! equal frequencies, by the larger byte value; it is written most
//! significant entry first. Each frame is padded at the *front* with zero
//! bits to a multiple of 64 bits before encoding.
//!
//! Unlike [`super::gowin`], this module both reads and writes the
//! compressed form, and that is not a change of policy: the dictionary
//! rule above is not a guess, it is what reproduces `ecppack`'s own
//! output for all three reference files exactly, dictionary bytes
//! included. Compression costs the part nothing — it is the form the
//! board on this bench is configured with every time it is plugged in —
//! and it makes the stream four to five times shorter, which matters when
//! it has to be shifted through a JTAG port over a full-speed USB link.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use super::arch::ConfigBit;

/// A dummy byte. As a command opcode it is skipped and does **not** enter
/// the check word.
pub const CMD_DUMMY: u8 = 0xff;
/// Write the eight-byte compression dictionary.
pub const CMD_LSC_WRITE_COMP_DIC: u8 = 0x02;
/// Set control register 0.
pub const CMD_LSC_PROG_CNTRL0: u8 = 0x22;
/// Set control register 1. Not written for an ECP5.
pub const CMD_LSC_PROG_CNTRL1: u8 = 0x23;
/// Reset the running check word. The first command of the stream.
pub const CMD_LSC_RESET_CRC: u8 = 0x3b;
/// Set the write address to the start of the configuration memory.
pub const CMD_LSC_INIT_ADDRESS: u8 = 0x46;
/// Configuration is complete. The last command of the stream.
pub const CMD_ISC_PROGRAM_DONE: u8 = 0x5e;
/// Jump to an address in the configuration flash.
pub const CMD_JUMP: u8 = 0x7e;
/// Select how the part reads an external SPI flash.
pub const CMD_SPI_MODE: u8 = 0x79;
/// The configuration payload, uncompressed.
pub const CMD_LSC_PROG_INCR_RTI: u8 = 0x82;
/// Set the SED check word.
pub const CMD_LSC_PROG_SED_CRC: u8 = 0xa2;
/// Write block-RAM initialisation data.
pub const CMD_LSC_EBR_WRITE: u8 = 0xb2;
/// Set a single frame's write address, for a partial bitstream.
pub const CMD_LSC_WRITE_ADDRESS: u8 = 0xb4;
/// The configuration payload, compressed.
pub const CMD_LSC_PROG_INCR_CMP: u8 = 0xb8;
/// Set the USERCODE.
pub const CMD_ISC_PROGRAM_USERCODE: u8 = 0xc2;
/// Program the security bits. Not written here.
pub const CMD_ISC_PROGRAM_SECURITY: u8 = 0xce;
/// Check the part's JTAG IDCODE against the stream's.
pub const CMD_VERIFY_ID: u8 = 0xe2;
/// Set the block-RAM write address.
pub const CMD_LSC_EBR_ADDRESS: u8 = 0xf6;

/// The four bytes that mark the start of the command stream.
pub const PREAMBLE: [u8; 4] = [0xff, 0xff, 0xbd, 0xb3];

/// The operand byte of the payload command on an ECP5: check words on
/// (bit 7), one after every frame (bit 6 clear), one dummy byte after
/// each (the low nibble).
pub const CRC_META: u8 = 0x91;

/// How many `0xff` bytes stand between the payload and the USERCODE, the
/// space a SECURITY and an SED command would occupy.
pub const SECURITY_SED_SPACE: usize = 12;

/// How many `0xff` bytes follow the preamble.
pub const PREAMBLE_PADDING: usize = 4;

/// The control register 0 value `ecppack` writes with no options, before
/// any clock-frequency or multiboot bits are added to it.
pub const CTRL0_DEFAULT: u32 = 0x4000_0000;

/// The polynomial of the check word, in its unreflected form.
pub const CRC16_POLY: u16 = 0x8005;

/// How many nine-bit words one block-RAM initialisation block carries on
/// an ECP5: 2048, written as 256 rows of eight.
pub const BRAM_WORDS: usize = 2048;

/// Why an ECP5 bitstream could not be written or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ecp5Error {
    /// The file does not start with `0xff 0x00` (or `LSCC` and then
    /// that), so it is not a Lattice `.bit` file at all.
    NotABitFile,
    /// The metadata header has no closing `0xff`.
    UnterminatedMetadata,
    /// There is no preamble anywhere after the metadata.
    NoPreamble,
    /// The stream ended in the middle of a command.
    Truncated {
        /// What was being read.
        what: String,
        /// How far in, in bytes.
        offset: usize,
    },
    /// A command this module does not know.
    UnknownCommand {
        /// The opcode.
        opcode: u8,
        /// Where it was.
        offset: usize,
    },
    /// A check word did not match.
    BadCrc {
        /// What the bytes covered.
        what: String,
        /// What was computed.
        computed: u16,
        /// What the file says.
        expected: u16,
    },
    /// The payload arrived before `VERIFY_ID`, so there is no way to know
    /// how large a frame is.
    NoIdcode,
    /// A compressed payload arrived before its dictionary.
    NoDictionary,
    /// The stream's IDCODE is not the part's.
    IdcodeMismatch {
        /// What the caller says the part is.
        wanted: u32,
        /// What the stream says.
        found: u32,
    },
    /// The stream says it carries a different number of frames than the
    /// format does.
    FrameCount {
        /// What the stream says.
        found: u32,
        /// What the format says.
        wanted: u32,
    },
    /// A frame's bit count is not a whole number of bytes, which every
    /// part Project Trellis describes is.
    RaggedFrame {
        /// The bit count.
        bits: u32,
    },
}

impl fmt::Display for Ecp5Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ecp5Error::NotABitFile => write!(
                f,
                "a Lattice .bit file starts with 0xff 0x00, or with `LSCC` and then that"
            ),
            Ecp5Error::UnterminatedMetadata => {
                write!(f, "the metadata header has no closing 0xff")
            }
            Ecp5Error::NoPreamble => write!(
                f,
                "no preamble ({}) anywhere after the metadata",
                PREAMBLE
                    .iter()
                    .map(|b| format!("{b:#04x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Ecp5Error::Truncated { what, offset } => {
                write!(f, "the stream ends inside {what}, at byte {offset}")
            }
            Ecp5Error::UnknownCommand { opcode, offset } => {
                write!(f, "unknown command {opcode:#04x} at byte {offset}")
            }
            Ecp5Error::BadCrc {
                what,
                computed,
                expected,
            } => write!(
                f,
                "the check word over {what} is {computed:#06x} and the file says {expected:#06x}"
            ),
            Ecp5Error::NoIdcode => write!(
                f,
                "the configuration payload arrives before VERIFY_ID, so the frame size is unknown"
            ),
            Ecp5Error::NoDictionary => write!(
                f,
                "a compressed payload arrives before its compression dictionary"
            ),
            Ecp5Error::IdcodeMismatch { wanted, found } => write!(
                f,
                "this bitstream is for IDCODE {found:#010x} and the part is {wanted:#010x}"
            ),
            Ecp5Error::FrameCount { found, wanted } => write!(
                f,
                "the payload says {found} frame(s) and this part has {wanted}"
            ),
            Ecp5Error::RaggedFrame { bits } => {
                write!(f, "a frame of {bits} bit(s) is not a whole number of bytes")
            }
        }
    }
}

impl Error for Ecp5Error {}

/// The running check word of a stream.
///
/// Width 16, polynomial [`CRC16_POLY`] unreflected, initial value zero.
/// [`Crc16::update`] takes the message bits most significant first and
/// [`Crc16::finalise`] pushes the last sixteen out, which together are
/// the long division that a Lattice bitstream's check word is.
///
/// ```
/// use reticle::fpga::ecp5::Crc16;
///
/// // The check word of a single zero byte.
/// let mut crc = Crc16::new();
/// crc.update(0x00);
/// assert_eq!(crc.finalise(), 0x0000);
///
/// // And of the four preamble bytes.
/// let mut crc = Crc16::new();
/// for byte in reticle::fpga::ecp5::PREAMBLE {
///     crc.update(byte);
/// }
/// assert_eq!(crc.finalise(), 0x0d87);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Crc16 {
    value: u16,
}

impl Crc16 {
    /// A fresh accumulator, at zero.
    pub fn new() -> Crc16 {
        Crc16 { value: 0 }
    }

    /// Folds one byte in, most significant bit first.
    pub fn update(&mut self, byte: u8) {
        for i in (0..8).rev() {
            let overflow = self.value & 0x8000 != 0;
            self.value = (self.value << 1) | u16::from((byte >> i) & 1);
            if overflow {
                self.value ^= CRC16_POLY;
            }
        }
    }

    /// Folds every byte of `data` in.
    pub fn update_all(&mut self, data: &[u8]) {
        for byte in data {
            self.update(*byte);
        }
    }

    /// Pushes the last sixteen bits out and returns the check word. The
    /// accumulator is left holding it; [`Crc16::reset`] starts again.
    pub fn finalise(&mut self) -> u16 {
        for _ in 0..16 {
            let overflow = self.value & 0x8000 != 0;
            self.value <<= 1;
            if overflow {
                self.value ^= CRC16_POLY;
            }
        }
        self.value
    }

    /// Starts again from zero.
    pub fn reset(&mut self) {
        self.value = 0;
    }

    /// The check word over `data` alone.
    pub fn of(data: &[u8]) -> u16 {
        let mut crc = Crc16::new();
        crc.update_all(data);
        crc.finalise()
    }
}

/// How one part's configuration memory is shaped.
///
/// Every field comes from Project Trellis' `devices.json`; nothing here
/// is a constant for a particular part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameFormat {
    /// How many frames the part has.
    pub frames: u32,
    /// How many configuration bits one frame holds.
    pub bits_per_frame: u32,
    /// Padding bits written before a frame's data.
    pub pad_bits_before: u32,
    /// Padding bits written after it — which, because a frame is packed
    /// from its last byte upwards, shift the data *up*.
    pub pad_bits_after: u32,
}

impl FrameFormat {
    /// The format of a part with no padding bits.
    pub fn new(frames: u32, bits_per_frame: u32) -> FrameFormat {
        FrameFormat {
            frames,
            bits_per_frame,
            pad_bits_before: 0,
            pad_bits_after: 0,
        }
    }

    /// How many bytes one frame occupies in the stream.
    pub fn bytes_per_frame(&self) -> usize {
        let bits = self.bits_per_frame + self.pad_bits_before + self.pad_bits_after;
        (bits as usize).div_ceil(8)
    }

    /// How many bytes a *compressed* frame occupies before encoding: the
    /// same, rounded up to a multiple of eight, because the encoder pads
    /// a frame to a 64-bit boundary with zero bytes.
    pub fn padded_bytes_per_frame(&self) -> usize {
        let bpf = self.bytes_per_frame();
        if bpf == 0 {
            return 0;
        }
        bpf + (7 - ((bpf - 1) % 8))
    }

    /// How many configuration bits the whole part holds.
    pub fn bit_count(&self) -> u64 {
        u64::from(self.frames) * u64::from(self.bits_per_frame)
    }
}

/// One part's configuration memory: `frames` rows of `bits_per_frame`
/// bits.
///
/// Stored bit packed, eight bits to a byte with bit *j* of a frame at bit
/// `j % 8` of byte `j / 8`. That is an internal choice and not the
/// stream's packing; [`Cram::frame_bytes`] does that conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cram {
    format: FrameFormat,
    row_bytes: usize,
    data: Vec<u8>,
}

impl Cram {
    /// An all-zero configuration memory.
    pub fn new(format: FrameFormat) -> Cram {
        let row_bytes = (format.bits_per_frame as usize).div_ceil(8);
        let data = vec![0u8; row_bytes * format.frames as usize];
        Cram {
            format,
            row_bytes,
            data,
        }
    }

    /// The shape of this memory.
    pub fn format(&self) -> FrameFormat {
        self.format
    }

    /// How many frames it has.
    pub fn frames(&self) -> u32 {
        self.format.frames
    }

    /// How many bits one frame holds.
    pub fn bits_per_frame(&self) -> u32 {
        self.format.bits_per_frame
    }

    fn index(&self, frame: u32, bit: u32) -> Option<(usize, u8)> {
        if frame >= self.format.frames || bit >= self.format.bits_per_frame {
            return None;
        }
        let byte = usize::try_from(frame).ok()? * self.row_bytes + usize::try_from(bit / 8).ok()?;
        Some((byte, 1u8 << (bit % 8)))
    }

    /// Sets one bit. Returns false when it is outside the memory, which
    /// is how a bit that belongs to no tile is counted rather than
    /// silently written somewhere else.
    pub fn set(&mut self, frame: u32, bit: u32) -> bool {
        match self.index(frame, bit) {
            Some((byte, mask)) => {
                self.data[byte] |= mask;
                true
            }
            None => false,
        }
    }

    /// Clears one bit.
    pub fn clear(&mut self, frame: u32, bit: u32) -> bool {
        match self.index(frame, bit) {
            Some((byte, mask)) => {
                self.data[byte] &= !mask;
                true
            }
            None => false,
        }
    }

    /// Reads one bit; false for anything outside the memory.
    pub fn get(&self, frame: u32, bit: u32) -> bool {
        self.index(frame, bit)
            .is_some_and(|(byte, mask)| self.data[byte] & mask != 0)
    }

    /// How many bits are set.
    pub fn count_ones(&self) -> usize {
        self.data.iter().map(|b| b.count_ones() as usize).sum()
    }

    /// One frame as the stream writes it: `bytes_per_frame` bytes, packed
    /// from the last byte upwards.
    pub fn frame_bytes(&self, frame: u32) -> Vec<u8> {
        let bpf = self.format.bytes_per_frame();
        let mut out = vec![0u8; bpf];
        if bpf == 0 {
            return out;
        }
        for j in 0..self.format.bits_per_frame {
            if !self.get(frame, j) {
                continue;
            }
            let ofs = (j + self.format.pad_bits_after) as usize;
            let byte = bpf - 1 - ofs / 8;
            out[byte] |= 1u8 << (ofs % 8);
        }
        out
    }

    /// Takes one frame from the bytes the stream carries, which is the
    /// inverse of [`Cram::frame_bytes`].
    pub fn set_frame_bytes(&mut self, frame: u32, bytes: &[u8]) {
        let bpf = bytes.len();
        for j in 0..self.format.bits_per_frame {
            let ofs = (j + self.format.pad_bits_after) as usize;
            let Some(index) = bpf.checked_sub(1 + ofs / 8) else {
                continue;
            };
            let bit = (bytes[index] >> (ofs % 8)) & 1;
            if bit == 1 {
                self.set(frame, j);
            } else {
                self.clear(frame, j);
            }
        }
    }
}

/// One rectangle of the configuration memory that a tile owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileWindow {
    /// The first frame the tile owns.
    pub start_frame: u32,
    /// The first bit within each of those frames.
    pub start_bit: u32,
    /// How many consecutive frames.
    pub frames: u32,
    /// How many bits of each.
    pub bits: u32,
}

impl TileWindow {
    /// How many configuration bits the window holds.
    pub fn bit_count(&self) -> usize {
        self.frames as usize * self.bits as usize
    }

    /// The configuration memory position of the window's own bit
    /// `(frame, bit)`, or `None` when it falls outside.
    pub fn locate(&self, frame: u32, bit: u32) -> Option<(u32, u32)> {
        if frame >= self.frames || bit >= self.bits {
            return None;
        }
        Some((self.start_frame + frame, self.start_bit + bit))
    }
}

/// Which windows of the configuration memory each grid position owns.
///
/// Project Trellis splits one position of the ECP5's fabric into up to
/// six tiles with separate bit regions, so a position has a *list* of
/// windows in a fixed order, and a [`ConfigBit`]'s `row` addresses them
/// end to end: window 0 owns rows `0 .. frames(0)`, window 1 the next
/// `frames(1)`, and so on. That is what lets one [`Arch`] tile type stand
/// for a whole position.
///
/// [`Arch`]: super::arch::Arch
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ecp5FrameMap {
    tiles: HashMap<(u32, u32), Vec<TileWindow>>,
}

impl Ecp5FrameMap {
    /// An empty map.
    pub fn new() -> Ecp5FrameMap {
        Ecp5FrameMap::default()
    }

    /// Adds a window to the position at `(x, y)`, after the ones already
    /// there.
    pub fn push(&mut self, tile: (u32, u32), window: TileWindow) {
        self.tiles.entry(tile).or_default().push(window);
    }

    /// The windows of the position at `(x, y)`, in order.
    pub fn windows(&self, tile: (u32, u32)) -> &[TileWindow] {
        self.tiles.get(&tile).map_or(&[], Vec::as_slice)
    }

    /// How many positions have a window.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// True when no position has one.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// How many frame rows a position's combined bitmap has: its windows'
    /// frame counts added up.
    pub fn bit_rows(&self, tile: (u32, u32)) -> u32 {
        self.windows(tile).iter().map(|w| w.frames).sum()
    }

    /// How many columns it has: the widest window's bit count.
    pub fn bit_cols(&self, tile: (u32, u32)) -> u32 {
        self.windows(tile).iter().map(|w| w.bits).max().unwrap_or(0)
    }

    /// Where a tile bit lands in the configuration memory, or `None` when
    /// it belongs to no window of that position.
    pub fn locate(&self, tile: (u32, u32), bit: ConfigBit) -> Option<(u32, u32)> {
        let mut row = bit.row;
        for window in self.windows(tile) {
            if row < window.frames {
                return window.locate(row, bit.col);
            }
            row -= window.frames;
        }
        None
    }
}

/// One block RAM's initialisation data: nine-bit words, as the stream
/// carries them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BramBlock {
    /// Which block, as the `LSC_EBR_ADDRESS` command's high bits name it.
    pub index: u32,
    /// The nine-bit words, low nine bits of each entry used.
    pub words: Vec<u16>,
}

/// A whole ECP5 bitstream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ecp5Stream {
    /// The NUL-terminated strings of the metadata header, in order. What
    /// `ecppack` writes is one, `Part: <part>-<speed><package>`.
    pub metadata: Vec<String>,
    /// The IDCODE the `VERIFY_ID` command checks.
    pub idcode: u32,
    /// Control register 0.
    pub ctrl0: u32,
    /// The USERCODE.
    pub usercode: u32,
    /// The SPI mode byte, when the stream carries a `SPI_MODE` command.
    pub spi_mode: Option<u8>,
    /// The SED check word, when the stream carries one.
    pub sed: Option<u32>,
    /// The configuration memory.
    pub cram: Cram,
    /// The block-RAM initialisation blocks, in stream order.
    pub bram: Vec<BramBlock>,
    /// True when the payload that was *read* was compressed. It does not
    /// decide how [`Ecp5Stream::to_bytes`] writes one.
    pub was_compressed: bool,
}

impl Ecp5Stream {
    /// An empty stream for a part: no configuration bits, no block RAM,
    /// control register 0 at [`CTRL0_DEFAULT`].
    pub fn new(format: FrameFormat, idcode: u32) -> Ecp5Stream {
        Ecp5Stream {
            metadata: Vec::new(),
            idcode,
            ctrl0: CTRL0_DEFAULT,
            usercode: 0,
            spi_mode: None,
            sed: None,
            cram: Cram::new(format),
            bram: Vec::new(),
            was_compressed: false,
        }
    }

    /// Checks that this stream is for the part the caller has.
    ///
    /// # Errors
    ///
    /// [`Ecp5Error::IdcodeMismatch`] when the two differ. This is the
    /// check that stops a bitstream built for one ECP5 being shifted into
    /// another, which on this family matters more than it looks: the
    /// LFE5U-12F and the LFE5U-25F are the *same die* with different
    /// identifiers, so only the IDCODE tells them apart.
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

    /// The whole file, ready to write.
    ///
    /// `compress` chooses the payload command: with it the frames go
    /// through the prefix-free code described at the top of this module,
    /// which is four to five times shorter and is what `ecppack` writes
    /// with `--compress`; without it they go out as plain bytes.
    pub fn to_bytes(&self, compress: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(0xff);
        out.push(0x00);
        for string in &self.metadata {
            out.extend_from_slice(string.as_bytes());
            out.push(0x00);
        }
        out.push(0xff);

        let mut w = StreamWriter::new(&mut out);
        w.plain(&PREAMBLE);
        w.dummy(PREAMBLE_PADDING);
        if let Some(mode) = self.spi_mode {
            w.byte(CMD_SPI_MODE);
            w.byte(mode);
            w.zeros(2);
        }
        w.byte(CMD_LSC_RESET_CRC);
        w.zeros(3);
        w.reset_crc();
        w.byte(CMD_VERIFY_ID);
        w.zeros(3);
        w.u32(self.idcode);
        w.byte(CMD_LSC_PROG_CNTRL0);
        w.zeros(3);
        w.u32(self.ctrl0);
        w.byte(CMD_LSC_INIT_ADDRESS);
        w.zeros(3);

        // Frames go out from the last to the first.
        let frames: Vec<Vec<u8>> = (0..self.cram.frames())
            .map(|i| self.cram.frame_bytes(self.cram.frames() - 1 - i))
            .collect();
        let count = u16::try_from(frames.len()).unwrap_or(u16::MAX);
        if compress {
            let dictionary = compression_dictionary(&frames);
            w.byte(CMD_LSC_WRITE_COMP_DIC);
            w.zeros(3);
            for entry in dictionary.iter().rev() {
                w.byte(*entry);
            }
            w.byte(CMD_LSC_PROG_INCR_CMP);
            w.byte(CRC_META);
            w.byte((count >> 8) as u8);
            w.byte((count & 0xff) as u8);
            let padded = self.cram.format().padded_bytes_per_frame();
            for frame in &frames {
                let bits = compress_frame(frame, &dictionary, padded);
                for byte in bits {
                    w.byte(byte);
                }
                w.crc();
                w.byte(0xff);
            }
        } else {
            w.byte(CMD_LSC_PROG_INCR_RTI);
            w.byte(CRC_META);
            w.byte((count >> 8) as u8);
            w.byte((count & 0xff) as u8);
            for frame in &frames {
                for byte in frame {
                    w.byte(*byte);
                }
                w.crc();
                w.byte(0xff);
            }
        }

        w.dummy(SECURITY_SED_SPACE);
        w.byte(CMD_ISC_PROGRAM_USERCODE);
        w.byte(0x80);
        w.zeros(2);
        w.u32(self.usercode);
        w.crc();
        for block in &self.bram {
            w.byte(CMD_LSC_EBR_ADDRESS);
            w.zeros(3);
            w.u32(block.index << 11);
            w.byte(CMD_LSC_EBR_WRITE);
            w.byte(0xd0);
            let rows = u16::try_from(block.words.len() / 8).unwrap_or(u16::MAX);
            w.byte((rows >> 8) as u8);
            w.byte((rows & 0xff) as u8);
            for group in block.words.chunks(8) {
                for byte in pack_bram_row(group) {
                    w.byte(byte);
                }
            }
            w.crc();
        }
        w.byte(CMD_ISC_PROGRAM_DONE);
        w.zeros(3);
        w.dummy(4);
        out
    }

    /// Reads a `.bit` file.
    ///
    /// `formats` says, for an IDCODE, what shape that part's
    /// configuration memory has — the caller supplies it because the
    /// answer is in Project Trellis' `devices.json` and this module reads
    /// no files.
    ///
    /// # Errors
    ///
    /// The variants of [`Ecp5Error`]: a file that is not one, a command
    /// this module does not know, a check word that does not match, or a
    /// payload whose frame count is not the part's.
    pub fn parse(
        bytes: &[u8],
        formats: &dyn Fn(u32) -> Option<FrameFormat>,
    ) -> Result<Ecp5Stream, Ecp5Error> {
        let mut at = 0usize;
        if bytes.len() >= 4 && &bytes[0..4] == b"LSCC" {
            at = 4;
        }
        if bytes.len() < at + 2 || bytes[at] != 0xff || bytes[at + 1] != 0x00 {
            return Err(Ecp5Error::NotABitFile);
        }
        at += 2;
        let mut metadata = Vec::new();
        let mut current = Vec::new();
        loop {
            let Some(byte) = bytes.get(at).copied() else {
                return Err(Ecp5Error::UnterminatedMetadata);
            };
            at += 1;
            if byte == 0xff {
                break;
            }
            if byte == 0x00 {
                metadata.push(String::from_utf8_lossy(&current).into_owned());
                current.clear();
            } else {
                current.push(byte);
            }
        }
        let start = find_preamble(bytes, at).ok_or(Ecp5Error::NoPreamble)?;
        let mut r = StreamReader::new(bytes, start);

        let mut stream: Option<Ecp5Stream> = None;
        let mut dictionary: Option<[u8; 8]> = None;
        let mut spi_mode = None;
        let mut sed = None;
        let mut bram_index = 0u32;
        let mut bram: Vec<BramBlock> = Vec::new();
        let mut was_compressed = false;

        while !r.at_end() {
            let opcode = r.opcode()?;
            match opcode {
                CMD_DUMMY => {}
                CMD_LSC_RESET_CRC => {
                    r.skip(3, "LSC_RESET_CRC")?;
                    r.reset_crc();
                }
                CMD_VERIFY_ID => {
                    r.skip(3, "VERIFY_ID")?;
                    let idcode = r.u32("VERIFY_ID")?;
                    let format = formats(idcode).ok_or(Ecp5Error::IdcodeMismatch {
                        wanted: 0,
                        found: idcode,
                    })?;
                    stream = Some(Ecp5Stream::new(format, idcode));
                }
                CMD_LSC_PROG_CNTRL0 => {
                    r.skip(3, "LSC_PROG_CNTRL0")?;
                    let value = r.u32("LSC_PROG_CNTRL0")?;
                    if let Some(s) = stream.as_mut() {
                        s.ctrl0 = value;
                    }
                }
                CMD_LSC_PROG_CNTRL1 => {
                    r.skip(3, "LSC_PROG_CNTRL1")?;
                    r.u32("LSC_PROG_CNTRL1")?;
                }
                CMD_LSC_INIT_ADDRESS => r.skip(3, "LSC_INIT_ADDRESS")?,
                CMD_SPI_MODE => {
                    spi_mode = Some(r.byte("SPI_MODE")?);
                    r.skip(2, "SPI_MODE")?;
                }
                CMD_JUMP => {
                    r.skip(3, "JUMP")?;
                    r.skip(4, "JUMP address")?;
                }
                CMD_LSC_PROG_SED_CRC => {
                    r.skip(3, "LSC_PROG_SED_CRC")?;
                    sed = Some(r.u32("LSC_PROG_SED_CRC")?);
                }
                CMD_ISC_PROGRAM_SECURITY => r.skip(3, "ISC_PROGRAM_SECURITY")?,
                CMD_LSC_WRITE_COMP_DIC => {
                    let meta = r.byte("LSC_WRITE_COMP_DIC")?;
                    r.skip(2, "LSC_WRITE_COMP_DIC")?;
                    let mut entries = [0u8; 8];
                    for slot in entries.iter_mut().rev() {
                        *slot = r.byte("the compression dictionary")?;
                    }
                    if meta & 0x80 != 0 {
                        r.check_crc("the compression dictionary")?;
                    }
                    dictionary = Some(entries);
                }
                CMD_LSC_PROG_INCR_RTI | CMD_LSC_PROG_INCR_CMP => {
                    let compressed = opcode == CMD_LSC_PROG_INCR_CMP;
                    was_compressed |= compressed;
                    if compressed && dictionary.is_none() {
                        return Err(Ecp5Error::NoDictionary);
                    }
                    let s = stream.as_mut().ok_or(Ecp5Error::NoIdcode)?;
                    let meta = r.byte("the payload")?;
                    let high = r.byte("the payload")?;
                    let low = r.byte("the payload")?;
                    let count = (u32::from(high) << 8) | u32::from(low);
                    if count != s.cram.frames() {
                        return Err(Ecp5Error::FrameCount {
                            found: count,
                            wanted: s.cram.frames(),
                        });
                    }
                    let check = meta & 0x80 != 0;
                    let per_frame = check && meta & 0x40 == 0;
                    let dummies = usize::from(meta & 0x0f);
                    let format = s.cram.format();
                    let width = if compressed {
                        format.padded_bytes_per_frame()
                    } else {
                        format.bytes_per_frame()
                    };
                    for i in 0..count {
                        let index = count - 1 - i;
                        let frame = if compressed {
                            r.compressed(width, &dictionary.unwrap_or_default())?
                        } else {
                            r.take(width, "a frame")?
                        };
                        s.cram.set_frame_bytes(index, &frame);
                        if per_frame || (check && i + 1 == count) {
                            r.check_crc("a frame")?;
                        }
                        r.skip(dummies, "a frame's dummy bytes")?;
                    }
                }
                CMD_ISC_PROGRAM_USERCODE => {
                    let meta = r.byte("ISC_PROGRAM_USERCODE")?;
                    r.skip(2, "ISC_PROGRAM_USERCODE")?;
                    let value = r.u32("ISC_PROGRAM_USERCODE")?;
                    if meta & 0x80 != 0 {
                        r.check_crc("the USERCODE")?;
                    }
                    r.reset_crc();
                    if let Some(s) = stream.as_mut() {
                        s.usercode = value;
                    }
                }
                CMD_LSC_EBR_ADDRESS => {
                    r.skip(3, "LSC_EBR_ADDRESS")?;
                    bram_index = (r.u32("LSC_EBR_ADDRESS")? >> 11) & 0x3ff;
                }
                CMD_LSC_EBR_WRITE => {
                    let meta = r.byte("LSC_EBR_WRITE")?;
                    let high = r.byte("LSC_EBR_WRITE")?;
                    let low = r.byte("LSC_EBR_WRITE")?;
                    let rows = (usize::from(high) << 8) | usize::from(low);
                    let mut words = Vec::with_capacity(rows * 8);
                    for _ in 0..rows {
                        let row = r.take(9, "a block RAM row")?;
                        words.extend_from_slice(&unpack_bram_row(&row));
                    }
                    if meta & 0x80 != 0 {
                        r.check_crc("a block RAM block")?;
                    }
                    bram.push(BramBlock {
                        index: bram_index,
                        words,
                    });
                }
                CMD_ISC_PROGRAM_DONE => {
                    let meta = r.byte("ISC_PROGRAM_DONE")?;
                    r.skip(2, "ISC_PROGRAM_DONE")?;
                    if meta & 0x80 != 0 {
                        r.check_crc("ISC_PROGRAM_DONE")?;
                    }
                }
                other => {
                    return Err(Ecp5Error::UnknownCommand {
                        opcode: other,
                        offset: r.offset() - 1,
                    });
                }
            }
        }
        let mut stream = stream.ok_or(Ecp5Error::NoIdcode)?;
        stream.metadata = metadata;
        stream.spi_mode = spi_mode;
        stream.sed = sed;
        stream.bram = bram;
        stream.was_compressed = was_compressed;
        Ok(stream)
    }
}

/// The first position at or after `from` where the preamble sits.
fn find_preamble(bytes: &[u8], from: usize) -> Option<usize> {
    bytes
        .windows(PREAMBLE.len())
        .enumerate()
        .skip(from)
        .find(|(_, w)| *w == PREAMBLE)
        .map(|(i, _)| i + PREAMBLE.len())
}

/// The eight dictionary entries for a set of frames.
///
/// The bytes that are neither zero nor a single set bit have their own
/// encodings, so the dictionary is chosen from the rest: the eight most
/// frequent, and for equal frequency the larger byte value first. That
/// tie-break is not decoration — it is what `ecppack`'s priority queue
/// does, and it is why the dictionary this writes for the three reference
/// bitstreams is theirs, byte for byte.
pub fn compression_dictionary(frames: &[Vec<u8>]) -> [u8; 8] {
    let mut histogram = [0usize; 256];
    for frame in frames {
        for byte in frame {
            histogram[*byte as usize] += 1;
        }
    }
    let mut candidates: Vec<(usize, u8)> = (0..=255u8)
        .filter(|b| *b != 0 && one_hot(*b).is_none())
        .map(|b| (histogram[b as usize], b))
        .collect();
    candidates.sort_by(|a, b| b.cmp(a));
    let mut out = [0u8; 8];
    for (slot, (_, byte)) in out.iter_mut().zip(candidates) {
        *slot = byte;
    }
    out
}

/// Which bit is set, for a byte with exactly one.
fn one_hot(byte: u8) -> Option<u8> {
    if byte.count_ones() == 1 {
        // A `u8` has at most eight trailing zeros, so this never narrows.
        u8::try_from(byte.trailing_zeros()).ok()
    } else {
        None
    }
}

/// One frame under the prefix-free code, padded at the front to
/// `padded_width` bytes.
pub fn compress_frame(frame: &[u8], dictionary: &[u8; 8], padded_width: usize) -> Vec<u8> {
    let mut bits = BitWriter::default();
    for _ in frame.len()..padded_width {
        bits.bit(false);
    }
    for byte in frame {
        if *byte == 0 {
            bits.bit(false);
            continue;
        }
        if let Some(index) = one_hot(*byte) {
            bits.bits(0b100, 3);
            bits.bits(u32::from(index), 3);
            continue;
        }
        if let Some(index) = dictionary.iter().position(|d| d == byte) {
            bits.bits(0b101, 3);
            bits.bits(u32::try_from(index).unwrap_or(0), 3);
            continue;
        }
        bits.bits(0b11, 2);
        bits.bits(u32::from(*byte), 8);
    }
    bits.finish()
}

/// Nine bytes holding eight nine-bit block-RAM words.
fn pack_bram_row(words: &[u16]) -> [u8; 9] {
    let w = |i: usize| -> u32 { u32::from(words.get(i).copied().unwrap_or(0)) };
    let byte = |v: u32| -> u8 { (v & 0xff) as u8 };
    [
        byte(w(0) >> 1),
        byte(((w(0) & 0x01) << 7) | (w(1) >> 2)),
        byte(((w(1) & 0x03) << 6) | (w(2) >> 3)),
        byte(((w(2) & 0x07) << 5) | (w(3) >> 4)),
        byte(((w(3) & 0x0f) << 4) | (w(4) >> 5)),
        byte(((w(4) & 0x1f) << 3) | (w(5) >> 6)),
        byte(((w(5) & 0x3f) << 2) | (w(6) >> 7)),
        byte(((w(6) & 0x7f) << 1) | (w(7) >> 8)),
        byte(w(7)),
    ]
}

/// The eight nine-bit words nine bytes hold.
fn unpack_bram_row(row: &[u8]) -> [u16; 8] {
    let b = |i: usize| -> u16 { u16::from(row.get(i).copied().unwrap_or(0)) };
    [
        (b(0) << 1) | (b(1) >> 7),
        ((b(1) & 0x7f) << 2) | (b(2) >> 6),
        ((b(2) & 0x3f) << 3) | (b(3) >> 5),
        ((b(3) & 0x1f) << 4) | (b(4) >> 4),
        ((b(4) & 0x0f) << 5) | (b(5) >> 3),
        ((b(5) & 0x07) << 6) | (b(6) >> 2),
        ((b(6) & 0x03) << 7) | (b(7) >> 1),
        ((b(7) & 0x01) << 8) | b(8),
    ]
}

/// A most-significant-bit-first bit stream, flushed to whole bytes.
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    byte: u8,
    filled: u32,
}

impl BitWriter {
    fn bit(&mut self, set: bool) {
        if set {
            self.byte |= 1 << (7 - self.filled);
        }
        self.filled += 1;
        if self.filled == 8 {
            self.out.push(self.byte);
            self.byte = 0;
            self.filled = 0;
        }
    }

    fn bits(&mut self, value: u32, len: u32) {
        for i in (0..len).rev() {
            self.bit((value >> i) & 1 == 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.filled != 0 {
            self.out.push(self.byte);
        }
        self.out
    }
}

/// Bytes out, with the running check word.
struct StreamWriter<'a> {
    out: &'a mut Vec<u8>,
    crc: Crc16,
}

impl<'a> StreamWriter<'a> {
    fn new(out: &'a mut Vec<u8>) -> StreamWriter<'a> {
        StreamWriter {
            out,
            crc: Crc16::new(),
        }
    }

    fn byte(&mut self, byte: u8) {
        self.out.push(byte);
        self.crc.update(byte);
    }

    fn plain(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.byte(*byte);
        }
    }

    fn zeros(&mut self, count: usize) {
        for _ in 0..count {
            self.byte(0);
        }
    }

    /// Dummy bytes, which do **not** enter the check word.
    fn dummy(&mut self, count: usize) {
        for _ in 0..count {
            self.out.push(0xff);
        }
    }

    fn u32(&mut self, value: u32) {
        for byte in value.to_be_bytes() {
            self.byte(byte);
        }
    }

    fn reset_crc(&mut self) {
        self.crc.reset();
    }

    /// Writes the check word, high byte first, and starts again.
    fn crc(&mut self) {
        let value = self.crc.finalise();
        self.byte((value >> 8) as u8);
        self.byte((value & 0xff) as u8);
        self.crc.reset();
    }
}

/// Bytes in, with the running check word.
struct StreamReader<'a> {
    data: &'a [u8],
    at: usize,
    crc: Crc16,
}

impl<'a> StreamReader<'a> {
    fn new(data: &'a [u8], at: usize) -> StreamReader<'a> {
        StreamReader {
            data,
            at,
            crc: Crc16::new(),
        }
    }

    fn at_end(&self) -> bool {
        self.at >= self.data.len()
    }

    fn offset(&self) -> usize {
        self.at
    }

    /// A command opcode. A dummy does not enter the check word, which is
    /// the one place the two differ.
    fn opcode(&mut self) -> Result<u8, Ecp5Error> {
        let byte = *self.data.get(self.at).ok_or(Ecp5Error::Truncated {
            what: "a command".to_owned(),
            offset: self.at,
        })?;
        self.at += 1;
        if byte != CMD_DUMMY {
            self.crc.update(byte);
        }
        Ok(byte)
    }

    fn byte(&mut self, what: &str) -> Result<u8, Ecp5Error> {
        let byte = *self.data.get(self.at).ok_or_else(|| Ecp5Error::Truncated {
            what: what.to_owned(),
            offset: self.at,
        })?;
        self.at += 1;
        self.crc.update(byte);
        Ok(byte)
    }

    fn skip(&mut self, count: usize, what: &str) -> Result<(), Ecp5Error> {
        for _ in 0..count {
            self.byte(what)?;
        }
        Ok(())
    }

    fn take(&mut self, count: usize, what: &str) -> Result<Vec<u8>, Ecp5Error> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(self.byte(what)?);
        }
        Ok(out)
    }

    fn u32(&mut self, what: &str) -> Result<u32, Ecp5Error> {
        let bytes = self.take(4, what)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn reset_crc(&mut self) {
        self.crc.reset();
    }

    fn check_crc(&mut self, what: &str) -> Result<(), Ecp5Error> {
        let computed = self.crc.finalise();
        let high = self.byte(what)?;
        let low = self.byte(what)?;
        let expected = (u16::from(high) << 8) | u16::from(low);
        self.crc.reset();
        if computed == expected {
            Ok(())
        } else {
            Err(Ecp5Error::BadCrc {
                what: what.to_owned(),
                computed,
                expected,
            })
        }
    }

    /// `count` bytes out of the prefix-free code.
    fn compressed(&mut self, count: usize, dictionary: &[u8; 8]) -> Result<Vec<u8>, Ecp5Error> {
        let mut out = Vec::with_capacity(count);
        let mut held: u32 = 0;
        let mut bits: u32 = 0;
        let next = |me: &mut Self, held: &mut u32, bits: &mut u32| -> Result<bool, Ecp5Error> {
            if *bits == 0 {
                *held = u32::from(me.byte("a compressed frame")?);
                *bits = 8;
            }
            *bits -= 1;
            Ok((*held >> *bits) & 1 == 1)
        };
        for _ in 0..count {
            let byte = if next(self, &mut held, &mut bits)? {
                if bits < 5 {
                    held = (held << 8) | u32::from(self.byte("a compressed frame")?);
                    bits += 8;
                }
                bits -= 1;
                if (held >> bits) & 1 == 1 {
                    // 11 xxxxxxxx: a literal byte.
                    if bits < 8 {
                        held = (held << 8) | u32::from(self.byte("a compressed frame")?);
                        bits += 8;
                    }
                    bits -= 8;
                    ((held >> bits) & 0xff) as u8
                } else {
                    bits -= 1;
                    let from_dictionary = (held >> bits) & 1 == 1;
                    bits -= 3;
                    let index = ((held >> bits) & 0x7) as usize;
                    if from_dictionary {
                        dictionary[index]
                    } else {
                        1u8 << index
                    }
                }
            } else {
                0
            };
            out.push(byte);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check word is CRC-16/BUYPASS, which the standard byte-at-a-time
    /// formulation must agree with — the stream's version appends sixteen
    /// zero bits at the end instead of pre-shifting, and those are the same
    /// long division.
    #[test]
    fn the_check_word_is_crc16_buypass() {
        fn table_driven(data: &[u8]) -> u16 {
            let mut crc = 0u16;
            for byte in data {
                crc ^= u16::from(*byte) << 8;
                for _ in 0..8 {
                    crc = if crc & 0x8000 != 0 {
                        (crc << 1) ^ CRC16_POLY
                    } else {
                        crc << 1
                    };
                }
            }
            crc
        }
        for case in [
            &b""[..],
            &b"\x00"[..],
            &b"\xff"[..],
            &b"123456789"[..],
            &PREAMBLE[..],
            &[0x3b, 0, 0, 0, 0xe2, 0, 0, 0, 0x21, 0x11, 0x10, 0x43][..],
        ] {
            assert_eq!(Crc16::of(case), table_driven(case), "over {case:02x?}");
        }
        // The check value every CRC catalogue lists for this algorithm.
        assert_eq!(Crc16::of(b"123456789"), 0xfee8);
    }

    #[test]
    fn a_frame_is_packed_from_its_last_byte_upwards() {
        let format = FrameFormat::new(2, 16);
        assert_eq!(format.bytes_per_frame(), 2);
        let mut cram = Cram::new(format);
        assert!(cram.set(0, 0));
        assert_eq!(cram.frame_bytes(0), vec![0x00, 0x01]);
        assert!(cram.set(0, 15));
        assert_eq!(cram.frame_bytes(0), vec![0x80, 0x01]);
        assert!(!cram.set(0, 16));
        assert!(!cram.set(2, 0));
        assert_eq!(cram.count_ones(), 2);

        // And back again.
        let mut other = Cram::new(format);
        other.set_frame_bytes(0, &[0x80, 0x01]);
        assert_eq!(other.frame_bytes(0), cram.frame_bytes(0));
        assert!(other.get(0, 0) && other.get(0, 15) && !other.get(0, 1));
        assert!(other.clear(0, 0));
        assert!(!other.get(0, 0));
    }

    /// The LFE5U-45F is the one ECP5 with padding bits before a frame, so
    /// its frame is 106 bytes for 846 bits.
    #[test]
    fn padding_bits_widen_a_frame_without_widening_the_memory() {
        let format = FrameFormat {
            frames: 9470,
            bits_per_frame: 846,
            pad_bits_before: 2,
            pad_bits_after: 0,
        };
        assert_eq!(format.bytes_per_frame(), 106);
        assert_eq!(format.padded_bytes_per_frame(), 112);
        assert_eq!(format.bit_count(), 9470 * 846);

        // The 12F and 25F have none, and their frame is exactly 74 bytes.
        let twelve = FrameFormat::new(7562, 592);
        assert_eq!(twelve.bytes_per_frame(), 74);
        assert_eq!(twelve.padded_bytes_per_frame(), 80);
    }

    #[test]
    fn a_window_list_addresses_a_positions_tiles_end_to_end() {
        let mut map = Ecp5FrameMap::new();
        map.push(
            (7, 0),
            TileWindow {
                start_frame: 100,
                start_bit: 0,
                frames: 3,
                bits: 1,
            },
        );
        map.push(
            (7, 0),
            TileWindow {
                start_frame: 100,
                start_bit: 12,
                frames: 2,
                bits: 4,
            },
        );
        assert_eq!(map.len(), 2 - 1);
        assert!(!map.is_empty());
        assert_eq!(map.bit_rows((7, 0)), 5);
        assert_eq!(map.bit_cols((7, 0)), 4);
        // Rows 0..3 are the first window, 3..5 the second.
        assert_eq!(map.locate((7, 0), ConfigBit::new(0, 0)), Some((100, 0)));
        assert_eq!(map.locate((7, 0), ConfigBit::new(2, 0)), Some((102, 0)));
        assert_eq!(map.locate((7, 0), ConfigBit::new(3, 3)), Some((100, 15)));
        assert_eq!(map.locate((7, 0), ConfigBit::new(4, 0)), Some((101, 12)));
        // A column the window it lands in has not got.
        assert_eq!(map.locate((7, 0), ConfigBit::new(0, 1)), None);
        assert_eq!(map.locate((7, 0), ConfigBit::new(5, 0)), None);
        assert!(map.windows((0, 0)).is_empty());
        assert_eq!(map.windows((7, 0))[0].bit_count(), 3);
    }

    /// A stream with nothing in it writes and reads back, and the reader
    /// checks every word the writer wrote.
    #[test]
    fn an_empty_stream_round_trips_both_ways() {
        let format = FrameFormat::new(8, 32);
        let mut stream = Ecp5Stream::new(format, 0x2111_1043);
        stream.metadata.push("Part: LFE5U-12F-8CABGA256".to_owned());
        stream.cram.set(3, 5);
        stream.cram.set(0, 31);
        for compress in [false, true] {
            let bytes = stream.to_bytes(compress);
            let back = Ecp5Stream::parse(&bytes, &|id| (id == 0x2111_1043).then_some(format))
                .unwrap_or_else(|err| panic!("compress={compress}: {err}"));
            assert_eq!(back.metadata, stream.metadata);
            assert_eq!(back.idcode, stream.idcode);
            assert_eq!(back.ctrl0, CTRL0_DEFAULT);
            assert_eq!(back.cram, stream.cram);
            assert_eq!(back.was_compressed, compress);
            assert!(back.check_idcode(0x2111_1043).is_ok());
            assert_eq!(
                back.check_idcode(0x4111_1043),
                Err(Ecp5Error::IdcodeMismatch {
                    wanted: 0x4111_1043,
                    found: 0x2111_1043
                })
            );
        }
    }

    #[test]
    fn block_ram_words_survive_the_nine_byte_packing() {
        let words = [
            0x1ff,
            0x000,
            0x0aa,
            0x155,
            0x001,
            0x100,
            0x0ff,
            0x123 & 0x1ff,
        ];
        let packed = pack_bram_row(&words);
        assert_eq!(unpack_bram_row(&packed), words);

        let format = FrameFormat::new(4, 16);
        let mut stream = Ecp5Stream::new(format, 0x2111_1043);
        stream.bram.push(BramBlock {
            index: 3,
            words: (0..16u16).map(|i| i * 31 % 512).collect(),
        });
        let bytes = stream.to_bytes(false);
        let back = Ecp5Stream::parse(&bytes, &|_| Some(format)).expect("round trip");
        assert_eq!(back.bram, stream.bram);
    }

    #[test]
    fn a_file_that_is_not_one_is_refused_by_name() {
        let format = FrameFormat::new(2, 16);
        let formats = |_: u32| Some(format);
        assert_eq!(
            Ecp5Stream::parse(b"nope", &formats),
            Err(Ecp5Error::NotABitFile)
        );
        assert_eq!(
            Ecp5Stream::parse(b"\xff\x00abc", &formats),
            Err(Ecp5Error::UnterminatedMetadata)
        );
        assert_eq!(
            Ecp5Stream::parse(b"\xff\x00\xff\x01\x02", &formats),
            Err(Ecp5Error::NoPreamble)
        );
        // A command nothing knows, right after the preamble.
        let bad = b"\xff\x00\xff\xff\xff\xbd\xb3\x77\x00\x00\x00";
        assert!(matches!(
            Ecp5Stream::parse(bad, &formats),
            Err(Ecp5Error::UnknownCommand { opcode: 0x77, .. })
        ));
        // Every error says something.
        for err in [
            Ecp5Error::NotABitFile,
            Ecp5Error::UnterminatedMetadata,
            Ecp5Error::NoPreamble,
            Ecp5Error::NoIdcode,
            Ecp5Error::NoDictionary,
            Ecp5Error::Truncated {
                what: "x".to_owned(),
                offset: 1,
            },
            Ecp5Error::UnknownCommand {
                opcode: 1,
                offset: 2,
            },
            Ecp5Error::BadCrc {
                what: "x".to_owned(),
                computed: 1,
                expected: 2,
            },
            Ecp5Error::IdcodeMismatch {
                wanted: 1,
                found: 2,
            },
            Ecp5Error::FrameCount {
                found: 1,
                wanted: 2,
            },
            Ecp5Error::RaggedFrame { bits: 3 },
        ] {
            assert!(!err.to_string().is_empty());
        }
    }

    /// A check word the file gets wrong is an error and not a warning: a
    /// stream that has been corrupted in the middle configures something,
    /// and what it configures is not what anyone asked for.
    #[test]
    fn a_wrong_check_word_is_refused() {
        let format = FrameFormat::new(2, 16);
        let stream = Ecp5Stream::new(format, 0x2111_1043);
        let mut bytes = stream.to_bytes(false);
        // Flip a bit in the first frame's data, leaving its check word.
        let payload = bytes
            .windows(4)
            .position(|w| w[0] == CMD_LSC_PROG_INCR_RTI && w[1] == CRC_META)
            .expect("the payload command is there");
        bytes[payload + 4] ^= 1;
        assert!(matches!(
            Ecp5Stream::parse(&bytes, &|_| Some(format)),
            Err(Ecp5Error::BadCrc { .. })
        ));
    }

    /// The dictionary is the eight most frequent bytes that have no
    /// encoding of their own, ties going to the larger value.
    #[test]
    fn the_dictionary_skips_zero_and_the_single_bit_bytes() {
        let frames = vec![
            vec![0u8; 100],
            vec![0x01; 50],
            vec![0x03; 40],
            vec![0xc0; 40],
        ];
        let dict = compression_dictionary(&frames);
        assert!(!dict.contains(&0));
        assert!(!dict.contains(&0x01));
        // 0x03 and 0xc0 both appear 40 times; the larger value comes first.
        assert_eq!(dict[0], 0xc0);
        assert_eq!(dict[1], 0x03);
        assert_eq!(one_hot(0x80), Some(7));
        assert_eq!(one_hot(0x03), None);
    }

    #[test]
    fn a_compressed_frame_decodes_to_what_it_encoded() {
        let dict = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        for frame in [
            vec![0u8; 8],
            vec![0x01, 0x00, 0x11, 0xab, 0x80, 0x22, 0xff, 0x00],
            (0..16u8).map(|i| i.wrapping_mul(37)).collect::<Vec<u8>>(),
        ] {
            let padded = frame.len() + (7 - ((frame.len() - 1) % 8));
            let encoded = compress_frame(&frame, &dict, padded);
            let mut wrapped = Vec::new();
            wrapped.extend_from_slice(&encoded);
            let mut r = StreamReader::new(&wrapped, 0);
            let back = r.compressed(padded, &dict).expect("decodes");
            assert_eq!(&back[padded - frame.len()..], &frame[..]);
            assert!(back[..padded - frame.len()].iter().all(|b| *b == 0));
        }
    }
}
