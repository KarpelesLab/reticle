//! A structural FST reader.
//!
//! This is the writer's mirror, and exists so the writer can be checked
//! without a copy of GTKWave: it walks the block list, decompresses the
//! geometry, hierarchy, frame, chain and time sections, and rebuilds the
//! flat list of `(time, handle, value)` changes that a waveform viewer would
//! display. It reads the blocks this crate emits and, being written from the
//! format rather than from the writer, the ones `fstapi.c` emits as well:
//! `FST_BL_VCDATA` and both dynamic-alias variants, gzip or LZ4 hierarchy
//! blocks, and zlib, LZ4 or uncompressed chains.
//!
//! It is not a streaming reader. The whole file is a `&[u8]` and the whole
//! change list comes back in memory, which suits tests and small dumps and
//! is why it lives next to the writer rather than pretending to be a
//! viewer's loader.

use super::{
    BL_BLACKOUT, BL_GEOM, BL_HDR, BL_HIER, BL_HIER_LZ4, BL_SKIP, BL_VCDATA, BL_VCDATA_DYN_ALIAS,
    BL_VCDATA_DYN_ALIAS2, RCV_STR, ST_GEN_ATTRBEGIN, ST_GEN_ATTREND, ST_VCD_SCOPE, ST_VCD_UPSCOPE,
};
use super::{lz4, varint, zlib};

/// One variable of the hierarchy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FstVar {
    /// The dotted scope path, top module first, without the variable name.
    pub scope: String,
    /// The variable name as stored, including any `[msb:lsb]` suffix.
    pub name: String,
    /// The `FST_VT_*` type byte.
    pub var_type: u8,
    /// The declared length: bits, or 8 for a real.
    pub length: u32,
    /// The one-based handle this name resolves to. Several names may share
    /// one handle; those are the aliases.
    pub handle: u32,
}

/// A decoded value.
#[derive(Clone, Debug, PartialEq)]
pub enum FstValue {
    /// One ASCII character per bit, most significant first.
    Bits(String),
    /// A double, for `FST_VT_VCD_REAL` variables.
    Real(f64),
}

impl std::fmt::Display for FstValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FstValue::Bits(bits) => f.write_str(bits),
            FstValue::Real(value) => write!(f, "r{value}"),
        }
    }
}

/// One value change.
#[derive(Clone, Debug, PartialEq)]
pub struct FstChange {
    /// The time, in units of the file's timescale exponent.
    pub time: u64,
    /// The one-based handle that changed.
    pub handle: u32,
    /// Its new value.
    pub value: FstValue,
}

/// A parsed FST file.
#[derive(Clone, Debug)]
pub struct FstFile {
    /// The first time in the file.
    pub start_time: u64,
    /// The last time in the file.
    pub end_time: u64,
    /// The power of ten of a second that one time unit represents.
    pub timescale: i8,
    /// The writer's version string.
    pub version: String,
    /// The writer's date string.
    pub date: String,
    /// The number of scopes the header declares.
    pub scope_count: u64,
    /// The number of hierarchy variables the header declares.
    pub var_count: u64,
    /// The number of value-change blocks the header declares.
    pub block_count: u64,
    /// Declared length of each handle, indexed by handle minus one.
    pub lengths: Vec<u32>,
    /// Which handles hold a double.
    pub reals: Vec<bool>,
    /// The hierarchy, in file order.
    pub vars: Vec<FstVar>,
    /// The first block's frame: the value of every handle when the capture
    /// started, indexed by handle minus one.
    pub frame: Vec<FstValue>,
    /// Every value change, ordered by block, then time, then handle.
    pub changes: Vec<FstChange>,
}

impl FstFile {
    /// The changes of one handle, in order.
    pub fn changes_of(&self, handle: u32) -> Vec<&FstChange> {
        self.changes.iter().filter(|c| c.handle == handle).collect()
    }

