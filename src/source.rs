//! Source files, spans and position lookup.
//!
//! A [`SourceMap`] owns the text of every file handed to the compiler and
//! assigns each a [`SourceId`]. Every AST node and IR object refers back to
//! its origin through a [`Span`], which is a pair of byte offsets inside one
//! file. Spans are small (12 bytes) so they can be carried everywhere; the
//! map turns them back into human-readable lines and columns only when a
//! diagnostic is rendered.
//!
//! The map never reads the filesystem. The caller (the CLI, a test, a
//! language server) loads the text and calls [`SourceMap::add`], which keeps
//! the core sans-I/O and lets a preprocessor add synthetic files for macro
//! expansions.

use std::fmt;

/// Narrows a byte offset or count to the `u32` used in spans.
///
/// Every value passed here is bounded by the length of a file that
/// [`SourceMap::add`] accepted, so the conversion cannot fail; the check is
/// kept as a cheap assertion rather than an unchecked cast.
fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("source offset exceeds u32")
}

/// Identifies one file inside a [`SourceMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(u32);

impl SourceId {
    /// The raw index of this file inside its map.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A half-open byte range `[start, end)` inside one source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// The file this span refers to.
    pub file: SourceId,
    /// Byte offset of the first byte covered.
    pub start: u32,
    /// Byte offset one past the last byte covered.
    pub end: u32,
}

impl Span {
    /// Builds a span from byte offsets; `end` is clamped to be at least `start`.
    pub fn new(file: SourceId, start: u32, end: u32) -> Self {
        Span {
            file,
            start,
            end: end.max(start),
        }
    }

    /// Number of bytes covered.
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// True when the span covers no bytes (a pure position).
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The smallest span covering both `self` and `other`.
    ///
    /// Both must be in the same file; the caller guarantees that, since
    /// joining across files is always a logic error upstream.
    pub fn to(self, other: Span) -> Span {
        debug_assert_eq!(self.file, other.file, "joining spans across files");
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A resolved 1-based line and column.
///
/// The column counts characters, not bytes, so it lines up with what an
/// editor shows for non-ASCII identifiers and comments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Loc {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column in characters.
    pub col: u32,
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// One loaded file: its display name, its text and a line-start index.
#[derive(Debug)]
pub struct SourceFile {
    name: String,
    text: String,
    /// Byte offset at which each line begins; `line_starts[0] == 0`.
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(name: String, text: String) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(offset(i + 1));
            }
        }
        SourceFile {
            name,
            text,
            line_starts,
        }
    }

    /// The name given when the file was added (usually its path).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The full text of the file.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of lines; a file always has at least one, even when empty.
    pub fn line_count(&self) -> u32 {
        offset(self.line_starts.len())
    }

    /// The 0-based index of the line containing byte `offset`.
    fn line_index(&self, offset: u32) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }

    /// Resolves a byte offset to a 1-based line and character column.
    ///
    /// Offsets past the end of the file resolve to the end of the last line.
    pub fn loc(&self, offset: u32) -> Loc {
        let offset = (offset as usize).min(self.text.len());
        let line = self.line_index(self::offset(offset));
        let line_start = self.line_starts[line] as usize;
        let col = self.text[line_start..offset].chars().count();
        Loc {
            line: self::offset(line + 1),
            col: self::offset(col + 1),
        }
    }

    /// The text of the 1-based line `line`, without its trailing newline.
    ///
    /// Returns `None` when the line does not exist.
    pub fn line_text(&self, line: u32) -> Option<&str> {
        let idx = line.checked_sub(1)? as usize;
        let start = *self.line_starts.get(idx)? as usize;
        let end = self
            .line_starts
            .get(idx + 1)
            .map_or(self.text.len(), |&s| s as usize);
        Some(self.text[start..end].trim_end_matches(['\n', '\r']))
    }
}

/// Owns every source file known to one compilation.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

