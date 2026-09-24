//! A MessagePack reader, hand written.
//!
//! The crate takes no dependencies, so it brings its own MessagePack, the
//! same way it brings its own [JSON](crate::json). The format is small and
//! completely specified — one leading byte per value, then a length and a
//! payload — so a reader is a few hundred lines and needs no library.
//!
//! It sits at the crate root rather than inside its caller for the reason
//! [`crate::json`] does: a second caller is expected. Today there is one,
//! the Gowin fabric loader ([`fpga::apicula`]), whose chip database is a
//! MessagePack map inside an xz stream.
//!
//! [`fpga::apicula`]: crate::fpga::apicula
//!
//! # What it reads, and what it does not
//!
//! Every type of the MessagePack specification is decoded: `nil`, the
//! booleans, the eight integer encodings, both floats, `str`, `bin`,
//! `array`, `map` and the five `ext` forms. **There is no writer.**
//! Nothing in Reticle produces MessagePack, and a serialiser that is
//! never exercised is a liability rather than a feature; if one is ever
//! needed it belongs here beside the reader.
//!
//! Three decisions are worth stating, because each is a place a looser
//! reader would quietly lose information.
//!
//! - **Signed and unsigned integers stay apart.** `uint 64` reaches
//!   `u64::MAX`, which no `i64` holds, so [`Msgpack::Uint`] and
//!   [`Msgpack::Int`] are separate variants and neither is cast into the
//!   other on the way in. [`Msgpack::as_i64`] and [`Msgpack::as_u64`]
//!   convert on the way out and return `None` when the value does not
//!   fit.
//! - **A map is a vector of pairs, and a key is a whole value.** Unlike
//!   JSON, a MessagePack key may be any value at all, and the database
//!   this reader exists for uses that: it is written by Python, where a
//!   `dict` keyed on a `(row, col)` tuple is idiomatic, so a great many
//!   of its keys are arrays. A `HashMap<String, _>` could not hold that.
//!   Keeping pairs in file order also means a reader sees the members in
//!   the order the writer wrote them.
//! - **A `str` must be UTF-8 and a `bin` is bytes.** The specification
//!   says `str` is UTF-8, so invalid UTF-8 in one is an error rather than
//!   a lossy replacement; `bin` has no encoding and is handed over as
//!   [`Msgpack::Bin`].
//!
//! # Refusing malformed input cheaply
//!
//! A length in a MessagePack header is attacker-controlled and as large
//! as 2³² − 1. Two guards keep a 12-byte file from asking for gigabytes:
//!
//! - a container's declared element count is checked against the bytes
//!   that are actually left, since every element costs at least one byte,
//!   before any capacity is reserved;
//! - nesting is limited to [`MAX_DEPTH`], so a file of nothing but array
//!   headers cannot overflow the stack.
//!
//! [`Msgpack::parse`] also requires that the value cover the whole input:
//! trailing bytes are an error, not something to ignore.
//!
//! ```
//! use reticle::msgpack::Msgpack;
//!
//! // {"grid": [1, 2], "rows": 2}
//! let bytes = [
//!     0x82, 0xa4, b'g', b'r', b'i', b'd', 0x92, 0x01, 0x02, 0xa4, b'r', b'o',
//!     b'w', b's', 0x02,
//! ];
//! let v = Msgpack::parse(&bytes).unwrap();
//! assert_eq!(v.get("rows").and_then(Msgpack::as_u64), Some(2));
//! assert_eq!(v.get("grid").map(|g| g.array().len()), Some(2));
//! ```

use std::fmt;

/// The deepest nesting [`Msgpack::parse`] accepts.
///
/// Parsing is recursive, so this is what stands between a file of
/// nothing but `0x91` bytes (an array of one element, repeated) and a
/// stack overflow. The chip database this reader was written for nests
/// seven deep; a hundred is room for any structure a person would write
/// and far from the frame budget of any thread.
pub const MAX_DEPTH: usize = 100;

