//! The document store: open documents, their parses and their diagnostics.
//!
//! One [`Document`] per URI the client has opened. It owns the text, a
//! [`LineIndex`] for the position arithmetic, the [`Index`] the features
//! answer from, and the diagnostics of the last parse. For VHDL it also
//! keeps the [`Analysis`] itself, together with the [`SourceMap`] it ran
//! against: the index holds what the features usually want, but the
//! analysis can answer about a span the index does not cover — the type
//! of an expression, or a name declared in a bundled `std` or `ieee`
//! source — and a hover on `std_logic` is worth the few hundred kilobytes
//! that keeping it costs.
//!
//! # Reparsing
//!
//! Every change reparses the whole file. That is not a placeholder: the
//! Verilog preprocessor and the VHDL analysis both work from the start of
//! the text, an HDL source file is rarely more than a few thousand lines,
//! and a full parse plus analysis of one is a few milliseconds — less than
//! the round trip that delivered the keystroke. Incremental reparsing
//! would buy nothing and would have to be kept correct against two
//! front ends. The *sync* is incremental (the client sends only the range
//! it changed, see [`Document::apply_change`]); only the parse is not.
//!
//! Debouncing is the client's business. The server never sleeps and never
//! spawns anything: a notification is handled, diagnostics come back, and
//! that is the whole cycle.
//!
//! # The previous good parse
//!
//! Both parsers recover, so a document being edited still yields a tree
//! and an index for the *current* revision; spans therefore always point
//! into the text the client has. When the current revision does not parse
//! cleanly, the document also keeps the index of the last revision that
//! did, and the features may fall back to it — but only for answers that
//! are not positions in the current document, which means hover contents
//! and completion. Go-to-definition, references, rename and the outline
//! answer from the current parse or not at all, because an answer whose
//! positions belong to a revision the client has thrown away is worse than
//! no answer.

use std::collections::BTreeMap;

use crate::diag::{Diagnostics, Severity};
use crate::source::{SourceId, SourceMap, Span};
use crate::verilog::lint::{LintConfig, LintSet};
use crate::verilog::{Dialect, NoIncludes};
use crate::vhdl::{Analysis, Standard};

#[cfg(test)]
use super::index::DeclClass;
use super::index::Index;
use super::protocol::{Diagnostic, DiagnosticSeverity, LineIndex, Location, Position, Range};
use super::{verilog, vhdl};

/// Which front end a document goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// Verilog or SystemVerilog, in the given dialect.
    Verilog(Dialect),
    /// VHDL, against the given standard.
    Vhdl(Standard),
}

impl Language {
    /// The language of a document, from the client's language identifier
    /// or, failing that, from the URI's extension.
    ///
    /// Returns `None` for a document Reticle has no front end for, which
    /// the server then simply does not open.
    pub fn detect(uri: &str, language_id: &str) -> Option<Language> {
        match language_id {
            "verilog" => return Some(Language::Verilog(Dialect::Verilog2005)),
            "systemverilog" => return Some(Language::Verilog(Dialect::SystemVerilog)),
            "vhdl" => return Some(Language::Vhdl(Standard::Vhdl2008)),
            _ => {}
        }
        let path = uri.rsplit('/').next().unwrap_or(uri);
        let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "vhd" | "vhdl" => Some(Language::Vhdl(Standard::Vhdl2008)),
            other => Dialect::for_extension(other).map(Language::Verilog),
        }
    }

    /// True for the two Verilog dialects.
    pub fn is_verilog(self) -> bool {
        matches!(self, Language::Verilog(_))
    }
}

/// One open document.
#[derive(Debug)]
pub struct Document {
    /// The URI the client opened it under.
    pub uri: String,
    /// Which front end it goes through.
    pub language: Language,
    /// The revision the client last sent.
    pub version: i64,
    text: String,
    lines: LineIndex,
    index: Index,
    diagnostics: Vec<Diagnostic>,
    healthy: bool,
    previous: Option<Box<Previous>>,
    /// The map the last parse ran against: this document, plus for VHDL
    /// the bundled `std` and `ieee` sources the analysis needed.
    map: SourceMap,
    /// This document's id inside [`Document::map`].
    source: SourceId,
    /// The semantic analysis, for VHDL only. It is kept, rather than
    /// dropped once the index is built, so that a feature can ask it about
    /// a span the index does not cover — the type of an expression, or of
    /// a name declared in a bundled library.
    vhdl: Option<Box<Analysis>>,
}

