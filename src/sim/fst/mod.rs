//! FST waveform output, the compressed format GTKWave reads natively.
//!
//! VCD is text and grows without bound; FST stores the same information as a
//! sequence of compressed binary blocks with a per-signal index, so a viewer
//! can jump to a time without reading everything before it. There is no
//! standards document: the format is what `fstapi.c` in GTKWave's `libfst`
//! writes, and this module implements that, including its two compressors
//! ([`lz4`], [`zlib`]) and its integer encoding ([`varint`]), from scratch.
//!
//! A capture is started with
//! [`Simulator::enable_fst`](crate::sim::Simulator::enable_fst) and written
//! out with [`Simulator::dump_fst`](crate::sim::Simulator::dump_fst).
//! [`read`] parses a file back, which is how the writer is tested.
//!
//! # File layout
//!
//! A file is a flat list of blocks. Each one is a type byte, a big-endian
//! `u64` section length that counts itself and everything after it but not
//! the type byte, and the payload. This writer emits them in the order
//! `fstapi.c` does:
//!
//! | Block                         | Contents                                   |
//! |-------------------------------|--------------------------------------------|
//! | [`BL_HDR`] (0)                | times, endian test, counts, timescale, version and date |
//! | [`BL_VCDATA`] (1), one or more | a frame, the value-change chains, the chain index and the time table |
//! | [`BL_GEOM`] (3)               | the bit length of every handle, zlib'd      |
//! | [`BL_HIER`] (4)               | scopes and variables, gzip'd (or LZ4 as [`BL_HIER_LZ4`]) |
//!
//! [`BL_BLACKOUT`] (2), the dynamic-alias value-change variants
//! ([`BL_VCDATA_DYN_ALIAS`], [`BL_VCDATA_DYN_ALIAS2`]) and [`BL_SKIP`] are
//! understood by [`read`] but not produced.
//!
//! ## Header
//!
//! 329 bytes including the length: start and end time, the double
//! `2.718281828459045` as an endian test, the writer's memory use, the scope,
//! variable and handle counts, the number of value-change blocks, the
//! timescale as a signed power of ten of a second, a 128 byte version string,
//! a 119 byte date, a file-type byte and a time-zero offset. Version and date
//! are fixed strings here so that two runs of the same design produce
//! identical files.
//!
//! ## Hierarchy
//!
//! A byte stream of entries: [`ST_VCD_SCOPE`] followed by a scope type and
//! two NUL-terminated names, [`ST_VCD_UPSCOPE`], or a variable, which is a
//! `FST_VT_*` type byte, a direction byte, a NUL-terminated name, a varint
//! length and a varint *alias*. A zero alias allocates the next handle; a
//! non-zero one names an existing handle, which is how two names for one
//! signal share storage. The simulator's flattened instance tree already
//! shares a signal between a port and the parent net it is wired to, so
//! those names become aliases here.
//!
//! ## Geometry
//!
//! One varint per handle, zlib compressed: the bit length, `0` for a real
//! (which occupies eight bytes) and `0xFFFFFFFF` for a variable-length
//! string, which this writer never emits.
//!
//! ## Value-change blocks
//!
//! Each block covers a time range and holds:
//!
//! * begin and end time, and a hint at how much memory a reader needs;
//! * the **frame**: the value of every handle when the block started,
//!   concatenated and optionally zlib compressed, so a reader that seeks to
//!   this block still knows every signal's state;
//! * the **chains**: for every handle that changed, a varint uncompressed
//!   length (`0` meaning "stored") followed by the chain. A chain is a run
//!   of changes, each a varint that carries the number of time-table entries
//!   since that handle last changed, plus the value. A single bit fits
//!   entirely in the varint (`delta << 2 | bit << 1` for `0`/`1`, or
//!   `delta << 4 | code << 1 | 1` for `x`, `z` and the `std_logic` extras);
//!   a vector is `delta << 1` with the bits packed eight to a byte when they
//!   are all `0` or `1`, or `delta << 1 | 1` with one ASCII character per
//!   bit when any is unknown; a real is `delta << 1 | 1` and the eight
//!   little-endian bytes of the double;
//! * the **chain index**: for every handle in order, a varint that is
//!   `(offset delta << 1) | 1` when the handle has a chain, or `count << 1`
//!   to skip a run of handles that do not, followed by a `u64` holding the
//!   index's own length;
//! * the **time table**: varint deltas from zero, zlib compressed, followed
//!   by its uncompressed length, compressed length and entry count.
//!
//! Memories are not dumped, matching the VCD writer.

pub mod lz4;
pub mod reader;
pub mod varint;
pub mod zlib;

mod writer;

pub use reader::{FstChange, FstFile, FstValue, FstVar, read};
pub use writer::{FstCapture, FstCompression};

/// Header block: times, counts and the endian test.
pub const BL_HDR: u8 = 0;
/// Value-change block.
pub const BL_VCDATA: u8 = 1;
/// Dump on/off regions.
pub const BL_BLACKOUT: u8 = 2;
/// Per-handle value lengths.
pub const BL_GEOM: u8 = 3;
/// Scopes and variables, gzip compressed.
pub const BL_HIER: u8 = 4;
/// Value-change block whose index may alias chains.
pub const BL_VCDATA_DYN_ALIAS: u8 = 5;
/// Scopes and variables, LZ4 compressed.
pub const BL_HIER_LZ4: u8 = 6;
/// Scopes and variables, LZ4 compressed twice.
pub const BL_HIER_LZ4DUO: u8 = 7;
/// Value-change block with a signed, aliasing chain index.
pub const BL_VCDATA_DYN_ALIAS2: u8 = 8;
/// A whole file wrapped in one zlib stream.
pub const BL_ZWRAPPER: u8 = 254;
/// A block being written, or one to ignore.
pub const BL_SKIP: u8 = 255;

/// Hierarchy entry: an attribute begins.
pub const ST_GEN_ATTRBEGIN: u8 = 252;
/// Hierarchy entry: an attribute ends.
pub const ST_GEN_ATTREND: u8 = 253;
/// Hierarchy entry: a scope begins.
pub const ST_VCD_SCOPE: u8 = 254;
/// Hierarchy entry: the current scope ends.
pub const ST_VCD_UPSCOPE: u8 = 255;

/// Variable type: a Verilog `integer`.
pub const VT_VCD_INTEGER: u8 = 1;
/// Variable type: a real.
pub const VT_VCD_REAL: u8 = 3;
/// Variable type: a `reg` or variable.
pub const VT_VCD_REG: u8 = 5;
/// Variable type: a net.
pub const VT_VCD_WIRE: u8 = 16;

/// Bytes of the header's version string.
pub const HDR_VERSION_SIZE: usize = 128;
/// Bytes of the header's date string.
pub const HDR_DATE_SIZE: usize = 119;

/// The double stored in the header so a reader can detect a byte order
/// mismatch and swap reals accordingly.
pub const DOUBLE_ENDTEST: f64 = std::f64::consts::E;

/// The characters a single-bit change's escape code selects, in order.
pub const RCV_STR: &str = "xzhuwl-?";
