//! Memory image files: what `$readmemh` / `$readmemb` load and
//! `$writememh` / `$writememb` save ([`super::StmtKind::MemFile`]).
//!
//! The simulator and synthesis both give a memory the contents of a file,
//! so the format and the address rules live here, next to the statement,
//! rather than in either of them. Nothing here touches the filesystem: a
//! caller hands in a [`FileProvider`], and a saved image comes back as
//! text.
//!
//! # Format
//!
//! IEEE 1364-2005 §17.2.9: whitespace-separated words, one per memory
//! element, in hexadecimal (`h`) or binary (`b`) digits that may include
//! `x`, `z` and `_`; `//` and `/* */` comments; and `@hex` address
//! specifications that move the next word to that address.
//!
//! # Addresses
//!
//! [`load`] takes the task's optional start and end addresses in the IR's
//! zero-based element numbering, and the `base` of the statement, the
//! address that an `@hex` line gives element 0. Without a start the load
//! begins at the lowest element; without an end it runs towards the
//! highest; with both it runs from start to end, downwards when the end is
//! the smaller. An `@hex` address outside that range is an error, and
//! words past its end are ignored with a warning, as the standard asks.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::types::Const;
use crate::logic::Bit;

/// Supplies file contents to `$readmemh` / `$readmemb`, in the simulator
/// and in synthesis, without the library touching the filesystem.
pub trait FileProvider {
    /// The text of the file named `path`, or `None` when it does not exist.
    fn read_file(&self, path: &str) -> Option<String>;

    /// The *bytes* of the file named `path`, or `None` when it does not
    /// exist.
    ///
    /// The default hands over the text's bytes, which is right for every
    /// provider whose files are text — the memory files, a `$readmemh`
    /// data file, a chip database of JSON and CSV. A provider that may be
    /// asked for a **binary** file has to override it, because
    /// [`FileProvider::read_file`] cannot represent bytes that are not
    /// UTF-8 and returns `None` for them, which a caller cannot tell from
    /// a missing file.
    ///
    /// One caller needs it: Project Apicula's Gowin chip database is a
    /// compressed archive (`<device>.msgpack.xz`), so
    /// [`fpga::apicula`](crate::fpga::apicula) reads it this way.
    fn read_bytes(&self, path: &str) -> Option<Vec<u8>> {
        self.read_file(path).map(String::into_bytes)
    }
}

/// An in-memory [`FileProvider`]: a map from names to contents.
#[derive(Clone, Debug, Default)]
pub struct MemoryFiles {
    files: BTreeMap<String, String>,
}

impl MemoryFiles {
    /// An empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a file.
    pub fn insert(&mut self, name: impl Into<String>, text: impl Into<String>) -> &mut Self {
        self.files.insert(name.into(), text.into());
        self
    }
}

impl FileProvider for MemoryFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }
}

