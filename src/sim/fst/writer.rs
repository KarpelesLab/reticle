//! The FST waveform writer.
//!
//! [`Simulator::enable_fst`](crate::sim::Simulator::enable_fst) hands back an
//! [`FstCapture`], a shared handle on the writer below. Registering one
//! change callback per signal is what feeds it, so the capture sees exactly
//! the value changes the VCD writer sees, at the same times.
//!
//! The writer keeps, for the block being built:
//!
//! * the *frame*, the value of every signal when the block started;
//! * the *current* values, so the next block's frame is ready on a flush;
//! * one *chain* of encoded changes per handle;
//! * the *time table*, the ordered times at which any change happened.
//!
//! A change appends to its handle's chain a varint holding the number of
//! time-table entries since that handle last changed, plus the new value;
//! see [`encode_change`]. When the flush interval is reached the block is
//! serialised into a finished `FST_BL_VCDATA` section and the state resets
//! with the current values as the new frame. Serialising the file appends
//! the pending block, then the geometry and hierarchy sections, and finally
//! prepends the header, which is the only place the totals are known.
//!
//! Signals that several instances share (a port wired straight to a net of
//! the parent) are one signal in the simulator and therefore one FST handle:
//! the extra names become hierarchy var entries with a non-zero alias
//! handle, which is how FST expresses the same thing.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ir::{NetKind, Type};
use crate::logic::{Bit, Logic};

use super::super::Simulator;
use super::super::api::NetHandle;
use super::super::elab::{InstId, SigId};
use super::{
    BL_GEOM, BL_HDR, BL_HIER, BL_HIER_LZ4, BL_VCDATA, DOUBLE_ENDTEST, HDR_DATE_SIZE,
    HDR_VERSION_SIZE, ST_VCD_SCOPE, ST_VCD_UPSCOPE, VT_VCD_INTEGER, VT_VCD_REAL, VT_VCD_REG,
    VT_VCD_WIRE,
};
use super::{lz4, varint, zlib};

/// How the value-change chains of a block are compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FstCompression {
    /// Store the chains verbatim. The block still declares the zlib pack
    /// type, which readers ignore for chains that are marked uncompressed.
    None,
    /// Deflate each chain (the pack type GTKWave writes by default).
    #[default]
    Zlib,
    /// LZ4 each chain (pack type `'4'`): faster, a little larger.
    Lz4,
}

/// A running FST capture.
///
/// Created by [`Simulator::enable_fst`](crate::sim::Simulator::enable_fst)
/// and written out by
/// [`Simulator::dump_fst`](crate::sim::Simulator::dump_fst). The handle is
/// cheap to clone and every clone refers to the same capture.
#[derive(Clone)]
pub struct FstCapture(Rc<RefCell<FstWriter>>);

impl std::fmt::Debug for FstCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let w = self.0.borrow();
        f.debug_struct("FstCapture")
            .field("handles", &w.lens.len())
            .field("blocks", &w.block_count)
            .field("changes", &w.total_changes)
            .finish_non_exhaustive()
    }
}

impl FstCapture {
    /// Selects the compression of the value-change chains. Takes effect for
    /// blocks serialised after the call, so set it right after enabling.
    pub fn set_compression(&self, compression: FstCompression) {
        self.0.borrow_mut().compression = compression;
    }

    /// Writes the hierarchy as `FST_BL_HIER_LZ4` instead of the gzip-wrapped
    /// `FST_BL_HIER`.
    pub fn set_hierarchy_lz4(&self, lz4: bool) {
        self.0.borrow_mut().hier_lz4 = lz4;
    }

    /// Starts a new value-change block once `changes` changes have been
    /// recorded in the current one. Zero (the default) keeps everything in
    /// one block.
    pub fn set_flush_interval(&self, changes: usize) {
        self.0.borrow_mut().flush_interval = changes;
    }

    /// Overrides the writer string stored in the header (at most 128 bytes,
    /// NUL padded).
    pub fn set_version(&self, version: &str) {
        self.0.borrow_mut().version = version.to_owned();
    }

    /// Overrides the creation date stored in the header (at most 119 bytes,
    /// NUL padded). The default is fixed, so output is reproducible.
    pub fn set_date(&self, date: &str) {
        self.0.borrow_mut().date = date.to_owned();
    }

    /// The number of value changes recorded so far.
    pub fn changes(&self) -> u64 {
        self.0.borrow().total_changes
    }

