//! Diagnostics: errors, warnings and notes tied to source spans.
//!
//! Every stage reports problems by pushing a [`Diagnostic`] into a
//! [`Diagnostics`] sink rather than by returning early, so one run reports
//! everything it can find. A diagnostic has a severity, a message, zero or
//! more labelled spans (one of them primary) and free-form notes.
//!
//! Rendering is separate from collection: [`Diagnostics::render`] turns the
//! sink into rustc-style text using a [`SourceMap`] for the excerpts. The
//! output is deterministic (sorted by file and position) so tests can
//! compare it against golden files.
//!
//! ```text
//! error[E0042]: width mismatch in continuous assignment
//!   --> top.v:12:14
//!    |
//! 12 |   assign y = a + b;
//!    |              ^^^^^ expression is 9 bits wide
//!    |
//!    = note: `y` is declared as 8 bits at top.v:4:8
//! ```

use std::fmt::{self, Write};

use crate::source::{SourceMap, Span};

/// How serious a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Extra guidance attached to another diagnostic; never emitted alone.
    Help,
    /// Informational; does not affect the exit status.
    Note,
    /// Something suspicious that still yields a valid design.
    Warning,
    /// The design is invalid; compilation cannot produce a result.
    Error,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Help => "help",
            Severity::Note => "note",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A span with an explanatory message, shown as an underline in excerpts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// The source range to underline.
    pub span: Span,
    /// Text shown next to the underline; may be empty.
    pub message: String,
    /// Primary labels use `^`, secondary ones `-`. The first primary label
    /// decides the `-->` location line.
    pub primary: bool,
}

/// One reported problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity of the problem.
    pub severity: Severity,
    /// Stable short code such as `E0042`, for documentation and filtering.
    pub code: Option<&'static str>,
    /// One-line headline.
    pub message: String,
    /// Underlined source ranges, in the order they were added.
    pub labels: Vec<Label>,
    /// Extra lines shown after the excerpt, prefixed with `= note:`.
    pub notes: Vec<String>,
}

impl Diagnostic {
    /// Starts a diagnostic of the given severity.
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Diagnostic {
            severity,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Starts an error.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Starts a warning.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    /// Starts a note.
    pub fn note(message: impl Into<String>) -> Self {
        Self::new(Severity::Note, message)
    }

    /// Attaches a stable code such as `E0042`.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    /// Adds a primary label (underlined with `^`).
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
            primary: true,
        });
        self
    }

    /// Adds a secondary label (underlined with `-`).
    pub fn with_secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
            primary: false,
        });
        self
    }

    /// Adds a primary label with no message: just points at the location.
    pub fn with_span(self, span: Span) -> Self {
        self.with_label(span, "")
    }

    /// Adds a note line shown after the excerpt.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// The span reported on the `-->` line: the first primary label, else
    /// the first label of any kind.
    pub fn primary_span(&self) -> Option<Span> {
        self.labels
            .iter()
            .find(|l| l.primary)
            .or_else(|| self.labels.first())
            .map(|l| l.span)
    }

    /// True for [`Severity::Error`].
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// An ordered collection of diagnostics.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Creates an empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a diagnostic.
    pub fn push(&mut self, diag: Diagnostic) {
        self.items.push(diag);
    }

    /// Records an error with a single primary span.
    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::error(message).with_span(span));
    }

    /// Records a warning with a single primary span.
    pub fn warning(&mut self, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::warning(message).with_span(span));
    }

    /// Moves every diagnostic out of `other` into this sink.
    pub fn append(&mut self, other: &mut Diagnostics) {
        self.items.append(&mut other.items);
    }

    /// All diagnostics in the order they were recorded.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    /// Number of diagnostics of any severity.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of diagnostics with [`Severity::Error`].
    pub fn error_count(&self) -> usize {
        self.items.iter().filter(|d| d.is_error()).count()
    }

    /// Number of diagnostics with [`Severity::Warning`].
    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// True when at least one error has been recorded.
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    /// Sorts by primary location (file, then offset), keeping the original
    /// order for diagnostics that share a location. Diagnostics without a
    /// span sort first.
    pub fn sort(&mut self) {
        self.items.sort_by_key(|d| {
            d.primary_span()
                .map(|s| (Some(s.file), s.start))
                .unwrap_or((None, 0))
        });
    }

    /// Renders every diagnostic as rustc-style text.
    ///
    /// Diagnostics are rendered in their current order; call [`sort`] first
    /// for deterministic output across parallel stages.
    ///
    /// [`sort`]: Diagnostics::sort
    pub fn render(&self, map: &SourceMap) -> String {
        let mut out = String::new();
        for d in &self.items {
            render_one(&mut out, d, map);
            out.push('\n');
        }
        out
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<I: IntoIterator<Item = Diagnostic>>(&mut self, iter: I) {
        self.items.extend(iter);
    }
}