    /// The variable whose scope and name match `path`, a dotted hierarchical
    /// name without any `[msb:lsb]` suffix.
    pub fn var(&self, path: &str) -> Option<&FstVar> {
        self.vars.iter().find(|v| {
            let bare = v.name.split(' ').next().unwrap_or(&v.name);
            format!("{}.{bare}", v.scope) == path
        })
    }
}

/// Parses a complete FST file.
///
/// # Errors
///
/// Returns a description of the first structural problem found: a truncated
/// or mistyped block, a section length that does not fit, a compressed
/// section that does not inflate, or a value chain that runs off its end.
pub fn read(bytes: &[u8]) -> Result<FstFile, String> {
    let mut file = FstFile {
        start_time: 0,
        end_time: 0,
        timescale: 0,
        version: String::new(),
        date: String::new(),
        scope_count: 0,
        var_count: 0,
        block_count: 0,
        lengths: Vec::new(),
        reals: Vec::new(),
        vars: Vec::new(),
        frame: Vec::new(),
        changes: Vec::new(),
    };
    let mut seen_header = false;
    let mut blocks: Vec<(u8, &[u8])> = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = bytes[pos];
        let seclen = read_u64(bytes, pos + 1).ok_or("truncated section length")?;
        let seclen = usize::try_from(seclen).map_err(|_| "section length out of range")?;
        if seclen < 8 {
            return Err(format!("section {tag} has an impossible length {seclen}"));
        }
        let end = pos
            .checked_add(1 + seclen)
            .ok_or("section length overflows the file")?;
        if end > bytes.len() {
            return Err(format!("section {tag} runs past the end of the file"));
        }
        let body = &bytes[pos + 9..end];
        if !seen_header && tag != BL_HDR {
            return Err("the file does not start with a header block".to_owned());
        }
        seen_header = true;
        blocks.push((tag, body));
        pos = end;
    }
    if blocks.is_empty() {
        return Err("the file is empty".to_owned());
    }

    // Header, geometry and hierarchy first: decoding a value-change block
    // needs the per-handle lengths.
    for (tag, body) in &blocks {
        match *tag {
            BL_HDR => read_header(&mut file, body)?,
            BL_GEOM => read_geometry(&mut file, body)?,
            BL_HIER | BL_HIER_LZ4 => read_hierarchy(&mut file, *tag, body)?,
            BL_BLACKOUT | BL_SKIP => {}
            BL_VCDATA | BL_VCDATA_DYN_ALIAS | BL_VCDATA_DYN_ALIAS2 => {}
            other => return Err(format!("unknown block type {other}")),
        }
    }
    let mut first = true;
    for (tag, body) in &blocks {
        if matches!(*tag, BL_VCDATA | BL_VCDATA_DYN_ALIAS | BL_VCDATA_DYN_ALIAS2) {
            read_vc_block(&mut file, *tag, body, first)?;
            first = false;
        }
    }
    Ok(file)
}

/// Reads the `FST_BL_HDR` payload.
fn read_header(file: &mut FstFile, body: &[u8]) -> Result<(), String> {
    if body.len() < 321 {
        return Err("the header block is too short".to_owned());
    }
    file.start_time = read_u64(body, 0).ok_or("short header")?;
    file.end_time = read_u64(body, 8).ok_or("short header")?;
    let endtest = f64::from_bits(u64::from_le_bytes(
        body[16..24].try_into().expect("eight bytes"),
    ));
    if endtest != super::DOUBLE_ENDTEST {
        return Err("the header endian test does not match".to_owned());
    }
    file.scope_count = read_u64(body, 32).ok_or("short header")?;
    file.var_count = read_u64(body, 40).ok_or("short header")?;
    file.block_count = read_u64(body, 56).ok_or("short header")?;
    file.timescale = body[64].cast_signed();
    file.version = c_string(&body[65..65 + super::HDR_VERSION_SIZE]);
    let date_at = 65 + super::HDR_VERSION_SIZE;
    file.date = c_string(&body[date_at..date_at + super::HDR_DATE_SIZE]);
    Ok(())
}

