//! A Wadler / Prettier-style document printer, shared by the source
//! formatters.
//!
//! A formatter never writes text directly. It builds a [`Doc`], a tree that
//! says what the output contains and *where it may break*, and the layout
//! algorithm then chooses, for every [`Doc::group`], whether that group fits
//! on the rest of the current line (printed flat) or must be broken. This is
//! the classic algorithm from Wadler's "A prettier printer", with the
//! pragmatic additions every real implementation needs: hard breaks,
//! verbatim text (for comments, which must survive byte for byte) and an
//! indentation unit that may be spaces or tabs.
//!
//! # The pieces
//!
//! | Constructor        | Flat            | Broken            |
//! |--------------------|-----------------|-------------------|
//! | [`Doc::text`]      | the text        | the text          |
//! | [`Doc::verbatim`]  | the text, newlines and all (never fits) |
//! | [`Doc::line`]      | one space       | a newline         |
//! | [`Doc::softline`]  | nothing         | a newline         |
//! | [`Doc::hardline`]  | a newline, and the enclosing groups break |
//! | [`Doc::concat`]    | its parts, in order                 |
//! | [`Doc::group`]     | flat if it fits, broken otherwise   |
//! | [`Doc::nest`]      | adds indentation levels to the newlines inside |
//!
//! Nesting counts *levels*, not columns, so the same document renders with
//! two spaces, four spaces or tabs depending on [`FormatOptions::indent`].
//! Column alignment that must survive a tab indent (aligned `=>` in a port
//! map, say) is done by the rules with explicit [`Doc::text`] padding, not
//! by nesting.
//!
//! Indentation is written lazily, just before the next piece of text, so a
//! blank line is really blank and no line ever ends in spaces.
//!
//! # Diffs
//!
//! The check mode of a formatter reports *what* it would change, so this
//! module also carries a small line diff ([`unified_diff`], on a Myers
//! shortest-edit-script search) with no dependency on anything outside the
//! crate.
//!
//! ```
//! use reticle::fmt_doc::{Doc, FormatOptions};
//!
//! let doc = Doc::concat([
//!     Doc::text("f("),
//!     Doc::concat([
//!         Doc::softline(),
//!         Doc::text("a,"),
//!         Doc::line(),
//!         Doc::text("b"),
//!     ])
//!     .nest(1),
//!     Doc::softline(),
//!     Doc::text(")"),
//! ])
//! .group();
//!
//! let wide = FormatOptions::default();
//! assert_eq!(doc.render(&wide), "f(a, b)\n");
//!
//! let narrow = FormatOptions {
//!     line_width: 4,
//!     ..FormatOptions::default()
//! };
//! assert_eq!(doc.render(&narrow), "f(\n  a,\n  b\n)\n");
//! ```

use std::fmt;

// --- options ---------------------------------------------------------------

/// What one level of indentation is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indent {
    /// That many spaces per level.
    Spaces(usize),
    /// One tab per level.
    Tabs,
}

impl Indent {
    /// The text of one level.
    pub fn unit(self) -> String {
        match self {
            Indent::Spaces(n) => " ".repeat(n),
            Indent::Tabs => "\t".to_owned(),
        }
    }

    /// How wide one level is taken to be when deciding whether a line
    /// fits. A tab is counted as four columns, the usual default.
    pub fn width(self) -> usize {
        match self {
            Indent::Spaces(n) => n,
            Indent::Tabs => 4,
        }
    }
}

/// How reserved words are spelled (VHDL only; Verilog keywords are
/// lowercase by definition).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordCase {
    /// `entity`, `port`, `downto`.
    Lower,
    /// `ENTITY`, `PORT`, `DOWNTO`.
    Upper,
}

impl KeywordCase {
    /// Spells `word`, which must be given in lowercase.
    pub fn apply(self, word: &str) -> String {
        match self {
            KeywordCase::Lower => word.to_owned(),
            KeywordCase::Upper => word.to_ascii_uppercase(),
        }
    }
}

/// The line ending of the formatted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    /// `\n`.
    Lf,
    /// `\r\n`.
    CrLf,
}

impl Newline {
    /// The text of one line break.
    pub fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::CrLf => "\r\n",
        }
    }
}

