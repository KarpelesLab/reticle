//! An in-crate LZ4 block compressor and decompressor.
//!
//! FST may compress a hierarchy block (`FST_BL_HIER_LZ4`) or the per-signal
//! value-change chains (pack type `'4'`) with LZ4. This is the *block*
//! format, not the framed `.lz4` file format: the caller always knows the
//! uncompressed size from the surrounding FST structure, so no frame header,
//! magic or checksum is stored.
//!
//! A block is a sequence of *sequences*. Each one starts with a token byte
//! whose high nibble is the literal run length and whose low nibble is the
//! match length minus four; a nibble of 15 means the real length continues
//! in following bytes, `255` meaning "add 255 and read another". The literal
//! bytes follow the token (and its literal-length extension), then a
//! little-endian 16-bit match *offset* (distance back into the already
//! decoded output) and the match-length extension. The last sequence of a
//! block carries literals only, and the format requires the final five bytes
//! of the input to be literals with no match starting in the last twelve, so
//! that decoders may copy in wide chunks; [`compress`] respects both.
//!
//! The compressor is the single-pass "fast" variant: a hash table maps the
//! four bytes at each position to the most recent position with the same
//! hash, giving greedy matches at a bounded cost. Output is deterministic.

/// The shortest match LZ4 can encode.
const MIN_MATCH: usize = 4;
/// No match may start within this many bytes of the end of the input.
const MATCH_LIMIT: usize = 12;
/// The input always ends with at least this many literal bytes.
const LAST_LITERALS: usize = 5;
/// Inputs shorter than this are emitted as one literal run.
const MIN_LENGTH: usize = MATCH_LIMIT + 1;
/// Log2 of the match hash table size.
const HASH_LOG: u32 = 16;
/// The largest distance a match may reach back.
const MAX_DISTANCE: usize = 0xffff;

/// Compresses `src` into an LZ4 block.
///
/// The result always decompresses back to `src` through [`decompress`], and
/// through any conforming LZ4 block decoder. It is not guaranteed to be
/// smaller than `src`; callers that need that store the input verbatim
/// instead, which is what the FST writer does.
pub fn compress(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() / 2 + 16);
    if src.len() < MIN_LENGTH {
        emit_last_literals(&mut out, src);
        return out;
    }
    let mut table = vec![0usize; 1 << HASH_LOG];
    let match_end = src.len() - LAST_LITERALS;
    let search_end = src.len() - MATCH_LIMIT;
    let mut anchor = 0usize;
    // The first byte is never a match target, which matches the reference
    // encoder and keeps at least one literal in the first sequence.
    let mut ip = 1usize;
    while ip < search_end {
        let h = hash4(read_u32(src, ip));
        let candidate = table[h];
        table[h] = ip + 1;
        if candidate != 0 {
            let cand = candidate - 1;
            if ip - cand <= MAX_DISTANCE && read_u32(src, cand) == read_u32(src, ip) {
                let mut len = MIN_MATCH;
                while ip + len < match_end && src[cand + len] == src[ip + len] {
                    len += 1;
                }
                emit_sequence(&mut out, &src[anchor..ip], ip - cand, len);
                ip += len;
                anchor = ip;
                continue;
            }
        }
        ip += 1;
    }
    emit_last_literals(&mut out, &src[anchor..]);
    out
}

/// Decompresses an LZ4 block known to expand to `out_len` bytes.
///
/// Returns `None` when the block is malformed or does not produce exactly
/// `out_len` bytes.
pub fn decompress(src: &[u8], out_len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(out_len);
    let mut pos = 0usize;
    while pos < src.len() {
        let token = src[pos];
        pos += 1;
        let mut literals = usize::from(token >> 4);
        if literals == 15 {
            literals += read_extension(src, &mut pos)?;
        }
        if literals > 0 {
            let end = pos.checked_add(literals)?;
            out.extend_from_slice(src.get(pos..end)?);
            pos = end;
        }
        if pos == src.len() {
            break;
        }
        let lo = *src.get(pos)?;
        let hi = *src.get(pos + 1)?;
        pos += 2;
        let offset = usize::from(u16::from_le_bytes([lo, hi]));
        if offset == 0 || offset > out.len() {
            return None;
        }
        let mut length = usize::from(token & 0x0f);
        if length == 15 {
            length += read_extension(src, &mut pos)?;
        }
        length += MIN_MATCH;
        for from in (out.len() - offset..).take(length) {
            let byte = out[from];
            out.push(byte);
        }
    }
    if out.len() == out_len {
        Some(out)
    } else {
        None
    }
}

