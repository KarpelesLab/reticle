//! The Xilinx 7-series configuration bitstream: frames, packets and the
//! `.bit` container.
//!
//! # Where every number in this file comes from
//!
//! This module is written from **public documentation**, not from
//! reverse engineering, and the split matters enough to spell out line
//! by line.
//!
//! | From Xilinx UG470, *7 Series FPGAs Configuration User Guide* | From the database |
//! |---|---|
//! | the sync word `0xAA995566` and the bus-width detect pattern | — |
//! | the type 1 and type 2 packet headers, their opcodes and word counts | — |
//! | the configuration register numbers ([`Register`]) | — |
//! | the `CMD` register's commands ([`Command`]) | — |
//! | the frame address register's fields ([`FrameAddress`]) | — |
//! | 101 32-bit words per frame on 7-series | — |
//! | the CRC is CRC-32C over `{register[4:0], data[31:0]}`, LSB first | — |
//! | the `.bit` wrapper's keyed fields (`a` design, `b` part, `c` date, `d` time, `e` data) | — |
//! | — | which frames a part has, and how many ([`FrameLayout`], from `part.json`) |
//! | — | the part's IDCODE (from `part.json`) |
//! | — | which frame and bit a tile's configuration bits land in ([`TileBits`], from `tilegrid.json`) |
//!
//! Two things are neither: the **two zero pad frames after each row** of
//! each configuration bus, and the exact command sequence and
//! register values [`write_bit`] emits. Those were *measured* against the
//! reference bitstream Project X-Ray ships in `artix7/harness/`, which
//! Vivado produced for this very part;
//! `the_container_matches_the_vivado_harness` in `tests/fpga_xray.rs` is
//! that comparison written down. UG470
//! describes each register and each command, but not the order a
//! particular tool writes them in, and a configuration engine is picky
//! about the order.
//!
//! # NOTHING HERE HAS BEEN LOADED INTO A PART
//!
//! The container is structurally right — the sync word, the packets, the
//! frame addresses, the frame count and both CRCs agree with a bitstream
//! Vivado made for an XC7A35T — and no output of this crate has ever
//! been sent down a JTAG cable. A bitstream that parses is not a
//! bitstream that configures: what the frames *contain* comes from
//! [`super::xray`] and from the placer and router above it, and their
//! correctness is not established by anything in this module.
//!
//! # The model
//!
//! A part's configuration memory is a list of **frames**, each 101 words
//! of 32 bits, each with a [`FrameAddress`]. [`FrameLayout`] is the list,
//! in the order the configuration engine consumes it when the frame
//! address register is set once to zero and every frame is streamed
//! through `FDRI`. [`FrameData`] is the contents.
//!
//! A tile's bits reach a frame through [`TileBits`], which is
//! `tilegrid.json`'s `baseaddr` / `frames` / `offset` / `words` for that
//! tile: configuration bit `(row, col)` of the tile — the same
//! [`ConfigBit`] the architecture hands out — is frame `baseaddr + row`,
//! word `offset + col / 32`, bit `col % 32`.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use super::arch::ConfigBit;
use super::bitstream::Bitstream;

/// 32-bit words in one 7-series configuration frame (UG470, table 1-1).
pub const WORDS_PER_FRAME: usize = 101;

/// The synchronisation word that starts the configuration packet stream
/// (UG470, "Bitstream Composition").
pub const SYNC_WORD: u32 = 0xAA99_5566;

/// The bus-width auto-detect pattern that precedes the sync word.
pub const BUS_WIDTH_PATTERN: [u32; 2] = [0x0000_00BB, 0x1122_0044];

/// The CRC-32C generator polynomial, bit-reversed for an LSB-first
/// shift register. The unreflected polynomial is `0x1EDC6F41`.
const CRC32C_REVERSED: u32 = 0x82F6_3B78;

/// Zero frames written after the last frame of each row of each
/// configuration bus.
///
/// **Not from UG470.** Measured from the Vivado-produced reference
/// bitstream in Project X-Ray's `artix7/harness/`: that stream is 5420
/// frames where `part.json` describes 5408, and the twelve extra are two
/// all-zero frames at the end of each of the six (bus, half, row)
/// groups.
pub const PAD_FRAMES_PER_ROW: usize = 2;

/// Why a 7-series bitstream could not be built or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xc7Error {
    /// A frame address the part's layout does not have.
    NoSuchFrame {
        /// The address that is not in the layout.
        address: FrameAddress,
    },
    /// A word or bit outside a frame.
    OutOfFrame {
        /// The frame.
        address: FrameAddress,
        /// The word index asked for.
        word: usize,
    },
    /// A tile's bits reach outside the tile's own frame window.
    OutOfTile {
        /// The tile whose window it is.
        tile: (u32, u32),
        /// The bit that does not fit.
        bit: ConfigBit,
    },
    /// The database and the requested part disagree about the IDCODE.
    IdcodeMismatch {
        /// What the flow was asked for.
        wanted: u32,
        /// What the database says.
        found: u32,
    },
    /// The file is not a `.bit`, or is truncated.
    Malformed(String),
    /// A CRC in the file does not match the data before it.
    Crc {
        /// The value the file carries.
        expected: u32,
        /// The value the data implies.
        computed: u32,
    },
}

impl fmt::Display for Xc7Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Xc7Error::NoSuchFrame { address } => {
                write!(f, "the part has no frame at {address}")
            }
            Xc7Error::OutOfFrame { address, word } => write!(
                f,
                "word {word} is outside the {WORDS_PER_FRAME}-word frame at {address}"
            ),
            Xc7Error::OutOfTile { tile, bit } => write!(
                f,
                "bit {}.{} is outside the frame window of the tile at ({}, {})",
                bit.row, bit.col, tile.0, tile.1
            ),
            Xc7Error::IdcodeMismatch { wanted, found } => write!(
                f,
                "this database is for IDCODE {found:#010x}, the part asked for is {wanted:#010x}"
            ),
            Xc7Error::Malformed(message) => f.write_str(message),
            Xc7Error::Crc { expected, computed } => write!(
                f,
                "the bitstream's CRC is {expected:#010x}, the data before it gives {computed:#010x}"
            ),
        }
    }
}

impl Error for Xc7Error {}