/// Error returned by [`SourceMap::add`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMapError {
    /// The file is too large to be addressed by 32-bit byte offsets.
    TooLarge,
    /// The map already holds `u32::MAX` files.
    TooManyFiles,
}

impl fmt::Display for SourceMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceMapError::TooLarge => f.write_str("source file exceeds 4 GiB"),
            SourceMapError::TooManyFiles => f.write_str("too many source files"),
        }
    }
}

impl std::error::Error for SourceMapError {}

impl SourceMap {
    /// Creates an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file and returns its id.
    ///
    /// `name` is only used for display; two files may share a name.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<SourceId, SourceMapError> {
        let text = text.into();
        // Spans are half-open, so `end` may equal the length; keep one byte
        // of headroom so `end` itself always fits in a `u32`.
        if text.len() >= u32::MAX as usize {
            return Err(SourceMapError::TooLarge);
        }
        let id = u32::try_from(self.files.len()).map_err(|_| SourceMapError::TooManyFiles)?;
        self.files.push(SourceFile::new(name.into(), text));
        Ok(SourceId(id))
    }

    /// The file with the given id.
    ///
    /// Ids come only from [`SourceMap::add`] on this map, so a miss is a
    /// programming error and panics.
    pub fn file(&self, id: SourceId) -> &SourceFile {
        &self.files[id.index()]
    }

    /// Iterates over all files in insertion order.
    pub fn files(&self) -> impl Iterator<Item = (SourceId, &SourceFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| (SourceId(offset(i)), f))
    }

    /// Number of files added so far.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// True when no file has been added.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Resolves the start of a span to a file name and location.
    pub fn locate(&self, span: Span) -> (&str, Loc) {
        let file = self.file(span.file);
        (file.name(), file.loc(span.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_lines_and_columns() {
        let mut map = SourceMap::new();
        let id = map
            .add("a.v", "module m;\n  wire é_x;\nendmodule\n")
            .unwrap();
        let f = map.file(id);
        assert_eq!(f.line_count(), 4);
        assert_eq!(f.loc(0), Loc { line: 1, col: 1 });
        assert_eq!(f.loc(9), Loc { line: 1, col: 10 });
        assert_eq!(f.loc(10), Loc { line: 2, col: 1 });
        // "  wire é_x;": `_` is the 9th character but the 10th byte.
        let underscore = offset(f.text().find('_').unwrap());
        assert_eq!(f.loc(underscore), Loc { line: 2, col: 9 });
        assert_eq!(f.line_text(2), Some("  wire é_x;"));
        assert_eq!(f.line_text(4), Some(""));
        assert_eq!(f.line_text(5), None);
        assert_eq!(f.line_text(0), None);
        // Past the end clamps to the last line.
        assert_eq!(f.loc(1000), Loc { line: 4, col: 1 });
    }

    #[test]
    fn empty_file_has_one_line() {
        let mut map = SourceMap::new();
        let id = map.add("e.v", "").unwrap();
        let f = map.file(id);
        assert_eq!(f.line_count(), 1);
        assert_eq!(f.loc(0), Loc { line: 1, col: 1 });
        assert_eq!(f.line_text(1), Some(""));
    }

    #[test]
    fn spans_join_and_clamp() {
        let id = SourceId(0);
        let a = Span::new(id, 4, 8);
        let b = Span::new(id, 6, 20);
        assert_eq!(a.to(b), Span::new(id, 4, 20));
        assert_eq!(b.to(a), Span::new(id, 4, 20));
        assert_eq!(Span::new(id, 9, 3).len(), 0);
        assert!(Span::new(id, 9, 3).is_empty());
    }

    #[test]
    fn crlf_lines_are_trimmed() {
        let mut map = SourceMap::new();
        let id = map.add("w.vhd", "entity e is\r\nend e;\r\n").unwrap();
        let f = map.file(id);
        assert_eq!(f.line_text(1), Some("entity e is"));
        assert_eq!(f.line_text(2), Some("end e;"));
        assert_eq!(f.loc(13), Loc { line: 2, col: 1 });
    }
}