/// Reads the `FST_BL_GEOM` payload.
fn read_geometry(file: &mut FstFile, body: &[u8]) -> Result<(), String> {
    if body.len() < 16 {
        return Err("the geometry block is too short".to_owned());
    }
    let uclen = as_usize(read_u64(body, 0).ok_or("short geometry")?)?;
    let handles = as_usize(read_u64(body, 8).ok_or("short geometry")?)?;
    let data = &body[16..];
    let plain = if data.len() == uclen {
        data.to_vec()
    } else {
        zlib::decompress(data, Some(uclen)).ok_or("the geometry block does not inflate")?
    };
    let mut pos = 0usize;
    file.lengths = Vec::with_capacity(handles);
    file.reals = Vec::with_capacity(handles);
    for _ in 0..handles {
        let value = varint::read_u64(&plain, &mut pos).ok_or("truncated geometry")?;
        if value == 0 {
            file.lengths.push(8);
            file.reals.push(true);
        } else if value == 0xffff_ffff {
            return Err("variable length variables are not supported".to_owned());
        } else {
            file.lengths
                .push(u32::try_from(value).map_err(|_| "a variable is too wide")?);
            file.reals.push(false);
        }
    }
    Ok(())
}

/// Reads an `FST_BL_HIER` or `FST_BL_HIER_LZ4` payload.
fn read_hierarchy(file: &mut FstFile, tag: u8, body: &[u8]) -> Result<(), String> {
    if body.len() < 8 {
        return Err("the hierarchy block is too short".to_owned());
    }
    let uclen = as_usize(read_u64(body, 0).ok_or("short hierarchy")?)?;
    let data = &body[8..];
    let plain = if tag == BL_HIER_LZ4 {
        lz4::decompress(data, uclen).ok_or("the hierarchy block does not decompress")?
    } else {
        zlib::gzip_decompress(data, Some(uclen)).ok_or("the hierarchy block does not inflate")?
    };
    let mut scopes: Vec<String> = Vec::new();
    let mut next_handle = 0u32;
    let mut pos = 0usize;
    while pos < plain.len() {
        let tag = plain[pos];
        pos += 1;
        match tag {
            ST_VCD_SCOPE => {
                if pos >= plain.len() {
                    return Err("truncated scope entry".to_owned());
                }
                pos += 1; // scope type
                let name = read_c_string(&plain, &mut pos)?;
                let _component = read_c_string(&plain, &mut pos)?;
                scopes.push(name);
            }
            ST_VCD_UPSCOPE => {
                scopes.pop();
            }
            ST_GEN_ATTRBEGIN => {
                if pos + 2 > plain.len() {
                    return Err("truncated attribute entry".to_owned());
                }
                pos += 2;
                let _name = read_c_string(&plain, &mut pos)?;
                varint::read_u64(&plain, &mut pos).ok_or("truncated attribute argument")?;
            }
            ST_GEN_ATTREND => {}
            var_type => {
                if pos >= plain.len() {
                    return Err("truncated variable entry".to_owned());
                }
                pos += 1; // direction
                let name = read_c_string(&plain, &mut pos)?;
                let length = varint::read_u64(&plain, &mut pos).ok_or("truncated var length")?;
                let alias = varint::read_u64(&plain, &mut pos).ok_or("truncated var alias")?;
                let handle = if alias == 0 {
                    next_handle += 1;
                    next_handle
                } else {
                    u32::try_from(alias).map_err(|_| "alias handle out of range")?
                };
                file.vars.push(FstVar {
                    scope: scopes.join("."),
                    name,
                    var_type,
                    length: u32::try_from(length).map_err(|_| "a variable is too wide")?,
                    handle,
                });
            }
        }
    }
    Ok(())
}