/// Everything a formatter can be told about the shape of its output.
///
/// The defaults are the house style: two spaces, a hundred columns, aligned
/// port lists and assignments, lowercase VHDL keywords and LF endings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOptions {
    /// What one level of indentation is.
    pub indent: Indent,
    /// The column the layout tries to stay inside.
    pub line_width: usize,
    /// Align the columns of a port, generic or parameter list.
    pub align_port_lists: bool,
    /// Align the operators of a run of consecutive assignments.
    pub align_assignments: bool,
    /// Keep a `case` / `when` arm whose body is one simple statement on the
    /// same line as its choices.
    pub case_items_on_one_line: bool,
    /// How VHDL reserved words are spelled.
    pub keyword_case: KeywordCase,
    /// Repeat the label of a construct after its `end` when the source left
    /// it out.
    pub complete_end_labels: bool,
    /// The line ending to write.
    pub newline: Newline,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            indent: Indent::Spaces(2),
            line_width: 100,
            align_port_lists: true,
            align_assignments: true,
            case_items_on_one_line: true,
            keyword_case: KeywordCase::Lower,
            complete_end_labels: true,
            newline: Newline::Lf,
        }
    }
}

impl FormatOptions {
    /// The text of one indentation level.
    pub fn indent_unit(&self) -> String {
        self.indent.unit()
    }

    /// Spells a lowercase reserved word according to
    /// [`FormatOptions::keyword_case`].
    pub fn keyword(&self, word: &str) -> String {
        self.keyword_case.apply(word)
    }
}

// --- documents -------------------------------------------------------------

/// A layout-independent description of some output.
///
/// Build one with the constructors ([`Doc::text`], [`Doc::line`], ...) and
/// render it with [`Doc::render`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Doc {
    /// Nothing at all.
    #[default]
    Nil,
    /// Literal text that contains no line break.
    Text(String),
    /// Literal text that may contain line breaks and is written exactly as
    /// given; it never fits on a line, so every enclosing group breaks.
    Verbatim(String),
    /// A space when flat, a line break when broken.
    Line,
    /// Nothing when flat, a line break when broken.
    SoftLine,
    /// Always a line break; every enclosing group breaks.
    HardLine,
    /// A sequence of documents.
    Concat(Vec<Doc>),
    /// A unit that is laid out flat when it fits on the rest of the line.
    Group(Box<Doc>),
    /// Adds indentation levels to every line break inside.
    Nest(usize, Box<Doc>),
}

impl Doc {
    /// Nothing.
    pub fn nil() -> Doc {
        Doc::Nil
    }

    /// Literal text. Line breaks in `s` would confuse the column
    /// bookkeeping; use [`Doc::verbatim`] for those.
    pub fn text(s: impl Into<String>) -> Doc {
        let s = s.into();
        debug_assert!(!s.contains('\n'), "Doc::text with a line break: {s:?}");
        if s.is_empty() { Doc::Nil } else { Doc::Text(s) }
    }

    /// Text written exactly as given, line breaks included.
    pub fn verbatim(s: impl Into<String>) -> Doc {
        let s = s.into();
        if s.is_empty() {
            Doc::Nil
        } else if s.contains('\n') {
            Doc::Verbatim(s)
        } else {
            Doc::Text(s)
        }
    }

    /// A space, or a line break when the enclosing group breaks.
    pub fn line() -> Doc {
        Doc::Line
    }

    /// Nothing, or a line break when the enclosing group breaks.
    pub fn softline() -> Doc {
        Doc::SoftLine
    }

    /// An unconditional line break.
    pub fn hardline() -> Doc {
        Doc::HardLine
    }

    /// Two unconditional line breaks: one blank line.
    pub fn blankline() -> Doc {
        Doc::Concat(vec![Doc::HardLine, Doc::HardLine])
    }

    /// One space.
    pub fn space() -> Doc {
        Doc::Text(" ".to_owned())
    }

    /// The documents one after another.
    pub fn concat(parts: impl IntoIterator<Item = Doc>) -> Doc {
        let parts: Vec<Doc> = parts.into_iter().filter(|d| !d.is_nil()).collect();
        match parts.len() {
            0 => Doc::Nil,
            1 => parts.into_iter().next().expect("one element"),
            _ => Doc::Concat(parts),
        }
    }

    /// The documents separated by `sep`.
    pub fn join(sep: Doc, parts: impl IntoIterator<Item = Doc>) -> Doc {
        let mut out = Vec::new();
        for (i, part) in parts.into_iter().enumerate() {
            if i > 0 {
                out.push(sep.clone());
            }
            out.push(part);
        }
        Doc::concat(out)
    }

    /// `self` followed by `other`.
    pub fn append(self, other: Doc) -> Doc {
        Doc::concat([self, other])
    }

