//! Re-attaching comments to the tree, and remembering where the blank
//! lines were.
//!
//! The parser drops comments: they live in the side table the lexer fills,
//! [`crate::vhdl::Lexed::comments`], as spans in source order. The
//! formatter walks the tree in source order too, so putting the two back
//! together needs nothing cleverer than a cursor over that table:
//!
//! - before writing a construct, every comment that starts before it is a
//!   **leading** comment and goes on its own line above;
//! - after writing one, a comment that starts on the same source line is a
//!   **trailing** comment and goes at the end of that line;
//! - what is left over when a block closes is **dangling** and goes just
//!   before the closing keyword, and whatever remains at the end of the
//!   file is flushed there.
//!
//! Every comment is therefore written exactly once, in source order, and
//! never dropped. Its text is copied byte for byte out of the source (only
//! trailing whitespace is trimmed), so a block comment keeps its own
//! internal layout and the multiset of comment texts survives formatting.
//!
//! The same cursor answers the other question a formatter has about the
//! whitespace it is throwing away: *was there a blank line here?* A run of
//! blank lines in the source becomes exactly one blank line in the output,
//! which is what keeps a formatted file recognisable.

use crate::source::Span;
use crate::vhdl::lex::CommentKind;

/// One comment, ready to be written.
#[derive(Debug, Clone)]
pub(crate) struct Piece {
    /// The comment exactly as written, without trailing whitespace.
    pub(crate) text: String,
    /// True when at least one blank line separated it from whatever was
    /// written before it.
    pub(crate) blank_before: bool,
}

impl Piece {
    /// True for a comment that occupies more than one line, which must be
    /// written verbatim.
    pub(crate) fn is_multiline(&self) -> bool {
        self.text.contains('\n')
    }
}

/// A cursor over the lexer's comment table.
pub(crate) struct Comments<'a> {
    src: &'a str,
    /// `(start, end, kind)` in source order.
    items: Vec<(u32, u32, CommentKind)>,
    /// Index of the first comment not yet handed out.
    next: usize,
    /// End offset of the last thing written, for blank-line detection.
    last_end: u32,
}

/// Narrows a span offset to an index into the source text.
fn at(offset: u32) -> usize {
    usize::try_from(offset).expect("source offset fits usize")
}

impl<'a> Comments<'a> {
    /// Builds a cursor over `comments`, which must be in source order.
    pub(crate) fn new(src: &'a str, comments: &[(Span, CommentKind)]) -> Self {
        let mut items: Vec<(u32, u32, CommentKind)> = comments
            .iter()
            .map(|(span, kind)| (span.start, span.end, *kind))
            .collect();
        items.sort_by_key(|(start, _, _)| *start);
        Comments {
            src,
            items,
            next: 0,
            last_end: 0,
        }
    }

    /// The source text of `start..end`, without trailing whitespace.
    fn slice(&self, start: u32, end: u32) -> String {
        let end = at(end).min(self.src.len());
        let start = at(start).min(end);
        self.src[start..end].trim_end().to_owned()
    }

    /// True when the source between the last thing written and `start`
    /// holds a blank line.
    fn blank_before(&self, start: u32) -> bool {
        let from = at(self.last_end).min(self.src.len());
        let to = at(start).min(self.src.len());
        from < to && self.src[from..to].matches('\n').count() >= 2
    }

    /// True when the source between the last thing written and `start`
    /// holds a blank line, for a construct that is about to be written.
    pub(crate) fn blank_since(&self, start: u32) -> bool {
        self.blank_before(start)
    }

    /// Records that the text up to `end` has been written.
    pub(crate) fn advance(&mut self, end: u32) {
        self.last_end = self.last_end.max(end);
    }

    /// Every comment that starts before `before` and has not been written.
    pub(crate) fn leading(&mut self, before: u32) -> Vec<Piece> {
        let mut out = Vec::new();
        while self.next < self.items.len() && self.items[self.next].0 < before {
            let (start, end, _) = self.items[self.next];
            self.next += 1;
            out.push(Piece {
                text: self.slice(start, end),
                blank_before: self.blank_before(start),
            });
            self.advance(end);
        }
        out
    }

    /// A comment that starts on the same line as the construct that just
    /// ended at `after` and before `limit`.
    pub(crate) fn trailing(&mut self, after: u32, limit: u32) -> Option<Piece> {
        let (start, end, _) = *self.items.get(self.next)?;
        if start < after || start >= limit {
            return None;
        }
        let gap = &self.src[at(after).min(self.src.len())..at(start).min(self.src.len())];
        if gap.contains('\n') {
            return None;
        }
        self.next += 1;
        self.advance(end);
        Some(Piece {
            text: self.slice(start, end),
            blank_before: false,
        })
    }

    /// True when a comment starts in `a..b`, used to decide whether two
    /// statements are close enough for their operators to be aligned.
    pub(crate) fn any_between(&self, a: u32, b: u32) -> bool {
        self.items
            .iter()
            .any(|(start, _, _)| *start >= a && *start < b)
    }

    /// True when the source in `a..b` holds a blank line.
    pub(crate) fn blank_between(&self, a: u32, b: u32) -> bool {
        let from = at(a).min(self.src.len());
        let to = at(b).min(self.src.len());
        from < to && self.src[from..to].matches('\n').count() >= 2
    }

    /// The offset just past the next `byte` at or after `from`, or `from`
    /// when there is none.
    pub(crate) fn scan_past(&self, from: u32, byte: u8) -> u32 {
        let bytes = self.src.as_bytes();
        let start = at(from).min(bytes.len());
        match bytes[start..].iter().position(|b| *b == byte) {
            Some(offset) => u32::try_from(start + offset + 1).expect("source offset fits u32"),
            None => from,
        }
    }

    /// The source text of `span`, used to keep identifiers, literals and
    /// operator symbols spelled exactly as they were written.
    pub(crate) fn raw(&self, span: Span) -> &'a str {
        let end = at(span.end).min(self.src.len());
        let start = at(span.start).min(end);
        &self.src[start..end]
    }
}
