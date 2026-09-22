//! A JSON parser and serialiser, hand written.
//!
//! The crate takes no dependencies, so the language server brings its own
//! JSON. The subset is exactly what JSON-RPC and LSP need, which is all of
//! RFC 8259 minus any tolerance for extensions: no comments, no trailing
//! commas, no `NaN`.
//!
//! Two decisions are worth stating.
//!
//! - **Objects keep insertion order.** A [`Json::Object`] is a vector of
//!   pairs, not a map, so serialising twice yields the same bytes and a
//!   golden test can compare a whole session byte for byte. Lookup is a
//!   linear scan, which is the right trade for the handful of members an
//!   LSP message carries.
//! - **Integers stay integers.** [`Json::Int`] and [`Json::Float`] are
//!   separate, so a request id round-trips exactly and no `f64 as i64`
//!   cast is ever needed. A literal with a `.`, an `e` or an `E`, or one
//!   too large for an `i64`, parses as a float.
//!
//! ```
//! use reticle::lsp::json::Json;
//!
//! let v = Json::parse(r#"{"jsonrpc":"2.0","id":1,"method":"shutdown"}"#).unwrap();
//! assert_eq!(v.get("method").and_then(Json::as_str), Some("shutdown"));
//! assert_eq!(v.to_string(), r#"{"jsonrpc":"2.0","id":1,"method":"shutdown"}"#);
//! ```

use std::fmt;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number with no fractional or exponent part that fits an `i64`.
    Int(i64),
    /// Any other number.
    Float(f64),
    /// A string, unescaped.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, in the order its members were written.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// An empty object.
    pub fn object() -> Json {
        Json::Object(Vec::new())
    }

    /// Builds an object from `(key, value)` pairs, in order.
    pub fn obj<I, K>(pairs: I) -> Json
    where
        I: IntoIterator<Item = (K, Json)>,
        K: Into<String>,
    {
        Json::Object(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// Builds a string value.
    pub fn str(s: impl Into<String>) -> Json {
        Json::String(s.into())
    }

    /// Builds an array.
    pub fn array(items: impl IntoIterator<Item = Json>) -> Json {
        Json::Array(items.into_iter().collect())
    }

    /// The member named `key`, for an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sets (or appends) the member named `key`, for an object.
    ///
    /// Does nothing when `self` is not an object.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Json::Object(members) = self {
            match members.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => members.push((key.to_string(), value)),
            }
        }
    }

    /// The string contents, for [`Json::String`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    /// The value as an `i64`, for [`Json::Int`] and for a [`Json::Float`]
    /// that is exactly an integer.
    ///
    /// The float case formats and reparses rather than casting, since a
    /// `f64 as i64` would silently saturate.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(i) => Some(*i),
            Json::Float(f) if f.is_finite() && *f == f.trunc() => {
                format!("{f:.0}").parse::<i64>().ok()
            }
            _ => None,
        }
    }

    /// The value as a `u32`, when it is a non-negative integer that fits.
    pub fn as_u32(&self) -> Option<u32> {
        u32::try_from(self.as_i64()?).ok()
    }

    /// The boolean, for [`Json::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The elements, for [`Json::Array`].
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The members, for [`Json::Object`].
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(members) => Some(members),
            _ => None,
        }
    }

    /// True for [`Json::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// Parses one JSON value from `text`, which must hold nothing but
    /// whitespace around it.
    pub fn parse(text: &str) -> Result<Json, ParseError> {
        let mut p = Parser {
            bytes: text.as_bytes(),
            pos: 0,
            depth: 0,
        };
        p.skip_ws();
        let value = p.value()?;
        p.skip_ws();
        if p.pos != p.bytes.len() {
            return Err(p.error("trailing data after the value"));
        }
        Ok(value)
    }

    /// Serialises the value, compactly and deterministically.
    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Int(i) => out.push_str(&i.to_string()),
            Json::Float(f) => out.push_str(&write_float(*f)),
            Json::String(s) => write_string(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(members) => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }

    /// Serialises the value with two-space indentation, for golden files
    /// and for a human reading a session transcript.
    pub fn write_pretty(&self, out: &mut String, indent: usize) {
        match self {
            Json::Array(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(",\n");
                    }
                    pad(out, indent + 1);
                    item.write_pretty(out, indent + 1);
                }
                out.push('\n');
                pad(out, indent);
                out.push(']');
            }
            Json::Object(members) if !members.is_empty() => {
                out.push_str("{\n");
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push_str(",\n");
                    }
                    pad(out, indent + 1);
                    write_string(key, out);
                    out.push_str(": ");
                    value.write_pretty(out, indent + 1);
                }
                out.push('\n');
                pad(out, indent);
                out.push('}');
            }
            other => other.write(out),
        }
    }

    /// The value as pretty-printed text.
    pub fn to_pretty_string(&self) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0);
        out
    }
}