/// The file name a bit vector holds, as Verilog stores a string in one:
/// eight bits per character, the last character in the low byte, leading
/// NUL bytes ignored. `None` when a bit is unknown, a byte is not
/// printable ASCII, or nothing is left.
pub fn name_from_bits(bits: &Const) -> Option<String> {
    let bytes = bits.width().div_ceil(8);
    let mut out = String::new();
    for i in (0..bytes).rev() {
        let lo = i * 8;
        let hi = (lo + 7).min(bits.width() - 1);
        let byte = u8::try_from(bits.slice(hi, lo).to_u64()?).ok()?;
        match byte {
            0 if out.is_empty() => {}
            b' '..=b'~' => out.push(char::from(byte)),
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// One token of a memory file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// `@hex`: the next word goes to this address, in the file's
    /// numbering.
    Addr(u64),
    /// A word, sized to the memory's element width.
    Word(Const),
}

/// Splits memory file text into its addresses and words.
///
/// Every word is parsed at `width` bits, in hexadecimal when `hex` is
/// true and binary otherwise. The error names the offending token and
/// its line.
pub fn parse(text: &str, hex: bool, width: u32) -> Result<Vec<Item>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    let mut token = String::new();
    let mut token_line = 1usize;
    let mut line = 1usize;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let flush = |token: &mut String, at: usize, out: &mut Vec<Item>| -> Result<(), String> {
        if token.is_empty() {
            return Ok(());
        }
        if let Some(a) = token.strip_prefix('@') {
            let addr = u64::from_str_radix(&a.replace('_', ""), 16)
                .map_err(|_| format!("line {at}: bad address `{token}`"))?;
            out.push(Item::Addr(addr));
        } else {
            let literal = format!(
                "{}'{}{}",
                width,
                if hex { 'h' } else { 'b' },
                token.replace('_', "")
            );
            let value = Const::parse_verilog(&literal)
                .map_err(|e| format!("line {at}: bad value `{token}`: {e}"))?;
            out.push(Item::Word(value));
        }
        token.clear();
        Ok(())
    };
    while let Some(c) = chars.next() {
        if c == '\n' {
            line += 1;
        }
        if in_line_comment {
            in_line_comment = c != '\n';
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if c == '/' && matches!(chars.peek(), Some('/' | '*')) {
            flush(&mut token, token_line, &mut out)?;
            in_line_comment = chars.next() == Some('/');
            in_block_comment = !in_line_comment;
            continue;
        }
        if c.is_whitespace() {
            flush(&mut token, token_line, &mut out)?;
        } else {
            if token.is_empty() {
                token_line = line;
            }
            token.push(c);
        }
    }
    flush(&mut token, token_line, &mut out)?;
    Ok(out)
}

/// What [`load`] puts into a memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    /// `(element, word)` pairs in file order; a later pair for the same
    /// element overrides an earlier one.
    pub words: Vec<(u64, Const)>,
    /// Problems that do not stop the load.
    pub warnings: Vec<String>,
}

/// The elements a task covers: `(first, last)` in the order it walks
/// them, or an error when either bound is outside a memory of `size`
/// elements.
pub fn window(size: u64, start: Option<u64>, end: Option<u64>) -> Result<(u64, u64), String> {
    let highest = size.saturating_sub(1);
    let first = start.unwrap_or(0);
    let last = end.unwrap_or(if first <= highest { highest } else { first });
    for (what, a) in [("start", first), ("end", last)] {
        if a >= size {
            return Err(format!(
                "the {what} address {a} is outside the memory's {size} element(s)"
            ));
        }
    }
    Ok((first, last))
}

/// Places the contents of a memory file into a memory of `size` elements
/// of `width` bits; see the module docs for the address rules.
pub fn load(
    text: &str,
    hex: bool,
    width: u32,
    size: u64,
    start: Option<u64>,
    end: Option<u64>,
    base: i64,
) -> Result<Loaded, String> {
    let items = parse(text, hex, width)?;
    let (first, last) = window(size, start, end)?;
    let (low, high) = (first.min(last), first.max(last));
    let down = last < first;
    let mut out = Loaded::default();
    // `None` once the walk has left the range.
    let mut next = Some(first);
    for item in items {
        match item {
            Item::Addr(a) => {
                let element = i128::from(a) - i128::from(base);
                match u64::try_from(element) {
                    Ok(e) if (low..=high).contains(&e) => next = Some(e),
                    _ => {
                        return Err(format!(
                            "address @{a:x} is outside the range loaded (@{:x} to @{:x})",
                            i128::from(low) + i128::from(base),
                            i128::from(high) + i128::from(base)
                        ));
                    }
                }
            }
            Item::Word(value) => {
                let Some(at) = next else {
                    out.warnings.push(format!(
                        "the file has more words than the {} element(s) loaded; the rest are ignored",
                        high - low + 1
                    ));
                    break;
                };
                out.words.push((at, value));
                next = if down {
                    at.checked_sub(1).filter(|n| *n >= low)
                } else {
                    at.checked_add(1).filter(|n| *n <= high)
                };
            }
        }
    }
    Ok(out)
}

/// Renders memory contents as a memory file that [`load`] reads back.
///
/// `words` are `(element, word)` pairs in the order they are written. An
/// `@hex` line, in the file's numbering (`base` added), precedes every word
/// that does not follow its predecessor's element directly, including a
/// first word that is not element 0. Digits that are all `z` print as `z`
/// and any other unknown digit as `x`.
pub fn render(words: &[(u64, Const)], hex: bool, base: i64) -> String {
    let mut out = String::new();
    let mut expect = Some(0u64);
    for (element, word) in words {
        if expect != Some(*element) {
            let addr = i128::from(*element) + i128::from(base);
            let _ = writeln!(out, "@{addr:x}");
        }
        out.push_str(&digits(word, if hex { 4 } else { 1 }));
        out.push('\n');
        expect = element.checked_add(1);
    }
    out
}

/// A word in digits of `bits` bits each, most significant first.
fn digits(word: &Const, bits: u32) -> String {
    let width = word.width();
    if width == 0 {
        return "0".to_owned();
    }
    let count = width.div_ceil(bits);
    let mut out = String::new();
    for d in (0..count).rev() {
        let lo = d * bits;
        let hi = (lo + bits - 1).min(width - 1);
        let chunk = word.slice(hi, lo);
        let ch = match chunk.to_u64().and_then(|v| u32::try_from(v).ok()) {
            Some(v) => char::from_digit(v, 16).unwrap_or('x'),
            None if chunk.bits().iter().all(|b| *b == Bit::Z) => 'z',
            None => 'x',
        };
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(s: &str) -> Const {
        Const::parse_verilog(s).unwrap()
    }

    #[test]
    fn parsing() {
        let text = "// header\n@2 ab cd /* skip */ 1_0\n@0 ff\n";
        assert_eq!(
            parse(text, true, 8).unwrap(),
            vec![
                Item::Addr(2),
                Item::Word(l("8'hab")),
                Item::Word(l("8'hcd")),
                Item::Word(l("8'h10")),
                Item::Addr(0),
                Item::Word(l("8'hff")),
            ]
        );
        assert_eq!(
            parse("1x 01", false, 2).unwrap(),
            vec![Item::Word(l("2'b1x")), Item::Word(l("2'b01"))]
        );
        assert_eq!(parse("a\n/* x\ny */ b", true, 4).unwrap().len(), 2);
        assert_eq!(
            parse("@zz", true, 8).unwrap_err(),
            "line 1: bad address `@zz`"
        );
        assert!(
            parse("00\n\ngg", true, 8)
                .unwrap_err()
                .starts_with("line 3: bad value `gg`")
        );
    }

    #[test]
    fn loading_follows_the_address_rules() {
        let words = |r: Result<Loaded, String>| -> Vec<(u64, u64)> {
            r.unwrap()
                .words
                .into_iter()
                .map(|(a, w)| (a, w.to_u64().unwrap()))
                .collect()
        };
        // From the lowest element, upwards.
        assert_eq!(
            words(load("1 2 3", true, 8, 4, None, None, 0)),
            [(0, 1), (1, 2), (2, 3)]
        );
        // A start, and a range walked downwards.
        assert_eq!(
            words(load("1 2", true, 8, 4, Some(2), None, 0)),
            [(2, 1), (3, 2)]
        );
        assert_eq!(
            words(load("1 2 3", true, 8, 4, Some(3), Some(1), 0)),
            [(3, 1), (2, 2), (1, 3)]
        );
        // `@` addresses are in the declared numbering.
        assert_eq!(words(load("@11 7", true, 8, 4, None, None, 16)), [(1, 7)]);
        // Too many words: a warning, and the rest ignored.
        let loaded = load("1 2 3", true, 8, 2, None, None, 0).unwrap();
        assert_eq!(loaded.words.len(), 2);
        assert_eq!(loaded.warnings.len(), 1);
        // Addresses outside what is loaded are errors.
        assert!(load("@5 1", true, 8, 4, None, None, 0).is_err());
        assert!(load("@0 1", true, 8, 4, Some(1), Some(2), 0).is_err());
        assert!(load("1", true, 8, 4, Some(4), None, 0).is_err());
        assert!(load("1", true, 8, 4, Some(0), Some(9), 0).is_err());
    }

    #[test]
    fn rendering_reads_back() {
        let words = vec![(0, l("8'h0f")), (1, l("8'bxxxx0000")), (3, l("8'hzz"))];
        let text = render(&words, true, 0);
        assert_eq!(text, "0f\nx0\n@3\nzz\n");
        let back = load(&text, true, 8, 4, None, None, 0).unwrap();
        assert_eq!(back.words, words);
        assert_eq!(render(&[(0, l("3'b101"))], false, 8), "101\n");
        assert_eq!(render(&[(1, l("3'b101"))], false, 8), "@9\n101\n");
        assert_eq!(digits(&Const::zero(0), 4), "0");
    }

    #[test]
    fn memory_files() {
        let mut files = MemoryFiles::new();
        files.insert("a.hex", "00");
        assert_eq!(files.read_file("a.hex").as_deref(), Some("00"));
        assert_eq!(files.read_file("b.hex"), None);
    }
}
