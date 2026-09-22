//! An in-crate DEFLATE codec with the zlib and gzip wrappers FST needs.
//!
//! FST leans on zlib in four places: the geometry block, the value-change
//! frame, the per-signal chains and the time table are `compress2` streams
//! (zlib wrapper, RFC 1950), while the hierarchy block is written through
//! `gzdopen` and is therefore a *gzip* stream (RFC 1952). Both wrap the same
//! DEFLATE payload (RFC 1951) and differ only in header, checksum and
//! trailer, so both live here.
//!
//! # What is implemented
//!
//! The **compressor** emits a single DEFLATE block with the *fixed* Huffman
//! tables (`BTYPE = 01`) over an LZ77 pass: a chained hash table on three
//! byte sequences, a 32 KiB window and greedy matching. Dynamic Huffman
//! tables are not generated; that costs a few percent of ratio and no
//! correctness, since a fixed-table stream is an ordinary DEFLATE stream
//! that every decoder accepts. Output is deterministic for a given input.
//!
//! The **decompressor** is complete: stored, fixed and dynamic blocks, so it
//! reads streams from zlib, gzip or any other conforming writer. That is
//! what makes the FST reader in this module able to open files this crate
//! did not write.

/// The DEFLATE sliding window.
const WINDOW: usize = 32_768;
/// The shortest match DEFLATE can encode.
const MIN_MATCH: usize = 3;
/// The longest match DEFLATE can encode.
const MAX_MATCH: usize = 258;
/// How many positions of one hash bucket a match search inspects.
const MAX_CHAIN: usize = 128;
/// Log2 of the match hash table size.
const HASH_BITS: u32 = 15;

/// First length symbol; `257 + i` encodes lengths from `LENGTH_BASE[i]`.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// Extra bits carried by each length symbol.
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Base distance of each distance symbol.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12_289, 16_385, 24_577,
];
/// Extra bits carried by each distance symbol.
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order in which a dynamic block stores its code-length code lengths.
const CLEN_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// The Adler-32 checksum used by the zlib wrapper.
pub fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for chunk in data.chunks(5552) {
        for byte in chunk {
            a += u32::from(*byte);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    (b << 16) | a
}

/// The CRC-32 (IEEE) checksum used by the gzip wrapper.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Compresses `data` into a raw DEFLATE stream (RFC 1951).
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut bits = BitWriter::new(data.len() / 2 + 16);
    bits.write_bits(1, 1); // BFINAL
    bits.write_bits(1, 2); // BTYPE = fixed Huffman
    let mut head = vec![usize::MAX; 1 << HASH_BITS];
    let mut prev = vec![usize::MAX; data.len()];
    let mut i = 0usize;
    while i < data.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if i + MIN_MATCH <= data.len() {
            let h = hash3(data, i);
            let max_len = (data.len() - i).min(MAX_MATCH);
            let mut candidate = head[h];
            let mut chain = 0;
            while candidate != usize::MAX && i - candidate <= WINDOW && chain < MAX_CHAIN {
                if best_len < max_len && data[candidate + best_len] == data[i + best_len] {
                    let mut len = 0usize;
                    while len < max_len && data[candidate + len] == data[i + len] {
                        len += 1;
                    }
                    if len > best_len {
                        best_len = len;
                        best_dist = i - candidate;
                        if len == max_len {
                            break;
                        }
                    }
                }
                candidate = prev[candidate];
                chain += 1;
            }
            prev[i] = head[h];
            head[h] = i;
        }
        if best_len >= MIN_MATCH {
            emit_match(&mut bits, best_len, best_dist);
            for (j, slot) in prev.iter_mut().enumerate().take(i + best_len).skip(i + 1) {
                if j + MIN_MATCH <= data.len() {
                    let h = hash3(data, j);
                    *slot = head[h];
                    head[h] = j;
                }
            }
            i += best_len;
        } else {
            emit_literal(&mut bits, data[i]);
            i += 1;
        }
    }
    emit_symbol(&mut bits, 256);
    bits.finish()
}