/// One 7-series frame address, as the frame address register holds it.
///
/// UG470, "Frame Address Register Description": block type in bits
/// 25:23, top/bottom in bit 22, row in bits 21:17, column in bits 16:7
/// and minor (the frame within the column) in bits 6:0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameAddress {
    /// The block type: 0 is `CLB_IO_CLK`, 1 is `BLOCK_RAM`.
    pub block: u8,
    /// 0 for the top half of the die, 1 for the bottom.
    pub half: u8,
    /// The clock region row within that half.
    pub row: u8,
    /// The configuration column.
    pub column: u16,
    /// The frame within the column.
    pub minor: u8,
}

impl FrameAddress {
    /// The address as the frame address register word.
    pub fn to_u32(self) -> u32 {
        (u32::from(self.block & 0x7) << 23)
            | (u32::from(self.half & 0x1) << 22)
            | (u32::from(self.row & 0x1F) << 17)
            | (u32::from(self.column & 0x3FF) << 7)
            | u32::from(self.minor & 0x7F)
    }

    /// The address a frame address register word names.
    pub fn from_u32(word: u32) -> FrameAddress {
        FrameAddress {
            block: u8::try_from((word >> 23) & 0x7).unwrap_or(0),
            half: u8::try_from((word >> 22) & 0x1).unwrap_or(0),
            row: u8::try_from((word >> 17) & 0x1F).unwrap_or(0),
            column: u16::try_from((word >> 7) & 0x3FF).unwrap_or(0),
            minor: u8::try_from(word & 0x7F).unwrap_or(0),
        }
    }
}

impl fmt::Display for FrameAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:#010x} (block {}, {}, row {}, column {}, minor {})",
            self.to_u32(),
            self.block,
            if self.half == 0 { "top" } else { "bottom" },
            self.row,
            self.column,
            self.minor
        )
    }
}

/// One row of one configuration bus: how many frames each of its columns
/// holds.
///
/// This is exactly one `configuration_buses` entry of one row of
/// `part.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigRow {
    /// The block type, as [`FrameAddress::block`].
    pub block: u8,
    /// Top (0) or bottom (1).
    pub half: u8,
    /// The row within the half.
    pub row: u8,
    /// Frames per configuration column, indexed by column number.
    pub columns: Vec<u32>,
}

/// Every frame of one part, in the order the configuration engine reads
/// them.
///
/// The order is the frame address register counting up — block type,
/// then half, then row, then column, then minor — with
/// [`PAD_FRAMES_PER_ROW`] all-zero frames after each row. A bitstream
/// therefore sets `FAR` once, to zero, and streams the whole part.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameLayout {
    rows: Vec<ConfigRow>,
    /// One entry per frame in stream order; `None` is a pad frame.
    order: Vec<Option<FrameAddress>>,
    index: HashMap<u32, usize>,
}

impl FrameLayout {
    /// The layout the given rows describe.
    ///
    /// The rows are sorted into frame-address order here, so a caller
    /// may hand them over in whatever order the database listed them.
    pub fn new(mut rows: Vec<ConfigRow>) -> FrameLayout {
        rows.sort_by_key(|r| (r.block, r.half, r.row));
        let mut order = Vec::new();
        let mut index = HashMap::new();
        for row in &rows {
            for (column, frames) in row.columns.iter().enumerate() {
                let column = u16::try_from(column).unwrap_or(u16::MAX);
                for minor in 0..*frames {
                    let address = FrameAddress {
                        block: row.block,
                        half: row.half,
                        row: row.row,
                        column,
                        minor: u8::try_from(minor).unwrap_or(u8::MAX),
                    };
                    index.insert(address.to_u32(), order.len());
                    order.push(Some(address));
                }
            }
            for _ in 0..PAD_FRAMES_PER_ROW {
                order.push(None);
            }
        }
        FrameLayout { rows, order, index }
    }

    /// The rows it was built from, in frame-address order.
    pub fn rows(&self) -> &[ConfigRow] {
        &self.rows
    }

    /// Every frame in stream order, `None` for a pad frame.
    pub fn order(&self) -> &[Option<FrameAddress>] {
        &self.order
    }

    /// How many frames the stream holds, pad frames included.
    pub fn frames(&self) -> usize {
        self.order.len()
    }

    /// How many frames carry data.
    pub fn data_frames(&self) -> usize {
        self.order.iter().filter(|f| f.is_some()).count()
    }

    /// How many 32-bit words the frame stream holds.
    pub fn words(&self) -> usize {
        self.frames() * WORDS_PER_FRAME
    }

    /// Where in the stream a frame address sits.
    pub fn position(&self, address: FrameAddress) -> Option<usize> {
        self.index.get(&address.to_u32()).copied()
    }

    /// True when the part has a frame at this address.
    pub fn contains(&self, address: FrameAddress) -> bool {
        self.index.contains_key(&address.to_u32())
    }
}

/// One part: what a bitstream for it has to say about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Part {
    /// The part name as the `.bit` header spells it (`7a35tcpg236`).
    pub name: String,
    /// The IDCODE the bitstream writes to the `IDCODE` register.
    pub idcode: u32,
    /// Its frame layout.
    pub layout: FrameLayout,
}

/// Where one tile's configuration bits live in the frame stream.
///
/// From `tilegrid.json`: a tile's `bits` entry for one configuration
/// bus. Bit `(row, col)` of the tile is frame `baseaddr + row`, word
/// `offset + col / 32`, bit `col % 32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileBits {
    /// The frame address of the tile's first frame.
    pub baseaddr: u32,
    /// How many consecutive frames the tile occupies.
    pub frames: u32,
    /// The tile's first word within each of those frames.
    pub offset: u32,
    /// How many words of each frame the tile owns.
    pub words: u32,
}

impl TileBits {
    /// How many configuration bits one tile holds: one per frame per bit
    /// of its words.
    pub fn bit_count(&self) -> usize {
        self.frames as usize * self.words as usize * 32
    }

    /// The frame address and bit position of tile bit `bit`, or `None`
    /// when it falls outside the tile's window.
    pub fn locate(&self, bit: ConfigBit) -> Option<(FrameAddress, usize, u32)> {
        if bit.row >= self.frames || bit.col >= self.words * 32 {
            return None;
        }
        let address = FrameAddress::from_u32(self.baseaddr.checked_add(bit.row)?);
        let word = usize::try_from(self.offset + bit.col / 32).ok()?;
        Some((address, word, bit.col % 32))
    }
}