    /// Lays `self` out flat when it fits on the rest of the line.
    pub fn group(self) -> Doc {
        Doc::Group(Box::new(self))
    }

    /// Indents every line break inside `self` by `levels` more levels.
    pub fn nest(self, levels: usize) -> Doc {
        if levels == 0 {
            self
        } else {
            Doc::Nest(levels, Box::new(self))
        }
    }

    /// Indents every line break inside `self` by one more level.
    pub fn indent(self) -> Doc {
        self.nest(1)
    }

    /// True when the document produces no output.
    pub fn is_nil(&self) -> bool {
        matches!(self, Doc::Nil)
    }

    /// True when the document contains a break that cannot be flattened.
    pub fn has_hard_break(&self) -> bool {
        match self {
            Doc::HardLine | Doc::Verbatim(_) => true,
            Doc::Concat(parts) => parts.iter().any(Doc::has_hard_break),
            Doc::Group(inner) | Doc::Nest(_, inner) => inner.has_hard_break(),
            _ => false,
        }
    }

    /// The text of the document laid out flat, ignoring the line width.
    ///
    /// Used to measure a piece when a rule aligns a column.
    pub fn flat_text(&self) -> String {
        let mut out = String::new();
        self.write_flat(&mut out);
        out
    }

    /// The width, in characters, of the document laid out flat.
    pub fn flat_width(&self) -> usize {
        self.flat_text().chars().count()
    }

    fn write_flat(&self, out: &mut String) {
        match self {
            Doc::Nil | Doc::SoftLine => {}
            Doc::Text(s) | Doc::Verbatim(s) => out.push_str(s),
            Doc::Line => out.push(' '),
            Doc::HardLine => out.push('\n'),
            Doc::Concat(parts) => {
                for p in parts {
                    p.write_flat(out);
                }
            }
            Doc::Group(inner) | Doc::Nest(_, inner) => inner.write_flat(out),
        }
    }

    /// Lays the document out under `opts`.
    ///
    /// The result always ends in exactly one line break and no line ever
    /// ends in whitespace.
    pub fn render(&self, opts: &FormatOptions) -> String {
        let unit = opts.indent_unit();
        let mut printer = Printer {
            out: String::new(),
            col: 0,
            pending: None,
            unit: &unit,
            unit_width: opts.indent.width(),
            width: opts.line_width,
        };
        printer.run(self);
        let mut out = printer.out;
        while out.ends_with('\n') || out.ends_with(' ') || out.ends_with('\t') {
            out.pop();
        }
        out.push('\n');
        if opts.newline == Newline::CrLf {
            out = out.replace('\n', "\r\n");
        }
        out
    }
}

/// Whether a piece is being laid out flat or broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// The layout state: output built so far plus the current column.
struct Printer<'a> {
    out: String,
    col: usize,
    /// Indentation level owed to the next piece of text, so that empty
    /// lines stay empty.
    pending: Option<usize>,
    unit: &'a str,
    unit_width: usize,
    width: usize,
}