/// Decompresses a raw DEFLATE stream.
///
/// `hint` is the expected output size when the container knows it, used only
/// to pre-allocate. Returns `None` on malformed input.
pub fn inflate(data: &[u8], hint: Option<usize>) -> Option<Vec<u8>> {
    let mut reader = BitReader::new(data);
    let mut out = Vec::with_capacity(hint.unwrap_or(data.len() * 4));
    loop {
        let final_block = reader.bits(1)? == 1;
        match reader.bits(2)? {
            0 => inflate_stored(&mut reader, &mut out)?,
            1 => {
                let (lit, dist) = fixed_tables();
                inflate_block(&mut reader, &mut out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_tables(&mut reader)?;
                inflate_block(&mut reader, &mut out, &lit, &dist)?;
            }
            _ => return None,
        }
        if final_block {
            return Some(out);
        }
    }
}

/// Compresses `data` into a zlib stream (RFC 1950), as `compress2` does.
pub fn compress(data: &[u8]) -> Vec<u8> {
    // CMF 0x78: deflate, 32 KiB window. FLG 0x01 completes the check value
    // and selects the "fastest" compression-level hint.
    let mut out = vec![0x78, 0x01];
    out.extend_from_slice(&deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Decompresses a zlib stream, checking the Adler-32 trailer.
///
/// `hint` is the expected size, used to pre-allocate.
pub fn decompress(data: &[u8], hint: Option<usize>) -> Option<Vec<u8>> {
    if data.len() < 6 {
        return None;
    }
    let cmf = data[0];
    let flg = data[1];
    if cmf & 0x0f != 8 || (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 || flg & 0x20 != 0 {
        return None;
    }
    let body = &data[2..data.len() - 4];
    let out = inflate(body, hint)?;
    let want = u32::from_be_bytes([
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);
    if adler32(&out) == want {
        Some(out)
    } else {
        None
    }
}

/// Compresses `data` into a gzip stream (RFC 1952).
pub fn gzip_compress(data: &[u8]) -> Vec<u8> {
    // Magic, deflate method, no flags, no timestamp, no extra flags, and
    // "unknown" as the operating system so the output never varies.
    let mut out = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xff];
    out.extend_from_slice(&deflate(data));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    let size = u32::try_from(data.len() % (1 << 32)).expect("masked to 32 bits");
    out.extend_from_slice(&size.to_le_bytes());
    out
}

/// Decompresses a gzip stream, checking the CRC-32 trailer.
pub fn gzip_decompress(data: &[u8], hint: Option<usize>) -> Option<Vec<u8>> {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return None;
    }
    let flags = data[3];
    let mut pos = 10usize;
    if flags & 0x04 != 0 {
        let len = usize::from(u16::from_le_bytes([*data.get(pos)?, *data.get(pos + 1)?]));
        pos = pos.checked_add(2)?.checked_add(len)?;
    }
    for mask in [0x08u8, 0x10] {
        if flags & mask != 0 {
            while *data.get(pos)? != 0 {
                pos += 1;
            }
            pos += 1;
        }
    }
    if flags & 0x02 != 0 {
        pos = pos.checked_add(2)?;
    }
    if pos + 8 > data.len() {
        return None;
    }
    let body = data.get(pos..data.len() - 8)?;
    let out = inflate(body, hint)?;
    let want = u32::from_le_bytes([
        data[data.len() - 8],
        data[data.len() - 7],
        data[data.len() - 6],
        data[data.len() - 5],
    ]);
    if crc32(&out) == want { Some(out) } else { None }
}

/// Hashes the three bytes at `pos`.
fn hash3(data: &[u8], pos: usize) -> usize {
    let v =
        (u32::from(data[pos]) << 16) | (u32::from(data[pos + 1]) << 8) | u32::from(data[pos + 2]);
    let h = v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS);
    usize::try_from(h).expect("hash fits in a usize")
}

/// Writes one literal byte with the fixed literal/length table.
fn emit_literal(bits: &mut BitWriter, byte: u8) {
    emit_symbol(bits, u16::from(byte));
}

/// Writes a length/distance pair with the fixed tables.
fn emit_match(bits: &mut BitWriter, length: usize, distance: usize) {
    let len = u16::try_from(length).expect("length below 259");
    let mut li = 28;
    while li > 0 && LENGTH_BASE[li] > len {
        li -= 1;
    }
    emit_symbol(bits, 257 + u16::try_from(li).expect("29 length symbols"));
    let extra = LENGTH_EXTRA[li];
    if extra > 0 {
        bits.write_bits(u32::from(len - LENGTH_BASE[li]), u32::from(extra));
    }
    let dist = u16::try_from(distance).expect("distance below 32769");
    let mut di = 29;
    while di > 0 && DIST_BASE[di] > dist {
        di -= 1;
    }
    bits.write_code(u16::try_from(di).expect("30 distance symbols"), 5);
    let extra = DIST_EXTRA[di];
    if extra > 0 {
        bits.write_bits(u32::from(dist - DIST_BASE[di]), u32::from(extra));
    }
}

/// Writes a literal/length symbol with the fixed table's code.
fn emit_symbol(bits: &mut BitWriter, symbol: u16) {
    match symbol {
        0..=143 => bits.write_code(0x30 + symbol, 8),
        144..=255 => bits.write_code(0x190 + (symbol - 144), 9),
        256..=279 => bits.write_code(symbol - 256, 7),
        _ => bits.write_code(0xc0 + (symbol - 280), 8),
    }
}

/// Copies a stored (uncompressed) block.
fn inflate_stored(reader: &mut BitReader, out: &mut Vec<u8>) -> Option<()> {
    reader.align();
    let len = usize::from(u16::from_le_bytes([reader.byte()?, reader.byte()?]));
    let nlen = usize::from(u16::from_le_bytes([reader.byte()?, reader.byte()?]));
    if len ^ 0xffff != nlen {
        return None;
    }
    for _ in 0..len {
        out.push(reader.byte()?);
    }
    Some(())
}

/// Decodes one Huffman-coded block into `out`.
fn inflate_block(
    reader: &mut BitReader,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
) -> Option<()> {
    loop {
        let symbol = lit.decode(reader)?;
        if symbol < 256 {
            out.push(u8::try_from(symbol).expect("below 256"));
            continue;
        }
        if symbol == 256 {
            return Some(());
        }
        let li = usize::from(symbol - 257);
        if li >= LENGTH_BASE.len() {
            return None;
        }
        let length =
            usize::from(LENGTH_BASE[li]) + bits_usize(reader, u32::from(LENGTH_EXTRA[li]))?;
        let dsym = usize::from(dist.decode(reader)?);
        if dsym >= DIST_BASE.len() {
            return None;
        }
        let distance =
            usize::from(DIST_BASE[dsym]) + bits_usize(reader, u32::from(DIST_EXTRA[dsym]))?;
        if distance == 0 || distance > out.len() {
            return None;
        }
        for from in (out.len() - distance..).take(length) {
            let byte = out[from];
            out.push(byte);
        }
    }
}

/// Reads `count` bits as a `usize`.
fn bits_usize(reader: &mut BitReader, count: u32) -> Option<usize> {
    usize::try_from(reader.bits(count)?).ok()
}

/// The fixed literal/length and distance tables of RFC 1951 §3.2.6.
fn fixed_tables() -> (Huffman, Huffman) {
    let mut lengths = [0u8; 288];
    for (i, slot) in lengths.iter_mut().enumerate() {
        *slot = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let lit = Huffman::new(&lengths).expect("fixed literal table is well formed");
    let dist = Huffman::new(&[5u8; 30]).expect("fixed distance table is well formed");
    (lit, dist)
}

/// Reads the code lengths of a dynamic block and builds its tables.
fn dynamic_tables(reader: &mut BitReader) -> Option<(Huffman, Huffman)> {
    let hlit = 257 + bits_usize(reader, 5)?;
    let hdist = 1 + bits_usize(reader, 5)?;
    let hclen = 4 + bits_usize(reader, 4)?;
    if hlit > 288 || hdist > 32 {
        return None;
    }
    let mut clen = [0u8; 19];
    for slot in CLEN_ORDER.iter().take(hclen) {
        clen[*slot] = u8::try_from(reader.bits(3)?).expect("three bits fit in a byte");
    }
    let clen_table = Huffman::new(&clen)?;
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0usize;
    while i < lengths.len() {
        let symbol = clen_table.decode(reader)?;
        match symbol {
            0..=15 => {
                lengths[i] = u8::try_from(symbol).expect("below 16");
                i += 1;
            }
            16 => {
                if i == 0 {
                    return None;
                }
                let prev = lengths[i - 1];
                let count = 3 + bits_usize(reader, 2)?;
                for _ in 0..count {
                    *lengths.get_mut(i)? = prev;
                    i += 1;
                }
            }
            17 => {
                let count = 3 + bits_usize(reader, 3)?;
                i = i.checked_add(count)?;
                if i > lengths.len() {
                    return None;
                }
            }
            18 => {
                let count = 11 + bits_usize(reader, 7)?;
                i = i.checked_add(count)?;
                if i > lengths.len() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let lit = Huffman::new(&lengths[..hlit])?;
    let dist = Huffman::new(&lengths[hlit..])?;
    Some((lit, dist))
}

/// A canonical Huffman decoding table, in the counts-and-symbols form of
/// zlib's `puff` reference decoder.
struct Huffman {
    /// How many codes have each length, indexed by length.
    counts: [u16; 16],
    /// The symbols, ordered by code length then by symbol.
    symbols: Vec<u16>,
}

impl Huffman {
    /// Builds a table from per-symbol code lengths (0 = symbol unused).
    fn new(lengths: &[u8]) -> Option<Huffman> {
        let mut counts = [0u16; 16];
        for len in lengths {
            if *len > 15 {
                return None;
            }
            counts[usize::from(*len)] += 1;
        }
        counts[0] = 0;
        let mut offsets = [0u16; 16];
        for len in 1..16 {
            offsets[len] = offsets[len - 1] + counts[len - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, len) in lengths.iter().enumerate() {
            if *len != 0 {
                let slot = usize::from(offsets[usize::from(*len)]);
                symbols[slot] = u16::try_from(symbol).expect("under 65536 symbols");
                offsets[usize::from(*len)] += 1;
            }
        }
        Some(Huffman { counts, symbols })
    }

    /// Decodes the next symbol, consuming its bits.
    fn decode(&self, reader: &mut BitReader) -> Option<u16> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= i32::try_from(reader.bits(1)?).expect("one bit");
            let count = i32::from(self.counts[len]);
            if code - first < count {
                let slot = usize::try_from(index + (code - first)).expect("non-negative index");
                return self.symbols.get(slot).copied();
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

/// A least-significant-bit-first bit writer.
struct BitWriter {
    out: Vec<u8>,
    accumulator: u32,
    count: u32,
}

impl BitWriter {
    /// A writer with room for `capacity` output bytes.
    fn new(capacity: usize) -> BitWriter {
        BitWriter {
            out: Vec::with_capacity(capacity),
            accumulator: 0,
            count: 0,
        }
    }

    /// Writes `count` bits of `value`, least significant bit first.
    fn write_bits(&mut self, value: u32, count: u32) {
        self.accumulator |= (value & ((1u32 << count) - 1)) << self.count;
        self.count += count;
        while self.count >= 8 {
            self.out
                .push(u8::try_from(self.accumulator & 0xff).expect("masked to a byte"));
            self.accumulator >>= 8;
            self.count -= 8;
        }
    }

    /// Writes a Huffman code, whose bits go out most significant first.
    fn write_code(&mut self, code: u16, len: u32) {
        let mut reversed = 0u32;
        for i in 0..len {
            reversed |= ((u32::from(code) >> i) & 1) << (len - 1 - i);
        }
        self.write_bits(reversed, len);
    }

    /// Flushes the partial byte and returns the stream.
    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out
                .push(u8::try_from(self.accumulator & 0xff).expect("masked to a byte"));
        }
        self.out
    }
}

/// A least-significant-bit-first bit reader.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    accumulator: u32,
    count: u32,
}

impl<'a> BitReader<'a> {
    /// A reader over `data`.
    fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader {
            data,
            pos: 0,
            accumulator: 0,
            count: 0,
        }
    }

    /// Reads `count` bits (at most 24), least significant bit first.
    fn bits(&mut self, count: u32) -> Option<u32> {
        if count == 0 {
            return Some(0);
        }
        while self.count < count {
            let byte = *self.data.get(self.pos)?;
            self.pos += 1;
            self.accumulator |= u32::from(byte) << self.count;
            self.count += 8;
        }
        let value = self.accumulator & ((1u32 << count) - 1);
        self.accumulator >>= count;
        self.count -= count;
        Some(value)
    }

    /// Discards the bits left in the current byte.
    fn align(&mut self) {
        self.accumulator = 0;
        self.count = 0;
    }

    /// Reads one whole byte; only valid right after [`BitReader::align`].
    fn byte(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic pseudo-random generator.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            self.0 >> 11
        }

        fn byte(&mut self) -> u8 {
            u8::try_from(self.next() & 0xff).expect("masked to a byte")
        }
    }

    fn round_trip(data: &[u8]) {
        let packed = compress(data);
        assert_eq!(
            decompress(&packed, Some(data.len())).as_deref(),
            Some(data),
            "zlib round trip failed for {} bytes",
            data.len()
        );
        let gz = gzip_compress(data);
        assert_eq!(
            gzip_decompress(&gz, Some(data.len())).as_deref(),
            Some(data),
            "gzip round trip failed for {} bytes",
            data.len()
        );
    }

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"abc"), 0x024D_0127);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn small_inputs_round_trip() {
        for n in 0..64usize {
            let data: Vec<u8> = (0..n)
                .map(|i| u8::try_from(i % 251).expect("below 256"))
                .collect();
            round_trip(&data);
        }
    }

    #[test]
    fn repetitive_input_compresses_well() {
        let data = b"0000000000000000xxxxxxxxxxxxxxxx".repeat(4000);
        let packed = compress(&data);
        assert!(
            packed.len() * 50 < data.len(),
            "expected strong compression, got {} from {}",
            packed.len(),
            data.len()
        );
        round_trip(&data);
    }

    #[test]
    fn random_input_round_trips() {
        let mut rng = Rng(0xdead_beef);
        let data: Vec<u8> = (0..200_000).map(|_| rng.byte()).collect();
        round_trip(&data);
    }

    #[test]
    fn structured_input_round_trips() {
        let mut rng = Rng(7);
        let mut data = Vec::new();
        for i in 0..20_000u32 {
            if rng.next().is_multiple_of(3) {
                data.extend_from_slice(format!("signal_{i} = 0\n").as_bytes());
            } else {
                data.extend_from_slice(b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
            }
        }
        round_trip(&data);
    }

    #[test]
    fn stored_and_dynamic_blocks_inflate() {
        // A stored block holding "hi", then a final empty fixed block.
        let stored = [0x00u8, 0x02, 0x00, 0xfd, 0xff, b'h', b'i', 0x03, 0x00];
        assert_eq!(inflate(&stored, None).as_deref(), Some(&b"hi"[..]));
        // A dynamic-table stream, as produced by zlib at level 9.
        let dynamic = [
            0x78, 0xda, 0x75, 0x93, 0x51, 0x0e, 0x02, 0x21, 0x0c, 0x44, 0xaf, 0xc2, 0xd5, 0x20,
            0x12, 0x35, 0x61, 0x75, 0x13, 0xfd, 0xda, 0xd3, 0xaf, 0x11, 0xb1, 0xaf, 0x1d, 0xf8,
            0x61, 0x81, 0x6d, 0x67, 0xa6, 0xd3, 0x52, 0xf7, 0xd7, 0xbd, 0x3d, 0x1f, 0xe9, 0x9a,
            0xb7, 0x2d, 0xa7, 0xa3, 0xbe, 0x33, 0xb7, 0xb6, 0xd4, 0x5f, 0x5c, 0x6e, 0xfb, 0x2d,
            0xa7, 0x4b, 0x6d, 0x9f, 0xbb, 0xf2, 0xff, 0xdb, 0x6f, 0x8b, 0x6d, 0x3b, 0x06, 0xc2,
            0xfa, 0xd6, 0xa3, 0x8c, 0x13, 0xf2, 0xbe, 0x68, 0x48, 0xe8, 0x38, 0xb8, 0x28, 0x81,
            0x6e, 0x60, 0x8c, 0x2f, 0xa2, 0x10, 0x8a, 0x84, 0xb0, 0x74, 0x02, 0xd0, 0x0c, 0x20,
            0x2b, 0xdd, 0x82, 0xe7, 0xea, 0x16, 0x15, 0x03, 0x9e, 0x11, 0x7d, 0x8d, 0xb2, 0xbd,
            0x21, 0xc6, 0xbd, 0xca, 0x27, 0x1b, 0x02, 0xa1, 0x0d, 0xff, 0xa4, 0x2f, 0xf0, 0x9a,
            0x40, 0x5e, 0x04, 0x4b, 0x9c, 0x75, 0x8f, 0x39, 0x46, 0x33, 0xe9, 0xa9, 0x18, 0x56,
            0xdd, 0xcc, 0xf9, 0xd3, 0x4c, 0xc9, 0x11, 0x1b, 0xa5, 0x02, 0x60, 0x76, 0xb4, 0x16,
            0xce, 0x10, 0x63, 0x32, 0xd5, 0xf2, 0x06, 0x90, 0x1a, 0xeb, 0x8c, 0x2d, 0x9a, 0xbf,
            0x08, 0x5f, 0x87, 0x77, 0x16, 0xc2, 0x9d, 0x96, 0x45, 0x83, 0x65, 0xfc, 0x41, 0x23,
            0x6f, 0x45, 0x51, 0xed, 0x46, 0xd8, 0x25, 0x7b, 0x01, 0xea, 0x87, 0xc0, 0x75, 0x5b,
            0x46, 0x4e, 0x9c, 0xd1, 0xaa, 0xa4, 0x20, 0xc5, 0x3f, 0x01, 0x75, 0x2b, 0xac, 0x30,
        ];
        let plain = decompress(&dynamic, None).expect("inflates");
        assert_eq!(plain.len(), 1181);
        assert!(plain.starts_with(b"epsilon gamma zeta gamma zeta zeta zeta epsilon alpha delta "));
    }

    #[test]
    fn corrupt_streams_are_rejected() {
        assert_eq!(decompress(&[0, 0, 0, 0, 0, 0], None), None);
        let mut packed = compress(b"hello hello hello");
        let last = packed.len() - 1;
        packed[last] ^= 0xff;
        assert_eq!(decompress(&packed, None), None);
        assert_eq!(gzip_decompress(b"not a gzip stream!!", None), None);
    }
}