/// Which tiles of a [`Bitstream`] land where in the frame stream.
///
/// A tile may appear more than once when it straddles two configuration
/// buses; both windows are kept and a bit is placed in whichever one
/// claims it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameMap {
    tiles: HashMap<(u32, u32), Vec<TileBits>>,
}

impl FrameMap {
    /// An empty map.
    pub fn new() -> FrameMap {
        FrameMap::default()
    }

    /// Records a window for the tile at `(x, y)`.
    pub fn insert(&mut self, tile: (u32, u32), bits: TileBits) {
        self.tiles.entry(tile).or_default().push(bits);
    }

    /// The windows of the tile at `(x, y)`.
    pub fn windows(&self, tile: (u32, u32)) -> &[TileBits] {
        self.tiles.get(&tile).map_or(&[], Vec::as_slice)
    }

    /// How many tiles have a window.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// True when no tile has a window.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

/// The configuration memory of one part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameData {
    layout: FrameLayout,
    words: Vec<u32>,
}

impl FrameData {
    /// An all-zero image in the given layout.
    pub fn empty(layout: FrameLayout) -> FrameData {
        let words = vec![0u32; layout.words()];
        FrameData { layout, words }
    }

    /// The layout it was built in.
    pub fn layout(&self) -> &FrameLayout {
        &self.layout
    }

    /// Every word of the frame stream, in stream order.
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    /// One frame's words.
    pub fn frame(&self, position: usize) -> Option<&[u32]> {
        let start = position.checked_mul(WORDS_PER_FRAME)?;
        self.words.get(start..start + WORDS_PER_FRAME)
    }

    /// Sets one bit of one word of one frame.
    ///
    /// # Errors
    ///
    /// [`Xc7Error::NoSuchFrame`] when the part has no such frame and
    /// [`Xc7Error::OutOfFrame`] when the word is past the end of one.
    pub fn set(&mut self, address: FrameAddress, word: usize, bit: u32) -> Result<(), Xc7Error> {
        let position = self
            .layout
            .position(address)
            .ok_or(Xc7Error::NoSuchFrame { address })?;
        if word >= WORDS_PER_FRAME {
            return Err(Xc7Error::OutOfFrame { address, word });
        }
        self.words[position * WORDS_PER_FRAME + word] |= 1u32 << (bit & 31);
        Ok(())
    }

    /// Whether one bit is set, `None` when there is no such frame or
    /// word.
    pub fn get(&self, address: FrameAddress, word: usize, bit: u32) -> Option<bool> {
        let position = self.layout.position(address)?;
        if word >= WORDS_PER_FRAME {
            return None;
        }
        Some((self.words[position * WORDS_PER_FRAME + word] >> (bit & 31)) & 1 == 1)
    }

    /// How many bits are set.
    pub fn ones(&self) -> usize {
        self.words
            .iter()
            .map(|w| usize::try_from(w.count_ones()).unwrap_or(0))
            .sum()
    }

    /// The frames that hold at least one set bit, in stream order.
    pub fn used_frames(&self) -> Vec<FrameAddress> {
        let mut out = Vec::new();
        for (position, address) in self.layout.order.iter().enumerate() {
            let Some(address) = address else { continue };
            let start = position * WORDS_PER_FRAME;
            if self.words[start..start + WORDS_PER_FRAME]
                .iter()
                .any(|w| *w != 0)
            {
                out.push(*address);
            }
        }
        out
    }
}

/// Paints a [`Bitstream`]'s tile bitmaps into a part's frames.
///
/// This is the one place a tile coordinate becomes a frame address, and
/// it reads [`TileBits`] from the database rather than knowing anything
/// about 7-series geometry. A tile of the bitstream that the map does
/// not mention contributes nothing, which is what a tile outside the
/// loaded region is.
///
/// # Errors
///
/// [`Xc7Error::OutOfTile`] when a set bit falls outside every window the
/// map gives its tile, and the errors of [`FrameData::set`] when the map
/// and the layout disagree.
pub fn frames_from_bitstream(
    part: &Part,
    bitstream: &Bitstream,
    map: &FrameMap,
) -> Result<FrameData, Xc7Error> {
    let mut data = FrameData::empty(part.layout.clone());
    for (format, bits) in bitstream.used_tiles() {
        let windows = map.windows(format.tile);
        for bit in bits {
            let mut placed = false;
            for window in windows {
                if let Some((address, word, index)) = window.locate(bit) {
                    data.set(address, word, index)?;
                    placed = true;
                    break;
                }
            }
            if !placed {
                return Err(Xc7Error::OutOfTile {
                    tile: format.tile,
                    bit,
                });
            }
        }
    }
    Ok(data)
}

/// A configuration register (UG470, table 5-23).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Register {
    /// CRC check value.
    Crc = 0,
    /// Frame address.
    Far = 1,
    /// Frame data input, the write path into configuration memory.
    Fdri = 2,
    /// Frame data output, the read path.
    Fdro = 3,
    /// Command.
    Cmd = 4,
    /// Control 0.
    Ctl0 = 5,
    /// Mask for `CTL0` and `CTL1`.
    Mask = 6,
    /// Status.
    Stat = 7,
    /// Legacy output.
    Lout = 8,
    /// Configuration option 0.
    Cor0 = 9,
    /// Multiple frame write.
    Mfwr = 10,
    /// Initial CBC value.
    Cbc = 11,
    /// Device ID.
    Idcode = 12,
    /// User access.
    Axss = 13,
    /// Configuration option 1.
    Cor1 = 14,
    /// Warm boot start address.
    Wbstar = 16,
    /// Watchdog timer.
    Timer = 17,
    /// Boot history status.
    Bootsts = 22,
    /// Control 1.
    Ctl1 = 24,
}

impl Register {
    /// The register a header's address field names, when it is one this
    /// module knows.
    pub fn from_u32(value: u32) -> Option<Register> {
        Some(match value {
            0 => Register::Crc,
            1 => Register::Far,
            2 => Register::Fdri,
            3 => Register::Fdro,
            4 => Register::Cmd,
            5 => Register::Ctl0,
            6 => Register::Mask,
            7 => Register::Stat,
            8 => Register::Lout,
            9 => Register::Cor0,
            10 => Register::Mfwr,
            11 => Register::Cbc,
            12 => Register::Idcode,
            13 => Register::Axss,
            14 => Register::Cor1,
            16 => Register::Wbstar,
            17 => Register::Timer,
            22 => Register::Bootsts,
            24 => Register::Ctl1,
            _ => return None,
        })
    }