    /// The complete FST file, ending at `end_time` ticks.
    ///
    /// Calling it does not disturb the capture, so a run may be dumped more
    /// than once.
    pub fn to_bytes(&self, end_time: u64) -> Vec<u8> {
        self.0.borrow().to_bytes(end_time)
    }
}

/// The accumulated waveform.
pub(crate) struct FstWriter {
    compression: FstCompression,
    hier_lz4: bool,
    flush_interval: usize,
    version: String,
    date: String,
    /// Power of ten of a second that one tick represents.
    timescale: i8,
    /// Ticks are multiplied by this before being written, for precisions
    /// that are not a power of ten.
    time_mult: u64,
    /// Instance count, for the header.
    scope_count: u64,
    /// Hierarchy var entries, aliases included.
    var_count: u64,
    /// The hierarchy stream (scope, upscope and var entries).
    hier: Vec<u8>,
    /// The geometry stream: one varint per handle.
    geom: Vec<u8>,
    /// Value length in bytes per handle (bit count, or 8 for a real).
    lens: Vec<u32>,
    /// Which handles hold a double.
    reals: Vec<bool>,
    /// Offset of each handle's value in the frame.
    offsets: Vec<usize>,
    /// Values when the current block started.
    frame: Vec<u8>,
    /// Values as of the last recorded change.
    current: Vec<u8>,
    /// Encoded changes of the current block, per handle.
    chains: Vec<Vec<u8>>,
    /// Time-table index of each handle's last change in this block.
    last_index: Vec<u64>,
    /// The times of the current block, ascending.
    times: Vec<u64>,
    /// Time of the first entry of the current block when it has none yet.
    block_start: u64,
    /// The earliest time in the capture.
    first_time: u64,
    /// The latest time recorded.
    last_time: u64,
    /// Already serialised value-change blocks.
    blocks: Vec<u8>,
    /// How many of them there are.
    block_count: u64,
    /// Changes in the current block.
    pending: usize,
    /// Changes over the whole capture.
    total_changes: u64,
}

impl FstWriter {
    /// Records a value change of `handle` (zero based) at `time`.
    fn change(&mut self, time: u64, handle: usize, value: &Logic) {
        let real = self.reals[handle];
        let len = self.lens[handle];
        if len == 0 {
            return;
        }
        let time = time.saturating_mul(self.time_mult);
        if self.times.last() != Some(&time) {
            self.times.push(time);
            self.last_time = time;
        }
        let index = u64::try_from(self.times.len() - 1).expect("time index fits in a u64");
        let delta = index - self.last_index[handle];
        self.last_index[handle] = index;
        let bytes = value_bytes(value, len, real);
        let offset = self.offsets[handle];
        self.current[offset..offset + bytes.len()].copy_from_slice(&bytes);
        encode_change(&mut self.chains[handle], delta, real, &bytes);
        self.pending += 1;
        self.total_changes += 1;
        if self.flush_interval != 0 && self.pending >= self.flush_interval {
            self.flush();
        }
    }

    /// Closes the current block and starts a new one from the current
    /// values.
    fn flush(&mut self) {
        if self.times.is_empty() {
            return;
        }
        let block = self.encode_block(&self.frame, &self.times, &self.chains);
        self.blocks.extend_from_slice(&block);
        self.block_count += 1;
        self.block_start = *self.times.last().expect("the block has a time");
        self.times.clear();
        self.frame.clone_from(&self.current);
        for chain in &mut self.chains {
            chain.clear();
        }
        self.last_index.fill(0);
        self.pending = 0;
    }

    /// The whole file, with the capture closed off at `end_time` ticks.
    fn to_bytes(&self, end_time: u64) -> Vec<u8> {
        let end_time = end_time.saturating_mul(self.time_mult);
        let mut blocks = self.blocks.clone();
        let mut block_count = self.block_count;
        let mut times = self.times.clone();
        if !times.is_empty() || block_count == 0 {
            if times.is_empty() {
                times.push(self.block_start);
            }
            if end_time > *times.last().expect("the block has a time") {
                times.push(end_time);
            }
            blocks.extend_from_slice(&self.encode_block(&self.frame, &times, &self.chains));
            block_count += 1;
        }
        let last = times.last().copied().unwrap_or(self.last_time);
        let mut out = self.header(self.first_time, last.max(end_time), block_count);
        out.extend_from_slice(&blocks);
        out.extend_from_slice(&self.geometry_block());
        out.extend_from_slice(&self.hierarchy_block());
        out
    }