/// Reads one value-change block, appending its changes.
fn read_vc_block(file: &mut FstFile, tag: u8, body: &[u8], first: bool) -> Result<(), String> {
    if body.len() < 48 {
        return Err("a value change block is too short".to_owned());
    }
    let mut pos = 24usize; // past begin time, end time and the memory hint
    let frame_uclen = as_usize(varint::read_u64(body, &mut pos).ok_or("short frame")?)?;
    let frame_clen = as_usize(varint::read_u64(body, &mut pos).ok_or("short frame")?)?;
    let frame_handles = as_usize(varint::read_u64(body, &mut pos).ok_or("short frame")?)?;
    let frame_data = body
        .get(pos..pos + frame_clen)
        .ok_or("the frame runs past the block")?;
    let frame = if frame_uclen == frame_clen {
        frame_data.to_vec()
    } else {
        zlib::decompress(frame_data, Some(frame_uclen)).ok_or("the frame does not inflate")?
    };
    pos += frame_clen;
    let handles = as_usize(varint::read_u64(body, &mut pos).ok_or("short handle count")?)?;
    let vc_start = pos;

    // Trailer, read back to front.
    let len = body.len();
    let time_items = as_usize(read_u64(body, len - 8).ok_or("short time table")?)?;
    let time_clen = as_usize(read_u64(body, len - 16).ok_or("short time table")?)?;
    let time_uclen = as_usize(read_u64(body, len - 24).ok_or("short time table")?)?;
    let time_at = len
        .checked_sub(24 + time_clen)
        .ok_or("the time table does not fit in the block")?;
    let time_data = &body[time_at..len - 24];
    let time_plain = if time_uclen == time_clen {
        time_data.to_vec()
    } else {
        zlib::decompress(time_data, Some(time_uclen)).ok_or("the time table does not inflate")?
    };
    let mut times = Vec::with_capacity(time_items);
    let mut tpos = 0usize;
    let mut now = 0u64;
    for _ in 0..time_items {
        now += varint::read_u64(&time_plain, &mut tpos).ok_or("truncated time table")?;
        times.push(now);
    }

    let index_len_at = time_at
        .checked_sub(8)
        .ok_or("the chain index does not fit in the block")?;
    let index_len = as_usize(read_u64(body, index_len_at).ok_or("short chain index")?)?;
    let index_at = index_len_at
        .checked_sub(index_len)
        .ok_or("the chain index does not fit in the block")?;
    let index = &body[index_at..index_len_at];
    let sentinel = index_at
        .checked_sub(vc_start)
        .ok_or("the chain index starts before the value changes")?;

    // Chain index: offsets, relative to the pack-type byte, of the handles
    // that changed in this block.
    let mut offsets: Vec<usize> = vec![0; handles.max(frame_handles)];
    let mut aliases: Vec<Option<usize>> = vec![None; offsets.len()];
    let dynamic = tag != BL_VCDATA;
    let mut ipos = 0usize;
    let mut handle = 0usize;
    let mut previous = 0usize;
    let mut previous_alias = 0i64;
    while ipos < index.len() {
        if handle >= offsets.len() {
            return Err("the chain index names more handles than the block has".to_owned());
        }
        if tag == BL_VCDATA_DYN_ALIAS2 {
            if index[ipos] & 1 == 1 {
                let value = varint::read_i64(index, &mut ipos).ok_or("truncated chain index")? >> 1;
                if value > 0 {
                    previous += usize::try_from(value).map_err(|_| "chain offset out of range")?;
                    offsets[handle] = previous;
                } else {
                    if value < 0 {
                        previous_alias = value;
                    }
                    aliases[handle] = alias_target(previous_alias);
                }
                handle += 1;
            } else {
                let run = varint::read_u64(index, &mut ipos).ok_or("truncated chain index")? >> 1;
                handle += as_usize(run)?;
            }
            continue;
        }
        let value = varint::read_u64(index, &mut ipos).ok_or("truncated chain index")?;
        if value == 0 {
            if !dynamic {
                return Err("a plain value change block used a dynamic alias".to_owned());
            }
            let target = varint::read_u64(index, &mut ipos).ok_or("truncated chain alias")?;
            aliases[handle] = alias_target(-i64::try_from(target).unwrap_or(0));
            handle += 1;
        } else if value & 1 == 1 {
            previous += as_usize(value >> 1)?;
            offsets[handle] = previous;
            handle += 1;
        } else {
            handle += as_usize(value >> 1)?;
        }
    }

    // Chain lengths run to the next chain that is present, and the last one
    // to the start of the index.
    let present: Vec<usize> = (0..offsets.len()).filter(|h| offsets[*h] != 0).collect();
    let mut chains: Vec<Option<Vec<u8>>> = vec![None; offsets.len()];
    for (i, handle) in present.iter().enumerate() {
        let start = offsets[*handle];
        let stop = present.get(i + 1).map_or(sentinel, |next| offsets[*next]);
        if stop < start || vc_start + stop > body.len() {
            return Err("a value chain runs past the block".to_owned());
        }
        let raw = &body[vc_start + start..vc_start + stop];
        let mut cpos = 0usize;
        let uclen = as_usize(varint::read_u64(raw, &mut cpos).ok_or("truncated chain")?)?;
        let data = &raw[cpos..];
        let plain = if uclen == 0 {
            data.to_vec()
        } else {
            match body[vc_start] {
                b'4' => lz4::decompress(data, uclen).ok_or("a chain does not decompress")?,
                b'F' => return Err("fastlz compressed chains are not supported".to_owned()),
                _ => zlib::decompress(data, Some(uclen)).ok_or("a chain does not inflate")?,
            }
        };
        chains[*handle] = Some(plain);
    }
    for handle in 0..chains.len() {
        if let Some(target) = aliases[handle]
            && target < chains.len()
        {
            chains[handle] = chains[target].clone();
        }
    }

    // Frame, for the first block only: the state the capture started from.
    if first {
        let mut offset = 0usize;
        for handle in 0..frame_handles.min(file.lengths.len()) {
            let len = as_usize(u64::from(file.lengths[handle]))?;
            let bytes = frame
                .get(offset..offset + len)
                .ok_or("the frame is shorter than the geometry")?;
            file.frame.push(decode_value(bytes, file.reals[handle])?);
            offset += len;
        }
    }

    // Decode every chain, then order the changes by time and handle.
    let mut decoded: Vec<(usize, u32, FstValue)> = Vec::new();
    for (handle, chain) in chains.iter().enumerate() {
        let Some(chain) = chain else { continue };
        if handle >= file.lengths.len() {
            return Err("a chain names a handle the geometry does not describe".to_owned());
        }
        let len = file.lengths[handle];
        let real = file.reals[handle];
        let number = u32::try_from(handle + 1).map_err(|_| "handle out of range")?;
        for (index, value) in decode_chain(chain, len, real)? {
            decoded.push((index, number, value));
        }
    }
    decoded.sort_by_key(|(index, handle, _)| (*index, *handle));
    for (index, handle, value) in decoded {
        let time = *times
            .get(index)
            .ok_or("a change names a time the time table does not have")?;
        file.changes.push(FstChange {
            time,
            handle,
            value,
        });
    }
    Ok(())
}