    /// The register's address.
    pub fn address(self) -> u32 {
        self as u32
    }
}

/// A value written to the `CMD` register (UG470, table 5-25).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Command {
    /// Null command.
    Null = 0,
    /// Write configuration data.
    Wcfg = 1,
    /// Multiple frame write.
    Mfw = 2,
    /// Last frame; deasserts `GHIGH_B`.
    Lfrm = 3,
    /// Read configuration data.
    Rcfg = 4,
    /// Begin startup.
    Start = 5,
    /// Reset the CRC register.
    Rcrc = 7,
    /// Assert `GHIGH_B`.
    Aghigh = 8,
    /// Switch the configuration clock.
    Switch = 9,
    /// Pulse `GRESTORE`.
    Grestore = 10,
    /// Shut down.
    Shutdown = 11,
    /// Desynchronise.
    Desync = 13,
    /// Internal reconfiguration.
    Iprog = 15,
}

impl Command {
    /// The command a `CMD` write carries, when it is one this module
    /// knows.
    pub fn from_u32(value: u32) -> Option<Command> {
        Some(match value {
            0 => Command::Null,
            1 => Command::Wcfg,
            2 => Command::Mfw,
            3 => Command::Lfrm,
            4 => Command::Rcfg,
            5 => Command::Start,
            7 => Command::Rcrc,
            8 => Command::Aghigh,
            9 => Command::Switch,
            10 => Command::Grestore,
            11 => Command::Shutdown,
            13 => Command::Desync,
            15 => Command::Iprog,
            _ => return None,
        })
    }
}

/// One configuration packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    /// A no-operation, which the stream is padded with.
    Nop,
    /// A write of `words` to a register.
    Write {
        /// The register's address; [`Register::from_u32`] names it when
        /// it is a documented one.
        register: u32,
        /// The words written.
        words: Vec<u32>,
    },
    /// A read request, which a `.bit` for configuration never contains
    /// but a readback stream does.
    Read {
        /// The register's address.
        register: u32,
        /// How many words were asked for.
        count: usize,
    },
}

/// Splits a word stream into packets.
///
/// A type 2 packet inherits the register of the type 1 packet before it,
/// as UG470 specifies, so what comes back is already flattened: a
/// `FDRI` write of 547 420 words is one [`Packet::Write`].
///
/// # Errors
///
/// [`Xc7Error::Malformed`] for an unknown packet type or a packet whose
/// payload runs past the end of the stream.
pub fn packets(words: &[u32]) -> Result<Vec<Packet>, Xc7Error> {
    let mut out = Vec::new();
    let mut last_register = 0u32;
    let mut i = 0usize;
    while i < words.len() {
        let header = words[i];
        i += 1;
        let kind = header >> 29;
        let (opcode, register, count) = match kind {
            1 => {
                let register = (header >> 13) & 0x3FFF;
                last_register = register;
                (
                    (header >> 27) & 0x3,
                    register,
                    usize::try_from(header & 0x7FF).unwrap_or(0),
                )
            }
            2 => (
                (header >> 27) & 0x3,
                last_register,
                usize::try_from(header & 0x07FF_FFFF).unwrap_or(0),
            ),
            other => {
                return Err(Xc7Error::Malformed(format!(
                    "word {i} is a type {other} packet header ({header:#010x}), which UG470 does not define"
                )));
            }
        };
        if i + count > words.len() {
            return Err(Xc7Error::Malformed(format!(
                "a packet claims {count} word(s) but only {} are left",
                words.len() - i
            )));
        }
        let payload = &words[i..i + count];
        i += count;
        match opcode {
            0 => out.push(Packet::Nop),
            1 => out.push(Packet::Read { register, count }),
            _ => out.push(Packet::Write {
                register,
                words: payload.to_vec(),
            }),
        }
    }
    Ok(out)
}

/// Folds one register write into the running CRC.
///
/// UG470: the configuration CRC is CRC-32C (polynomial `0x1EDC6F41`)
/// over the 37-bit value `{register address[4:0], data[31:0]}`, shifted
/// in least significant bit first. One call per written word; the
/// register's address goes in with every word of a multi-word write.
pub fn crc_update(crc: u32, register: u32, value: u32) -> u32 {
    let payload = (u64::from(register & 0x1F) << 32) | u64::from(value);
    let mut crc = crc;
    for index in 0..37 {
        let bit = u32::try_from((payload >> index) & 1).unwrap_or(0);
        if (crc ^ bit) & 1 == 1 {
            crc = (crc >> 1) ^ CRC32C_REVERSED;
        } else {
            crc >>= 1;
        }
    }
    crc
}

/// The keyed fields of the `.bit` wrapper.
///
/// The wrapper is not part of the configuration stream: it is the
/// header Xilinx tools put in front of it, four NUL-terminated strings
/// and then the stream itself. A `.bin` is the same file without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitHeader {
    /// Field `a`: the design name, which Vivado writes as
    /// `<top>;UserID=0X...;Version=...`.
    pub design: String,
    /// Field `b`: the part, without the speed grade (`7a35tcpg236`).
    pub part: String,
    /// Field `c`: the date, `YYYY/MM/DD`.
    pub date: String,
    /// Field `d`: the time, `HH:MM:SS`.
    pub time: String,
}

impl BitHeader {
    /// A header naming a design and a part, with a fixed date and time.
    ///
    /// The date and time are `0000/00/00` and `00:00:00` rather than the
    /// clock, so that two runs of one design produce identical bytes —
    /// the same reason [`Bitstream::write_asc`] carries no timestamp.
    pub fn new(design: impl Into<String>, part: impl Into<String>) -> BitHeader {
        BitHeader {
            design: design.into(),
            part: part.into(),
            date: "0000/00/00".to_owned(),
            time: "00:00:00".to_owned(),
        }
    }
}

/// The nine magic bytes every `.bit` starts with, after their length.
const BIT_MAGIC: [u8; 9] = [0x0F, 0xF0, 0x0F, 0xF0, 0x0F, 0xF0, 0x0F, 0xF0, 0x00];