    /// The `FST_BL_HDR` section.
    fn header(&self, start: u64, end: u64, block_count: u64) -> Vec<u8> {
        let mut out = vec![BL_HDR];
        out.extend_from_slice(&329u64.to_be_bytes());
        out.extend_from_slice(&start.to_be_bytes());
        out.extend_from_slice(&end.to_be_bytes());
        // Written little endian on every host; a reader on a big endian
        // machine detects the mismatch here and byte swaps reals to match.
        out.extend_from_slice(&DOUBLE_ENDTEST.to_bits().to_le_bytes());
        let frame_len = u64::try_from(self.frame.len()).expect("frame length fits in a u64");
        out.extend_from_slice(&frame_len.to_be_bytes());
        out.extend_from_slice(&self.scope_count.to_be_bytes());
        out.extend_from_slice(&self.var_count.to_be_bytes());
        let maxhandle = u64::try_from(self.lens.len()).expect("handle count fits in a u64");
        out.extend_from_slice(&maxhandle.to_be_bytes());
        out.extend_from_slice(&block_count.to_be_bytes());
        out.push(self.timescale.cast_unsigned());
        out.extend_from_slice(&fixed_string(&self.version, HDR_VERSION_SIZE));
        out.extend_from_slice(&fixed_string(&self.date, HDR_DATE_SIZE));
        out.push(0); // FST_FT_VERILOG
        out.extend_from_slice(&0u64.to_be_bytes()); // time zero
        out
    }

    /// The `FST_BL_GEOM` section: the value length of every handle.
    fn geometry_block(&self) -> Vec<u8> {
        let packed = zlib::compress(&self.geom);
        let data = if packed.len() < self.geom.len() {
            &packed
        } else {
            &self.geom
        };
        let mut out = vec![BL_GEOM];
        let seclen = u64::try_from(data.len() + 24).expect("section length fits in a u64");
        out.extend_from_slice(&seclen.to_be_bytes());
        let uclen = u64::try_from(self.geom.len()).expect("geometry length fits in a u64");
        out.extend_from_slice(&uclen.to_be_bytes());
        let maxhandle = u64::try_from(self.lens.len()).expect("handle count fits in a u64");
        out.extend_from_slice(&maxhandle.to_be_bytes());
        out.extend_from_slice(data);
        out
    }

