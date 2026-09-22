//! Comment attachment and compiler-directive handling for the formatter.
//!
//! The parser throws comments away — they are kept in a side table by the
//! lexer ([`crate::verilog::Lexed::comments`]) — so the formatter has to put
//! them back. It does so by position: before emitting an item or statement
//! it hands out every comment that starts before that item ("leading"),
//! after emitting it, a comment that starts on the same line ("trailing"),
//! and at the end of a block whatever is left over between the last item and
//! the closing keyword ("dangling"). Nothing is ever dropped: whatever is
//! still unclaimed at the end of the file is flushed there.
//!
//! # Compiler directives
//!
//! The formatter deliberately does **not** run the preprocessor: expanding
//! `` `define `` would rewrite the user's source into something they never
//! wrote. Instead, every line whose first non-blank character is a backtick
//! is treated as opaque text: it is blanked out of the copy handed to the
//! lexer and the parser (blanked, not removed, so every span still points at
//! the original text) and re-emitted verbatim as a leading comment of the
//! following item.
//!
//! That keeps `` `define ``, `` `ifdef `` / `` `else `` / `` `endif `` and
//! macro uses byte for byte where they were, in order, so re-preprocessing
//! the formatted text yields the same design. It also means macro-heavy
//! files format conservatively: the *contents* of both arms of an `` `ifdef ``
//! are formatted as if both were active (which they are, syntactically), and
//! a directive that splits a construct in half — an `` `ifdef `` inside an
//! expression or between a `begin` and its `end` — makes the file
//! unparseable without the preprocessor, so the formatter reports the parse
//! errors and leaves the file alone.

use crate::fmt_doc::Doc;
use crate::source::Span;
use crate::verilog::CommentKind;

/// Narrows a byte offset to the `u32` used in spans; the text came from a
/// [`crate::source::SourceMap`] and is bounded by the same limit.
fn offset(n: usize) -> u32 {
    u32::try_from(n).expect("source offset exceeds u32")
}

/// One comment or directive, ready to be emitted.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    /// The text exactly as written, with trailing blanks removed.
    pub(crate) text: String,
    /// True when the source had at least one blank line before it.
    pub(crate) blank_before: bool,
}

impl Piece {
    /// The document for the piece: one line of text, or verbatim text when
    /// it spans lines. A block comment or a `` `define `` continuation is
    /// never re-indented: its text must survive byte for byte.
    pub(crate) fn doc(&self) -> Doc {
        Doc::verbatim(self.text.clone())
    }
}

/// The comments and directives of one file, handed out by position.
pub(crate) struct Comments<'a> {
    src: &'a str,
    entries: Vec<Entry>,
    next: usize,
    /// End offset of the last thing emitted, for blank-line detection.
    last_end: u32,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    start: u32,
    end: u32,
}

impl<'a> Comments<'a> {
    /// Collects the lexer's comments and the directive regions of `src`
    /// into one position-ordered table.
    pub(crate) fn new(
        src: &'a str,
        comments: &[(Span, CommentKind)],
        directives: &[(usize, usize)],
    ) -> Self {
        let mut entries: Vec<Entry> = comments
            .iter()
            .map(|(span, _)| Entry {
                start: span.start,
                end: span.end,
            })
            .chain(directives.iter().map(|&(start, end)| Entry {
                start: offset(start),
                end: offset(end),
            }))
            .collect();
        entries.sort_by_key(|e| (e.start, e.end));
        Comments {
            src,
            entries,
            next: 0,
            last_end: 0,
        }
    }

    /// The source text between two offsets, for deciding whether two
    /// constructs are close enough to be aligned together.
    pub(crate) fn between(&self, from: u32, to: u32) -> &str {
        let from = (from as usize).min(self.src.len());
        let to = (to as usize).clamp(from, self.src.len());
        self.src.get(from..to).unwrap_or("")
    }

    /// True when at least one blank line separates the last emitted thing
    /// from `start`.
    pub(crate) fn gap_blank(&self, start: u32) -> bool {
        let from = self.last_end.min(start);
        let text = &self.src[from as usize..start as usize];
        text.bytes().filter(|&b| b == b'\n').count() >= 2
    }

    /// Records that everything up to `end` has been emitted.
    pub(crate) fn advance(&mut self, end: u32) {
        self.last_end = self.last_end.max(end);
    }

    /// Every unclaimed comment that starts before `before`.
    pub(crate) fn leading(&mut self, before: u32) -> Vec<Piece> {
        let mut out = Vec::new();
        while let Some(entry) = self.entries.get(self.next).copied() {
            if entry.start >= before {
                break;
            }
            self.next += 1;
            out.push(self.piece(entry));
        }
        out
    }

    /// Everything still unclaimed, for the end of the file.
    pub(crate) fn remaining(&mut self) -> Vec<Piece> {
        self.leading(u32::MAX)
    }