/// Dummy words before the bus-width detect pattern (UG470).
const LEADING_DUMMY_WORDS: usize = 8;
/// Dummy words between the bus-width pattern and the sync word.
const PRE_SYNC_DUMMY_WORDS: usize = 2;

/// The configuration option register values Vivado writes for this
/// family.
///
/// UG470 gives each field of `COR0`, `CTL0` and `MASK`; it does not say
/// what a tool puts there. These are the words the reference bitstream
/// in Project X-Ray's `artix7/harness/` carries, kept verbatim so that a
/// stream from here differs from one from Vivado only in its frames.
const COR0: u32 = 0x0200_3FE5;
const CTL0: u32 = 0x0000_0501;
const CTL0_MASK: u32 = 0x0000_0401;
const STARTUP_MASK: u32 = 0x0000_0501;
/// Register 19 is reserved in UG470; Vivado writes zero to it here.
const RESERVED_REGISTER_19: u32 = 19;

/// The frame address left in `FAR` after configuration: block type 7,
/// row 31, which is no real frame, so a stray write goes nowhere.
const PARKED_FAR: u32 = 0x03BE_0000;

/// Builds a type 1 packet header.
fn type1(opcode: u32, register: u32, count: usize) -> u32 {
    (1 << 29)
        | (opcode << 27)
        | ((register & 0x3FFF) << 13)
        | (u32::try_from(count).unwrap_or(0) & 0x7FF)
}

/// Builds a type 2 packet header.
fn type2(opcode: u32, count: usize) -> u32 {
    (2 << 29) | (opcode << 27) | (u32::try_from(count).unwrap_or(0) & 0x07FF_FFFF)
}

/// The no-operation word.
fn nop() -> u32 {
    type1(0, 0, 0)
}

/// Accumulates the configuration packet stream, keeping the CRC as it
/// goes.
struct Stream {
    words: Vec<u32>,
    crc: u32,
}

impl Stream {
    fn new() -> Stream {
        Stream {
            words: Vec::new(),
            crc: 0,
        }
    }

    fn nops(&mut self, count: usize) {
        for _ in 0..count {
            self.words.push(nop());
        }
    }

    /// A one-word write, which is every register but `FDRI`.
    fn write(&mut self, register: Register, value: u32) {
        self.words.push(type1(2, register.address(), 1));
        self.words.push(value);
        self.crc = crc_update(self.crc, register.address(), value);
    }

    /// A `CMD` write. `RCRC` resets the CRC register, so nothing before
    /// it counts.
    fn command(&mut self, command: Command) {
        let value = command as u32;
        self.words.push(type1(2, Register::Cmd.address(), 1));
        self.words.push(value);
        if command == Command::Rcrc {
            self.crc = 0;
        } else {
            self.crc = crc_update(self.crc, Register::Cmd.address(), value);
        }
    }

    /// A write to a register this module has no name for.
    fn write_raw(&mut self, register: u32, value: u32) {
        self.words.push(type1(2, register, 1));
        self.words.push(value);
        self.crc = crc_update(self.crc, register, value);
    }

    /// The frame stream: an empty type 1 `FDRI` write followed by a type
    /// 2 packet carrying every word.
    fn frames(&mut self, data: &[u32]) {
        self.words.push(type1(2, Register::Fdri.address(), 0));
        self.words.push(type2(2, data.len()));
        self.words.extend_from_slice(data);
        for word in data {
            self.crc = crc_update(self.crc, Register::Fdri.address(), *word);
        }
    }

    /// Writes the running CRC as a check value; the register resets
    /// itself afterwards.
    fn checkpoint(&mut self) {
        let crc = self.crc;
        self.words.push(type1(2, Register::Crc.address(), 1));
        self.words.push(crc);
        self.crc = 0;
    }
}

