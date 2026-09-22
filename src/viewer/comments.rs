//! The comment side table the documentation view reads.
//!
//! Neither frontend attaches comments to the tree: both lexers push every
//! comment into a side table (`verilog::Lexed::comments`,
//! `vhdl::Lexed::comments`) keyed by span, which is what a formatter needs
//! and what this module needs too. [`Comments`] collects those spans, in
//! any mixture of languages, and answers the two questions the
//! documentation view asks:
//!
//! - [`Comments::leading`]: the block of whole-line comments immediately
//!   above an object's line — the module's description.
//! - [`Comments::trailing`]: the comment after an object on its own line —
//!   the per-port note in a port list.
//!
//! The table is language-independent on purpose: it holds spans, and the
//! marker (`//`, `--`, `/* */`) is recognised when the text is read back
//! out of the [`SourceMap`]. That keeps the `viewer` feature buildable on
//! its own, with the two frontends only needed for the convenience
//! constructors below.

use std::collections::BTreeMap;

use crate::source::{SourceId, SourceMap, Span};

/// Every comment the caller knows about, grouped by file and sorted by
/// position.
#[derive(Clone, Debug, Default)]
pub struct Comments {
    by_file: BTreeMap<SourceId, Vec<Span>>,
}

impl Comments {
    /// An empty table. A design rendered with it simply has no prose.
    pub fn new() -> Self {
        Comments::default()
    }

    /// Records one comment span.
    ///
    /// Spans may arrive in any order; each file's list is kept sorted by
    /// start offset, and an exact duplicate is dropped so lexing a file
    /// twice is harmless.
    pub fn add(&mut self, span: Span) {
        let list = self.by_file.entry(span.file).or_default();
        match list.binary_search_by_key(&(span.start, span.end), |s| (s.start, s.end)) {
            Ok(_) => {}
            Err(at) => list.insert(at, span),
        }
    }

    /// Records every span of an iterator.
    pub fn extend(&mut self, spans: impl IntoIterator<Item = Span>) {
        for span in spans {
            self.add(span);
        }
    }

    /// Number of comments recorded.
    pub fn len(&self) -> usize {
        self.by_file.values().map(Vec::len).sum()
    }

    /// True when nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.by_file.values().all(Vec::is_empty)
    }

    /// The comments recorded for one file, in source order.
    pub fn in_file(&self, file: SourceId) -> &[Span] {
        self.by_file.get(&file).map_or(&[], Vec::as_slice)
    }

    /// Records the comments of a lexed Verilog file.
    #[cfg(feature = "verilog")]
    pub fn add_verilog(&mut self, lexed: &crate::verilog::Lexed) {
        self.extend(lexed.comments.iter().map(|(span, _)| *span));
    }

    /// Records the comments of a lexed VHDL file.
    #[cfg(feature = "vhdl")]
    pub fn add_vhdl(&mut self, lexed: &crate::vhdl::Lexed<'_>) {
        self.extend(lexed.comments.iter().map(|(span, _)| *span));
    }

    /// The description above `span`: the run of comments that occupy whole
    /// lines immediately before it, with their markers stripped.
    ///
    /// A blank line, or anything else on one of those lines, ends the run,
    /// so the comment on an unrelated declaration two lines up is not
    /// mistaken for a description.
    pub fn leading(&self, map: &SourceMap, span: Span) -> Option<String> {
        let list = self.in_file(span.file);
        if list.is_empty() {
            return None;
        }
        let file = map.file(span.file);
        let mut want = file.loc(span.start).line.checked_sub(1)?;
        let mut parts: Vec<String> = Vec::new();
        // Walk upwards: a comment counts when it ends on the line we are
        // looking for and starts at the beginning of its own first line.
        while let Some(found) = list
            .iter()
            .rev()
            .find(|c| end_line(map, **c) == want && starts_line(map, **c))
        {
            parts.push(clean(text_of(map, *found)));
            match file.loc(found.start).line.checked_sub(1) {
                // Line 0 does not exist, so the file starts with the block.
                Some(0) | None => break,
                Some(above) => want = above,
            }
        }
        if parts.is_empty() {
            return None;
        }
        parts.reverse();
        let text = parts.join("\n");
        let text = text.trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    /// The comment written after `span` on the same line, with its marker
    /// stripped: the note beside a port in a port list.
    pub fn trailing(&self, map: &SourceMap, span: Span) -> Option<String> {
        let list = self.in_file(span.file);
        if list.is_empty() {
            return None;
        }
        let file = map.file(span.file);
        let line = file.loc(span.end).line;
        let found = list
            .iter()
            .find(|c| c.start >= span.end && file.loc(c.start).line == line)?;
        let text = clean(text_of(map, *found));
        let text = text.trim().to_owned();
        (!text.is_empty()).then_some(text)
    }
}

/// The source text a span covers.
fn text_of(map: &SourceMap, span: Span) -> &str {
    let text = map.file(span.file).text();
    let start = (span.start as usize).min(text.len());
    let end = (span.end as usize).min(text.len()).max(start);
    // Spans come from a lexer and are on character boundaries, but a
    // hand-built table might not be; fall back to the whole file rather
    // than panicking on a slice.
    text.get(start..end).unwrap_or("")
}

/// The 1-based line the span's last byte is on.
fn end_line(map: &SourceMap, span: Span) -> u32 {
    let last = span.end.max(span.start.saturating_add(1)) - 1;
    map.file(span.file).loc(last).line
}