/// A MessagePack value.
#[derive(Clone, Debug, PartialEq)]
pub enum Msgpack {
    /// `nil`.
    Nil,
    /// `true` or `false`.
    Bool(bool),
    /// A negative fixint or one of the `int 8/16/32/64` forms, and any
    /// unsigned form small enough to be one.
    Int(i64),
    /// An unsigned value that does not fit an `i64`, or one written in a
    /// `uint` form. Kept apart from [`Msgpack::Int`] so `u64::MAX`
    /// survives the round trip.
    Uint(u64),
    /// `float 32` or `float 64`, both widened to `f64`.
    Float(f64),
    /// A `str`, which the specification requires to be UTF-8.
    Str(String),
    /// A `bin`: bytes with no encoding.
    Bin(Vec<u8>),
    /// An `array`.
    Array(Vec<Msgpack>),
    /// A `map`, in the order its pairs were written. A key may be any
    /// value; see the module documentation for why that matters.
    Map(Vec<(Msgpack, Msgpack)>),
    /// An `ext`: an application-defined type tag and its payload. Also
    /// what the fixed-width `fixext` forms and `timestamp` (tag `-1`)
    /// decode to; this reader does not interpret the payload.
    Ext(i8, Vec<u8>),
}

impl Msgpack {
    /// Decodes one value that covers the whole of `bytes`.
    ///
    /// # Errors
    ///
    /// [`MsgpackError`] saying what went wrong and at which byte: input
    /// that ends inside a value, a reserved leading byte, a `str` that is
    /// not UTF-8, nesting deeper than [`MAX_DEPTH`], a declared length
    /// larger than the bytes that remain, or bytes left over after the
    /// value.
    pub fn parse(bytes: &[u8]) -> Result<Msgpack, MsgpackError> {
        let mut parser = Parser { bytes, at: 0 };
        let value = parser.value(0)?;
        if parser.at != bytes.len() {
            return Err(MsgpackError {
                offset: parser.at,
                kind: MsgpackErrorKind::Trailing,
            });
        }
        Ok(value)
    }

    /// Decodes one value from the start of `bytes`, returning it and the
    /// bytes after it, for a stream of concatenated values.
    ///
    /// # Errors
    ///
    /// As [`Msgpack::parse`], except that trailing bytes are the point.
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Msgpack, &[u8]), MsgpackError> {
        let mut parser = Parser { bytes, at: 0 };
        let value = parser.value(0)?;
        Ok((value, &bytes[parser.at..]))
    }

    /// The name of this value's type, for a diagnostic.
    pub fn type_name(&self) -> &'static str {
        match self {
            Msgpack::Nil => "nil",
            Msgpack::Bool(_) => "bool",
            Msgpack::Int(_) => "int",
            Msgpack::Uint(_) => "uint",
            Msgpack::Float(_) => "float",
            Msgpack::Str(_) => "str",
            Msgpack::Bin(_) => "bin",
            Msgpack::Array(_) => "array",
            Msgpack::Map(_) => "map",
            Msgpack::Ext(..) => "ext",
        }
    }

    /// True for [`Msgpack::Nil`].
    pub fn is_nil(&self) -> bool {
        matches!(self, Msgpack::Nil)
    }

    /// The value of the pair whose key is the string `key`, for a map.
    ///
    /// A linear scan, which is the right trade for the tens of members a
    /// record-shaped map has; a caller that reads every pair of a large
    /// map should iterate [`Msgpack::pairs`] instead.
    pub fn get(&self, key: &str) -> Option<&Msgpack> {
        self.pairs()
            .iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    }

    /// The string contents, for [`Msgpack::Str`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Msgpack::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The boolean, for [`Msgpack::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Msgpack::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The value as an `i64`, for an integer that fits one.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Msgpack::Int(i) => Some(*i),
            Msgpack::Uint(u) => i64::try_from(*u).ok(),
            _ => None,
        }
    }

    /// The value as a `u64`, for a non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Msgpack::Uint(u) => Some(*u),
            Msgpack::Int(i) => u64::try_from(*i).ok(),
            _ => None,
        }
    }

    /// The value as a `u32`, for a non-negative integer that fits one.
    ///
    /// Grid coordinates and bit positions are `u32` throughout the FPGA
    /// side, and this is the conversion that would otherwise be written
    /// at every one of those call sites.
    pub fn as_u32(&self) -> Option<u32> {
        u32::try_from(self.as_u64()?).ok()
    }

    /// The value as an `f64`, for a float or for an integer that converts
    /// exactly.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Msgpack::Float(f) => Some(*f),
            Msgpack::Int(i) => Some(*i as f64),
            Msgpack::Uint(u) => Some(*u as f64),
            _ => None,
        }
    }

    /// The bytes, for [`Msgpack::Bin`].
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Msgpack::Bin(b) => Some(b),
            _ => None,
        }
    }

    /// The elements, for [`Msgpack::Array`]; an empty slice otherwise.
    ///
    /// The empty-slice fallback is deliberate: a database field that is
    /// absent, `nil` or an empty array all mean "nothing here" to every
    /// caller in this crate, and making them one case removes a `match`
    /// from each one.
    pub fn array(&self) -> &[Msgpack] {
        match self {
            Msgpack::Array(items) => items,
            _ => &[],
        }
    }

    /// The pairs, for [`Msgpack::Map`]; an empty slice otherwise, for the
    /// reason [`Msgpack::array`] gives.
    pub fn pairs(&self) -> &[(Msgpack, Msgpack)] {
        match self {
            Msgpack::Map(pairs) => pairs,
            _ => &[],
        }
    }
}