/// Writes one diagnostic in the rustc style documented at the module level.
fn render_one(out: &mut String, d: &Diagnostic, map: &SourceMap) {
    // Writing to a `String` cannot fail, so the `fmt::Result`s are dropped.
    let _ = write!(out, "{}", d.severity);
    if let Some(code) = d.code {
        let _ = write!(out, "[{code}]");
    }
    let _ = writeln!(out, ": {}", d.message);

    // Gutter width: enough for the largest line number shown.
    let max_line = d
        .labels
        .iter()
        .map(|l| map.file(l.span.file).loc(l.span.start).line)
        .max()
        .unwrap_or(0);
    let gutter = if max_line == 0 {
        1
    } else {
        max_line.to_string().len()
    };
    let pad = " ".repeat(gutter);

    if let Some(span) = d.primary_span() {
        let (name, loc) = map.locate(span);
        let _ = writeln!(out, "{pad}--> {name}:{loc}");
    }

    // Group labels by file so the excerpt for each file is introduced once.
    // Labels stay in insertion order inside a file, which is what callers
    // expect when they add the primary label first.
    let mut done = vec![false; d.labels.len()];
    for i in 0..d.labels.len() {
        if done[i] {
            continue;
        }
        let file_id = d.labels[i].span.file;
        let file = map.file(file_id);
        if i > 0 {
            // Secondary file: say where we are, since `-->` covered only the
            // primary one.
            let loc = file.loc(d.labels[i].span.start);
            let _ = writeln!(out, "{pad}::: {}:{loc}", file.name());
        }
        let _ = writeln!(out, "{pad} |");
        for (j, label) in d.labels.iter().enumerate().skip(i) {
            if done[j] || label.span.file != file_id {
                continue;
            }
            done[j] = true;
            let start = file.loc(label.span.start);
            let text = file.line_text(start.line).unwrap_or("");
            let _ = writeln!(out, "{:>gutter$} | {text}", start.line);

            // Underline from the start column to the end of the span, or the
            // end of the line for multi-line spans.
            let end = file.loc(label.span.end);
            let width = if end.line == start.line {
                (end.col - start.col).max(1) as usize
            } else {
                (text.chars().count() + 1)
                    .saturating_sub(start.col as usize)
                    .max(1)
            };
            let marker = if label.primary { '^' } else { '-' };
            let _ = write!(
                out,
                "{pad} | {}{}",
                " ".repeat(start.col as usize - 1),
                String::from_iter(std::iter::repeat_n(marker, width))
            );
            if label.message.is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, " {}", label.message);
            }
        }
    }

    if !d.notes.is_empty() {
        if !d.labels.is_empty() {
            let _ = writeln!(out, "{pad} |");
        }
        for note in &d.notes {
            let _ = writeln!(out, "{pad} = note: {note}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn at(text: &str, needle: &str) -> u32 {
        u32::try_from(text.find(needle).unwrap()).unwrap()
    }

    fn map() -> (SourceMap, crate::source::SourceId) {
        let mut map = SourceMap::new();
        let id = map
            .add(
                "top.v",
                "module top(input a, output y);\n  wire [7:0] y;\n  assign y = a + b;\nendmodule\n",
            )
            .unwrap();
        (map, id)
    }

    #[test]
    fn renders_rustc_style() {
        let (map, id) = map();
        let text = map.file(id).text();
        let expr = at(text, "a + b");
        let decl = at(text, "[7:0] y");
        let mut diags = Diagnostics::new();
        diags.push(
            Diagnostic::error("width mismatch in continuous assignment")
                .with_code("E0042")
                .with_label(Span::new(id, expr, expr + 5), "expression is 9 bits wide")
                .with_secondary(
                    Span::new(id, decl + 6, decl + 7),
                    "declared 8 bits wide here",
                )
                .with_note("the top bit is discarded"),
        );
        let expected = "\
error[E0042]: width mismatch in continuous assignment
 --> top.v:3:14
  |
3 |   assign y = a + b;
  |              ^^^^^ expression is 9 bits wide
2 |   wire [7:0] y;
  |              - declared 8 bits wide here
  |
  = note: the top bit is discarded

";
        assert_eq!(diags.render(&map), expected);
    }

    #[test]
    fn renders_without_span() {
        let (map, _) = map();
        let mut diags = Diagnostics::new();
        diags.push(Diagnostic::warning("no top module selected").with_note("use --top"));
        assert_eq!(
            diags.render(&map),
            "warning: no top module selected\n  = note: use --top\n\n"
        );
    }

    #[test]
    fn counts_and_sorts() {
        let (_, id) = map();
        let mut diags = Diagnostics::new();
        diags.error(Span::new(id, 40, 41), "second");
        diags.warning(Span::new(id, 2, 3), "first");
        diags.push(Diagnostic::note("global"));
        assert_eq!(diags.len(), 3);
        assert_eq!(diags.error_count(), 1);
        assert_eq!(diags.warning_count(), 1);
        assert!(diags.has_errors());
        diags.sort();
        let msgs: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(msgs, ["global", "first", "second"]);
    }

    #[test]
    fn multiline_span_underlines_to_end_of_line() {
        let (map, id) = map();
        let text = map.file(id).text();
        let start = at(text, "wire");
        let end = at(text, "endmodule");
        let mut diags = Diagnostics::new();
        diags.error(Span::new(id, start, end), "spans lines");
        let rendered = diags.render(&map);
        assert!(rendered.contains("2 |   wire [7:0] y;\n  |   ^^^^^^^^^^^^^\n"));
    }
}