    /// The `FST_BL_HIER` (or `FST_BL_HIER_LZ4`) section.
    fn hierarchy_block(&self) -> Vec<u8> {
        let (tag, data) = if self.hier_lz4 {
            (BL_HIER_LZ4, lz4::compress(&self.hier))
        } else {
            (BL_HIER, zlib::gzip_compress(&self.hier))
        };
        let mut out = vec![tag];
        let seclen = u64::try_from(data.len() + 16).expect("section length fits in a u64");
        out.extend_from_slice(&seclen.to_be_bytes());
        let uclen = u64::try_from(self.hier.len()).expect("hierarchy length fits in a u64");
        out.extend_from_slice(&uclen.to_be_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// Serialises one value-change block.
    fn encode_block(&self, frame: &[u8], times: &[u64], chains: &[Vec<u8>]) -> Vec<u8> {
        let begin = times.first().copied().unwrap_or(self.block_start);
        let end = times.last().copied().unwrap_or(begin);

        // Frame: the value of every handle when the block started.
        let frame_packed = zlib::compress(frame);
        let frame_data = if frame_packed.len() < frame.len() {
            &frame_packed
        } else {
            frame
        };

        // Value changes: one optionally compressed chain per handle that
        // changed, after a pack-type byte that offset zero points at.
        let mut vc = vec![self.pack_type()];
        let mut positions = vec![None; chains.len()];
        let mut memreq = 0u64;
        for (handle, chain) in chains.iter().enumerate() {
            if chain.is_empty() {
                continue;
            }
            positions[handle] = Some(vc.len());
            memreq += u64::try_from(chain.len()).expect("chain length fits in a u64");
            let packed = if chain.len() > 32 {
                match self.compression {
                    FstCompression::None => None,
                    FstCompression::Zlib => Some(zlib::compress(chain)),
                    FstCompression::Lz4 => Some(lz4::compress(chain)),
                }
            } else {
                None
            };
            match packed {
                Some(bytes) if bytes.len() < chain.len() => {
                    let len = u64::try_from(chain.len()).expect("chain length fits in a u64");
                    varint::write_u64(&mut vc, len);
                    vc.extend_from_slice(&bytes);
                }
                _ => {
                    varint::write_u64(&mut vc, 0);
                    vc.extend_from_slice(chain);
                }
            }
        }

        // Chain index: a delta to each chain's offset, with runs of handles
        // that did not change collapsed into a count.
        let mut index = Vec::new();
        let mut previous = 0usize;
        let mut zeros = 0u64;
        for position in &positions {
            match position {
                None => zeros += 1,
                Some(offset) => {
                    if zeros != 0 {
                        varint::write_u64(&mut index, zeros << 1);
                        zeros = 0;
                    }
                    let delta = u64::try_from(offset - previous).expect("offset fits in a u64");
                    varint::write_u64(&mut index, (delta << 1) | 1);
                    previous = *offset;
                }
            }
        }
        if zeros != 0 {
            varint::write_u64(&mut index, zeros << 1);
        }

        // Time table: varint deltas from zero.
        let mut table = Vec::new();
        let mut previous_time = 0u64;
        for time in times {
            varint::write_u64(&mut table, time - previous_time);
            previous_time = *time;
        }
        let table_packed = zlib::compress(&table);
        let table_data = if table_packed.len() < table.len() {
            &table_packed
        } else {
            &table
        };

        let mut body = Vec::new();
        body.extend_from_slice(&begin.to_be_bytes());
        body.extend_from_slice(&end.to_be_bytes());
        body.extend_from_slice(&memreq.to_be_bytes());
        let frame_uclen = u64::try_from(frame.len()).expect("frame length fits in a u64");
        varint::write_u64(&mut body, frame_uclen);
        let frame_clen = u64::try_from(frame_data.len()).expect("frame length fits in a u64");
        varint::write_u64(&mut body, frame_clen);
        let maxhandle = u64::try_from(chains.len()).expect("handle count fits in a u64");
        varint::write_u64(&mut body, maxhandle);
        body.extend_from_slice(frame_data);
        varint::write_u64(&mut body, maxhandle);
        body.extend_from_slice(&vc);
        body.extend_from_slice(&index);
        let index_len = u64::try_from(index.len()).expect("index length fits in a u64");
        body.extend_from_slice(&index_len.to_be_bytes());
        body.extend_from_slice(table_data);
        let table_uclen = u64::try_from(table.len()).expect("time table fits in a u64");
        body.extend_from_slice(&table_uclen.to_be_bytes());
        let table_clen = u64::try_from(table_data.len()).expect("time table fits in a u64");
        body.extend_from_slice(&table_clen.to_be_bytes());
        let items = u64::try_from(times.len()).expect("time count fits in a u64");
        body.extend_from_slice(&items.to_be_bytes());

        let mut out = vec![BL_VCDATA];
        let seclen = u64::try_from(body.len() + 8).expect("section length fits in a u64");
        out.extend_from_slice(&seclen.to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// The pack-type byte of a value-change block.
    fn pack_type(&self) -> u8 {
        match self.compression {
            FstCompression::Lz4 => b'4',
            // Anything but '4' or 'F' means zlib; chains that are stored
            // verbatim are marked as such by their own length varint.
            FstCompression::None | FstCompression::Zlib => b'Z',
        }
    }
}

/// Appends one change to a handle's chain.
///
/// A single bit packs the value into the varint itself: two bits of value
/// and the rest of the time delta. A vector whose bits are all `0` or `1`
/// is written as a packed bit string (the delta varint's low bit clear),
/// anything else as one ASCII character per bit (low bit set). A real is
/// the eight little-endian bytes of the double, in the same per-byte form.
fn encode_change(chain: &mut Vec<u8>, delta: u64, real: bool, bytes: &[u8]) {
    if !real && bytes.len() == 1 {
        let code = match bytes[0] {
            b'0' => delta << 2,
            b'1' => (delta << 2) | 2,
            b'z' => (delta << 4) | 3,
            // 'x' and anything else the simulator cannot produce.
            _ => (delta << 4) | 1,
        };
        varint::write_u64(chain, code);
        return;
    }
    if !real && bytes.iter().all(|b| *b == b'0' || *b == b'1') {
        varint::write_u64(chain, delta << 1);
        let mut byte = 0u8;
        for (i, bit) in bytes.iter().enumerate() {
            byte = (byte << 1) | (bit & 1);
            if i % 8 == 7 {
                chain.push(byte);
                byte = 0;
            }
        }
        let leftover = bytes.len() % 8;
        if leftover != 0 {
            let shift = u32::try_from(8 - leftover).expect("below eight");
            chain.push(byte << shift);
        }
        return;
    }
    varint::write_u64(chain, (delta << 1) | 1);
    chain.extend_from_slice(bytes);
}

/// The stored bytes of a value: one ASCII character per bit, most
/// significant first, or the eight little-endian bytes of a double.
fn value_bytes(value: &Logic, len: u32, real: bool) -> Vec<u8> {
    if real {
        let bits = value.to_u64().unwrap_or_else(|| f64::NAN.to_bits());
        return bits.to_le_bytes().to_vec();
    }
    (0..len)
        .rev()
        .map(|i| {
            let bit = value.get(i).unwrap_or(Bit::X);
            u8::try_from(u32::from(bit.to_char())).expect("a logic character is ASCII")
        })
        .collect()
}

/// A NUL-padded fixed-width header string.
fn fixed_string(text: &str, size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size];
    let bytes = text.as_bytes();
    let n = bytes.len().min(size - 1);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

/// The FST timescale exponent (a power of ten of a second) for a precision
/// in femtoseconds, plus the factor ticks must be multiplied by.
///
/// Every power of ten maps exactly; anything else is recorded in
/// femtoseconds with the times scaled, as the VCD writer does.
pub(crate) fn timescale_exponent(precision_fs: u64) -> (i8, u64) {
    let mut unit = 1u64;
    for exponent in -15i8..=3 {
        if precision_fs == unit {
            return (exponent, 1);
        }
        match unit.checked_mul(10) {
            Some(next) => unit = next,
            None => break,
        }
    }
    (-15, precision_fs.max(1))
}

impl<'d> Simulator<'d> {
    /// Builds an FST capture over the current state and wires it to every
    /// signal's change callback.
    pub(crate) fn build_fst(&mut self) -> FstCapture {
        let (writer, signals) = self.fst_structure();
        let shared = Rc::new(RefCell::new(writer));
        for (sig, handle) in signals {
            let writer = Rc::clone(&shared);
            self.on_change(NetHandle(sig), move |time, value| {
                writer.borrow_mut().change(time, handle, value);
            });
        }
        FstCapture(shared)
    }

    /// Walks the instance tree, assigning handles and building the
    /// hierarchy, geometry and frame.
    fn fst_structure(&self) -> (FstWriter, Vec<(SigId, usize)>) {
        let (timescale, time_mult) = timescale_exponent(self.precision_fs);
        let mut writer = FstWriter {
            compression: FstCompression::default(),
            hier_lz4: false,
            flush_interval: 0,
            version: format!("Reticle {}", crate::VERSION),
            date: DEFAULT_DATE.to_owned(),
            timescale,
            time_mult,
            scope_count: 0,
            var_count: 0,
            hier: Vec::new(),
            geom: Vec::new(),
            lens: Vec::new(),
            reals: Vec::new(),
            offsets: Vec::new(),
            frame: Vec::new(),
            current: Vec::new(),
            chains: Vec::new(),
            last_index: Vec::new(),
            times: Vec::new(),
            block_start: self.now.saturating_mul(time_mult),
            first_time: self.now.saturating_mul(time_mult),
            last_time: self.now.saturating_mul(time_mult),
            blocks: Vec::new(),
            block_count: 0,
            pending: 0,
            total_changes: 0,
        };
        let mut handles: Vec<Option<usize>> = vec![None; self.signals.len()];
        let mut signals = Vec::new();
        if !self.instances.is_empty() {
            self.fst_scope(&mut writer, &mut handles, &mut signals, InstId(0));
        }
        writer.current.clone_from(&writer.frame);
        writer.chains = vec![Vec::new(); writer.lens.len()];
        writer.last_index = vec![0; writer.lens.len()];
        (writer, signals)
    }

    /// Emits one instance's scope, its vars and its children.
    fn fst_scope(
        &self,
        writer: &mut FstWriter,
        handles: &mut [Option<usize>],
        signals: &mut Vec<(SigId, usize)>,
        inst: InstId,
    ) {
        let state = &self.instances[inst.idx()];
        writer.hier.push(ST_VCD_SCOPE);
        writer.hier.push(0); // FST_ST_VCD_MODULE
        writer.hier.extend_from_slice(state.name.as_bytes());
        writer.hier.push(0);
        writer.hier.push(0); // no component name
        writer.scope_count += 1;
        for (nid, net) in state.m.nets.iter() {
            let sig = state.nets[nid.index()];
            let signal = &self.signals[sig.idx()];
            let width = signal.width();
            if width == 0 {
                continue;
            }
            let real = net.ty == Type::Real;
            let var_type = match &net.ty {
                Type::Real => VT_VCD_REAL,
                Type::Integer => VT_VCD_INTEGER,
                _ => match signal.kind {
                    NetKind::Wire => VT_VCD_WIRE,
                    NetKind::Reg | NetKind::Variable => VT_VCD_REG,
                },
            };
            let name = if width == 1 || real {
                net.name.to_string()
            } else {
                format!("{} [{}:0]", net.name, width - 1)
            };
            let len = if real { 8 } else { width };
            let alias = match handles[sig.idx()] {
                Some(existing) => u64::try_from(existing + 1).expect("handle fits in a u64"),
                None => {
                    let handle = writer.lens.len();
                    handles[sig.idx()] = Some(handle);
                    writer.lens.push(len);
                    writer.reals.push(real);
                    writer.offsets.push(writer.frame.len());
                    let bytes = value_bytes(signal.effective(), len, real);
                    writer.frame.extend_from_slice(&bytes);
                    varint::write_u64(&mut writer.geom, if real { 0 } else { u64::from(len) });
                    signals.push((sig, handle));
                    0
                }
            };
            writer.hier.push(var_type);
            writer.hier.push(0); // FST_VD_IMPLICIT
            writer.hier.extend_from_slice(name.as_bytes());
            writer.hier.push(0);
            varint::write_u64(&mut writer.hier, u64::from(len));
            varint::write_u64(&mut writer.hier, alias);
            writer.var_count += 1;
        }
        for child in &state.children {
            self.fst_scope(writer, handles, signals, *child);
        }
        writer.hier.push(ST_VCD_UPSCOPE);
    }
}

/// The header date of a capture, fixed so that output is reproducible.
const DEFAULT_DATE: &str = "Thu Jan  1 00:00:00 1970";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timescale_exponents() {
        assert_eq!(timescale_exponent(1), (-15, 1));
        assert_eq!(timescale_exponent(1_000), (-12, 1));
        assert_eq!(timescale_exponent(1_000_000), (-9, 1));
        assert_eq!(timescale_exponent(10_000_000), (-8, 1));
        assert_eq!(timescale_exponent(1_000_000_000_000_000), (0, 1));
        assert_eq!(timescale_exponent(123), (-15, 123));
    }

    #[test]
    fn single_bit_changes_pack_into_the_varint() {
        let mut chain = Vec::new();
        encode_change(&mut chain, 0, false, b"0");
        assert_eq!(chain, [0]);
        chain.clear();
        encode_change(&mut chain, 0, false, b"1");
        assert_eq!(chain, [2]);
        chain.clear();
        encode_change(&mut chain, 3, false, b"x");
        assert_eq!(chain, [(3 << 4) | 1]);
        chain.clear();
        encode_change(&mut chain, 1, false, b"z");
        assert_eq!(chain, [(1 << 4) | 3]);
    }

    #[test]
    fn vectors_pack_when_two_state() {
        let mut chain = Vec::new();
        encode_change(&mut chain, 2, false, b"1010");
        assert_eq!(chain, [2 << 1, 0b1010_0000]);
        chain.clear();
        encode_change(&mut chain, 0, false, b"10xz");
        assert_eq!(chain, [1, b'1', b'0', b'x', b'z']);
        chain.clear();
        encode_change(&mut chain, 0, false, b"1111111100000001");
        assert_eq!(chain, [0, 0xff, 0x01]);
    }

    #[test]
    fn reals_store_eight_raw_bytes() {
        let mut chain = Vec::new();
        let value = Logic::from_u64(1.5f64.to_bits(), 64);
        let bytes = value_bytes(&value, 8, true);
        assert_eq!(bytes, 1.5f64.to_bits().to_le_bytes());
        encode_change(&mut chain, 0, true, &bytes);
        assert_eq!(chain[0], 1);
        assert_eq!(&chain[1..], &1.5f64.to_bits().to_le_bytes());
    }

    #[test]
    fn value_bytes_are_msb_first() {
        let value = Logic::parse_verilog("4'b10xz").unwrap();
        assert_eq!(value_bytes(&value, 4, false), b"10xz");
    }

    #[test]
    fn header_strings_are_padded() {
        let padded = fixed_string("abc", 8);
        assert_eq!(padded, [b'a', b'b', b'c', 0, 0, 0, 0, 0]);
        let clipped = fixed_string("abcdefghij", 4);
        assert_eq!(clipped, [b'a', b'b', b'c', 0]);
    }
}