/// Writes `levels` indentation units.
fn pad(out: &mut String, levels: usize) {
    for _ in 0..levels {
        out.push_str("  ");
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        self.write(&mut out);
        f.write_str(&out)
    }
}

impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}

impl From<i64> for Json {
    fn from(i: i64) -> Json {
        Json::Int(i)
    }
}

impl From<u32> for Json {
    fn from(i: u32) -> Json {
        Json::Int(i64::from(i))
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::String(s.to_string())
    }
}

impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::String(s)
    }
}

/// Formats a float so that it parses back to the same value.
///
/// JSON has no `NaN` or infinity, so those become `null`, and an integral
/// float keeps no `.0` suffix, which is what every JSON writer emits.
fn write_float(f: f64) -> String {
    if !f.is_finite() {
        return "null".to_string();
    }
    if f == f.trunc() && f.abs() < 1e15 {
        format!("{f:.0}")
    } else {
        format!("{f}")
    }
}

/// Writes a JSON string literal, escaping what RFC 8259 requires.
///
/// Non-ASCII characters are written as themselves: the transport is UTF-8
/// and the `Content-Length` header counts bytes, so escaping them would
/// only make messages longer.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = fmt::Write::write_fmt(out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Why a document is not JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset at which parsing stopped.
    pub offset: usize,
    /// What was wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid JSON at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

/// How deep nesting may go before the parser gives up, so a hostile
/// message cannot overflow the stack of the recursive descent.
const MAX_DEPTH: u32 = 128;

/// The recursive-descent parser behind [`Json::parse`].
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: u32,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            offset: self.pos,
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), ParseError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(format!("expected `{}`", byte as char)))
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, ParseError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error(format!("expected `{word}`")))
        }
    }

    fn value(&mut self) -> Result<Json, ParseError> {
        match self.peek() {
            None => Err(self.error("unexpected end of input")),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(c) => Err(self.error(format!("unexpected `{}`", c as char))),
        }
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Json, ParseError> {
        self.enter()?;
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, ParseError> {
        self.enter()?;
        self.expect(b'{')?;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.value()?;
            // A duplicate key keeps the last value, as every JSON consumer
            // in practice does, and does not grow the member list.
            match members.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = value,
                None => members.push((key, value)),
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(self.error("expected a digit"));
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            let frac_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(self.error("expected a digit after `.`"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == exp_start {
                return Err(self.error("expected a digit in the exponent"));
            }
        }
        // Every byte consumed is ASCII, so the slice is valid UTF-8.
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.error("number is not ASCII"))?;
        if !is_float && let Ok(i) = text.parse::<i64>() {
            return Ok(Json::Int(i));
        }
        text.parse::<f64>()
            .map(Json::Float)
            .map_err(|_| self.error("number is out of range"))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error("unterminated string"));
            };
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                0x00..=0x1f => return Err(self.error("unescaped control character")),
                _ => {
                    // Copy one whole UTF-8 sequence, so multi-byte
                    // characters survive unharmed.
                    let start = self.pos;
                    self.pos += utf8_len(byte);
                    let Some(chunk) = self.bytes.get(start..self.pos) else {
                        return Err(self.error("truncated UTF-8 sequence"));
                    };
                    let text =
                        std::str::from_utf8(chunk).map_err(|_| self.error("invalid UTF-8"))?;
                    out.push_str(text);
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        let Some(byte) = self.peek() else {
            return Err(self.error("unterminated escape"));
        };
        self.pos += 1;
        let ch = match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let first = self.hex4()?;
                // A high surrogate must be followed by `\uDC00`-`\uDFFF`;
                // anything else is an error rather than a replacement
                // character, so a mis-encoded client is not silently
                // accepted.
                if (0xd800..0xdc00).contains(&first) {
                    if !self.bytes[self.pos..].starts_with(b"\\u") {
                        return Err(self.error("lone high surrogate"));
                    }
                    self.pos += 2;
                    let second = self.hex4()?;
                    if !(0xdc00..0xe000).contains(&second) {
                        return Err(self.error("high surrogate not followed by a low one"));
                    }
                    let combined = 0x1_0000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                    let Some(ch) = char::from_u32(combined) else {
                        return Err(self.error("surrogate pair is not a character"));
                    };
                    out.push(ch);
                    return Ok(());
                }
                let Some(ch) = char::from_u32(first) else {
                    return Err(self.error("lone low surrogate"));
                };
                ch
            }
            c => return Err(self.error(format!("unknown escape `\\{}`", c as char))),
        };
        out.push(ch);
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let Some(chunk) = self.bytes.get(self.pos..self.pos + 4) else {
            return Err(self.error("truncated `\\u` escape"));
        };
        let text = std::str::from_utf8(chunk).map_err(|_| self.error("invalid `\\u` escape"))?;
        // `from_str_radix` accepts a leading sign, which is not hex here.
        if text.bytes().any(|b| !b.is_ascii_hexdigit()) {
            return Err(self.error("invalid `\\u` escape"));
        }
        let value =
            u32::from_str_radix(text, 16).map_err(|_| self.error("invalid `\\u` escape"))?;
        self.pos += 4;
        Ok(value)
    }
}