impl Printer<'_> {
    fn run(&mut self, doc: &Doc) {
        let mut stack: Vec<(usize, Mode, &Doc)> = vec![(0, Mode::Break, doc)];
        while let Some((ind, mode, doc)) = stack.pop() {
            match doc {
                Doc::Nil => {}
                Doc::Text(s) => self.write_text(ind, s),
                Doc::Verbatim(s) => self.write_verbatim(ind, s),
                Doc::Line => match mode {
                    Mode::Flat => self.write_text(ind, " "),
                    Mode::Break => self.newline(ind),
                },
                Doc::SoftLine => {
                    if mode == Mode::Break {
                        self.newline(ind);
                    }
                }
                Doc::HardLine => self.newline(ind),
                Doc::Concat(parts) => {
                    for part in parts.iter().rev() {
                        stack.push((ind, mode, part));
                    }
                }
                Doc::Nest(levels, inner) => stack.push((ind + levels, mode, inner)),
                Doc::Group(inner) => {
                    let mode = if self.fits(ind, inner, &stack) {
                        Mode::Flat
                    } else {
                        Mode::Break
                    };
                    stack.push((ind, mode, inner));
                }
            }
        }
    }

    /// Whether `doc`, laid out flat at indentation `ind`, plus whatever
    /// follows it up to the next line break, stays inside the line width.
    fn fits(&self, ind: usize, doc: &Doc, rest: &[(usize, Mode, &Doc)]) -> bool {
        let mut budget = match self.width.checked_sub(self.col.max(self.indent_width(ind))) {
            Some(b) => b,
            None => return false,
        };
        // The group itself is measured flat; everything after it keeps the
        // mode it already has, so a pending break ends the line and the
        // group fits.
        let mut work: Vec<(Mode, &Doc)> = vec![(Mode::Flat, doc)];
        let mut tail = rest.len();
        loop {
            let Some((mode, doc)) = work.pop() else {
                if tail == 0 {
                    return true;
                }
                tail -= 1;
                let (_, mode, doc) = rest[tail];
                work.push((mode, doc));
                continue;
            };
            match doc {
                Doc::Nil | Doc::SoftLine if mode == Mode::Flat => {}
                Doc::Nil => {}
                Doc::Text(s) => {
                    let w = s.chars().count();
                    if w > budget {
                        return false;
                    }
                    budget -= w;
                }
                Doc::Verbatim(_) | Doc::HardLine => return mode == Mode::Break,
                Doc::Line => {
                    if mode == Mode::Break {
                        return true;
                    }
                    if budget == 0 {
                        return false;
                    }
                    budget -= 1;
                }
                Doc::SoftLine => return true,
                Doc::Concat(parts) => {
                    for part in parts.iter().rev() {
                        work.push((mode, part));
                    }
                }
                Doc::Nest(_, inner) => work.push((mode, inner)),
                Doc::Group(inner) => work.push((Mode::Flat, inner)),
            }
        }
    }

    fn indent_width(&self, ind: usize) -> usize {
        if self.pending.is_some() {
            ind * self.unit_width
        } else {
            self.col
        }
    }

    fn flush_indent(&mut self) {
        if let Some(level) = self.pending.take() {
            for _ in 0..level {
                self.out.push_str(self.unit);
            }
            self.col = level * self.unit_width;
        }
    }

    fn write_text(&mut self, _ind: usize, s: &str) {
        if s.is_empty() {
            return;
        }
        self.flush_indent();
        self.out.push_str(s);
        self.col += s.chars().count();
    }

    fn write_verbatim(&mut self, ind: usize, s: &str) {
        if !s.contains('\n') {
            self.write_text(ind, s);
            return;
        }
        self.flush_indent();
        self.out.push_str(s);
        let last = s.rsplit('\n').next().unwrap_or("");
        self.col = last.chars().count();
    }

    fn newline(&mut self, ind: usize) {
        while self.out.ends_with(' ') || self.out.ends_with('\t') {
            self.out.pop();
        }
        self.out.push('\n');
        self.col = 0;
        self.pending = Some(ind);
    }
}

// --- check mode ------------------------------------------------------------

/// What a formatter's check mode found: whether the file is already
/// formatted, the formatted text, and a unified diff from one to the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatCheck {
    /// True when formatting would change the file.
    pub changed: bool,
    /// The formatted text.
    pub formatted: String,
    /// A unified diff from the original to the formatted text, empty when
    /// nothing would change.
    pub diff: String,
}

impl FormatCheck {
    /// Compares `original` with `formatted`, naming the file `name` in the
    /// diff header.
    pub fn new(original: &str, formatted: String, name: &str) -> Self {
        let changed = original != formatted;
        let diff = if changed {
            unified_diff(original, &formatted, name, name)
        } else {
            String::new()
        };
        FormatCheck {
            changed,
            formatted,
            diff,
        }
    }
}

// --- line diff -------------------------------------------------------------

/// One step of a line-level edit script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// A line present in both texts, given as `(old index, new index)`.
    Keep(usize, usize),
    /// A line of the old text that is not in the new one.
    Delete(usize),
    /// A line of the new text that is not in the old one.
    Insert(usize),
}