/// The index of the last revision that parsed cleanly.
#[derive(Debug)]
struct Previous {
    text: String,
    index: Index,
}

impl Document {
    /// Opens a document and parses it.
    pub fn new(uri: impl Into<String>, language: Language, version: i64, text: String) -> Document {
        let uri = uri.into();
        // The map is built here as well as in `reparse` so that `source`
        // is always a real id. A document too large for 32-bit spans is
        // held as an empty one: it can still be edited down to size, and
        // until then every feature simply answers nothing.
        let mut map = SourceMap::new();
        let source = map
            .add(uri.clone(), text.clone())
            .or_else(|_| map.add(uri.clone(), String::new()))
            .expect("an empty source fits");
        let mut doc = Document {
            uri,
            language,
            version,
            lines: LineIndex::new(&text),
            text,
            index: Index::default(),
            diagnostics: Vec::new(),
            healthy: false,
            previous: None,
            map,
            source,
            vhdl: None,
        };
        doc.reparse();
        doc
    }

    /// The source map the last parse ran against.
    pub fn source_map(&self) -> &SourceMap {
        &self.map
    }

    /// This document's id inside [`Document::source_map`].
    ///
    /// Spans handed to [`Document::vhdl_analysis`] must carry it.
    pub fn source_id(&self) -> SourceId {
        self.source
    }

    /// The VHDL semantic analysis of the current revision, for a feature
    /// that needs more than the index holds.
    pub fn vhdl_analysis(&self) -> Option<&Analysis> {
        self.vhdl.as_deref()
    }

    /// The span of `range` in this document.
    pub fn span(&self, range: Range) -> Span {
        self.lines.span(&self.text, self.source, range)
    }

    /// The current text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The line index of the current text.
    pub fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// The index of the current parse.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The diagnostics of the current parse.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// True when the current revision parsed and analysed without errors.
    pub fn is_healthy(&self) -> bool {
        self.healthy
    }

    /// The index of the last clean revision and the text it describes,
    /// when the current revision is not clean.
    ///
    /// Features may read it for answers that carry no position, and must
    /// not use its spans as positions in the current document.
    pub fn previous(&self) -> Option<(&str, &Index)> {
        if self.healthy {
            return None;
        }
        self.previous.as_ref().map(|p| (p.text.as_str(), &p.index))
    }

    /// Converts a byte offset to an LSP position in the current text.
    pub fn position(&self, offset: u32) -> Position {
        self.lines.position(&self.text, offset)
    }

    /// Converts an LSP position to a byte offset in the current text.
    pub fn offset(&self, position: Position) -> u32 {
        self.lines.offset(&self.text, position)
    }

    /// Converts a span of the current text to an LSP range.
    pub fn range(&self, span: Span) -> Range {
        self.lines.range(&self.text, span)
    }

    /// The whole document as one range.
    pub fn full_range(&self) -> Range {
        let len = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        Range::new(Position::new(0, 0), self.position(len))
    }