/// The zero-based handle a dynamic alias entry points at.
fn alias_target(alias: i64) -> Option<usize> {
    if alias >= 0 {
        return None;
    }
    let target = (-alias) - 1;
    usize::try_from(target).ok()
}

/// Decodes one handle's chain into `(time index, value)` pairs.
fn decode_chain(chain: &[u8], len: u32, real: bool) -> Result<Vec<(usize, FstValue)>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut index = 0u64;
    let width = as_usize(u64::from(len))?;
    while pos < chain.len() {
        let vli = varint::read_u64(chain, &mut pos).ok_or("truncated value chain")?;
        if width == 1 && !real {
            let shift = if vli & 1 == 1 { 4 } else { 2 };
            index += vli >> shift;
            let character = if vli & 1 == 0 {
                if (vli >> 1) & 1 == 1 { '1' } else { '0' }
            } else {
                let slot = as_usize((vli >> 1) & 7)?;
                char::from(RCV_STR.as_bytes()[slot])
            };
            out.push((as_usize(index)?, FstValue::Bits(character.to_string())));
            continue;
        }
        index += vli >> 1;
        let bytes = if vli & 1 == 0 {
            let packed = width.div_ceil(8);
            let data = chain
                .get(pos..pos + packed)
                .ok_or("a packed value runs past its chain")?;
            pos += packed;
            let mut expanded = Vec::with_capacity(width);
            for bit in 0..width {
                let byte = data[bit / 8];
                let value = (byte >> (7 - (bit % 8))) & 1;
                expanded.push(b'0' + value);
            }
            expanded
        } else {
            let data = chain
                .get(pos..pos + width)
                .ok_or("a value runs past its chain")?;
            pos += width;
            data.to_vec()
        };
        out.push((as_usize(index)?, decode_value(&bytes, real)?));
    }
    Ok(out)
}