/// True when nothing but whitespace precedes the span on its first line.
fn starts_line(map: &SourceMap, span: Span) -> bool {
    let file = map.file(span.file);
    let loc = file.loc(span.start);
    let Some(text) = file.line_text(loc.line) else {
        return false;
    };
    let before = loc.col.saturating_sub(1) as usize;
    text.chars().take(before).all(char::is_whitespace)
}

/// Strips a comment's markers and the decoration of its continuation
/// lines, leaving the prose.
fn clean(text: &str) -> String {
    let text = text.trim_end();
    if let Some(rest) = text.strip_prefix("//") {
        return rest.trim_start_matches('/').trim().to_owned();
    }
    if let Some(rest) = text.strip_prefix("--") {
        return rest.trim_start_matches('-').trim().to_owned();
    }
    let body = match text.strip_prefix("/*") {
        Some(rest) => rest.strip_suffix("*/").unwrap_or(rest),
        None => text,
    };
    let lines: Vec<String> = body
        .lines()
        .map(|line| {
            let line = line.trim();
            line.strip_prefix('*')
                .map_or(line, |rest| rest.trim_start())
                .to_owned()
        })
        .collect();
    // Drop the blank first and last lines a `/**\n * ...\n */` block leaves.
    let mut slice = lines.as_slice();
    while slice.first().is_some_and(|l| l.is_empty()) {
        slice = &slice[1..];
    }
    while slice.last().is_some_and(|l| l.is_empty()) {
        slice = &slice[..slice.len() - 1];
    }
    slice.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lexes the `//`, `--` and `/* */` comments out of `text` the way a
    /// frontend's side table would, so the tests here need no frontend.
    fn scan(map: &SourceMap, file: SourceId) -> Comments {
        let text = map.file(file).text();
        let bytes = text.as_bytes();
        let mut out = Comments::new();
        let mut i = 0;
        while i < bytes.len() {
            let two = &text[i..(i + 2).min(text.len())];
            if two == "//" || two == "--" {
                let end = text[i..].find('\n').map_or(text.len(), |n| i + n);
                out.add(Span::new(
                    file,
                    u32::try_from(i).unwrap(),
                    u32::try_from(end).unwrap(),
                ));
                i = end;
            } else if two == "/*" {
                let end = text[i..].find("*/").map_or(text.len(), |n| i + n + 2);
                out.add(Span::new(
                    file,
                    u32::try_from(i).unwrap(),
                    u32::try_from(end).unwrap(),
                ));
                i = end;
            } else {
                i += 1;
            }
        }
        out
    }

    fn at(map: &SourceMap, file: SourceId, needle: &str) -> Span {
        let text = map.file(file).text();
        let start = text.find(needle).expect("needle in the fixture");
        Span::new(
            file,
            u32::try_from(start).unwrap(),
            u32::try_from(start + needle.len()).unwrap(),
        )
    }

    #[test]
    fn leading_block_stops_at_a_blank_line() {
        let src = "// unrelated\n\n// A counter.\n// Two lines.\nmodule m;\n";
        let mut map = SourceMap::new();
        let file = map.add("t.v", src).unwrap();
        let comments = scan(&map, file);
        assert_eq!(comments.len(), 3);
        assert_eq!(
            comments.leading(&map, at(&map, file, "module m;")),
            Some("A counter.\nTwo lines.".to_owned())
        );
    }

    #[test]
    fn trailing_takes_the_comment_on_the_same_line() {
        let src = "port (\n  clk : in bit; -- the clock\n  d : in bit\n);\n";
        let mut map = SourceMap::new();
        let file = map.add("t.vhd", src).unwrap();
        let comments = scan(&map, file);
        let clk = at(&map, file, "clk : in bit;");
        assert_eq!(comments.trailing(&map, clk), Some("the clock".to_owned()));
        let d = at(&map, file, "d : in bit");
        assert_eq!(comments.trailing(&map, d), None);
        // A trailing comment is not a description.
        assert_eq!(comments.leading(&map, d), None);
    }

    #[test]
    fn block_comments_lose_their_decoration() {
        let src = "/**\n * A block.\n * With two lines.\n */\nentity e is\n";
        let mut map = SourceMap::new();
        let file = map.add("t.vhd", src).unwrap();
        let comments = scan(&map, file);
        assert_eq!(
            comments.leading(&map, at(&map, file, "entity e is")),
            Some("A block.\nWith two lines.".to_owned())
        );
    }

    #[test]
    fn an_empty_table_answers_nothing() {
        let mut map = SourceMap::new();
        let file = map.add("t.v", "module m;\n").unwrap();
        let comments = Comments::new();
        assert!(comments.is_empty());
        assert!(comments.in_file(file).is_empty());
        assert_eq!(comments.leading(&map, at(&map, file, "module m;")), None);
        assert_eq!(comments.trailing(&map, at(&map, file, "module m;")), None);
    }

    #[test]
    fn duplicates_and_order_are_normalised() {
        let mut map = SourceMap::new();
        let file = map.add("t.v", "// a\n// b\nmodule m;\n").unwrap();
        let mut comments = Comments::new();
        let b = at(&map, file, "// b");
        let a = at(&map, file, "// a");
        comments.add(b);
        comments.add(a);
        comments.add(a);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments.in_file(file), [a, b]);
        assert_eq!(
            comments.leading(&map, at(&map, file, "module m;")),
            Some("a\nb".to_owned())
        );
    }
}