    /// Applies one `textDocument/didChange` content change and reparses.
    ///
    /// `range` is `None` for the full-text form; otherwise the range is
    /// replaced by `new_text`, which is the incremental form.
    pub fn apply_change(&mut self, range: Option<Range>, new_text: &str) {
        match range {
            None => self.text = new_text.to_string(),
            Some(range) => {
                let start = self.offset(range.start);
                let end = self.offset(range.end).max(start);
                let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
                    return;
                };
                let start = start.min(self.text.len());
                let end = end.clamp(start, self.text.len());
                if !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
                    return;
                }
                self.text.replace_range(start..end, new_text);
            }
        }
        self.lines = LineIndex::new(&self.text);
        self.reparse();
    }

    /// Replaces the whole text and reparses, as `didSave` with text does.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.lines = LineIndex::new(&self.text);
        self.reparse();
    }

    /// Parses (and for VHDL analyses) the whole document again.
    pub fn reparse(&mut self) {
        let mut map = SourceMap::new();
        let Ok(id) = map.add(self.uri.clone(), self.text.clone()) else {
            // Only a document of 4 GiB gets here; keep the old index.
            return;
        };
        self.vhdl = None;
        let (index, diags) = match self.language {
            Language::Verilog(dialect) => self.parse_verilog(&mut map, id, dialect),
            Language::Vhdl(standard) => {
                let (index, analysis, diags) = self.parse_vhdl(&mut map, id, standard);
                self.vhdl = Some(Box::new(analysis));
                (index, diags)
            }
        };

        self.healthy = !diags.has_errors();
        self.diagnostics =
            convert_diagnostics(&diags, &map, id, &self.uri, &self.lines, &self.text);
        self.index = index;
        self.map = map;
        self.source = id;
        if self.healthy {
            // Keep this revision as the fallback for the next one that
            // does not parse. A copy per clean keystroke is a few kilobytes
            // and saves having to reason about a half-updated document.
            self.previous = Some(Box::new(Previous {
                text: self.text.clone(),
                index: self.index.clone(),
            }));
        }
    }

    /// Lexes, parses and lints a Verilog document.
    fn parse_verilog(
        &self,
        map: &mut SourceMap,
        id: SourceId,
        dialect: Dialect,
    ) -> (Index, Diagnostics) {
        let mut diags = Diagnostics::new();
        // The preprocessor runs once; the parser then works from its
        // tokens, so a macro-heavy file is not expanded twice.
        let lexed = crate::verilog::lex_source_full(map, id, dialect, &mut NoIncludes, &mut diags);
        let mut parser = crate::verilog::Parser::new(&lexed.tokens, dialect);
        let file = parser.parse_source_file();
        diags.append(&mut parser.take_diagnostics());
        let text = map.file(id).text();
        let index = verilog::index(text, id, &file);
        // Linting a file that does not parse would report on the holes the
        // recovery left, so it waits for a clean parse.
        if !diags.has_errors() {
            crate::verilog::lint::run(
                &LintConfig::new(),
                map,
                id,
                &file,
                &lexed.comments,
                dialect,
                &mut diags,
            );
        }
        (index, diags)
    }

    /// Parses and analyses a VHDL document against the bundled libraries.
    fn parse_vhdl(
        &self,
        map: &mut SourceMap,
        id: SourceId,
        standard: Standard,
    ) -> (Index, Analysis, Diagnostics) {
        let mut diags = Diagnostics::new();
        let analysis = crate::vhdl::analyze_source(map, id, standard, &mut diags);
        let tokens: Vec<(crate::vhdl::TokenKind, Span)> =
            crate::vhdl::lex_source(map, id, standard, &mut Diagnostics::new())
                .iter()
                .map(|token| (token.kind, token.span))
                .collect();
        // The analysis owns the tree it parsed, so there is no second parse.
        let index = match analysis.files.iter().find(|file| file.source == id) {
            Some(file) => vhdl::index(map, id, &analysis, &file.ast, &tokens),
            None => Index::default(),
        };
        (index, analysis, diags)
    }
}

/// The documents the client has open, keyed by URI.
///
/// A `BTreeMap` rather than a hash map so that anything iterating the
/// store — a rename checking other documents, for one — does so in a
/// deterministic order.
#[derive(Debug, Default)]
pub struct DocumentStore {
    documents: BTreeMap<String, Document>,
}

impl DocumentStore {
    /// An empty store.
    pub fn new() -> DocumentStore {
        DocumentStore::default()
    }

    /// Opens (or replaces) a document.
    pub fn open(&mut self, document: Document) {
        self.documents.insert(document.uri.clone(), document);
    }

    /// Closes a document, returning it.
    pub fn close(&mut self, uri: &str) -> Option<Document> {
        self.documents.remove(uri)
    }

    /// The document at `uri`.
    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }

    /// The document at `uri`, mutably.
    pub fn get_mut(&mut self, uri: &str) -> Option<&mut Document> {
        self.documents.get_mut(uri)
    }

    /// Every open document, in URI order.
    pub fn iter(&self) -> impl Iterator<Item = &Document> {
        self.documents.values()
    }

    /// Number of open documents.
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// True when nothing is open.
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

/// The LSP severity for a Reticle one.
fn severity_of(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::Error,
        Severity::Warning => DiagnosticSeverity::Warning,
        Severity::Note => DiagnosticSeverity::Information,
        Severity::Help => DiagnosticSeverity::Hint,
    }
}

/// The lint rule names by their diagnostic code (`L0007` -> `unused-net`).
///
/// The linter puts its stable code on the diagnostic and its kebab-case
/// name in a note; LSP clients show the code and let the user filter on
/// it, so the name — which is what `reticle-lint: off` takes and what the
/// documentation lists — is the better thing to show.
fn lint_names() -> Vec<(&'static str, &'static str)> {
    let set = LintSet::all();
    let mut names: Vec<(&'static str, &'static str)> = set
        .rules()
        .map(|(rule, _)| (rule.id(), rule.name()))
        .collect();
    names.sort_unstable();
    names
}