/// Interprets stored value bytes.
fn decode_value(bytes: &[u8], real: bool) -> Result<FstValue, String> {
    if real {
        let raw: [u8; 8] = bytes
            .try_into()
            .map_err(|_| "a real value is not eight bytes".to_owned())?;
        return Ok(FstValue::Real(f64::from_bits(u64::from_le_bytes(raw))));
    }
    let text = bytes.iter().map(|b| char::from(*b)).collect();
    Ok(FstValue::Bits(text))
}

/// Reads a big-endian `u64` at `at`.
fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let slice = bytes.get(at..at + 8)?;
    Some(u64::from_be_bytes(slice.try_into().ok()?))
}

/// Narrows a file-supplied length to a `usize`.
fn as_usize(value: u64) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| "a length does not fit in memory".to_owned())
}

/// A NUL-terminated string from a fixed-width header field.
fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Reads a NUL-terminated string at `*pos`, advancing past the terminator.
fn read_c_string(bytes: &[u8], pos: &mut usize) -> Result<String, String> {
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos] != 0 {
        *pos += 1;
    }
    if *pos >= bytes.len() {
        return Err("an unterminated name in the hierarchy".to_owned());
    }
    let text = String::from_utf8_lossy(&bytes[start..*pos]).into_owned();
    *pos += 1;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage() {
        assert!(read(&[]).is_err());
        assert!(read(&[1, 0, 0, 0, 0, 0, 0, 0, 8]).is_err());
    }

    #[test]
    fn decodes_a_single_bit_chain() {
        // delta 0 value 0, delta 1 value 1, delta 2 value x.
        let chain = [0u8, (1 << 2) | 2, (2 << 4) | 1];
        let decoded = decode_chain(&chain, 1, false).unwrap();
        assert_eq!(
            decoded,
            vec![
                (0, FstValue::Bits("0".to_owned())),
                (1, FstValue::Bits("1".to_owned())),
                (3, FstValue::Bits("x".to_owned())),
            ]
        );
    }

    #[test]
    fn decodes_packed_and_raw_vectors() {
        let chain = [0u8, 0b1010_0000, 1, b'1', b'0', b'x', b'z'];
        let decoded = decode_chain(&chain, 4, false).unwrap();
        assert_eq!(
            decoded,
            vec![
                (0, FstValue::Bits("1010".to_owned())),
                (0, FstValue::Bits("10xz".to_owned())),
            ]
        );
    }
}