/// Why a MessagePack value could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsgpackError {
    /// The byte the reader was looking at.
    pub offset: usize,
    /// What was wrong there.
    pub kind: MsgpackErrorKind,
}

/// The kinds of malformed MessagePack [`MsgpackError`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgpackErrorKind {
    /// The input ended inside a value.
    Truncated,
    /// A leading byte the specification reserves (`0xc1`).
    Reserved(u8),
    /// Nesting deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A `str` whose bytes are not UTF-8.
    BadUtf8,
    /// A declared length or element count larger than the input can hold,
    /// carrying the number that was asked for.
    ///
    /// This is reported in place of [`MsgpackErrorKind::Truncated`]
    /// whenever the *header* already asks for more than is left, which
    /// covers both a file cut short and a length invented to make a
    /// reader allocate. Truncation is reported for a value whose header
    /// cannot be read at all, or which runs out inside a container whose
    /// count was plausible.
    LengthTooLarge(u64),
    /// Bytes after the value, for [`Msgpack::parse`].
    Trailing,
}

impl fmt::Display for MsgpackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "byte {}: ", self.offset)?;
        match self.kind {
            MsgpackErrorKind::Truncated => f.write_str("the input ends inside a value"),
            MsgpackErrorKind::Reserved(byte) => {
                write!(f, "`{byte:#04x}` is not a MessagePack type")
            }
            MsgpackErrorKind::TooDeep => {
                write!(f, "nested deeper than {MAX_DEPTH} values")
            }
            MsgpackErrorKind::BadUtf8 => f.write_str("a `str` that is not UTF-8"),
            MsgpackErrorKind::LengthTooLarge(n) => {
                write!(f, "a length of {n}, which is more than the input holds")
            }
            MsgpackErrorKind::Trailing => f.write_str("bytes left over after the value"),
        }
    }
}

