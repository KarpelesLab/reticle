//! LEB128 variable-length integers, as FST encodes them.
//!
//! FST stores nearly every count, length and offset that is not a fixed
//! 64-bit field as a varint: seven payload bits per byte, least significant
//! group first, the high bit set on every byte but the last. Signed values
//! (used by the dynamic-alias chain index) use the sign-extending LEB128
//! variant, where the last byte's bit 6 is the sign.

/// Appends `value` as an unsigned LEB128 varint.
pub fn write_u64(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = low7(value);
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// The number of bytes [`write_u64`] appends for `value`.
pub fn len_u64(mut value: u64) -> usize {
    let mut n = 1;
    while value >= 0x80 {
        value >>= 7;
        n += 1;
    }
    n
}

/// Appends `value` as a signed (sign-extending) LEB128 varint.
pub fn write_i64(out: &mut Vec<u8>, mut value: i64) {
    loop {
        // An arithmetic shift keeps the sign, so the loop ends on 0 or -1.
        let byte = low7_signed(value);
        value >>= 7;
        let sign_bit = byte & 0x40 != 0;
        if (value == 0 && !sign_bit) || (value == -1 && sign_bit) {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Reads an unsigned varint at `*pos`, advancing it past the value.
///
/// Returns `None` on a truncated or unrepresentable encoding.
pub fn read_u64(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *buf.get(*pos)?;
        *pos += 1;
        if shift >= 64 {
            // Past 64 bits only a zero continuation is representable.
            if byte & 0x7f != 0 {
                return None;
            }
        } else {
            value |= u64::from(byte & 0x7f) << shift;
        }
        shift += 7;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        if shift > 70 {
            return None;
        }
    }
}

/// Reads a signed varint at `*pos`, advancing it past the value.
pub fn read_i64(buf: &[u8], pos: &mut usize) -> Option<i64> {
    let mut value: i64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *buf.get(*pos)?;
        *pos += 1;
        if shift < 64 {
            value |= i64::from(byte & 0x7f).wrapping_shl(shift);
        }
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 64 && byte & 0x40 != 0 {
                value |= -1i64 << shift;
            }
            return Some(value);
        }
        if shift > 70 {
            return None;
        }
    }
}

/// The low seven bits of `value` as a byte.
fn low7(value: u64) -> u8 {
    u8::try_from(value & 0x7f).expect("seven bits fit in a byte")
}

/// The low seven bits of a signed `value` as a byte.
fn low7_signed(value: i64) -> u8 {
    low7(value.cast_unsigned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_u(v: u64) -> u64 {
        let mut buf = Vec::new();
        write_u64(&mut buf, v);
        assert_eq!(buf.len(), len_u64(v));
        let mut pos = 0;
        let got = read_u64(&buf, &mut pos).unwrap();
        assert_eq!(pos, buf.len());
        got
    }

    fn round_i(v: i64) -> i64 {
        let mut buf = Vec::new();
        write_i64(&mut buf, v);
        let mut pos = 0;
        let got = read_i64(&buf, &mut pos).unwrap();
        assert_eq!(pos, buf.len());
        got
    }

    #[test]
    fn unsigned_encoding_matches_leb128() {
        let mut buf = Vec::new();
        write_u64(&mut buf, 0);
        assert_eq!(buf, [0]);
        buf.clear();
        write_u64(&mut buf, 127);
        assert_eq!(buf, [0x7f]);
        buf.clear();
        write_u64(&mut buf, 128);
        assert_eq!(buf, [0x80, 0x01]);
        buf.clear();
        write_u64(&mut buf, 300);
        assert_eq!(buf, [0xac, 0x02]);
    }

    #[test]
    fn signed_encoding_matches_leb128() {
        let mut buf = Vec::new();
        write_i64(&mut buf, -1);
        assert_eq!(buf, [0x7f]);
        buf.clear();
        write_i64(&mut buf, 63);
        assert_eq!(buf, [0x3f]);
        buf.clear();
        write_i64(&mut buf, 64);
        assert_eq!(buf, [0xc0, 0x00]);
        buf.clear();
        write_i64(&mut buf, -64);
        assert_eq!(buf, [0x40]);
        buf.clear();
        write_i64(&mut buf, -65);
        assert_eq!(buf, [0xbf, 0x7f]);
    }

    #[test]
    fn round_trips() {
        for v in [0u64, 1, 127, 128, 16_383, 16_384, 1 << 31, u64::MAX] {
            assert_eq!(round_u(v), v);
        }
        let mut x = 1u64;
        for _ in 0..63 {
            assert_eq!(round_u(x), x);
            assert_eq!(round_u(x - 1), x - 1);
            x = x.wrapping_mul(2).wrapping_add(1);
        }
        for v in [0i64, 1, -1, 63, -64, 64, -65, i64::MIN, i64::MAX] {
            assert_eq!(round_i(v), v);
        }
    }

    #[test]
    fn truncated_input_fails() {
        let mut pos = 0;
        assert_eq!(read_u64(&[0x80], &mut pos), None);
        let mut pos = 0;
        assert_eq!(read_i64(&[0x80, 0x80], &mut pos), None);
        let mut pos = 0;
        assert_eq!(read_u64(&[], &mut pos), None);
    }
}