/// Reads a `255`-terminated length extension at `*pos`.
fn read_extension(src: &[u8], pos: &mut usize) -> Option<usize> {
    let mut total = 0usize;
    loop {
        let byte = *src.get(*pos)?;
        *pos += 1;
        total = total.checked_add(usize::from(byte))?;
        if byte != 255 {
            return Some(total);
        }
    }
}

/// Appends a literal run and a match.
fn emit_sequence(out: &mut Vec<u8>, literals: &[u8], offset: usize, match_len: usize) {
    let match_code = match_len - MIN_MATCH;
    let lit_nibble = if literals.len() >= 15 {
        15
    } else {
        literals.len()
    };
    let match_nibble = if match_code >= 15 { 15 } else { match_code };
    out.push(nibbles(lit_nibble, match_nibble));
    if literals.len() >= 15 {
        write_extension(out, literals.len() - 15);
    }
    out.extend_from_slice(literals);
    let off = u16::try_from(offset).expect("offset within the LZ4 window");
    out.extend_from_slice(&off.to_le_bytes());
    if match_code >= 15 {
        write_extension(out, match_code - 15);
    }
}

/// Appends the trailing literal-only sequence.
fn emit_last_literals(out: &mut Vec<u8>, literals: &[u8]) {
    let lit_nibble = if literals.len() >= 15 {
        15
    } else {
        literals.len()
    };
    out.push(nibbles(lit_nibble, 0));
    if literals.len() >= 15 {
        write_extension(out, literals.len() - 15);
    }
    out.extend_from_slice(literals);
}

/// Appends a `255`-terminated length extension.
fn write_extension(out: &mut Vec<u8>, mut remaining: usize) {
    while remaining >= 255 {
        out.push(255);
        remaining -= 255;
    }
    out.push(u8::try_from(remaining).expect("remainder below 255"));
}

/// Packs two four-bit values into a token byte.
fn nibbles(high: usize, low: usize) -> u8 {
    let packed = (high << 4) | low;
    u8::try_from(packed).expect("two nibbles fit in a byte")
}

/// The four bytes at `pos` as a little-endian `u32`.
fn read_u32(src: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes([src[pos], src[pos + 1], src[pos + 2], src[pos + 3]])
}

/// The match hash of a four-byte sequence.
fn hash4(value: u32) -> usize {
    let h = value.wrapping_mul(2_654_435_761) >> (32 - HASH_LOG);
    usize::try_from(h).expect("hash fits in a usize")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic pseudo-random generator, so tests need no
    /// dependency and always produce the same data.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            self.0 >> 11
        }
    }

    fn round_trip(data: &[u8]) {
        let packed = compress(data);
        let back = decompress(&packed, data.len()).expect("decompresses");
        assert_eq!(back, data, "round trip failed for {} bytes", data.len());
    }

    #[test]
    fn short_and_empty_inputs() {
        for n in 0..40usize {
            let data: Vec<u8> = (0..n)
                .map(|i| u8::try_from(i % 251).expect("below 256"))
                .collect();
            round_trip(&data);
        }
    }

    #[test]
    fn highly_repetitive_input_compresses() {
        let data = b"the quick brown fox ".repeat(2000);
        let packed = compress(&data);
        assert!(
            packed.len() * 20 < data.len(),
            "repetitive data should compress well, got {} from {}",
            packed.len(),
            data.len()
        );
        round_trip(&data);
    }

    #[test]
    fn long_run_of_one_byte() {
        let data = vec![0x5au8; 100_000];
        let packed = compress(&data);
        assert!(packed.len() < 1000);
        round_trip(&data);
    }

    #[test]
    fn random_input_round_trips() {
        let mut rng = Rng(0x1234_5678);
        let data: Vec<u8> = (0..200_000)
            .map(|_| u8::try_from(rng.next() & 0xff).expect("masked to a byte"))
            .collect();
        round_trip(&data);
    }

    #[test]
    fn mixed_input_round_trips() {
        let mut rng = Rng(99);
        let mut data = Vec::new();
        for _ in 0..500 {
            if rng.next().is_multiple_of(2) {
                data.extend_from_slice(b"0000000011111111xxxxxxxx");
            } else {
                for _ in 0..17 {
                    data.push(u8::try_from(rng.next() & 0xff).expect("masked to a byte"));
                }
            }
        }
        round_trip(&data);
    }

    #[test]
    fn malformed_blocks_are_rejected() {
        assert_eq!(decompress(&[0x10], 1), None);
        // A match with a zero offset is invalid.
        assert_eq!(decompress(&[0x10, b'a', 0, 0], 8), None);
        // Correct data but the wrong expected length.
        let packed = compress(b"hello world");
        assert_eq!(decompress(&packed, 10), None);
    }
}