impl std::error::Error for MsgpackError {}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn fail(&self, kind: MsgpackErrorKind) -> MsgpackError {
        MsgpackError {
            offset: self.at,
            kind,
        }
    }

    fn byte(&mut self) -> Result<u8, MsgpackError> {
        let Some(b) = self.bytes.get(self.at) else {
            return Err(self.fail(MsgpackErrorKind::Truncated));
        };
        self.at += 1;
        Ok(*b)
    }

    /// The next `n` bytes, which must be there.
    fn take(&mut self, n: usize) -> Result<&'a [u8], MsgpackError> {
        let Some(end) = self.at.checked_add(n).filter(|e| *e <= self.bytes.len()) else {
            return Err(self.fail(MsgpackErrorKind::Truncated));
        };
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    /// A big-endian unsigned integer of `n` bytes, `n` at most 8.
    fn be(&mut self, n: usize) -> Result<u64, MsgpackError> {
        let mut value = 0u64;
        for byte in self.take(n)? {
            value = (value << 8) | u64::from(*byte);
        }
        Ok(value)
    }

    /// A length from a header, checked against the bytes that are left.
    ///
    /// `per` is the smallest number of bytes one element costs: one for a
    /// byte of a string, one for an array element, two for a map pair
    /// (a key and a value). Checking before reserving is what keeps a
    /// twelve-byte file from asking for four gigabytes.
    fn length(&self, declared: u64, per: usize) -> Result<usize, MsgpackError> {
        let remaining = (self.bytes.len() - self.at) as u64;
        let needed = declared.saturating_mul(per as u64);
        if needed > remaining {
            return Err(self.fail(MsgpackErrorKind::LengthTooLarge(declared)));
        }
        // The check above bounds `declared` by the input length, which is
        // a `usize`, so this cannot truncate.
        usize::try_from(declared).map_err(|_| self.fail(MsgpackErrorKind::LengthTooLarge(declared)))
    }

    fn string(&mut self, len: usize) -> Result<Msgpack, MsgpackError> {
        let start = self.at;
        let bytes = self.take(len)?;
        match std::str::from_utf8(bytes) {
            Ok(s) => Ok(Msgpack::Str(s.to_owned())),
            Err(_) => Err(MsgpackError {
                offset: start,
                kind: MsgpackErrorKind::BadUtf8,
            }),
        }
    }

    fn array(&mut self, len: usize, depth: usize) -> Result<Msgpack, MsgpackError> {
        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(self.value(depth + 1)?);
        }
        Ok(Msgpack::Array(items))
    }

    fn map(&mut self, len: usize, depth: usize) -> Result<Msgpack, MsgpackError> {
        let mut pairs = Vec::with_capacity(len);
        for _ in 0..len {
            let key = self.value(depth + 1)?;
            let value = self.value(depth + 1)?;
            pairs.push((key, value));
        }
        Ok(Msgpack::Map(pairs))
    }

    fn ext(&mut self, len: usize) -> Result<Msgpack, MsgpackError> {
        let tag = self.byte()? as i8;
        Ok(Msgpack::Ext(tag, self.take(len)?.to_vec()))
    }

    fn value(&mut self, depth: usize) -> Result<Msgpack, MsgpackError> {
        if depth > MAX_DEPTH {
            return Err(self.fail(MsgpackErrorKind::TooDeep));
        }
        let tag = self.byte()?;
        match tag {
            // positive fixint: the byte is the value.
            0x00..=0x7f => Ok(Msgpack::Uint(u64::from(tag))),
            // fixmap, fixarray, fixstr.
            0x80..=0x8f => {
                let n = self.length(u64::from(tag & 0x0f), 2)?;
                self.map(n, depth)
            }
            0x90..=0x9f => {
                let n = self.length(u64::from(tag & 0x0f), 1)?;
                self.array(n, depth)
            }
            0xa0..=0xbf => {
                let n = self.length(u64::from(tag & 0x1f), 1)?;
                self.string(n)
            }
            0xc0 => Ok(Msgpack::Nil),
            // The specification never assigns 0xc1.
            0xc1 => Err(MsgpackError {
                offset: self.at - 1,
                kind: MsgpackErrorKind::Reserved(tag),
            }),
            0xc2 => Ok(Msgpack::Bool(false)),
            0xc3 => Ok(Msgpack::Bool(true)),
            // bin 8 / 16 / 32.
            0xc4..=0xc6 => {
                let width = 1usize << (tag - 0xc4);
                let declared = self.be(width)?;
                let n = self.length(declared, 1)?;
                Ok(Msgpack::Bin(self.take(n)?.to_vec()))
            }
            // ext 8 / 16 / 32: a length, then a one-byte tag, then the
            // payload. The tag is not part of the length, so the check
            // asks for one byte more than the payload.
            0xc7..=0xc9 => {
                let width = 1usize << (tag - 0xc7);
                let declared = self.be(width)?;
                let n = self.length(declared.saturating_add(1), 1)?;
                self.ext(n - 1)
            }
            0xca => {
                let bits = u32::try_from(self.be(4)?).unwrap_or(0);
                Ok(Msgpack::Float(f64::from(f32::from_bits(bits))))
            }
            0xcb => Ok(Msgpack::Float(f64::from_bits(self.be(8)?))),
            // uint 8 / 16 / 32 / 64.
            0xcc..=0xcf => {
                let width = 1usize << (tag - 0xcc);
                Ok(Msgpack::Uint(self.be(width)?))
            }
            // int 8 / 16 / 32 / 64: sign-extend from the written width.
            0xd0..=0xd3 => {
                let width = 1usize << (tag - 0xd0);
                let raw = self.be(width)?;
                let shift = 64 - width * 8;
                Ok(Msgpack::Int(((raw << shift) as i64) >> shift))
            }
            // fixext 1 / 2 / 4 / 8 / 16.
            0xd4..=0xd8 => {
                let n = 1usize << (tag - 0xd4);
                self.ext(n)
            }
            // str 8 / 16 / 32.
            0xd9..=0xdb => {
                let width = 1usize << (tag - 0xd9);
                let declared = self.be(width)?;
                let n = self.length(declared, 1)?;
                self.string(n)
            }
            // array 16 / 32.
            0xdc | 0xdd => {
                let width = if tag == 0xdc { 2 } else { 4 };
                let declared = self.be(width)?;
                let n = self.length(declared, 1)?;
                self.array(n, depth)
            }
            // map 16 / 32.
            0xde | 0xdf => {
                let width = if tag == 0xde { 2 } else { 4 };
                let declared = self.be(width)?;
                let n = self.length(declared, 2)?;
                self.map(n, depth)
            }
            // negative fixint: the low five bits, sign-extended.
            0xe0..=0xff => Ok(Msgpack::Int(i64::from(tag as i8))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixed_width_scalars_decode() {
        for (bytes, want) in [
            (vec![0xc0], Msgpack::Nil),
            (vec![0xc2], Msgpack::Bool(false)),
            (vec![0xc3], Msgpack::Bool(true)),
            (vec![0x00], Msgpack::Uint(0)),
            (vec![0x7f], Msgpack::Uint(127)),
            (vec![0xff], Msgpack::Int(-1)),
            (vec![0xe0], Msgpack::Int(-32)),
            (vec![0xcc, 0x80], Msgpack::Uint(128)),
            (vec![0xcd, 0x01, 0x00], Msgpack::Uint(256)),
            (vec![0xce, 0, 1, 0, 0], Msgpack::Uint(65536)),
            (
                vec![0xcf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
                Msgpack::Uint(u64::MAX),
            ),
            (vec![0xd0, 0xff], Msgpack::Int(-1)),
            (vec![0xd1, 0xff, 0x00], Msgpack::Int(-256)),
            (vec![0xd2, 0xff, 0xff, 0xff, 0xff], Msgpack::Int(-1)),
            (
                vec![0xd3, 0x80, 0, 0, 0, 0, 0, 0, 0],
                Msgpack::Int(i64::MIN),
            ),
        ] {
            assert_eq!(Msgpack::parse(&bytes).unwrap(), want, "{bytes:02x?}");
        }
    }

    #[test]
    fn a_u64_that_is_not_an_i64_keeps_its_value() {
        // The whole reason `Uint` and `Int` are separate variants.
        let v = Msgpack::parse(&[0xcf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).unwrap();
        assert_eq!(v.as_u64(), Some(u64::MAX));
        assert_eq!(v.as_i64(), None);
        assert_eq!(v.as_u32(), None);
    }

    #[test]
    fn floats_widen_and_both_widths_agree() {
        let thirty_two = Msgpack::parse(&[0xca, 0x40, 0x49, 0x0f, 0xdb]).unwrap();
        let sixty_four =
            Msgpack::parse(&[0xcb, 0x40, 0x09, 0x21, 0xfb, 0x54, 0x44, 0x2d, 0x18]).unwrap();
        let a = thirty_two.as_f64().unwrap();
        let b = sixty_four.as_f64().unwrap();
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }

    #[test]
    fn strings_and_bins_are_told_apart() {
        // fixstr "ab", str 8 "ab", bin 8 {0, 0xff}.
        assert_eq!(
            Msgpack::parse(&[0xa2, b'a', b'b']).unwrap(),
            Msgpack::Str("ab".into())
        );
        assert_eq!(
            Msgpack::parse(&[0xd9, 0x02, b'a', b'b']).unwrap(),
            Msgpack::Str("ab".into())
        );
        assert_eq!(
            Msgpack::parse(&[0xc4, 0x02, 0x00, 0xff]).unwrap(),
            Msgpack::Bin(vec![0x00, 0xff])
        );
        // A `bin` holding those same two bytes is not UTF-8 and is fine,
        // because a `bin` has no encoding.
        assert!(Msgpack::parse(&[0xd9, 0x02, 0x00, 0xff]).is_err());
    }

    #[test]
    fn a_str_that_is_not_utf8_is_refused_at_its_own_offset() {
        let err = Msgpack::parse(&[0xa2, 0xff, 0xfe]).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::BadUtf8);
        assert_eq!(err.offset, 1);
    }

    #[test]
    fn a_map_key_may_be_an_array() {
        // {[1, 2]: "x"} — the shape the chip database is full of, and the
        // one a `HashMap<String, _>` could not hold.
        let bytes = [0x81, 0x92, 0x01, 0x02, 0xa1, b'x'];
        let v = Msgpack::parse(&bytes).unwrap();
        let pairs = v.pairs();
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            pairs[0]
                .0
                .array()
                .iter()
                .filter_map(Msgpack::as_u32)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(pairs[0].1.as_str(), Some("x"));
        // And it is not reachable by a string key, which is right.
        assert!(v.get("x").is_none());
    }

    #[test]
    fn maps_keep_the_order_they_were_written_in() {
        // {"b": 1, "a": 2}
        let bytes = [0x82, 0xa1, b'b', 0x01, 0xa1, b'a', 0x02];
        let v = Msgpack::parse(&bytes).unwrap();
        let keys: Vec<_> = v.pairs().iter().filter_map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["b", "a"]);
        assert_eq!(v.get("a").and_then(Msgpack::as_u64), Some(2));
    }

    #[test]
    fn the_wide_container_headers_decode() {
        // array 16 of three, map 16 of one, array 32 of one.
        assert_eq!(
            Msgpack::parse(&[0xdc, 0, 3, 0x01, 0x02, 0x03])
                .unwrap()
                .array()
                .len(),
            3
        );
        assert_eq!(
            Msgpack::parse(&[0xde, 0, 1, 0x01, 0x02])
                .unwrap()
                .pairs()
                .len(),
            1
        );
        assert_eq!(
            Msgpack::parse(&[0xdd, 0, 0, 0, 1, 0x01])
                .unwrap()
                .array()
                .len(),
            1
        );
    }

    #[test]
    fn every_ext_form_decodes_to_a_tag_and_a_payload() {
        // fixext 1, fixext 4, ext 8 of three bytes, and a timestamp.
        assert_eq!(
            Msgpack::parse(&[0xd4, 0x05, 0x42]).unwrap(),
            Msgpack::Ext(5, vec![0x42])
        );
        assert_eq!(
            Msgpack::parse(&[0xd6, 0xff, 0, 0, 0, 0]).unwrap(),
            Msgpack::Ext(-1, vec![0, 0, 0, 0])
        );
        assert_eq!(
            Msgpack::parse(&[0xc7, 0x03, 0x07, 1, 2, 3]).unwrap(),
            Msgpack::Ext(7, vec![1, 2, 3])
        );
    }

    #[test]
    fn the_reserved_byte_is_refused() {
        let err = Msgpack::parse(&[0xc1]).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::Reserved(0xc1));
        assert_eq!(err.offset, 0);
    }

    #[test]
    fn truncation_is_an_error_and_not_a_short_value() {
        for bytes in [
            // Nothing at all.
            vec![],
            // A `uint 16` missing a byte of its own header.
            vec![0xcd, 0x01],
            // A `fixext 1` with no type tag.
            vec![0xd4],
            // A map of one pair whose count is plausible and whose value
            // is simply not there.
            vec![0x81, 0xa1, b'k'],
        ] {
            let err = Msgpack::parse(&bytes).unwrap_err();
            assert_eq!(err.kind, MsgpackErrorKind::Truncated, "{bytes:02x?}");
        }
    }

    #[test]
    fn a_length_larger_than_the_input_costs_nothing_to_refuse() {
        // The point of checking before reserving: `array 32` of
        // 4 294 967 295 elements in a six-byte file. A reader that
        // reserved first would ask for tens of gigabytes.
        let err = Msgpack::parse(&[0xdd, 0xff, 0xff, 0xff, 0xff, 0x01]).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::LengthTooLarge(0xffff_ffff));
        // A map pair costs two bytes, so half the input's worth of pairs
        // is already too many.
        let err = Msgpack::parse(&[0xde, 0x00, 0x02, 0x01, 0x02, 0x03]).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::LengthTooLarge(2));
        // And the same check catches the ordinary short file: a `fixstr`
        // of four bytes with one byte after it.
        let err = Msgpack::parse(&[0xa4, b'a']).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::LengthTooLarge(4));
        let err = Msgpack::parse(&[0x92, 0x01]).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::LengthTooLarge(2));
    }

    #[test]
    fn nesting_is_bounded_rather_than_overflowing_the_stack() {
        // MAX_DEPTH + 2 nested one-element arrays.
        let deep = vec![0x91u8; MAX_DEPTH + 2];
        let mut bytes = deep;
        bytes.push(0xc0);
        let err = Msgpack::parse(&bytes).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::TooDeep);
        // One less is accepted, so the limit is the limit and not an
        // off-by-one somewhere else.
        let mut ok = vec![0x91u8; MAX_DEPTH];
        ok.push(0xc0);
        assert!(Msgpack::parse(&ok).is_ok());
    }

    #[test]
    fn trailing_bytes_are_refused_by_parse_and_returned_by_parse_prefix() {
        let bytes = [0xc0, 0xc3, 0x01];
        let err = Msgpack::parse(&bytes).unwrap_err();
        assert_eq!(err.kind, MsgpackErrorKind::Trailing);
        assert_eq!(err.offset, 1);
        let (first, rest) = Msgpack::parse_prefix(&bytes).unwrap();
        assert_eq!(first, Msgpack::Nil);
        assert_eq!(rest, &[0xc3, 0x01]);
    }

    #[test]
    fn the_accessors_decline_rather_than_coerce() {
        let s = Msgpack::Str("7".into());
        assert_eq!(s.as_u64(), None);
        assert_eq!(s.as_f64(), None);
        assert_eq!(s.array(), &[]);
        assert_eq!(s.pairs(), &[]);
        assert_eq!(s.type_name(), "str");
        assert!(!s.is_nil());
        assert!(Msgpack::Nil.is_nil());
        assert_eq!(Msgpack::Nil.array(), &[]);
    }
}