/// The length in bytes of the UTF-8 sequence starting with `first`.
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str) {
        let value = Json::parse(text).expect("parses");
        assert_eq!(value.to_string(), text, "round trip of {text}");
        assert_eq!(Json::parse(&value.to_string()), Ok(value));
    }

    #[test]
    fn round_trips_scalars() {
        for text in [
            "null",
            "true",
            "false",
            "0",
            "-1",
            "42",
            "9223372036854775807",
            "-9223372036854775808",
            "1.5",
            "-0.25",
            "\"\"",
            "\"hello\"",
        ] {
            round_trip(text);
        }
    }

    #[test]
    fn round_trips_containers() {
        for text in [
            "[]",
            "{}",
            "[1,2,3]",
            "[[[1]]]",
            r#"{"a":1,"b":[true,null],"c":{"d":"e"}}"#,
            r#"{"z":1,"a":2}"#,
        ] {
            round_trip(text);
        }
    }

    #[test]
    fn keeps_member_order() {
        let v = Json::parse(r#"{"z":1,"m":2,"a":3}"#).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().iter().map(|(k, _)| &**k).collect();
        assert_eq!(keys, ["z", "m", "a"]);
    }

    #[test]
    fn duplicate_keys_keep_the_last() {
        let v = Json::parse(r#"{"a":1,"a":2}"#).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert_eq!(v.get("a"), Some(&Json::Int(2)));
    }

    #[test]
    fn parses_escapes() {
        let v = Json::parse(r#""a\"b\\c\/d\be\ff\ng\rh\ti""#).unwrap();
        assert_eq!(v.as_str().unwrap(), "a\"b\\c/d\u{8}e\u{c}f\ng\rh\ti");
        // `/` needs no escape on the way out, the others do.
        assert_eq!(v.to_string(), r#""a\"b\\c/d\be\ff\ng\rh\ti""#);
    }

    #[test]
    fn parses_unicode_escapes() {
        assert_eq!(Json::parse(r#""\u00e9""#).unwrap().as_str(), Some("é"));
        assert_eq!(Json::parse(r#""\u4e2d""#).unwrap().as_str(), Some("中"));
        // A surrogate pair for U+1F600.
        assert_eq!(
            Json::parse(r#""\ud83d\ude00""#).unwrap().as_str(),
            Some("😀")
        );
        assert_eq!(Json::parse(r#""\u0000""#).unwrap().as_str(), Some("\u{0}"));
    }

    #[test]
    fn escapes_controls_on_output() {
        let v = Json::String("a\u{1}b\u{7f}".to_string());
        // DEL is not a JSON control character, so it stays as itself.
        assert_eq!(v.to_string(), "\"a\\u0001b\u{7f}\"");
    }

    #[test]
    fn keeps_non_ascii_unescaped() {
        let v = Json::String("héllo 中 😀".to_string());
        assert_eq!(v.to_string(), "\"héllo 中 😀\"");
        round_trip("\"héllo 中 😀\"");
    }

    #[test]
    fn numbers() {
        assert_eq!(Json::parse("1e3").unwrap(), Json::Float(1000.0));
        assert_eq!(Json::parse("1.0").unwrap(), Json::Float(1.0));
        assert_eq!(Json::parse("-0").unwrap(), Json::Int(0));
        assert_eq!(Json::parse("0.5e-2").unwrap(), Json::Float(0.005));
        // Too large for an i64, so it becomes a float rather than failing.
        assert!(matches!(
            Json::parse("123456789012345678901234567890").unwrap(),
            Json::Float(_)
        ));
        assert_eq!(Json::Float(1000.0).to_string(), "1000");
        assert_eq!(Json::Float(1.5).to_string(), "1.5");
        assert_eq!(Json::Float(f64::NAN).to_string(), "null");
        assert_eq!(Json::Int(7).as_i64(), Some(7));
        assert_eq!(Json::Float(7.0).as_i64(), Some(7));
        assert_eq!(Json::Float(7.5).as_i64(), None);
        assert_eq!(Json::Float(1e30).as_i64(), None);
        assert_eq!(Json::Int(-1).as_u32(), None);
        assert_eq!(Json::Int(3).as_u32(), Some(3));
    }

    #[test]
    fn whitespace_is_allowed_everywhere() {
        let v = Json::parse(" {\n\t\"a\" : [ 1 , 2 ]\r\n} ").unwrap();
        assert_eq!(v.to_string(), r#"{"a":[1,2]}"#);
    }

    #[test]
    fn rejects_malformed_input() {
        for text in [
            "",
            "  ",
            "{",
            "}",
            "[",
            "]",
            "[1,]",
            "{\"a\":}",
            "{a:1}",
            "nul",
            "tru",
            "01x",
            "1.",
            "1e",
            "-",
            "\"",
            "\"\\q\"",
            "\"\\u12\"",
            "\"\\u12g4\"",
            "\"\\ud800\"",
            "\"\\ud800\\u0041\"",
            "1 2",
            "{\"a\":1} {}",
            "\"a\nb\"",
        ] {
            assert!(Json::parse(text).is_err(), "{text:?} must not parse");
        }
    }

    #[test]
    fn rejects_deep_nesting() {
        let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
        assert!(Json::parse(&deep).is_err());
        let ok = format!("{}1{}", "[".repeat(100), "]".repeat(100));
        assert!(Json::parse(&ok).is_ok());
    }

    #[test]
    fn accessors_and_builders() {
        let mut v = Json::obj([("a", Json::Int(1))]);
        v.set("b", Json::str("x"));
        v.set("a", Json::Int(2));
        assert_eq!(v.to_string(), r#"{"a":2,"b":"x"}"#);
        assert_eq!(v.get("b").and_then(Json::as_str), Some("x"));
        assert_eq!(v.get("missing"), None);
        assert!(Json::Null.is_null());
        assert_eq!(Json::array([Json::Bool(true)]).as_array().unwrap().len(), 1);
        assert_eq!(Json::object().as_object().unwrap().len(), 0);
        assert_eq!(Json::Bool(true).as_bool(), Some(true));
        // `set` on a non-object is a no-op rather than a panic.
        let mut scalar = Json::Int(1);
        scalar.set("a", Json::Null);
        assert_eq!(scalar, Json::Int(1));
    }

    #[test]
    fn pretty_printing_nests() {
        let v = Json::parse(r#"{"a":[1,{"b":null}],"c":{}}"#).unwrap();
        assert_eq!(
            v.to_pretty_string(),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": {}\n}"
        );
    }
}