/// The shortest edit script turning `old` into `new`, line by line.
///
/// This is Myers' greedy algorithm (O(ND) time and space). Beyond
/// `max_edits` differing lines it gives up and reports the whole of `old`
/// deleted and the whole of `new` inserted, which is still a correct — if
/// unhelpful — script; a formatter never gets near that bound.
pub fn diff_lines<T: PartialEq>(old: &[T], new: &[T], max_edits: usize) -> Vec<Edit> {
    let n = old.len();
    let m = new.len();
    let max = (n + m).min(max_edits);
    let offset = isize::try_from(max + 1).expect("edit budget fits isize");
    let at = |k: isize| usize::try_from(k + offset).expect("diagonal within the band");
    let idx = |i: usize| isize::try_from(i).expect("index fits isize");

    // `v[at(k)]` is the furthest `x` reached on diagonal `k = x - y`, and
    // `trace[d]` is that whole vector as it stood before the `d`-th edit.
    let mut v: Vec<usize> = vec![0; 2 * max + 3];
    let mut trace: Vec<Vec<usize>> = Vec::new();
    let mut found = None;
    'search: for d in 0..=max {
        trace.push(v.clone());
        let di = idx(d);
        let mut k = -di;
        while k <= di {
            let down = k == -di || (k != di && v[at(k - 1)] < v[at(k + 1)]);
            let mut x = if down { v[at(k + 1)] } else { v[at(k - 1)] + 1 };
            let mut y = usize::try_from(idx(x) - k).expect("y stays non-negative");
            while x < n && y < m && old[x] == new[y] {
                x += 1;
                y += 1;
            }
            v[at(k)] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'search;
            }
            k += 2;
        }
    }

    let Some(d_end) = found else {
        let mut edits: Vec<Edit> = (0..n).map(Edit::Delete).collect();
        edits.extend((0..m).map(Edit::Insert));
        return edits;
    };

    // Walk the trace backwards, emitting the script in reverse.
    let mut edits = Vec::new();
    let mut x = n;
    let mut y = m;
    for d in (0..=d_end).rev() {
        let v = &trace[d];
        let di = idx(d);
        let k = idx(x) - idx(y);
        let prev_k = if k == -di || (k != di && v[at(k - 1)] < v[at(k + 1)]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[at(prev_k)];
        // `prev_y` is -1 on the initial snake, so it is kept signed.
        let prev_y = idx(prev_x) - prev_k;
        while x > prev_x && idx(y) > prev_y {
            x -= 1;
            y -= 1;
            edits.push(Edit::Keep(x, y));
        }
        if d > 0 {
            if x == prev_x {
                y -= 1;
                edits.push(Edit::Insert(y));
            } else {
                x -= 1;
                edits.push(Edit::Delete(x));
            }
        }
    }
    edits.reverse();
    edits
}

/// A unified diff of two texts, with three lines of context.
///
/// Returns an empty string when the texts are equal. Line endings are
/// normalised to `\n` in the diff itself, so a CRLF-only change shows as a
/// whole-file rewrite only if something else changed too; the caller
/// compares the raw texts to decide whether anything changed at all.
pub fn unified_diff(old: &str, new: &str, old_name: &str, new_name: &str) -> String {
    if old == new {
        return String::new();
    }
    let old_lines: Vec<&str> = split_lines(old);
    let new_lines: Vec<&str> = split_lines(new);
    let edits = diff_lines(&old_lines, &new_lines, 50_000);

    // Group the script into hunks: every run of changes, padded with up to
    // three lines of context, merging runs that are close enough to share
    // their context.
    const CONTEXT: usize = 3;
    let changed: Vec<usize> = edits
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e, Edit::Keep(..)))
        .map(|(i, _)| i)
        .collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &i in &changed {
        let start = i.saturating_sub(CONTEXT);
        let end = (i + CONTEXT + 1).min(edits.len());
        match ranges.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => ranges.push((start, end)),
        }
    }
    let hunks: Vec<Vec<Edit>> = ranges
        .into_iter()
        .map(|(a, b)| edits[a..b].to_vec())
        .collect();

    let mut out = format!("--- {old_name}\n+++ {new_name}\n");
    for hunk in &hunks {
        let mut old_start = None;
        let mut new_start = None;
        let mut old_count = 0;
        let mut new_count = 0;
        for edit in hunk {
            match edit {
                Edit::Keep(a, b) => {
                    old_start.get_or_insert(*a);
                    new_start.get_or_insert(*b);
                    old_count += 1;
                    new_count += 1;
                }
                Edit::Delete(a) => {
                    old_start.get_or_insert(*a);
                    old_count += 1;
                }
                Edit::Insert(b) => {
                    new_start.get_or_insert(*b);
                    new_count += 1;
                }
            }
        }
        let old_start = old_start.map_or(0, |s| s + 1);
        let new_start = new_start.map_or(0, |s| s + 1);
        let _ = fmt::Write::write_fmt(
            &mut out,
            format_args!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"),
        );
        for edit in hunk {
            match edit {
                Edit::Keep(a, _) => {
                    out.push(' ');
                    out.push_str(old_lines[*a]);
                }
                Edit::Delete(a) => {
                    out.push('-');
                    out.push_str(old_lines[*a]);
                }
                Edit::Insert(b) => {
                    out.push('+');
                    out.push_str(new_lines[*b]);
                }
            }
            out.push('\n');
        }
    }
    out
}