/// Turns Reticle diagnostics into LSP ones, dropping what belongs to
/// another file and sorting the result.
fn convert_diagnostics(
    diags: &Diagnostics,
    map: &SourceMap,
    source: SourceId,
    uri: &str,
    lines: &LineIndex,
    text: &str,
) -> Vec<Diagnostic> {
    let names = lint_names();
    let mut out: Vec<Diagnostic> = Vec::new();
    for d in diags.iter() {
        if d.severity == Severity::Help {
            // Help diagnostics are only ever attached to another one.
            continue;
        }
        let range = match d.primary_span() {
            Some(span) if span.file == source => lines.range(text, span),
            // A diagnostic about a bundled library, or with no span at all,
            // still concerns this document; report it at its start.
            Some(_) | None => Range::new(Position::new(0, 0), Position::new(0, 0)),
        };
        let code = d.code.map(|code| {
            names
                .iter()
                .find(|(id, _)| *id == code)
                .map_or(code, |(_, name)| *name)
                .to_string()
        });
        let mut message = d.message.clone();
        for note in &d.notes {
            // The linter's own "how to silence me" note repeats the rule
            // name, which the code already carries.
            if note.starts_with("lint: ") {
                continue;
            }
            message.push_str("\nnote: ");
            message.push_str(note);
        }
        let related = d
            .labels
            .iter()
            .filter(|label| !label.primary && label.span.file == source)
            .map(|label| {
                let message = if label.message.is_empty() {
                    map.file(label.span.file).loc(label.span.start).to_string()
                } else {
                    label.message.clone()
                };
                (Location::new(uri, lines.range(text, label.span)), message)
            })
            .collect();
        out.push(Diagnostic {
            range,
            severity: severity_of(d.severity),
            code,
            source: "reticle".to_string(),
            message,
            related,
        });
    }
    out.sort_by(|a, b| {
        (a.range.start, a.range.end, a.severity.code(), &a.message).cmp(&(
            b.range.start,
            b.range.end,
            b.severity.code(),
            &b.message,
        ))
    });
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERILOG: &str = "module m(input a, output y);\n  assign y = a;\nendmodule\n";
    const VHDL: &str = "entity e is port (a : in bit; y : out bit); end entity;\narchitecture r of e is\nbegin\n  y <= a;\nend architecture;\n";

    fn verilog_doc(text: &str) -> Document {
        Document::new(
            "file:///t.v",
            Language::Verilog(Dialect::Verilog2005),
            1,
            text.to_string(),
        )
    }

    fn vhdl_doc(text: &str) -> Document {
        Document::new(
            "file:///t.vhd",
            Language::Vhdl(Standard::Vhdl2008),
            1,
            text.to_string(),
        )
    }

    #[test]
    fn detects_the_language() {
        assert_eq!(
            Language::detect("file:///a.v", ""),
            Some(Language::Verilog(Dialect::Verilog2005))
        );
        assert_eq!(
            Language::detect("file:///a.SV", ""),
            Some(Language::Verilog(Dialect::SystemVerilog))
        );
        assert_eq!(
            Language::detect("file:///a.vhdl", ""),
            Some(Language::Vhdl(Standard::Vhdl2008))
        );
        // The client's identifier wins over the extension.
        assert_eq!(
            Language::detect("file:///a.v", "systemverilog"),
            Some(Language::Verilog(Dialect::SystemVerilog))
        );
        assert_eq!(Language::detect("file:///a.txt", ""), None);
        assert_eq!(Language::detect("file:///noext", ""), None);
        assert!(Language::Verilog(Dialect::Verilog2005).is_verilog());
        assert!(!Language::Vhdl(Standard::Vhdl2008).is_verilog());
    }

    #[test]
    fn parses_both_languages_on_open() {
        let doc = verilog_doc(VERILOG);
        assert!(doc.is_healthy(), "{:?}", doc.diagnostics());
        assert!(doc.index().find("m", &[DeclClass::Module]).is_some());

        let doc = vhdl_doc(VHDL);
        assert!(doc.is_healthy(), "{:?}", doc.diagnostics());
        assert!(doc.index().find("e", &[DeclClass::Entity]).is_some());
    }

    #[test]
    fn reports_parse_errors_as_diagnostics() {
        let doc = verilog_doc("module m(;\nendmodule\n");
        assert!(!doc.is_healthy());
        let diag = &doc.diagnostics()[0];
        assert_eq!(diag.severity, DiagnosticSeverity::Error);
        assert_eq!(diag.source, "reticle");
        assert_eq!(diag.range.start.line, 0);
    }

    #[test]
    fn reports_lint_findings_with_the_rule_name() {
        let doc = verilog_doc("module m;\n  wire unused_one;\nendmodule\n");
        assert!(doc.is_healthy());
        let lint = doc
            .diagnostics()
            .iter()
            .find(|d| d.code.as_deref() == Some("unused-signal"))
            .expect("the unused-signal rule fires");
        assert_eq!(lint.severity, DiagnosticSeverity::Warning);
        // The "disable with" note is dropped: the code already names the
        // rule.
        assert!(!lint.message.contains("reticle-lint"));
    }

    #[test]
    fn reports_vhdl_semantic_errors() {
        let doc = vhdl_doc(
            "entity e is end entity;\narchitecture r of e is\n  signal s : bit;\nbegin\n  s <= no_such_name;\nend architecture;\n",
        );
        assert!(!doc.is_healthy());
        assert!(
            doc.diagnostics()
                .iter()
                .any(|d| d.message.contains("no_such_name")),
            "{:?}",
            doc.diagnostics()
        );
    }

    #[test]
    fn applies_an_incremental_change() {
        let mut doc = verilog_doc(VERILOG);
        // Replace `a` with `b` in the port list and the assignment.
        let range = Range::new(Position::new(1, 13), Position::new(1, 14));
        doc.apply_change(Some(range), "zz");
        assert_eq!(doc.text().lines().nth(1), Some("  assign y = zz;"));
        // Still parses, so the index is current.
        assert!(doc.index().find("zz", &[DeclClass::Net]).is_none());
        assert!(doc.is_healthy());
    }

    #[test]
    fn an_incremental_change_respects_utf16_offsets() {
        let mut doc = verilog_doc("module m;\n  // é comment\n  wire a;\nendmodule\n");
        // `é` is character 5 of line 1 and two bytes wide; replacing from
        // character 6 must land after it, not inside it.
        let range = Range::new(Position::new(1, 6), Position::new(1, 14));
        doc.apply_change(Some(range), "X");
        assert_eq!(doc.text().lines().nth(1), Some("  // éX"));
    }

    #[test]
    fn a_full_text_change_replaces_everything() {
        let mut doc = verilog_doc(VERILOG);
        doc.apply_change(None, "module other; endmodule\n");
        assert!(doc.index().find("other", &[DeclClass::Module]).is_some());
        doc.set_text(VERILOG.to_string());
        assert!(doc.index().find("m", &[DeclClass::Module]).is_some());
    }

    #[test]
    fn the_previous_good_index_survives_a_broken_edit() {
        let mut doc = verilog_doc(VERILOG);
        assert!(doc.previous().is_none());
        // Break the file.
        doc.apply_change(None, "module m(input a, output y);\n  assign y = ;\n");
        assert!(!doc.is_healthy());
        let (text, index) = doc.previous().expect("the last clean parse is kept");
        assert_eq!(text, VERILOG);
        assert!(index.find("m", &[DeclClass::Module]).is_some());
        // Fixing it drops the fallback again.
        doc.apply_change(None, VERILOG);
        assert!(doc.is_healthy());
        assert!(doc.previous().is_none());
    }

    #[test]
    fn positions_and_ranges_use_the_current_text() {
        let doc = verilog_doc(VERILOG);
        assert_eq!(doc.offset(Position::new(1, 2)), 31);
        assert_eq!(doc.position(31), Position::new(1, 2));
        assert_eq!(doc.full_range().start, Position::new(0, 0));
        assert_eq!(doc.full_range().end, Position::new(3, 0));
    }

    #[test]
    fn the_store_keeps_documents_by_uri() {
        let mut store = DocumentStore::new();
        assert!(store.is_empty());
        store.open(verilog_doc(VERILOG));
        store.open(vhdl_doc(VHDL));
        assert_eq!(store.len(), 2);
        assert!(store.get("file:///t.v").is_some());
        assert!(store.get_mut("file:///t.vhd").is_some());
        let uris: Vec<&str> = store.iter().map(|d| d.uri.as_str()).collect();
        assert_eq!(uris, ["file:///t.v", "file:///t.vhd"]);
        assert!(store.close("file:///t.v").is_some());
        assert!(store.close("file:///t.v").is_none());
        assert_eq!(store.len(), 1);
    }
}