    /// A comment on the same line as `after` and before `before`, which
    /// belongs at the end of the line just written.
    pub(crate) fn trailing(&mut self, after: u32, before: u32) -> Option<Piece> {
        let entry = self.entries.get(self.next).copied()?;
        if entry.start >= before {
            return None;
        }
        let from = after.min(entry.start) as usize;
        if self.src[from..entry.start as usize].contains('\n') {
            return None;
        }
        self.next += 1;
        let mut piece = self.piece(entry);
        piece.blank_before = false;
        Some(piece)
    }

    /// The comment that closes the header line of a construct: one that
    /// follows `head_end` with nothing but the header's own punctuation
    /// (`)`, `;`, `,`) in between, on that same line.
    pub(crate) fn head_trailing(&mut self, head_end: u32, limit: u32) -> Option<Piece> {
        let entry = self.entries.get(self.next).copied()?;
        if entry.start >= limit {
            return None;
        }
        let gap = self.between(head_end.min(entry.start), entry.start);
        if !gap
            .chars()
            .all(|c| c.is_whitespace() || matches!(c, ')' | ';' | ','))
        {
            return None;
        }
        let newlines = gap.bytes().filter(|&b| b == b'\n').count();
        // The header may end on the next line (`)` and `;` of a broken
        // port list), but the comment must sit after that punctuation.
        if newlines > 1 {
            return None;
        }
        if newlines == 1 && gap.rsplit('\n').next().unwrap_or("").trim().is_empty() {
            return None;
        }
        self.next += 1;
        let mut piece = self.piece(entry);
        piece.blank_before = false;
        Some(piece)
    }

    fn piece(&mut self, entry: Entry) -> Piece {
        let blank_before = self.gap_blank(entry.start);
        let text = self.src[entry.start as usize..entry.end as usize]
            .trim_end()
            .to_owned();
        self.advance(entry.end);
        Piece { text, blank_before }
    }
}

/// The byte ranges of the compiler-directive lines of `src`.
///
/// A directive line is one whose first non-blank character is a backtick,
/// outside any comment, string or escaped identifier. The range runs from
/// the backtick to the end of the line, and on over any line that ends in a
/// backslash continuation, so a multi-line `` `define `` stays in one piece.
pub(crate) fn directive_regions(src: &str) -> Vec<(usize, usize)> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    // True while only blanks have been seen since the last line break.
    let mut at_line_start = true;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\n' => {
                at_line_start = true;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                at_line_start = false;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                at_line_start = false;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                at_line_start = false;
            }
            b'\\' => {
                // An escaped identifier runs to the next whitespace.
                i += 1;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                at_line_start = false;
            }
            b'`' if at_line_start => {
                let start = i;
                let mut end = i;
                loop {
                    while end < bytes.len() && bytes[end] != b'\n' {
                        end += 1;
                    }
                    let line = src[start..end].trim_end();
                    if line.ends_with('\\') && end < bytes.len() {
                        end += 1;
                    } else {
                        break;
                    }
                }
                out.push((start, end));
                i = end;
                at_line_start = false;
            }
            _ => {
                i += 1;
                at_line_start = false;
            }
        }
    }
    out
}

/// A copy of `src` with every directive region replaced by blanks.
///
/// Line breaks are kept so that every byte offset — and therefore every
/// span the lexer and parser report — still refers to the same place in the
/// original text.
pub(crate) fn blank_directives(src: &str, regions: &[(usize, usize)]) -> String {
    if regions.is_empty() {
        return src.to_owned();
    }
    let mut out = src.as_bytes().to_vec();
    for &(start, end) in regions {
        for b in &mut out[start..end] {
            if *b != b'\n' && *b != b'\r' {
                *b = b' ';
            }
        }
    }
    String::from_utf8(out).expect("blanking only replaces whole ASCII bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_directive_lines() {
        let src = "`define A 1\nmodule m;\n  `ifdef X\n  wire a;\n  `endif\nendmodule\n";
        let regions = directive_regions(src);
        let texts: Vec<&str> = regions.iter().map(|&(a, b)| &src[a..b]).collect();
        assert_eq!(texts, ["`define A 1", "`ifdef X", "`endif"]);
    }

    #[test]
    fn ignores_backticks_in_comments_and_strings() {
        let src = "// `define A\n/*\n`define B\n*/\nwire x = \"`c\";\n";
        assert!(directive_regions(src).is_empty());
    }

    #[test]
    fn follows_a_line_continuation() {
        let src = "`define M(a) \\\n  wire a;\nmodule m;\n";
        let regions = directive_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(
            &src[regions[0].0..regions[0].1],
            "`define M(a) \\\n  wire a;"
        );
    }

    #[test]
    fn blanking_keeps_offsets() {
        let src = "`define A 1\nwire x;\n";
        let regions = directive_regions(src);
        let blanked = blank_directives(src, &regions);
        assert_eq!(blanked.len(), src.len());
        assert_eq!(&blanked[12..], "wire x;\n");
        assert!(blanked[..11].trim().is_empty());
    }
}