/// Splits a text into lines, dropping a single trailing line break and
/// keeping `\r` out of the way so CRLF and LF texts compare by content.
fn split_lines(text: &str) -> Vec<&str> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(width: usize) -> FormatOptions {
        FormatOptions {
            line_width: width,
            ..FormatOptions::default()
        }
    }

    fn list() -> Doc {
        Doc::concat([
            Doc::text("("),
            Doc::concat([
                Doc::softline(),
                Doc::join(
                    Doc::concat([Doc::text(","), Doc::line()]),
                    [Doc::text("aaa"), Doc::text("bbb"), Doc::text("ccc")],
                ),
            ])
            .nest(1),
            Doc::softline(),
            Doc::text(")"),
        ])
        .group()
    }

    #[test]
    fn flat_when_it_fits() {
        assert_eq!(list().render(&opts(80)), "(aaa, bbb, ccc)\n");
    }

    #[test]
    fn breaks_when_it_does_not() {
        assert_eq!(list().render(&opts(10)), "(\n  aaa,\n  bbb,\n  ccc\n)\n");
    }

    #[test]
    fn tabs_indent_one_per_level() {
        let o = FormatOptions {
            indent: Indent::Tabs,
            line_width: 10,
            ..FormatOptions::default()
        };
        assert_eq!(list().render(&o), "(\n\taaa,\n\tbbb,\n\tccc\n)\n");
    }

    #[test]
    fn hard_break_propagates() {
        let doc = Doc::concat([
            Doc::text("a"),
            Doc::line(),
            Doc::text("b"),
            Doc::hardline(),
            Doc::text("c"),
        ])
        .group();
        assert_eq!(doc.render(&opts(80)), "a\nb\nc\n");
    }

    #[test]
    fn blank_lines_carry_no_indentation() {
        let doc = Doc::concat([
            Doc::text("begin"),
            Doc::concat([
                Doc::hardline(),
                Doc::text("a"),
                Doc::blankline(),
                Doc::text("b"),
            ])
            .nest(1),
            Doc::hardline(),
            Doc::text("end"),
        ]);
        assert_eq!(doc.render(&opts(80)), "begin\n  a\n\n  b\nend\n");
    }

    #[test]
    fn verbatim_keeps_its_own_lines() {
        let doc = Doc::concat([
            Doc::text("x"),
            Doc::line(),
            Doc::verbatim("/* one\n   two */"),
            Doc::line(),
            Doc::text("y"),
        ])
        .group();
        assert_eq!(doc.render(&opts(200)), "x\n/* one\n   two */\ny\n");
    }

    #[test]
    fn crlf_endings() {
        let o = FormatOptions {
            newline: Newline::CrLf,
            ..FormatOptions::default()
        };
        assert_eq!(Doc::text("a").append(Doc::hardline()).render(&o), "a\r\n");
    }

    #[test]
    fn upper_keywords() {
        assert_eq!(KeywordCase::Upper.apply("downto"), "DOWNTO");
        assert_eq!(KeywordCase::Lower.apply("downto"), "downto");
    }

    #[test]
    fn diff_of_equal_texts_is_empty() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n", "a", "b"), "");
    }

    #[test]
    fn diff_reports_one_changed_line() {
        let d = unified_diff("a\nb\nc\n", "a\nB\nc\n", "old", "new");
        assert_eq!(d, "--- old\n+++ new\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n");
    }

    #[test]
    fn diff_script_is_minimal() {
        let old = ["a", "b", "c", "d"];
        let new = ["a", "c", "d", "e"];
        let edits = diff_lines(&old, &new, 100);
        let kept = edits.iter().filter(|e| matches!(e, Edit::Keep(..))).count();
        assert_eq!(kept, 3);
        assert_eq!(
            edits
                .iter()
                .filter(|e| matches!(e, Edit::Delete(_)))
                .count(),
            1
        );
        assert_eq!(
            edits
                .iter()
                .filter(|e| matches!(e, Edit::Insert(_)))
                .count(),
            1
        );
    }

    #[test]
    fn diff_handles_insertion_at_the_end() {
        let d = unified_diff("a\n", "a\nb\n", "old", "new");
        assert!(d.ends_with("+b\n"), "{d}");
    }
}