/// Writes a complete `.bit` file for a part.
///
/// The command sequence is the one the reference bitstream in Project
/// X-Ray's `artix7/harness/` uses, word for word, with this design's
/// frames and this part's IDCODE in place of Vivado's. Every register
/// and command in it is UG470's; the *order* is Vivado's, because UG470
/// documents the pieces and not the recipe.
///
/// The output is deterministic: no clock is read, so two runs of one
/// design produce identical bytes.
///
/// # Errors
///
/// [`Xc7Error::IdcodeMismatch`] when `data`'s layout does not belong to
/// `part` — the check that stops a bitstream built against one
/// database being labelled with another part's IDCODE.
pub fn write_bit(header: &BitHeader, part: &Part, data: &FrameData) -> Result<Vec<u8>, Xc7Error> {
    if data.layout() != &part.layout {
        return Err(Xc7Error::Malformed(
            "the frame data was built in a different layout from the part's".to_owned(),
        ));
    }
    let mut stream = Stream::new();
    stream.nops(1);
    stream.write(Register::Timer, 0);
    stream.write(Register::Wbstar, 0);
    stream.command(Command::Null);
    stream.nops(1);
    stream.command(Command::Rcrc);
    stream.nops(2);
    stream.write_raw(RESERVED_REGISTER_19, 0);
    stream.write(Register::Cor0, COR0);
    stream.write(Register::Cor1, 0);
    stream.write(Register::Idcode, part.idcode);
    stream.command(Command::Switch);
    stream.nops(1);
    stream.write(Register::Mask, CTL0_MASK);
    stream.write(Register::Ctl0, CTL0);
    stream.write(Register::Mask, 0);
    stream.write(Register::Ctl1, 0);
    stream.nops(8);
    stream.write(Register::Far, 0);
    stream.command(Command::Wcfg);
    stream.nops(1);
    stream.frames(data.words());
    stream.checkpoint();
    stream.nops(2);
    stream.command(Command::Grestore);
    stream.nops(1);
    stream.command(Command::Lfrm);
    stream.nops(100);
    stream.command(Command::Start);
    stream.nops(1);
    stream.write(Register::Far, PARKED_FAR);
    stream.write(Register::Mask, STARTUP_MASK);
    stream.write(Register::Ctl0, CTL0);
    stream.checkpoint();
    stream.nops(2);
    stream.command(Command::Desync);
    stream.nops(400);

    let mut payload = Vec::with_capacity(stream.words.len() * 4 + 64);
    for _ in 0..LEADING_DUMMY_WORDS {
        payload.extend_from_slice(&u32::MAX.to_be_bytes());
    }
    for word in BUS_WIDTH_PATTERN {
        payload.extend_from_slice(&word.to_be_bytes());
    }
    for _ in 0..PRE_SYNC_DUMMY_WORDS {
        payload.extend_from_slice(&u32::MAX.to_be_bytes());
    }
    payload.extend_from_slice(&SYNC_WORD.to_be_bytes());
    for word in &stream.words {
        payload.extend_from_slice(&word.to_be_bytes());
    }

    let mut out = Vec::with_capacity(payload.len() + 128);
    out.extend_from_slice(&u16::try_from(BIT_MAGIC.len()).unwrap_or(0).to_be_bytes());
    out.extend_from_slice(&BIT_MAGIC);
    out.extend_from_slice(&1u16.to_be_bytes());
    write_field(&mut out, b'a', &header.design);
    write_field(&mut out, b'b', &header.part);
    write_field(&mut out, b'c', &header.date);
    write_field(&mut out, b'd', &header.time);
    out.push(b'e');
    out.extend_from_slice(&u32::try_from(payload.len()).unwrap_or(0).to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Writes one keyed, NUL-terminated header field.
fn write_field(out: &mut Vec<u8>, key: u8, value: &str) {
    out.push(key);
    let length = u16::try_from(value.len() + 1).unwrap_or(u16::MAX);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

/// What [`read_bit`] found in a `.bit` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bit {
    /// The wrapper's fields.
    pub header: BitHeader,
    /// The IDCODE the stream writes, when it writes one.
    pub idcode: Option<u32>,
    /// The frame address the stream starts writing at.
    pub start_address: Option<FrameAddress>,
    /// Every word written to `FDRI`, in stream order.
    pub frames: Vec<u32>,
    /// The commands the stream issues, in order.
    pub commands: Vec<Command>,
    /// Every CRC check value in the stream, with the value the data
    /// before it implies. [`read_bit`] has already compared them.
    pub crc_checks: Vec<(u32, u32)>,
    /// The packets, for a test that wants to walk the sequence.
    pub packets: Vec<Packet>,
}

impl Bit {
    /// How many frames the stream carries.
    pub fn frame_count(&self) -> usize {
        self.frames.len() / WORDS_PER_FRAME
    }
}

/// Reads a `.bit` file back.
///
/// The reader is independent of the writer: it finds the sync word,
/// splits the stream into packets, replays the register writes through
/// [`crc_update`] and checks every CRC the stream carries. A file that
/// comes back through this has a valid container; whether its frames
/// mean anything is a different question, and not one a parser can
/// answer.
///
/// # Errors
///
/// [`Xc7Error::Malformed`] for a file that is not a `.bit` or is
/// truncated, and [`Xc7Error::Crc`] for a stream whose CRC does not
/// match the data before it.
pub fn read_bit(bytes: &[u8]) -> Result<Bit, Xc7Error> {
    let mut reader = ByteReader { bytes, pos: 0 };
    let magic_len = usize::from(reader.u16()?);
    reader.take(magic_len)?;
    let one = reader.u16()?;
    if one != 1 {
        return Err(Xc7Error::Malformed(format!(
            "the header's separator is {one}, not 1: this is not a .bit file"
        )));
    }
    let mut header = BitHeader {
        design: String::new(),
        part: String::new(),
        date: String::new(),
        time: String::new(),
    };
    let payload = loop {
        let key = *reader
            .take(1)?
            .first()
            .ok_or_else(|| Xc7Error::Malformed("the header ends early".to_owned()))?;
        if key == b'e' {
            let length = usize::try_from(reader.u32()?).unwrap_or(0);
            break reader.take(length)?;
        }
        let length = usize::from(reader.u16()?);
        let value = reader.take(length)?;
        let text = String::from_utf8_lossy(value)
            .trim_end_matches('\0')
            .to_owned();
        match key {
            b'a' => header.design = text,
            b'b' => header.part = text,
            b'c' => header.date = text,
            b'd' => header.time = text,
            other => {
                return Err(Xc7Error::Malformed(format!(
                    "the header carries an unknown field `{}`",
                    char::from(other)
                )));
            }
        }
    };

    let sync = payload
        .windows(4)
        .position(|w| w == SYNC_WORD.to_be_bytes())
        .ok_or_else(|| {
            Xc7Error::Malformed(format!(
                "no sync word {SYNC_WORD:#010x} in {} byte(s) of configuration data",
                payload.len()
            ))
        })?;
    let body = &payload[sync + 4..];
    if body.len() % 4 != 0 {
        return Err(Xc7Error::Malformed(format!(
            "the configuration stream is {} byte(s), not a whole number of words",
            body.len()
        )));
    }
    let words: Vec<u32> = body
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();

    let packets = packets(&words)?;
    let mut idcode = None;
    let mut start_address = None;
    let mut frames = Vec::new();
    let mut commands = Vec::new();
    let mut crc_checks = Vec::new();
    let mut crc = 0u32;
    let mut seen_far = false;
    for packet in &packets {
        let Packet::Write { register, words } = packet else {
            continue;
        };
        if *register == Register::Crc.address() {
            let expected = words.first().copied().unwrap_or(0);
            crc_checks.push((expected, crc));
            if expected != crc {
                return Err(Xc7Error::Crc {
                    expected,
                    computed: crc,
                });
            }
            crc = 0;
            continue;
        }
        if *register == Register::Cmd.address() {
            let value = words.first().copied().unwrap_or(0);
            if let Some(command) = Command::from_u32(value) {
                commands.push(command);
                if command == Command::Rcrc {
                    crc = 0;
                    continue;
                }
            }
        }
        if *register == Register::Idcode.address() {
            idcode = words.first().copied();
        }
        if *register == Register::Far.address() && !seen_far {
            start_address = words.first().copied().map(FrameAddress::from_u32);
            seen_far = true;
        }
        if *register == Register::Fdri.address() {
            frames.extend_from_slice(words);
        }
        for word in words {
            crc = crc_update(crc, *register, *word);
        }
    }

    Ok(Bit {
        header,
        idcode,
        start_address,
        frames,
        commands,
        crc_checks,
        packets,
    })
}

/// A bounds-checked cursor over a `.bit` file.
struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Xc7Error> {
        let end = self.pos.checked_add(n).ok_or_else(|| {
            Xc7Error::Malformed("a header field claims more bytes than exist".to_owned())
        })?;
        if end > self.bytes.len() {
            return Err(Xc7Error::Malformed(format!(
                "the file ends after {} byte(s), in the middle of a field",
                self.bytes.len()
            )));
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16, Xc7Error> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, Xc7Error> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-row, four-column part, small enough to count by hand.
    fn tiny_part() -> Part {
        let layout = FrameLayout::new(vec![
            ConfigRow {
                block: 0,
                half: 0,
                row: 0,
                columns: vec![3, 2],
            },
            ConfigRow {
                block: 1,
                half: 1,
                row: 0,
                columns: vec![4],
            },
        ]);
        Part {
            name: "7atiny".to_owned(),
            idcode: 0x0362_d093,
            layout,
        }
    }

    #[test]
    fn frame_addresses_round_trip_through_the_register_word() {
        let far = FrameAddress {
            block: 1,
            half: 1,
            row: 17,
            column: 300,
            minor: 100,
        };
        assert_eq!(FrameAddress::from_u32(far.to_u32()), far);
        // The part's own parked address, decoded.
        let parked = FrameAddress::from_u32(PARKED_FAR);
        assert_eq!(parked.block, 7);
        assert_eq!(parked.row, 31);
        assert_eq!(parked.column, 0);
        assert!(FrameAddress::default().to_u32() == 0);
        assert!(far.to_string().contains("row 17"));
        assert!(FrameAddress::default().to_string().contains("top"));
    }

    #[test]
    fn the_layout_counts_frames_and_pads_every_row() {
        let part = tiny_part();
        let layout = &part.layout;
        assert_eq!(layout.data_frames(), 3 + 2 + 4);
        assert_eq!(layout.frames(), 9 + 2 * PAD_FRAMES_PER_ROW);
        assert_eq!(layout.words(), layout.frames() * WORDS_PER_FRAME);
        assert_eq!(layout.rows().len(), 2);
        // Stream order: block 0 first, its two columns in order, then
        // two pads, then block 1.
        let order = layout.order();
        assert_eq!(order[0].unwrap().minor, 0);
        assert_eq!(order[3].unwrap().column, 1);
        assert!(order[5].is_none() && order[6].is_none());
        assert_eq!(order[7].unwrap().block, 1);
        let first = FrameAddress {
            block: 0,
            half: 0,
            row: 0,
            column: 1,
            minor: 1,
        };
        assert_eq!(layout.position(first), Some(4));
        assert!(layout.contains(first));
        let absent = FrameAddress {
            block: 3,
            half: 0,
            row: 0,
            column: 0,
            minor: 0,
        };
        assert!(!layout.contains(absent));
        assert!(layout.position(absent).is_none());
    }

    #[test]
    fn bits_reach_frames_through_the_tile_window() {
        let part = tiny_part();
        let mut data = FrameData::empty(part.layout.clone());
        assert_eq!(data.ones(), 0);
        let window = TileBits {
            baseaddr: 0,
            frames: 3,
            offset: 5,
            words: 2,
        };
        assert_eq!(window.bit_count(), 3 * 2 * 32);
        let (address, word, bit) = window.locate(ConfigBit::new(2, 33)).unwrap();
        assert_eq!(address, FrameAddress::from_u32(2));
        assert_eq!((word, bit), (6, 1));
        assert!(window.locate(ConfigBit::new(3, 0)).is_none());
        assert!(window.locate(ConfigBit::new(0, 64)).is_none());

        data.set(address, word, bit).unwrap();
        assert_eq!(data.ones(), 1);
        assert_eq!(data.get(address, word, bit), Some(true));
        assert_eq!(data.get(address, word, 0), Some(false));
        assert_eq!(data.used_frames(), vec![address]);
        assert_eq!(data.frame(2).unwrap()[6], 2);
        assert!(data.frame(9999).is_none());

        let nowhere = FrameAddress {
            block: 5,
            half: 0,
            row: 0,
            column: 0,
            minor: 0,
        };
        assert_eq!(
            data.set(nowhere, 0, 0).unwrap_err(),
            Xc7Error::NoSuchFrame { address: nowhere }
        );
        assert!(
            data.set(address, 500, 0)
                .unwrap_err()
                .to_string()
                .contains("outside the 101-word frame")
        );
        assert!(data.get(nowhere, 0, 0).is_none());
        assert!(data.get(address, 500, 0).is_none());
    }

    /// The CRC, computed the other way round: a byte-at-a-time table,
    /// where [`crc_update`] shifts one bit at a time. Two implementations
    /// that agree on the same input are worth more than one that is
    /// merely self-consistent.
    fn crc_by_table(crc: u32, register: u32, value: u32) -> u32 {
        // The value is 37 bits, which is not a whole number of bytes, so
        // the table covers the 32 data bits and the five address bits go
        // in by hand afterwards.
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut c = u32::try_from(index).unwrap_or(0);
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    (c >> 1) ^ CRC32C_REVERSED
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        let mut crc = crc;
        for byte in value.to_le_bytes() {
            let index = usize::try_from((crc ^ u32::from(byte)) & 0xFF).unwrap_or(0);
            crc = (crc >> 8) ^ table[index];
        }
        for index in 0..5 {
            let bit = (register >> index) & 1;
            crc = if (crc ^ bit) & 1 == 1 {
                (crc >> 1) ^ CRC32C_REVERSED
            } else {
                crc >> 1
            };
        }
        crc
    }

    #[test]
    fn the_crc_agrees_with_a_table_driven_one() {
        let mut bitwise = 0u32;
        let mut table = 0u32;
        for (register, value) in [
            (12u32, 0x0362_d093u32),
            (2, 0x0000_0000),
            (2, 0xFFFF_FFFF),
            (1, 0x0000_0001),
            (9, 0x0200_3FE5),
            (5, 0x0000_0501),
        ] {
            bitwise = crc_update(bitwise, register, value);
            table = crc_by_table(table, register, value);
            assert_eq!(bitwise, table, "register {register} value {value:#010x}");
        }
        assert_ne!(bitwise, 0);
    }

    #[test]
    fn a_bitstream_round_trips_through_the_reader() {
        let part = tiny_part();
        let mut data = FrameData::empty(part.layout.clone());
        data.set(FrameAddress::from_u32(1), 7, 3).unwrap();
        data.set(FrameAddress::from_u32(0x0080_0002), 100, 31)
            .unwrap();
        let header = BitHeader::new("tiny;UserID=0XFFFFFFFF", "7atiny");
        let bytes = write_bit(&header, &part, &data).unwrap();

        // The wrapper, then the stream.
        assert_eq!(&bytes[..2], &[0x00, 0x09]);
        let back = read_bit(&bytes).unwrap();
        assert_eq!(back.header, header);
        assert_eq!(back.idcode, Some(part.idcode));
        assert_eq!(back.start_address, Some(FrameAddress::default()));
        assert_eq!(back.frames, data.words());
        assert_eq!(back.frame_count(), part.layout.frames());
        assert_eq!(back.crc_checks.len(), 2);
        for (expected, computed) in &back.crc_checks {
            assert_eq!(expected, computed);
        }
        assert_eq!(
            back.commands,
            vec![
                Command::Null,
                Command::Rcrc,
                Command::Switch,
                Command::Wcfg,
                Command::Grestore,
                Command::Lfrm,
                Command::Start,
                Command::Desync,
            ]
        );
        // Writing twice gives the same bytes.
        assert_eq!(write_bit(&header, &part, &data).unwrap(), bytes);
    }

    #[test]
    fn the_packet_stream_is_ug470_shaped() {
        let part = tiny_part();
        let data = FrameData::empty(part.layout.clone());
        let bytes = write_bit(&BitHeader::new("t", "7atiny"), &part, &data).unwrap();
        let back = read_bit(&bytes).unwrap();
        // A type 1 FDRI write of nothing, then the type 2 that carries
        // every frame word.
        let fdri: Vec<&Packet> = back
            .packets
            .iter()
            .filter(|p| matches!(p, Packet::Write { register, .. } if *register == 2))
            .collect();
        assert_eq!(fdri.len(), 2);
        assert!(matches!(fdri[0], Packet::Write { words, .. } if words.is_empty()));
        assert!(
            matches!(fdri[1], Packet::Write { words, .. } if words.len() == part.layout.words())
        );
        assert_eq!(Register::from_u32(12), Some(Register::Idcode));
        assert_eq!(Register::from_u32(30), None);
        assert_eq!(Register::Fdri.address(), 2);
        assert_eq!(Command::from_u32(13), Some(Command::Desync));
        assert_eq!(Command::from_u32(6), None);
    }

    #[test]
    fn a_broken_file_is_refused_rather_than_guessed_at() {
        assert!(
            read_bit(b"")
                .unwrap_err()
                .to_string()
                .contains("ends after 0 byte")
        );
        assert!(
            read_bit(&[0, 1, 0, 0, 2])
                .unwrap_err()
                .to_string()
                .contains("not a .bit file")
        );
        // A well-formed wrapper with no sync word.
        let mut bytes = vec![0x00, 0x09];
        bytes.extend_from_slice(&BIT_MAGIC);
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.push(b'e');
        bytes.extend_from_slice(&4u32.to_be_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        assert!(
            read_bit(&bytes)
                .unwrap_err()
                .to_string()
                .contains("no sync")
        );

        // A corrupted CRC is caught.
        let part = tiny_part();
        let data = FrameData::empty(part.layout.clone());
        let mut good = write_bit(&BitHeader::new("t", "7atiny"), &part, &data).unwrap();
        let idcode_at = good
            .windows(4)
            .position(|w| w == part.idcode.to_be_bytes())
            .unwrap();
        good[idcode_at + 3] ^= 0x01;
        assert!(matches!(read_bit(&good), Err(Xc7Error::Crc { .. })));

        assert!(
            packets(&[0xE000_0000])
                .unwrap_err()
                .to_string()
                .contains("type 7 packet")
        );
        assert!(
            packets(&[type1(2, 1, 4)])
                .unwrap_err()
                .to_string()
                .contains("only 0 are left")
        );
        assert_eq!(packets(&[type1(1, 7, 1), 0]).unwrap().len(), 1);
    }

    #[test]
    fn painting_a_tile_bitmap_reports_a_bit_that_fits_nowhere() {
        use crate::fpga::bitstream::{BitstreamFormat, TileFormat};

        let part = tiny_part();
        let format = BitstreamFormat {
            device: "xc7a35t".to_owned(),
            tiles: vec![TileFormat {
                tile: (0, 0),
                keyword: "CLBLL_L".to_owned(),
                rows: 3,
                cols: 64,
            }],
        };
        let mut bitstream = Bitstream::empty(format);
        bitstream.set((0, 0), ConfigBit::new(1, 40)).unwrap();
        let mut map = FrameMap::new();
        assert!(map.is_empty());
        map.insert(
            (0, 0),
            TileBits {
                baseaddr: 0,
                frames: 3,
                offset: 0,
                words: 2,
            },
        );
        assert_eq!(map.len(), 1);
        assert_eq!(map.windows((9, 9)), &[]);
        let data = frames_from_bitstream(&part, &bitstream, &map).unwrap();
        assert_eq!(data.ones(), 1);
        assert_eq!(
            data.get(FrameAddress::from_u32(1), 1, 8),
            Some(true),
            "{:?}",
            data.used_frames()
        );

        // A tile the map does not know contributes nothing; a bit
        // outside every window is an error rather than a silent drop.
        let empty = FrameMap::new();
        assert_eq!(
            frames_from_bitstream(&part, &bitstream, &empty).unwrap_err(),
            Xc7Error::OutOfTile {
                tile: (0, 0),
                bit: ConfigBit::new(1, 40),
            }
        );
        assert!(
            frames_from_bitstream(&part, &bitstream, &empty)
                .unwrap_err()
                .to_string()
                .contains("outside the frame window")
        );
    }

    #[test]
    fn frame_data_from_another_layout_is_refused() {
        let part = tiny_part();
        let other = FrameLayout::new(vec![ConfigRow {
            block: 0,
            half: 0,
            row: 0,
            columns: vec![1],
        }]);
        let data = FrameData::empty(other);
        assert!(
            write_bit(&BitHeader::new("t", "x"), &part, &data)
                .unwrap_err()
                .to_string()
                .contains("different layout")
        );
        assert!(
            Xc7Error::IdcodeMismatch {
                wanted: 1,
                found: 2
            }
            .to_string()
            .contains("0x00000002")
        );
    }
}
